#!/usr/bin/env python3
"""Fail-closed, byte-exact correctness gate over the benchmark registry.

No timings are compared. Every registered variant retains its own macro flags;
reference failures, signals, timeouts and empty selections are failures, never
skips. Only the recursion-only TCE witness is bounded at O0/O1: ten million
unoptimized stack frames are not a meaningful correctness comparison.

Concurrency
-----------
``--jobs N`` runs up to N ``(case, level)`` units at once.  Nothing in this
gate is timed -- every unit compiles the oracle's binary and the candidate's,
runs both, and compares bytes -- so the units are independent by construction
and the verdict is bit-identical to a serial run.  It exists because the gate
took 338 s of hosted CI on run 39686387347, all of it serial; the runner has
four cores and each unit spends nearly all of its wall clock in a child
process.  Diagnostics are ordered by case and level, never by completion, so
two runs print the same log.  ``--list`` prints the complete case set without
touching a compiler.
"""
from __future__ import annotations

import argparse
import concurrent.futures
import os
from pathlib import Path
import re
import shlex
import subprocess
import sys
import tempfile

ROOT = Path(os.environ.get("LCCC_REPO", Path(__file__).resolve().parents[1]))
sys.path.insert(0, str(ROOT))
from tests.benchmark import run_benchmarks as bench


def all_cases():
    # Preserve the old gate's extra .c probes as well as the timed registry.
    # Only registered variants supply additional macro flags; shared sources
    # are not a reason to collapse those distinct compilation requests.
    registered = list(bench.BENCHMARKS)
    names = {item.name for item in registered}
    if not registered or len(names) != len(registered):
        raise ValueError("empty benchmark registry or duplicate registry identifiers")
    used = {bench.source_for(item) for item in registered}
    for source in sorted(bench.PROGRAMS.glob("*.c")):
        if source not in used:
            if source.stem in names:
                raise ValueError("ambiguous benchmark/probe name: " + source.stem)
            registered.append(bench.Benchmark(source.stem, "legacy correctness probe",
                                              (source.name,), ("correctness-probe",)))
            names.add(source.stem)
    return registered


def case_flags(item, level):
    flags = [level, "-fwrapv", *item.extra_flags]
    if item.name == "tce_sum" and level in ("-O0", "-O1"):
        flags.append("-DTCE_N=4096")
    return flags


def command(argv, *, timeout=60):
    result = subprocess.run(argv, capture_output=True, timeout=timeout)
    if result.returncode:
        diagnostic = result.stderr.decode(errors="replace")[:500]
        raise ValueError(f"command exited {result.returncode}: {shlex.join(argv)}\n{diagnostic}")
    return result.stdout


def check_unit(item, level, source, binary_prefix, gcc, lccc, include):
    """One (case, level): compile oracle and candidate, run both, compare bytes.

    Returns None on agreement and a diagnostic string on any disagreement.
    Every failure mode is a string, including a build failure or a signal, so
    the caller counts them exactly like the serial implementation did.
    """
    outputs = []
    try:
        for name, compiler in (("oracle", gcc), ("candidate", lccc)):
            binary = Path(f"{binary_prefix}-{name}")
            binary.unlink(missing_ok=True)
            flags = case_flags(item, level)
            headers = ["-I" + include] if name == "candidate" else []
            command([compiler, *flags, *headers, str(source), "-o", str(binary), "-lm"])
            outputs.append(command([str(binary)], timeout=300))
            binary.unlink(missing_ok=True)
    except (OSError, ValueError, subprocess.TimeoutExpired) as error:
        return f"{item.name} {level}: {error}"
    if outputs[0] != outputs[1]:
        return (f"{item.name} {level}: stdout bytes differ: "
                f"oracle={outputs[0][:200]!r}, candidate={outputs[1][:200]!r}")
    return None


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    parser = argparse.ArgumentParser(
        description="byte-exact benchmark output gate",
        epilog="positional FILTER is a substring of the case name")
    parser.add_argument("filter", nargs="?", default="",
                        help="substring filter on the benchmark name")
    parser.add_argument("--jobs", type=int, default=1, metavar="N",
                        help="run N cases concurrently (default %(default)s; "
                             "the gate compares bytes, never timings)")
    parser.add_argument("--list", action="store_true",
                        help="print every case name and exit (no compiler)")
    args = parser.parse_args(argv)
    if args.jobs < 1:
        raise ValueError(f"--jobs must be >= 1, got {args.jobs}")
    if args.list:
        for item in all_cases() if not args.filter else [
                item for item in all_cases() if args.filter in item.name]:
            print(item.name)
        return 0
    cases = [item for item in all_cases() if args.filter in item.name]
    if not cases:
        raise ValueError(f"no benchmark cases matched filter {args.filter!r}")
    levels = shlex.split(os.environ.get("OPT", "-O0 -O1 -O2 -O3"))
    if not levels or any(level not in ("-O0", "-O1", "-O2", "-O3", "-Os", "-Oz") for level in levels):
        raise ValueError("OPT must contain explicit supported optimization levels")
    gcc = os.environ.get("GCC_BIN", "gcc")
    lccc = os.environ.get("LCCC_BIN", str(ROOT / "target/fastbuild/lccc"))
    include = command([gcc, "-print-file-name=include"]).decode().strip()
    if not include or not Path(include).is_dir():
        raise ValueError(f"invalid builtin header directory from {gcc}: {include!r}")
    # Resolve every source BEFORE any work starts: a missing registered source
    # is a registry defect, not a per-case failure, and must abort the gate.
    sources = {}
    for item in cases:
        source = bench.source_for(item)
        if source is None:
            raise ValueError(f"missing registered benchmark source: {item.name}")
        sources[item.name] = source
    # Ordered (case, level) units: the same list a serial run walks, so the
    # parallel and serial gates cover exactly the same pairs.
    work = [(item, level) for item in cases for level in levels]
    passed = failed = 0
    failures = []
    with tempfile.TemporaryDirectory(prefix="lccc-benchcheck-") as temporary:
        directory = Path(temporary)
        with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
            futures = [
                pool.submit(check_unit, item, level, sources[item.name],
                            directory / f"unit-{index}", gcc, lccc, include)
                for index, (item, level) in enumerate(work)
            ]
            results = [future.result() for future in futures]
    for diagnostic in results:
        if diagnostic is None:
            passed += 1
        else:
            failed += 1
            failures.append(diagnostic)
    for diagnostic in failures:
        print(f"FAIL  {diagnostic}", flush=True)
    scope = f" jobs={args.jobs}" if args.jobs != 1 else ""
    print(f"benchmark output gate: PASS={passed} FAIL={failed} SKIP=0{scope}")
    return int(bool(failed))


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, subprocess.TimeoutExpired) as error:
        print(f"benchmark output gate FAILED: {error}", file=sys.stderr)
        raise SystemExit(1)
