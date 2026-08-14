#!/usr/bin/env python3
"""Rank LCCC's generated code against GCC, Clang, ICC and ICX, per function.

`godbolt.py compare` answers "how does LCCC do on THIS file". That is the right
tool once you know where to look. This one answers the prior question: across a
whole benchmark suite, WHICH functions are furthest behind the best compiler,
and by what margin -- so effort goes where the gap is largest instead of where
it is easiest to look.

For every source file it compiles with LCCC locally and with each oracle over
the Compiler Explorer API, then reports per function:

    insns    instruction count in the function body
    loads    memory reads          (the usual cause of a large gap)
    stores   memory writes
    spills   loads/stores through %rsp or %rbp, i.e. register-allocator churn
    branch   control transfers

Ranking is by `insns` relative to the best oracle, because that is the metric
that correlates with the work actually issued. It is a proxy, not a verdict:
this VM has no PMU, so the runner reports counts and leaves timing to
`run_benchmarks.py`.

Examples
--------
    # Survey the whole benchmark suite, worst gaps first
    scripts/codegen_scoreboard.py tests/benchmark/programs/*.c

    # One file, every function, full detail
    scripts/codegen_scoreboard.py tests/benchmark/programs/matmul.c -v

    # Re-check after a change, comparing to a saved baseline
    scripts/codegen_scoreboard.py --baseline before.json --json after.json ...
"""
from __future__ import annotations

import hashlib
import shlex
import argparse
import concurrent.futures
import json
import os
import re
import subprocess
import sys
from dataclasses import dataclass, field, asdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import godbolt  # noqa: E402  (same directory, deliberate)

DEFAULT_FLAGS = "-O2 -march=x86-64-v3"
ORACLES = ["gcc", "clang", "icc", "icx"]

# Directives and labels carry no instructions.
_SKIP = re.compile(r"^\s*(\.|#|//|$)")
_LABEL = re.compile(r"^[.\w$]+:")

_BRANCH = re.compile(r"^(j\w+|call|ret|loop\w*)$")
_STACK_MEM = re.compile(r"[-\d+x]*\(%r(sp|bp)\)")


@dataclass
class FuncStats:
    name: str
    insns: int = 0
    loads: int = 0
    stores: int = 0
    spills: int = 0
    branch: int = 0
    vector: int = 0

    def as_row(self) -> str:
        return (f"{self.insns:6d} {self.loads:6d} {self.stores:6d} "
                f"{self.spills:6d} {self.branch:6d} {self.vector:6d}")


def parse_functions(lines: list[str]) -> dict[str, FuncStats]:
    """Split an assembly listing into per-function statistics."""
    out: dict[str, FuncStats] = {}
    cur: FuncStats | None = None
    for raw in lines:
        line = raw.rstrip()
        if not line:
            continue
        m = _LABEL.match(line.strip())
        if m and not line.strip().startswith("."):
            cur = FuncStats(line.strip().rstrip(":"))
            out[cur.name] = cur
            continue
        if m and line.strip().startswith(".L"):
            continue
        if _SKIP.match(line):
            continue
        if cur is None:
            continue

        body = line.strip()
        mnem = body.split()[0].lower() if body.split() else ""
        if not mnem or mnem.endswith(":"):
            continue
        cur.insns += 1

        if _BRANCH.match(mnem):
            cur.branch += 1
        if mnem.startswith("v") or re.match(r"^p[a-z]", mnem):
            cur.vector += 1

        # Operand-side memory classification. In AT&T syntax the destination is
        # last, so a memory reference before the final comma is a read and one
        # after it is a write.
        ops = body[len(mnem):].strip()
        if "(" in ops:
            parts = ops.rsplit(",", 1)
            src = parts[0] if len(parts) > 1 else ops
            dst = parts[1] if len(parts) > 1 else ""
            if "(" in src:
                cur.loads += 1
            if "(" in dst:
                cur.stores += 1
            if _STACK_MEM.search(ops):
                cur.spills += 1
    return out


def strip_noise(lines: list[str]) -> list[str]:
    return [l for l in lines if not l.strip().startswith((".cfi", ".file",
                                                          ".ident", ".section",
                                                          ".size", ".type",
                                                          ".globl", ".p2align",
                                                          ".align", ".text",
                                                          ".data", ".bss"))]


# Godbolt compiler ids. scripts/godbolt.py exposes only compile_on_godbolt(),
# so the id table and the local-compile path live here.
ORACLE_IDS = {
    "gcc":   "cg162",
    "clang": "cclang2210",
    "icc":   "cicc2021100",
    "icx":   "cicx202400",
}

CACHE = Path(__file__).resolve().parent.parent / ".godbolt-cache"


def compile_local(lccc: str, path: Path, flags: str) -> list[str]:
    """Compile with the local lccc and return assembly lines."""
    out = path.with_suffix(".local.s")
    cmd = [lccc, *shlex.split(flags), "-S", str(path), "-o", str(out)]
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError((r.stderr or r.stdout).strip().splitlines()[-1:] or "lccc failed")
    try:
        return out.read_text().splitlines()
    finally:
        out.unlink(missing_ok=True)


def compile_remote(name: str, src: str, flags: str) -> list[str]:
    """Compile on godbolt, caching by (compiler, flags, source) digest."""
    cid = ORACLE_IDS.get(name, name)
    key = hashlib.sha256(f"{cid}\0{flags}\0{src}".encode()).hexdigest()[:32]
    CACHE.mkdir(exist_ok=True)
    hit = CACHE / f"{key}.s"
    if hit.exists():
        return hit.read_text().splitlines()
    data = godbolt.compile_on_godbolt(cid, src, flags)
    if data is None:
        raise RuntimeError(f"{name}: compile failed")
    lines = [x.get("text", "") for x in (data.get("asm") or [])]
    hit.write_text("\n".join(lines))
    return lines


@dataclass
class FileResult:
    path: str
    per_compiler: dict[str, dict[str, FuncStats]] = field(default_factory=dict)
    errors: dict[str, str] = field(default_factory=dict)


def measure(path: Path, lccc: str, flags: str, oracles: list[str],
            local_flags: str | None) -> FileResult:
    res = FileResult(str(path))
    src = path.read_text()

    try:
        lines = compile_local(lccc, path, local_flags or flags)
        res.per_compiler["lccc"] = parse_functions(strip_noise(lines))
    except Exception as e:  # noqa: BLE001
        res.errors["lccc"] = f"{type(e).__name__}: {e}"

    def one(name: str):
        try:
            lines = compile_remote(name, src, flags)
            return name, parse_functions(strip_noise(lines)), None
        except Exception as e:  # noqa: BLE001
            return name, {}, f"{type(e).__name__}: {e}"

    with concurrent.futures.ThreadPoolExecutor(max_workers=len(oracles)) as ex:
        for name, stats, err in ex.map(one, oracles):
            if err:
                res.errors[name] = err
            else:
                res.per_compiler[name] = stats
    return res


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("sources", nargs="+", type=Path)
    ap.add_argument("--lccc", default=os.environ.get("LCCC",
                                                     "./target/release/lccc"))
    ap.add_argument("--flags", default=DEFAULT_FLAGS)
    ap.add_argument("--local-flags", default=None,
                    help="flags for LCCC only (default: same as --flags)")
    ap.add_argument("--oracles", default=",".join(ORACLES))
    ap.add_argument("--min-insns", type=int, default=6,
                    help="ignore functions smaller than this")
    ap.add_argument("--json", type=Path)
    ap.add_argument("--baseline", type=Path,
                    help="compare against a previous --json run")
    ap.add_argument("-v", "--verbose", action="store_true")
    args = ap.parse_args()

    oracles = [o for o in args.oracles.split(",") if o]
    results = [measure(p, args.lccc, args.flags, oracles, args.local_flags)
               for p in args.sources]

    baseline = {}
    if args.baseline and args.baseline.exists():
        baseline = json.loads(args.baseline.read_text())

    rows = []
    for r in results:
        lstats = r.per_compiler.get("lccc", {})
        for fname, ls in lstats.items():
            if ls.insns < args.min_insns:
                continue
            best_n, best_v = None, None
            per = {}
            for o in oracles:
                os_ = r.per_compiler.get(o, {}).get(fname)
                if os_ is None:
                    continue
                per[o] = os_
                if best_v is None or os_.insns < best_v:
                    best_n, best_v = o, os_.insns
            if best_v is None:
                continue
            rows.append((ls.insns - best_v, Path(r.path).stem, fname,
                         ls, best_n, per))

    rows.sort(key=lambda t: -t[0])

    print(f"{'gap':>5} {'benchmark':<22} {'function':<22} "
          f"{'insns':>6} {'loads':>6} {'store':>6} {'spill':>6} "
          f"{'brnch':>6} {'vec':>6}   best")
    print("-" * 108)
    total_gap = 0
    for gap, bench, fname, ls, best_n, per in rows:
        if gap <= 0 and not args.verbose:
            continue
        total_gap += max(0, gap)
        best = per[best_n]
        delta = ""
        if baseline:
            key = f"{bench}:{fname}"
            old = baseline.get("gaps", {}).get(key)
            if old is not None:
                d = gap - old
                delta = f"  ({d:+d} vs baseline)" if d else "  (unchanged)"
        print(f"{gap:>5} {bench:<22} {fname:<22} {ls.as_row()}   "
              f"{best_n}={best.insns}{delta}")

    print("-" * 108)
    print(f"total instruction gap vs best-of-oracles: {total_gap}")

    behind = sum(1 for g, *_ in rows if g > 0)
    ahead = sum(1 for g, *_ in rows if g < 0)
    tied = sum(1 for g, *_ in rows if g == 0)
    print(f"functions: {behind} behind, {tied} tied, {ahead} ahead "
          f"({len(rows)} compared)")

    for r in results:
        for who, err in r.errors.items():
            print(f"  ! {Path(r.path).stem}: {who}: {err}", file=sys.stderr)

    if args.json:
        payload = {
            "flags": args.flags,
            "gaps": {f"{b}:{f}": g for g, b, f, *_ in rows},
            "detail": [
                {"file": r.path,
                 "compilers": {c: {k: asdict(v) for k, v in s.items()}
                               for c, s in r.per_compiler.items()},
                 "errors": r.errors}
                for r in results
            ],
        }
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(payload, indent=1))
        print(f"wrote {args.json}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
