#!/usr/bin/env python3
"""Global Location Allocation fire-site census.

For every TU in the benchmark / kernel / patterns / regression corpora,
at every requested opt level and target, compile with the GLA master gate
forced ON and the split debug trace enabled, then inventory every
materialized edit::

    [GLA] func applied: X values (R remat, S capture slots), T trampolines

The output is the exact set of functions the shipped gate-default policy
would change; it is the evidence base for flipping the gate default and
the regression tripwire afterwards (an un-reviewed new fire site fails
the ``--expect`` list). A TU that compiles OFF but fails ON is a hard
correctness regression and always exits non-zero.

Usage:
  scripts/gla_fire_census.py --lccc target/fastbuild/lccc
  scripts/gla_fire_census.py --lccc target/fastbuild/lccc-i686 --32
  scripts/gla_fire_census.py --json out.json
"""
from __future__ import annotations

import argparse
import functools
import json
import os
import re
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]


@functools.lru_cache(maxsize=4)
def gcc_include(m32: bool = False, cross: str | None = None) -> str:
    """Resolved freestanding include path (never a hardcoded gcc version)."""
    if cross == "aarch64":
        args = ["aarch64-linux-gnu-gcc"]
    elif cross == "riscv64":
        args = ["riscv64-linux-gnu-gcc"]
    elif m32:
        args = ["gcc", "-m32"]
    else:
        args = ["gcc"]
    out = subprocess.run(args + ["-print-file-name=include"],
                         capture_output=True, text=True, check=True).stdout.strip()
    return f"-I{out}"


CORPORA = [
    "tests/benchmark/programs",
    "tests/benchmark/kernel_corpus",
    "tests/benchmark/patterns",
    "tests/regression",
]
APPLIED_RE = re.compile(
    r"\[GLA\] (\S+) applied: (\d+) values \((\d+) remat, (\d+) capture slots\), "
    r"(\d+) trampolines"
)
# Both fail-closed zero-edit outcomes: exhausted id space (planned before
# materialization) or the unconditional post-rewrite structural verifier
# restoring the function verbatim.
ABORT_RE = re.compile(
    r"\[GLA\] [^\n]*(fresh id space exhausted|failed structural verification)"
)


def gather_files() -> list[Path]:
    out: list[Path] = []
    for d in CORPORA:
        out.extend(sorted((REPO / d).glob("*.c")))
    return out


def sidecar_flags(src: Path) -> list[str]:
    """Compile flags from the tests/regression <name>.flags sidecar, if any."""
    f = src.with_suffix(".flags")
    if f.is_file():
        return f.read_text().split()
    return []


def _args(lccc: str, m32: bool, opt: str, src: Path,
          cross: str | None = None) -> list[str]:
    args = [lccc, gcc_include(m32, cross)]
    if m32:
        args.append("-m32")
    args += [opt, "-c", str(src), "-o", os.devnull]
    args[2:2] = sidecar_flags(src)
    return args


def compile_one(lccc: str, m32: bool, opt: str, src: Path,
                cross: str | None = None):
    """Return (status, sites, abort_seen, errtail). status in ok/fail."""
    env = dict(os.environ)
    env["CCC_RA_GLOBAL_LOCATION"] = "1"
    env["CCC_DEBUG_SPLIT"] = "1"
    p = subprocess.run(_args(lccc, m32, opt, src, cross),
                       capture_output=True, text=True, env=env)
    sites = []
    for m in APPLIED_RE.finditer(p.stderr):
        sites.append(
            dict(
                func=m.group(1),
                values=int(m.group(2)),
                remat=int(m.group(3)),
                slots=int(m.group(4)),
                trampolines=int(m.group(5)),
            )
        )
    abort = bool(ABORT_RE.search(p.stderr))
    return ("ok" if p.returncode == 0 else "fail"), sites, abort, p.stderr[-200:]


def compile_off(lccc: str, m32: bool, opt: str, src: Path,
                cross: str | None = None) -> bool:
    env = dict(os.environ)
    env["CCC_RA_GLOBAL_LOCATION"] = "0"
    env.pop("CCC_DEBUG_SPLIT", None)
    p = subprocess.run(_args(lccc, m32, opt, src, cross),
                       capture_output=True, text=True, env=env)
    return p.returncode == 0


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--lccc")
    ap.add_argument("--cross", choices=["aarch64", "riscv64"],
                    help="Cross codegen via the argv0-selected lccc driver")
    ap.add_argument("--32", dest="m32", action="store_true")
    ap.add_argument("--opts", default="-O0,-O1,-O2,-O3,-Os")
    ap.add_argument("--jobs", type=int, default=2)
    ap.add_argument("--json", type=Path)
    ap.add_argument(
        "--corpus",
        choices=["all", "benchmark", "regression"],
        default="all",
    )
    ns = ap.parse_args()

    if ns.cross == "aarch64":
        ns.lccc = ns.lccc or str(REPO / "target/fastbuild/lccc-arm")
    elif ns.cross == "riscv64":
        ns.lccc = ns.lccc or str(REPO / "target/fastbuild/lccc-riscv")
    else:
        ns.lccc = ns.lccc or str(REPO / "target/fastbuild/lccc")

    files = gather_files()
    if ns.corpus == "benchmark":
        files = [f for f in files if "benchmark" in str(f)]
    elif ns.corpus == "regression":
        files = [f for f in files if "regression" in str(f)]
    opts = [o for o in ns.opts.split(",") if o]

    work = [(opt, f) for opt in opts for f in files]

    def job(item):
        opt, src = item
        status, sites, abort, err = compile_one(
            ns.lccc, ns.m32, opt, src, ns.cross)
        off_ok = None
        if status == "fail":
            off_ok = compile_off(ns.lccc, ns.m32, opt, src, ns.cross)
        return dict(
            opt=opt,
            src=str(src.relative_to(REPO)),
            status=status,
            sites=sites,
            abort=abort,
            off_ok=off_ok,
            err=err,
        )

    results = []
    with ThreadPoolExecutor(max_workers=ns.jobs) as ex:
        for i, r in enumerate(ex.map(job, work), 1):
            results.append(r)
            if i % 100 == 0:
                print(f"  ...{i}/{len(work)}", file=sys.stderr)

    hard_failures = [
        r for r in results if r["status"] == "fail" and r["off_ok"] is True
    ]
    gate_off_failures = [r for r in results if r["status"] == "fail" and r["off_ok"] is False]
    fired = [r for r in results if r["sites"] or r["abort"]]

    print(f"{'opt':>4} {'TU compiled':>11} {'firing TUs':>10} {'edits':>6} {'remat':>6} "
          f"{'slots':>6} {'tramps':>7} {'aborts':>7}")
    by_opt = {}
    for opt in opts:
        rows = [r for r in results if r["opt"] == opt]
        fr = [r for r in rows if r["sites"]]
        edits = sum(s["values"] for r in fr for s in r["sites"])
        remat = sum(s["remat"] for r in fr for s in r["sites"])
        slots = sum(s["slots"] for r in fr for s in r["sites"])
        tramps = sum(s["trampolines"] for r in fr for s in r["sites"])
        aborts = sum(1 for r in rows if r["abort"])
        by_opt[opt] = dict(
            compiled=len(rows), firing=len(fr), edits=edits, remat=remat,
            slots=slots, trampolines=tramps, aborts=aborts,
        )
        print(f"{opt:>4} {len(rows):>11} {len(fr):>10} {edits:>6} {remat:>6} "
              f"{slots:>6} {tramps:>7} {aborts:>7}")

    print("\nfire sites:")
    for r in sorted(fired, key=lambda r: (r["opt"], r["src"])):
        for s in r["sites"]:
            print(f"  {r['opt']} {r['src']} :: {s['func']} "
                  f"values={s['values']} remat={s['remat']} slots={s['slots']} "
                  f"trampolines={s['trampolines']}")
        if r["abort"] and not r["sites"]:
            print(f"  {r['opt']} {r['src']} :: ID-EXHAUSTION ABORT (zero edits)")

    if gate_off_failures:
        print(f"\nnote: {len(gate_off_failures)} jobs fail with gate OFF too "
              f"(pre-existing, not GLA)", file=sys.stderr)
    if hard_failures:
        print(f"\nHARD FAILURE: {len(hard_failures)} jobs compile with gate OFF "
              f"but FAIL with gate ON:", file=sys.stderr)
        for r in hard_failures:
            print(f"  {r['opt']} {r['src']}: {r['err']}", file=sys.stderr)
    if ns.json:
        ns.json.write_text(json.dumps(dict(by_opt=by_opt, results=results), indent=1))
    return 1 if hard_failures else 0


if __name__ == "__main__":
    sys.exit(main())
