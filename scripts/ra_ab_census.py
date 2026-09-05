#!/usr/bin/env python3
"""A/B a register-allocator configuration by STATIC code-quality census.

`ra_quality_census.py` answers "how does LCCC compare to GCC/Clang". This
script answers the question an RA experiment actually needs: "does flipping
one LCCC knob make LCCC's own output better or worse", with the same binary
on both sides so nothing but the knob differs.

Why static counting rather than wall clock: the reference environment for
this work is a 2-core VM with no hardware PMU, where a paired wall-clock
delta below roughly 3 % is not separable from scheduling noise. The census
buckets are exact, deterministic and reproducible, and they measure the
quantity the allocator actually controls:

    stkref  memory operands through %rsp/%rbp/%esp/%ebp   (spill traffic)
    rrmov   register-to-register moves                    (coalescing misses)
    push    callee-saved pushes                           (register pressure)
    insns   total instructions in the function body

A configuration wins when it removes spill traffic without inflating the
other buckets, and the per-function table shows WHERE it moved so a win can
be attributed instead of assumed.

Usage:
    scripts/ra_ab_census.py --env CCC_EVICT_MODE=6
    scripts/ra_ab_census.py --env CCC_EVICT_MODE=6 --opt -O3 --top 20
    scripts/ra_ab_census.py --env CCC_EVICT_MODE=6 --json results/mode6.json

Exit status is 1 when the experiment regresses the primary bucket, so it can
gate a default flip in CI.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
DEFAULT_DIRS = [
    REPO / "tests" / "benchmark" / "programs",
    REPO / "tests" / "benchmark" / "kernel_corpus",
]
LCCC = os.environ.get("LCCC", str(REPO / "target" / "fastbuild" / "lccc"))

_LABEL = re.compile(r"^([A-Za-z_$][\w.$@]*):")
_DIRECTIVE = re.compile(r"^\s*\.")
_REG_REG_MOV = re.compile(r"^\s*v?mov[lqwbsd]{0,2}\s+%[a-z0-9]+\s*,\s*%[a-z0-9]+\s*$")
_STKREF = re.compile(r"\(%(?:rsp|rbp|esp|ebp)\b")
_PUSH = re.compile(r"^\s*push[lq]?\s+%")
BUCKETS = ("insns", "rrmov", "stkref", "push")


def compile_asm(src: Path, flags: list[str], env_extra: dict[str, str]) -> str | None:
    fd, out = tempfile.mkstemp(suffix=".s")
    os.close(fd)
    env = dict(os.environ)
    env.update(env_extra)
    try:
        p = subprocess.run(
            [LCCC, *flags, "-S", "-o", out, str(src)],
            capture_output=True,
            text=True,
            timeout=300,
            env=env,
        )
        if p.returncode != 0:
            return None
        return Path(out).read_text(errors="replace")
    except (subprocess.TimeoutExpired, OSError):
        return None
    finally:
        try:
            os.unlink(out)
        except OSError:
            pass


def per_function(asm: str) -> dict[str, dict[str, int]]:
    """Bucket counts per function body."""
    out: dict[str, dict[str, int]] = {}
    cur: str | None = None
    for line in asm.splitlines():
        m = _LABEL.match(line)
        if m:
            name = m.group(1)
            if not name.startswith(".L"):
                cur = name
                out.setdefault(cur, dict.fromkeys(BUCKETS, 0))
            continue
        if cur is None or _DIRECTIVE.match(line) or not line.strip():
            continue
        b = out[cur]
        b["insns"] += 1
        if _REG_REG_MOV.match(line):
            b["rrmov"] += 1
        if _STKREF.search(line):
            b["stkref"] += 1
        if _PUSH.match(line):
            b["push"] += 1
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--env", action="append", default=[],
                    help="KEY=VALUE applied to the B side; repeatable")
    ap.add_argument("--opt", default="-O2")
    ap.add_argument("--cflag", action="append", default=[])
    ap.add_argument("--top", type=int, default=15)
    ap.add_argument("--json")
    ap.add_argument("files", nargs="*")
    args = ap.parse_args()

    env_b: dict[str, str] = {}
    for kv in args.env:
        k, _, v = kv.partition("=")
        env_b[k] = v
    if not env_b:
        ap.error("--env is required (nothing to A/B)")

    srcs: list[Path] = [Path(f) for f in args.files]
    if not srcs:
        for d in DEFAULT_DIRS:
            if d.is_dir():
                srcs.extend(sorted(d.glob("*.c")))
    flags = [args.opt, *args.cflag]

    tot_a = dict.fromkeys(BUCKETS, 0)
    tot_b = dict.fromkeys(BUCKETS, 0)
    rows: list[tuple[str, str, dict[str, int], dict[str, int]]] = []
    skipped: list[str] = []

    for src in srcs:
        a_asm = compile_asm(src, flags, {})
        b_asm = compile_asm(src, flags, env_b)
        if a_asm is None or b_asm is None:
            skipped.append(src.name)
            continue
        fa, fb = per_function(a_asm), per_function(b_asm)
        for fn in sorted(set(fa) & set(fb)):
            ba, bb = fa[fn], fb[fn]
            for k in BUCKETS:
                tot_a[k] += ba[k]
                tot_b[k] += bb[k]
            if ba != bb:
                rows.append((src.name, fn, ba, bb))

    rows.sort(key=lambda r: (r[3]["stkref"] - r[2]["stkref"], r[3]["insns"] - r[2]["insns"]))

    label = " ".join(f"{k}={v}" for k, v in env_b.items())
    print(f"# RA A/B census — A: default   B: {label}   flags: {' '.join(flags)}")
    print(f"# sources: {len(srcs)}  changed functions: {len(rows)}"
          + (f"  skipped: {len(skipped)}" if skipped else ""))
    print()
    if rows:
        print(f"{'file':<28}{'function':<30}" + "".join(f"{k:>10}" for k in BUCKETS))
        head = rows[: args.top]
        tail = rows[-args.top:] if len(rows) > args.top else []
        for group, title in ((head, "best"), (tail, "worst")):
            if not group:
                continue
            print(f"-- {title} --")
            for fname, fn, ba, bb in group:
                deltas = "".join(f"{bb[k] - ba[k]:>+10}" for k in BUCKETS)
                print(f"{fname:<28}{fn:<30}{deltas}")
        print()

    # Hot/cold split. A whole-corpus total silently weights a benchmark's
    # cold `main` (setup, timing, printing) the same as its hot kernel, and
    # those two can move in OPPOSITE directions — the first measurement of
    # CCC_EVICT_MODE=6 showed -26 stkref across driver functions and +17
    # across kernels, i.e. an aggregate "win" that was a loss everywhere
    # performance actually lives. Report both, and let the KERNEL total
    # decide the verdict.
    ka = dict.fromkeys(BUCKETS, 0)
    kb = dict.fromkeys(BUCKETS, 0)
    da = dict.fromkeys(BUCKETS, 0)
    db = dict.fromkeys(BUCKETS, 0)
    for _f, fn, ba, bb in rows:
        ta, tb = (da, db) if fn == "main" else (ka, kb)
        for k in BUCKETS:
            ta[k] += ba[k]
            tb[k] += bb[k]
    print(f"{'CHANGED FUNCTIONS ONLY':<58}" + "".join(f"{k:>10}" for k in BUCKETS))
    print(f"{'  driver (main) delta':<58}"
          + "".join(f"{db[k] - da[k]:>+10}" for k in BUCKETS))
    print(f"{'  kernel (non-main) delta':<58}"
          + "".join(f"{kb[k] - ka[k]:>+10}" for k in BUCKETS))
    print()
    print(f"{'TOTAL':<58}" + "".join(f"{k:>10}" for k in BUCKETS))
    print(f"{'  A (default)':<58}" + "".join(f"{tot_a[k]:>10}" for k in BUCKETS))
    print(f"{'  B (' + label + ')':<58}" + "".join(f"{tot_b[k]:>10}" for k in BUCKETS))
    print(f"{'  delta':<58}" + "".join(f"{tot_b[k] - tot_a[k]:>+10}" for k in BUCKETS))
    pct = []
    for k in BUCKETS:
        pct.append(0.0 if tot_a[k] == 0 else 100.0 * (tot_b[k] - tot_a[k]) / tot_a[k])
    print(f"{'  delta %':<58}" + "".join(f"{p:>+10.2f}" for p in pct))

    if args.json:
        Path(args.json).parent.mkdir(parents=True, exist_ok=True)
        Path(args.json).write_text(json.dumps({
            "env": env_b, "flags": flags,
            "total_a": tot_a, "total_b": tot_b,
            "skipped": skipped,
            "functions": [{"file": f, "fn": n, "a": a, "b": b} for f, n, a, b in rows],
        }, indent=2))

    # The verdict is decided by HOT code: spill traffic in non-`main`
    # functions, with instruction count as the tiebreak. A configuration that
    # only improves driver functions has not improved anything that runs.
    k_stk = kb["stkref"] - ka["stkref"]
    k_ins = kb["insns"] - ka["insns"]
    regressed = k_stk > 0 or (k_stk == 0 and k_ins > 0)
    print()
    print(f"VERDICT (kernel functions): "
          f"{'REGRESSION' if regressed else 'no regression'}"
          f"  [stkref {k_stk:+d}, insns {k_ins:+d}]")
    return 1 if regressed else 0


if __name__ == "__main__":
    sys.exit(main())
