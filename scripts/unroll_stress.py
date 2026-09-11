#!/usr/bin/env python3
"""Exhaustive differential stress test for LCCC's loop unroller.

Hardware counters are unavailable on the research VM, so correctness evidence
for `src/passes/loop_unroll.rs` has to come from *coverage of the input space*
rather than from a handful of hand-written loops.  This tool enumerates the
counted-loop shape space the unroller reasons about and checks every point
three ways:

    lccc -O3                               (unroller ON)
    lccc -O3 + CCC_DISABLE_PASSES=unroll   (unroller OFF, same compiler)
    gcc  -O0                               (independent reference)

Any divergence is a miscompile.  ON-vs-OFF isolates the unroller from the rest
of the pipeline; the GCC arm catches shared frontend bugs.

The enumerated dimensions are exactly the ones `loop_unroll.rs` pattern-matches
on, so a bug in any branch of its analysis has a generated witness:

  * IV type            : int, unsigned, long, unsigned long, short,
                         unsigned short, signed char, unsigned char
  * compare op         : <  <=  >  >=  !=
  * operand order      : `i OP lim` and `lim OP i` (iv_is_lhs)
  * branch polarity    : `for(;c;)`, `for(;!(nc);)`, `for(;;){if(nc)break;}`
  * stride             : +1 +2 +3 +7 -1 -2 -3
  * init / limit       : trip counts 0..17 plus signed/unsigned boundary pairs
                         (0xFFFFFFFC vs 4, INT_MIN neighbourhood, ...) whose
                         signed and unsigned orderings disagree
  * limit kind         : compile-time constant (complete unroll) or a runtime
                         parameter (partial unroll with intermediate exits)
  * body kind          : reduction, array store, multiplicative hash, guarded
                         accumulate (diamond), two accumulators, pointer walk,
                         early `break`, `continue`, nested fixed-trip loop,
                         IV live-out (`return i`), float accumulate

Every configuration is first run through a small C-semantics emulator that
rejects loops that would invoke UB (signed overflow) or fail to terminate
within a bound, so every generated program has a single well-defined answer.

Usage (from the repo root, after a fastbuild)::

    scripts/unroll_stress.py --lccc target/fastbuild/lccc
    scripts/unroll_stress.py --lccc target/fastbuild/lccc --limit 400 --seed 7
    scripts/unroll_stress.py --lccc target/fastbuild/lccc --keep results/unroll-fail

Exit status is 0 only when every configuration agrees across all arms.
"""
from __future__ import annotations

import argparse
import itertools
import os
import random
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, List, Optional, Sequence, Tuple

# ── Type model ────────────────────────────────────────────────────────────────

@dataclass(frozen=True)
class CType:
    name: str
    bits: int
    signed: bool
    suffix: str  # literal suffix for wide types

    @property
    def lo(self) -> int:
        return -(1 << (self.bits - 1)) if self.signed else 0

    @property
    def hi(self) -> int:
        return (1 << (self.bits - 1)) - 1 if self.signed else (1 << self.bits) - 1

    def wrap(self, v: int) -> int:
        m = 1 << self.bits
        v %= m
        if self.signed and v > self.hi:
            v -= m
        return v

    def literal(self, v: int) -> str:
        """Spell `v` (a representable value of this type) as a C expression of
        exactly this type, avoiding `-2147483648`-style negated literals."""
        if v == self.lo and self.signed and self.bits >= 32:
            return f"(({self.name})({self.lo + 1}{self.suffix} - 1))"
        if self.bits < 32:
            return f"(({self.name}){v})"
        return f"(({self.name}){v}{self.suffix})"


TYPES = [
    CType("int", 32, True, ""),
    CType("unsigned", 32, False, "u"),
    CType("long", 64, True, "l"),
    CType("unsigned long", 64, False, "ul"),
    CType("short", 16, True, ""),
    CType("unsigned short", 16, False, ""),
    CType("signed char", 8, True, ""),
    CType("unsigned char", 8, False, ""),
]

OPS = ["<", "<=", ">", ">=", "!="]
NEG = {"<": ">=", "<=": ">", ">": "<=", ">=": "<", "!=": "=="}
STEPS = [1, 2, 3, 7, -1, -2, -3]
POLARITIES = ["plain", "notneg", "break"]
LIMIT_KINDS = ["const", "runtime"]
BODIES = [
    "sum", "store", "hash", "guard", "twoacc", "ptrwalk",
    "early", "continue", "nested", "liveout", "fsum",
    # Inner loops whose init/limit is an affine function of the outer IV:
    # after the outer loop is completely unrolled these become constant
    # expressions that `resolve_const_operand` must evaluate in the IV's
    # own width and signedness.
    "nestedj", "nestedjd", "nestedlim",
]

MAX_TRIP = 4096  # emulation bound: anything longer is "does not terminate"


# ── Emulation ─────────────────────────────────────────────────────────────────

def cmp(op: str, a: int, b: int) -> bool:
    return {
        "<": a < b, "<=": a <= b, ">": a > b, ">=": a >= b,
        "!=": a != b, "==": a == b,
    }[op]


def emulate_trip(ty: CType, init: int, limit: int, op: str, step: int) -> Optional[int]:
    """Number of body executions, or None if UB / non-terminating.

    Both operands are spelled as values of `ty`, so after the usual arithmetic
    conversions the comparison is numerically the comparison of the
    represented values for every type (narrow types promote to int, which
    preserves value).  `i += step` is performed in int for narrow types and
    converted back (wrapping, well defined) and in `ty` for wide types (UB on
    signed overflow -> rejected)."""
    i = init
    trip = 0
    while cmp(op, i, limit):
        trip += 1
        if trip > MAX_TRIP:
            return None
        nxt = i + step
        if ty.bits >= 32 and ty.signed and not (ty.lo <= nxt <= ty.hi):
            return None  # signed overflow: UB
        i = ty.wrap(nxt)
    return trip


# ── Program generation ────────────────────────────────────────────────────────

@dataclass(frozen=True)
class Config:
    ty: CType
    op: str
    iv_lhs: bool
    polarity: str
    step: int
    init: int
    limit: int
    limit_kind: str
    body: str

    def ident(self, n: int) -> str:
        return f"f{n:04d}"

    def describe(self) -> str:
        return (f"{self.ty.name} i={self.init} {'i' if self.iv_lhs else 'lim'} "
                f"{self.op} {'lim' if self.iv_lhs else 'i'}={self.limit} "
                f"step={self.step:+d} pol={self.polarity} lim={self.limit_kind} "
                f"body={self.body}")


def cond_expr(cfg: Config, op: str) -> str:
    lim = "lim" if cfg.limit_kind == "runtime" else cfg.ty.literal(cfg.limit)
    if cfg.iv_lhs:
        return f"(i {op} {lim})"
    mirror = {"<": ">", "<=": ">=", ">": "<", ">=": "<=", "!=": "!=", "==": "=="}[op]
    return f"({lim} {mirror} i)"


def body_code(cfg: Config) -> Tuple[str, str, str]:
    """(pre-loop decls, loop body, post-loop expression producing u64)."""
    t = cfg.ty.name
    if cfg.body == "sum":
        return ("unsigned long long acc = 0;",
                "acc += (unsigned long long)i;",
                "acc")
    if cfg.body == "store":
        return ("unsigned long long acc = 0; unsigned buf[64]; unsigned k = 0;"
                " for (k = 0; k < 64; k++) buf[k] = 0; k = 0;",
                "buf[k & 63] = (unsigned)i * 2654435761u; k++;",
                "({ unsigned q; for (q = 0; q < 64; q++) acc = acc * 31 + buf[q]; acc; })")
    if cfg.body == "hash":
        return ("unsigned long long acc = 0x9E3779B97F4A7C15ull;",
                "acc ^= (unsigned long long)i; acc *= 0x100000001B3ull;",
                "acc")
    if cfg.body == "guard":
        return ("unsigned long long acc = 0, odd = 0;",
                "if ((i & 1) != 0) odd += (unsigned long long)i; else acc += 3;",
                "(acc * 1000003ull + odd)")
    if cfg.body == "twoacc":
        return ("unsigned long long a0 = 1, a1 = 2;",
                "a0 += (unsigned long long)i * 3; a1 ^= a0;",
                "(a0 * 65599ull + a1)")
    if cfg.body == "ptrwalk":
        return ("unsigned long long acc = 0; static unsigned tbl[4096 + 64];"
                " const unsigned *p = tbl; { unsigned q; for (q = 0; q < 4096 + 64; q++)"
                " tbl[q] = q * 7u; }",
                "acc += *p++;",
                "(acc + (unsigned long long)(p - tbl))")
    if cfg.body == "early":
        return ("unsigned long long acc = 0; unsigned n = 0;",
                "acc += (unsigned long long)i; if (++n == 5) break;",
                "(acc * 7 + n)")
    if cfg.body == "continue":
        return ("unsigned long long acc = 0;",
                "if ((i & 2) != 0) continue; acc += (unsigned long long)i + 1;",
                "acc")
    if cfg.body == "nested":
        return ("unsigned long long acc = 0;",
                "{ int j; for (j = 0; j < 3; j++) acc += (unsigned long long)i * (j + 1); }",
                "acc")
    if cfg.body == "liveout":
        return ("unsigned long long acc = 0;",
                "acc += 1;",
                f"(acc * 4096 + (unsigned long long)({t})i)")
    if cfg.body == "nestedj":
        return ("unsigned long long acc = 0;",
                f"{{ {t} j; for (j = ({t})(i + 1); j < ({t})6; j++) acc += (unsigned long long)j * 3u; }}",
                "acc")
    if cfg.body == "nestedjd":
        return ("unsigned long long acc = 0;",
                f"{{ {t} j; for (j = ({t})(i + 1); j < ({t})6; j++)"
                " { if (j & 1) acc += (unsigned long long)j; else acc ^= (unsigned long long)j; } }",
                "acc")
    if cfg.body == "nestedlim":
        return ("unsigned long long acc = 0;",
                f"{{ {t} j; for (j = ({t})2; j < ({t})(i + 4); j++)"
                " { if (j & 1) acc += (unsigned long long)j; else acc ^= (unsigned long long)j; } }",
                "acc")
    if cfg.body == "fsum":
        return ("double acc = 0.0;",
                "acc += (double)i * 0.5;",
                "(unsigned long long)(long long)(acc * 4.0)")
    raise ValueError(cfg.body)


def gen_function(n: int, cfg: Config) -> str:
    t = cfg.ty.name
    pre, body, post = body_code(cfg)
    param = f"{t} lim" if cfg.limit_kind == "runtime" else "int unused"
    lines = [f"__attribute__((noinline)) unsigned long long {cfg.ident(n)}({param}) {{"]
    if cfg.limit_kind != "runtime":
        lines.append("    (void)unused;")
    lines.append(f"    {pre}")
    lines.append(f"    {t} i;")
    step = f"i += {cfg.step}" if cfg.step > 0 else f"i -= {-cfg.step}"
    init = cfg.ty.literal(cfg.init)
    if cfg.polarity == "plain":
        lines.append(f"    for (i = {init}; {cond_expr(cfg, cfg.op)}; {step}) {{")
        lines.append(f"        {body}")
        lines.append("    }")
    elif cfg.polarity == "notneg":
        lines.append(f"    for (i = {init}; !{cond_expr(cfg, NEG[cfg.op])}; {step}) {{")
        lines.append(f"        {body}")
        lines.append("    }")
    else:  # break
        lines.append(f"    for (i = {init}; ; {step}) {{")
        lines.append(f"        if {cond_expr(cfg, NEG[cfg.op])} break;")
        lines.append(f"        {body}")
        lines.append("    }")
    lines.append(f"    return {post};")
    lines.append("}")
    return "\n".join(lines)


def gen_program(configs: Sequence[Config]) -> str:
    out = ["#include <stdio.h>", ""]
    for n, cfg in enumerate(configs):
        out.append(f"/* {cfg.describe()} */")
        out.append(gen_function(n, cfg))
        out.append("")
    out.append("int main(void) {")
    out.append("    volatile int zero = 0; (void)zero;")
    for n, cfg in enumerate(configs):
        if cfg.limit_kind == "runtime":
            lit = cfg.ty.literal(cfg.limit)
            arg = f"({cfg.ty.name})({lit} + zero)"
        else:
            arg = "zero"
        out.append(f'    printf("{cfg.ident(n)} %llu\\n", {cfg.ident(n)}({arg}));')
    out.append("    return 0;")
    out.append("}")
    return "\n".join(out) + "\n"


# ── Configuration space ───────────────────────────────────────────────────────

def init_limit_pairs(ty: CType, op: str, step: int) -> Iterable[Tuple[int, int]]:
    """Constant pairs giving trips 0..17 plus boundary pairs whose signed and
    unsigned orderings differ (the class of bug a signed-only trip-count
    computation cannot see)."""
    pairs = set()
    ascending = step > 0
    for base in (0, 5, -7 if ty.signed else 3):
        for trip in (0, 1, 2, 3, 4, 5, 7, 8, 9, 15, 16, 17):
            span = trip * step
            if op in ("<", ">", "!="):
                lim = base + span
            else:  # <=, >=
                lim = base + span - (1 if ascending else -1) if trip else base - step
            if ty.lo <= base <= ty.hi and ty.lo <= lim <= ty.hi:
                pairs.add((base, lim))
    top = ty.hi
    for a, b in ((top - 3, 4), (4, top - 3), (top - 8, top), (top, top - 8),
                 (ty.lo, ty.lo + 6), (ty.lo + 6, ty.lo), (top - 1, top),
                 (0, top), (top, 0)):
        if ty.lo <= a <= ty.hi and ty.lo <= b <= ty.hi:
            pairs.add((a, b))
    return sorted(pairs)


def enumerate_configs(rng: random.Random, limit: Optional[int]) -> List[Config]:
    """Covering design over the shape space.

    The full cross product is ~10^6 functions -- far beyond what three
    compiler arms can process in a session -- so the design is layered:

      core      every (type, op, order, polarity, step, limit-kind) tuple
                appears, each with `--per-tuple` seeded (init, limit, body)
                draws, so every branch of the unroller's analysis is hit for
                every type;
      boundary  every (type, op, step, order) with EVERY boundary pair and
                the nested/IV-dependent bodies, because those pairs are where
                signed-vs-unsigned reasoning diverges;
      nested    every (type, op, step) with the three nested bodies on small
                constant trips, both operand orders, constant limits.

    Configurations that the emulator classifies as UB or non-terminating are
    dropped before generation, so every emitted loop has one defined answer.
    """
    cfgs: set = set()
    for ty, op, iv_lhs, pol, step, lk in itertools.product(
            TYPES, OPS, (True, False), POLARITIES, STEPS, LIMIT_KINDS):
        pairs = [p for p in init_limit_pairs(ty, op, step)
                 if emulate_trip(ty, p[0], p[1], op, step) is not None]
        if not pairs:
            continue
        for _ in range(PER_TUPLE):
            init, lim = rng.choice(pairs)
            cfgs.add(Config(ty, op, iv_lhs, pol, step, init, lim, lk, rng.choice(BODIES)))
    for ty, op, step, iv_lhs in itertools.product(TYPES, OPS, STEPS, (True, False)):
        top = ty.hi
        for init, lim in ((top - 3, 4), (4, top - 3), (top - 8, top), (top, top - 8),
                          (ty.lo, ty.lo + 6), (ty.lo + 6, ty.lo), (top - 1, top),
                          (0, top), (top, 0), (top - 6, top - 1)):
            if not (ty.lo <= init <= ty.hi and ty.lo <= lim <= ty.hi):
                continue
            if emulate_trip(ty, init, lim, op, step) is None:
                continue
            for body in ("sum", "guard", "nestedj", "nestedjd", "nestedlim", "liveout"):
                cfgs.add(Config(ty, op, iv_lhs, "plain", step, init, lim, "const", body))
    for ty, op, step, iv_lhs in itertools.product(TYPES, OPS, STEPS, (True, False)):
        for init, lim in init_limit_pairs(ty, op, step):
            tr = emulate_trip(ty, init, lim, op, step)
            if tr is None or tr > 9:
                continue
            for body in ("nestedj", "nestedjd", "nestedlim"):
                cfgs.add(Config(ty, op, iv_lhs, "plain", step, init, lim, "const", body))
    out = sorted(cfgs, key=lambda c: (c.ty.name, c.op, c.iv_lhs, c.polarity, c.step,
                                     c.init, c.limit, c.limit_kind, c.body))
    rng.shuffle(out)
    if limit is not None:
        out = out[:limit]
    return out


PER_TUPLE = 2


# ── Execution ─────────────────────────────────────────────────────────────────

def run(cmd: Sequence[str], env: Optional[dict] = None, timeout: int = 600) -> subprocess.CompletedProcess:
    return subprocess.run(list(cmd), capture_output=True, text=True, timeout=timeout, env=env)


def compile_and_run(compiler: str, flags: Sequence[str], src: Path, exe: Path,
                    env_extra: Optional[dict] = None) -> Tuple[bool, str]:
    env = dict(os.environ)
    if env_extra:
        env.update(env_extra)
    cp = run([compiler, *flags, str(src), "-o", str(exe)], env=env)
    if cp.returncode != 0:
        return False, f"COMPILE FAILED ({compiler}):\n{cp.stderr[-4000:]}"
    try:
        rp = run([str(exe)], timeout=60)
    except subprocess.TimeoutExpired:
        return False, f"RUN TIMEOUT ({compiler})"
    if rp.returncode != 0:
        return False, f"RUN FAILED ({compiler}) rc={rp.returncode}\n{rp.stderr[-2000:]}"
    return True, rp.stdout


def parse_results(text: str) -> dict:
    res = {}
    for line in text.splitlines():
        parts = line.split()
        if len(parts) == 2:
            res[parts[0]] = parts[1]
    return res


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--lccc", required=True, help="path to the lccc driver under test")
    ap.add_argument("--gcc", default="gcc", help="reference compiler (default: gcc)")
    ap.add_argument("--lccc-flags", default="-O3", help="flags for the lccc arms (default: -O3)")
    ap.add_argument("--batch", type=int, default=48, help="functions per translation unit")
    ap.add_argument("--limit", type=int, default=None, help="cap on configurations (default: all)")
    ap.add_argument("--seed", type=int, default=1, help="shuffle seed for --limit sampling")
    ap.add_argument("--keep", default=None, help="directory to keep failing programs in")
    ap.add_argument("--no-gcc", action="store_true", help="skip the independent reference arm")
    ap.add_argument("--quiet", action="store_true")
    args = ap.parse_args()

    rng = random.Random(args.seed)
    cfgs = enumerate_configs(rng, args.limit)
    print(f"[unroll_stress] {len(cfgs)} configurations, batch={args.batch}")

    lccc_flags = args.lccc_flags.split()
    failures = 0
    checked = 0
    tmp = Path(tempfile.mkdtemp(prefix="unroll_stress."))
    keep = Path(args.keep) if args.keep else None
    if keep:
        keep.mkdir(parents=True, exist_ok=True)
    try:
        for bi in range(0, len(cfgs), args.batch):
            batch = cfgs[bi:bi + args.batch]
            src = tmp / f"b{bi // args.batch:04d}.c"
            src.write_text(gen_program(batch))
            arms = {
                "lccc-on": (args.lccc, lccc_flags, None),
                "lccc-off": (args.lccc, lccc_flags, {"CCC_DISABLE_PASSES": "unroll"}),
            }
            if not args.no_gcc:
                arms["gcc-O0"] = (args.gcc, ["-O0", "-w"], None)
            outputs = {}
            bad = []
            for name, (cc, fl, env) in arms.items():
                ok, out = compile_and_run(cc, fl, src, tmp / f"{src.stem}.{name}", env)
                if not ok:
                    bad.append(f"[{name}] {out}")
                    continue
                outputs[name] = parse_results(out)
            mism = []
            if "lccc-on" in outputs:
                for n, cfg in enumerate(batch):
                    key = cfg.ident(n)
                    vals = {a: o.get(key, "<missing>") for a, o in outputs.items()}
                    if len(set(vals.values())) > 1:
                        mism.append((cfg, vals))
            checked += len(batch)
            if bad or mism:
                failures += len(mism) + len(bad)
                print(f"\n=== FAIL batch {src.name} ===")
                for b in bad:
                    print(b)
                for cfg, vals in mism:
                    print(f"  MISMATCH {cfg.describe()}")
                    for a, v in vals.items():
                        print(f"      {a:9s} {v}")
                if keep:
                    shutil.copy(src, keep / src.name)
            elif not args.quiet:
                print(f"  ok  {src.name}  ({checked}/{len(cfgs)})", flush=True)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)

    print(f"\n[unroll_stress] checked={checked} failures={failures}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
