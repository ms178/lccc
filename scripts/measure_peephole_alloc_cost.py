#!/usr/bin/env python3
"""Allocation-cost A/B for the peephole, with byte-identical output as a hard gate.

MB-20's acceptance rule is that a compile-cost change must not alter generated
code. This harness therefore refuses to report a speedup unless both arms emit
byte-identical assembly for every input, and refuses to trust a count unless it
reproduces exactly on a repeat run.

The proxy is the number of heap allocations performed by the compiler, counted
by interposing glibc's allocator (tests/regression/alloc_cost/alloc_count_shim.c). That is
the quantity an "allocate before filtering" change removes, it needs no
profiler, and it is deterministic -- unlike wall time on a shared host.

Three traps are handled here rather than left to the caller; see the shim's
header for how each was discovered:
  * worker threads are counted (global atomics, not thread-local counters);
  * child processes are counted (per-pid reports, summed);
  * both arms get an IDENTICAL $ALLOC_COUNT_OUT path, because std::env copies
    the environment and a longer path perturbs the byte total.

Usage:
  python3 scripts/measure_peephole_alloc_cost.py \
      --before /path/to/baseline-lccc --after target/fastbuild/lccc \
      --out /tmp/alloc-cost [--glob 'tests/bench/k_*.c'] [--repeats 2]

Exit status is non-zero if assembly differs, if a count is not reproducible,
if a compile fails, or if the allocation count grows beyond --tolerance.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
# NOTE: the shim lives in a SUBDIRECTORY on purpose. tests/regression/*.c is
# globbed non-recursively by the SSA corpus runner and every match is compiled
# as a standalone program; a shared-library interposer has no main() and calls
# __libc_* internals, so placing it at the top level broke that corpus.
SHIM_SRC = ROOT / "tests/regression/alloc_cost/alloc_count_shim.c"
FIELDS = ("malloc", "calloc", "realloc", "free", "aligned", "bytes", "total")


def build_shim(cc: str, dest: Path) -> Path:
    """Compile the interposer. -Wall -Wextra -Werror: the shim must be clean."""
    so = dest / "alloc_count_shim.so"
    subprocess.run(
        [cc, "-O2", "-Wall", "-Wextra", "-Werror", "-shared", "-fPIC",
         "-o", str(so), str(SHIM_SRC)],
        check=True, capture_output=True, text=True,
    )
    return so


def parse_reports(prefix: Path) -> dict[str, int]:
    """Sum every per-pid report written for this run.

    Reports are removed before the run, so anything present belongs to it.
    A missing report means the interposer never ran -- that is an error, not
    a zero, because a zero would silently look like a perfect optimization.
    """
    reports = sorted(prefix.parent.glob(prefix.name + ".*"))
    if not reports:
        raise RuntimeError(f"no allocation report produced for {prefix}")
    total = dict.fromkeys(FIELDS, 0)
    for rep in reports:
        seen = set()
        for token in rep.read_text().split():
            key, _, value = token.partition("=")
            if key not in FIELDS:
                raise RuntimeError(f"unknown field {key!r} in {rep}")
            seen.add(key)
            total[key] += int(value)
        if seen != set(FIELDS):
            raise RuntimeError(f"incomplete report {rep}: missing {set(FIELDS) - seen}")
    return total


def run_once(binary: Path, source: Path, out_s: Path, shim: Path,
             report_prefix: Path, opts: list[str], env_extra: dict[str, str]) -> dict[str, int]:
    for stale in report_prefix.parent.glob(report_prefix.name + ".*"):
        stale.unlink()
    if out_s.exists():
        out_s.unlink()
    env = dict(os.environ)
    env.update(env_extra)
    env["LD_PRELOAD"] = str(shim)
    env["ALLOC_COUNT_OUT"] = str(report_prefix)
    proc = subprocess.run(
        [str(binary), *opts, str(source), "-o", str(out_s)],
        capture_output=True, text=True, env=env,
    )
    if proc.returncode != 0:
        raise RuntimeError(
            f"{binary.name} failed on {source.name} (rc={proc.returncode}):\n{proc.stderr[:2000]}"
        )
    if not out_s.exists():
        raise RuntimeError(f"{binary.name} produced no assembly for {source.name}")
    return parse_reports(report_prefix)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--before", required=True, type=Path)
    ap.add_argument("--after", required=True, type=Path)
    ap.add_argument("--out", required=True, type=Path)
    ap.add_argument("--glob", default="tests/bench/k_*.c")
    ap.add_argument("--opts", default="-O1 -S",
                    help="compile flags; must include -S (this harness compares assembly)")
    ap.add_argument("--repeats", type=int, default=2,
                    help="runs per (input, arm); counts must match exactly (default 2)")
    ap.add_argument("--tolerance", type=float, default=0.0,
                    help="allowed fractional growth in total allocations (default 0)")
    ap.add_argument("--cc", default=os.environ.get("CC", "cc"))
    ap.add_argument("--require-improvement", action="store_true",
                    help="fail unless total allocations strictly decrease")
    args = ap.parse_args()

    if args.repeats < 2:
        print("error: --repeats must be >= 2; an unrepeated count is not evidence", file=sys.stderr)
        return 2
    for binary in (args.before, args.after):
        if not binary.is_file():
            print(f"error: not a file: {binary}", file=sys.stderr)
            return 2
    sources = sorted(ROOT.glob(args.glob))
    if not sources:
        print(f"error: no inputs match {args.glob}", file=sys.stderr)
        return 2

    args.out.mkdir(parents=True, exist_ok=True)
    shim = build_shim(args.cc, args.out)
    opts = args.opts.split()
    if "-S" not in opts:
        print("error: --opts must include -S; this harness compares assembly, "
              "and linking would measure the linker instead", file=sys.stderr)
        return 2

    # Identical report path for both arms: the byte total depends on the
    # environment's length, so differing paths would bias the comparison.
    report_prefix = args.out / "alloc-report"
    rows = []
    failures = []
    with tempfile.TemporaryDirectory(dir=args.out) as tmp:
        tmp = Path(tmp)
        for source in sources:
            row = {"input": str(source.relative_to(ROOT))}
            digests = {}
            for arm, binary in (("before", args.before), ("after", args.after)):
                out_s = tmp / f"{source.stem}.{arm}.s"
                counts = None
                for rep in range(args.repeats):
                    got = run_once(binary, source, out_s, shim, report_prefix, opts, {})
                    if counts is None:
                        counts = got
                    elif got != counts:
                        failures.append(
                            f"{source.name} [{arm}]: allocation count is not reproducible "
                            f"({counts['total']} then {got['total']}); refusing to report a delta"
                        )
                        break
                if counts is None:
                    continue
                row[arm] = counts
                digests[arm] = sha256(out_s)
                shutil.copyfile(out_s, args.out / f"{source.stem}.{arm}.s")
            if len(digests) == 2:
                row["asm_identical"] = digests["before"] == digests["after"]
                row["asm_sha256"] = digests["before"]
                if not row["asm_identical"]:
                    failures.append(f"{source.name}: assembly DIFFERS between arms")
            if "before" in row and "after" in row:
                b, a = row["before"]["total"], row["after"]["total"]
                row["delta"] = a - b
                row["delta_pct"] = (100.0 * (a - b) / b) if b else 0.0
            rows.append(row)

    complete = [r for r in rows if "before" in r and "after" in r]
    before_total = sum(r["before"]["total"] for r in complete)
    after_total = sum(r["after"]["total"] for r in complete)
    before_bytes = sum(r["before"]["bytes"] for r in complete)
    after_bytes = sum(r["after"]["bytes"] for r in complete)
    delta_pct = (100.0 * (after_total - before_total) / before_total) if before_total else 0.0

    if before_total and after_total > before_total * (1.0 + args.tolerance):
        failures.append(
            f"allocation count grew {delta_pct:+.3f}% (tolerance {args.tolerance * 100:.3f}%)"
        )
    if args.require_improvement and after_total >= before_total:
        failures.append("no improvement: total allocations did not strictly decrease")

    manifest = {
        "inputs": len(sources),
        "compared": len(complete),
        "opts": opts,
        "repeats": args.repeats,
        "before_total_allocs": before_total,
        "after_total_allocs": after_total,
        "delta_allocs": after_total - before_total,
        "delta_pct": delta_pct,
        "before_bytes": before_bytes,
        "after_bytes": after_bytes,
        "asm_all_identical": all(r.get("asm_identical", False) for r in complete) and bool(complete),
        "rows": rows,
        "failures": failures,
        "ok": not failures,
    }
    (args.out / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")

    print(f"inputs={len(complete)}/{len(sources)} opts={' '.join(opts)} repeats={args.repeats}")
    print(f"allocations: {before_total} -> {after_total}  ({delta_pct:+.3f}%)")
    print(f"bytes:       {before_bytes} -> {after_bytes}")
    print(f"assembly byte-identical on every input: {manifest['asm_all_identical']}")
    for failure in failures:
        print(f"FAIL: {failure}", file=sys.stderr)
    print("PASS" if not failures else "FAIL")
    return 0 if not failures else 1


if __name__ == "__main__":
    sys.exit(main())
