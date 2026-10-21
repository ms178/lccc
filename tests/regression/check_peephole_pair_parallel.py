#!/usr/bin/env python3
"""Dual-layout metric pair: kill switch, host fallback and assembly identity.

The parallel path is invisible in its own output *by design*: both scheduling
modes must emit byte-identical assembly, so no assembly-level assertion can
tell them apart. This gate therefore proves the contract from the two things
that are observable — the assembly hash across all three modes, and the
`[pair]` telemetry line that records the mode each pair actually took.

Contracts:

* identity   — `LCCC_PEEPHOLE_PAIR_PARALLEL=0`, unset (heuristic) and `=1`
               produce the same assembly for every source: the split is
               latency-only and can never change generated code;
* kill switch— `=0` never reports `mode=parallel`;
* host gate  — the forced mode reports `mode=parallel` iff this host really
               can run two threads (the compiler's `available_parallelism`
               view, mirrored here from affinity and cgroup CPU quota), and
               never on a single-thread host;
* liveness   — at least one dual-layout pair is measured, so the gate cannot
               pass vacuously on a source set that stopped exploring.

Run: `python3 tests/regression/check_peephole_pair_parallel.py [--compiler C]`.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]

# Sources that measurably explore dual-layout pairs at -O1 (checked by the
# liveness contract below, so a change in layout policy fails loudly instead of
# silently shrinking this gate to a no-op).
SOURCES = [
    "tests/benchmark/programs/expat_siphash24.c",
    "tests/benchmark/programs/zstd_count.c",
    "tests/benchmark/programs/glibc_memcmp.c",
]

MODE = re.compile(r"mode=(\w+)")


def effective_threads():
    """Mirror `std::thread::available_parallelism` on Linux: affinity ∩ quota.

    Python's `os.cpu_count()` reports the machine, not the sandbox, so a
    cgroup-limited CI runner would make the forced-mode contract below flaky.
    """
    try:
        threads = len(os.sched_getaffinity(0))
    except AttributeError:
        threads = os.cpu_count() or 1
    quota = None
    try:  # cgroup v2: "<quota> <period>" or "max <period>"
        fields = (Path("/sys/fs/cgroup/cpu.max").read_text().split())
        if fields[0] != "max":
            quota = int(fields[0]) / int(fields[1])
    except (OSError, ValueError, IndexError):
        pass
    if quota is None:
        try:  # cgroup v1: -1 means unlimited
            raw = int(Path("/sys/fs/cgroup/cpu/cpu.cfs_quota_us").read_text())
            period = int(Path("/sys/fs/cgroup/cpu/cpu.cfs_period_us").read_text())
            if raw > 0 and period > 0:
                quota = raw / period
        except (OSError, ValueError):
            pass
    if quota is not None:
        threads = min(threads, max(1, int(quota)))
    return max(1, threads)


def compile_asm(compiler, source, knob, workdir, tag):
    """Compile one source in one scheduling mode; return (sha256, pair modes)."""
    out = workdir / f"{source.stem}-{tag}.s"
    env = dict(os.environ)
    env.pop("LCCC_PEEPHOLE_PAIR_PARALLEL", None)
    if knob is not None:
        env["LCCC_PEEPHOLE_PAIR_PARALLEL"] = knob
    env["CCC_TIME_PEEPHOLE"] = "1"
    result = subprocess.run(
        [str(compiler), "-O1", "-S", str(source), "-o", str(out)],
        capture_output=True, text=True, env=env,
    )
    if result.returncode != 0:
        raise ValueError(f"{source.name} [{tag}] failed: {result.stderr.strip()[:400]}")
    return hashlib.sha256(out.read_bytes()).hexdigest(), MODE.findall(result.stderr)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--compiler", default=os.environ.get("CCC", str(ROOT / "target/fastbuild/lccc"))
    )
    parser.add_argument("--json", help="write the observed contract table here")
    args = parser.parse_args()

    compiler = Path(args.compiler)
    if not compiler.exists():
        raise ValueError(f"compiler not found: {compiler}")
    sources = [ROOT / name for name in SOURCES]
    missing = [str(s) for s in sources if not s.is_file()]
    if missing:
        raise ValueError(f"missing sources: {missing}")

    threads = effective_threads()
    observed = {}
    seen_pairs = 0
    with tempfile.TemporaryDirectory() as temp:
        workdir = Path(temp)
        for source in sources:
            hashes = {}
            modes = {}
            for tag, knob in (("serial", "0"), ("auto", None), ("forced", "1")):
                hashes[tag], modes[tag] = compile_asm(compiler, source, knob, workdir, tag)
            if len(set(hashes.values())) != 1:
                raise ValueError(
                    f"{source.name}: scheduling mode changed the assembly {hashes}"
                )
            seen_pairs += len(modes["auto"])
            if "parallel" in modes["serial"]:
                raise ValueError(f"{source.name}: kill switch ignored {modes['serial']}")
            if threads > 1 and "serial" in modes["forced"]:
                raise ValueError(
                    f"{source.name}: forced parallel fell back on a {threads}-thread host "
                    f"{modes['forced']}"
                )
            if threads == 1 and "parallel" in modes["forced"]:
                raise ValueError(
                    f"{source.name}: spawned on a single-thread host {modes['forced']}"
                )
            observed[source.name] = {
                "assembly_sha256": hashes["serial"],
                "pairs_auto": modes["auto"],
                "pairs_forced": modes["forced"],
            }

    if seen_pairs == 0:
        raise ValueError(
            "no dual-layout pair was measured at -O1; the liveness contract is void"
        )

    summary = {
        "effective_threads": threads,
        "total_pairs": seen_pairs,
        "sources": observed,
    }
    if args.json:
        Path(args.json).parent.mkdir(parents=True, exist_ok=True)
        Path(args.json).write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n")

    parallel = sum(mode == "parallel" for row in observed.values() for mode in row["pairs_auto"])
    print(
        f"PASS {len(sources)} sources, {seen_pairs} pairs ({parallel} heuristic-parallel), "
        f"threads={threads}, identical assembly across serial/auto/forced"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
