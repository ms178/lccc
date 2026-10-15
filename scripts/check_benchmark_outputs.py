#!/usr/bin/env python3
"""Fail-closed, byte-exact correctness gate over the benchmark registry.

No timings are compared. Every registered variant retains its own macro flags;
reference failures, signals, timeouts and empty selections are failures, never
skips. Only the recursion-only TCE witness is bounded at O0/O1: ten million
unoptimized stack frames are not a meaningful correctness comparison.
"""
from __future__ import annotations

import os
from pathlib import Path
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


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    if len(argv) > 1:
        raise ValueError("usage: check_benchmark_outputs.sh [substring-filter]")
    selection = argv[0] if argv else ""
    cases = [item for item in all_cases() if selection in item.name]
    if not cases:
        raise ValueError(f"no benchmark cases matched filter {selection!r}")
    levels = shlex.split(os.environ.get("OPT", "-O0 -O1 -O2 -O3"))
    if not levels or any(level not in ("-O0", "-O1", "-O2", "-O3", "-Os", "-Oz") for level in levels):
        raise ValueError("OPT must contain explicit supported optimization levels")
    gcc = os.environ.get("GCC_BIN", "gcc")
    lccc = os.environ.get("LCCC_BIN", str(ROOT / "target/fastbuild/lccc"))
    include = command([gcc, "-print-file-name=include"]).decode().strip()
    if not include or not Path(include).is_dir():
        raise ValueError(f"invalid builtin header directory from {gcc}: {include!r}")
    passed = failed = 0
    with tempfile.TemporaryDirectory(prefix="lccc-benchcheck-") as temporary:
        directory = Path(temporary)
        for item in cases:
            source = bench.source_for(item)
            if source is None:
                raise ValueError(f"missing registered benchmark source: {item.name}")
            for level in levels:
                try:
                    outputs = []
                    for name, cc in (("oracle", gcc), ("candidate", lccc)):
                        binary = directory / name
                        binary.unlink(missing_ok=True)
                        flags = case_flags(item, level)
                        headers = ["-I" + include] if name == "candidate" else []
                        command([cc, *flags, *headers, str(source), "-o", str(binary), "-lm"])
                        outputs.append(command([str(binary)], timeout=300))
                    if outputs[0] != outputs[1]:
                        raise ValueError(f"stdout bytes differ: oracle={outputs[0][:200]!r}, candidate={outputs[1][:200]!r}")
                    passed += 1
                except (OSError, ValueError, subprocess.TimeoutExpired) as error:
                    failed += 1
                    print(f"FAIL  {item.name} {level}: {error}", flush=True)
    print(f"benchmark output gate: PASS={passed} FAIL={failed} SKIP=0")
    return int(bool(failed))


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, subprocess.TimeoutExpired) as error:
        print(f"benchmark output gate FAILED: {error}", file=sys.stderr)
        raise SystemExit(1)
