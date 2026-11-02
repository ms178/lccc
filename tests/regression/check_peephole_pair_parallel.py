#!/usr/bin/env python3
"""Dual-layout metric pairs: scheduling contract and reproducible output.

Three properties are checked for **every backend** (x86-64, i686, AArch64,
RISC-V), because all four run a text peephole over both exploration
candidates:

1. *Scheduling* — `LCCC_PEEPHOLE_PAIR_PARALLEL=0`, unset (heuristic) and `=1`
   must emit byte-identical assembly. The split is latency-only, so no
   assembly-level difference is permitted; the only observable proof that the
   concurrent branch ran is the `[pair] … mode=…` telemetry, which also pins
   the kill switch and the single-thread fallback.
2. *Accounting* — the counters on the `[pair]` line must add up to the number
   of pairs the compilation reported. A split that silently stops counting
   would pass the identity check while proving nothing about which path ran.
3. *Reproducibility* — repeated compilation of one source must produce one
   assembly hash. This is what caught RS-1: the RISC-V identical-block pass
   iterated a `std::collections::HashMap`, so its per-process hash seed decided
   `.Lskip_N` labels and block placement, and five serial compilations produced
   five different objects.

The cross backends are driven with a header-free fixture that triggers ten
explorations per compile (`peephole_pair/chain_wide.c`), so the gate needs no
cross sysroot and does not rest on a single lucky pair. The fixture lives in a
subdirectory because `run_regression.py` globs `tests/regression/*.c`
non-recursively and links every one of them as a standalone program; a
header-free fragment library belongs outside that glob, which the runner's
own diagnostic prescribes. x86-64 additionally
compiles real benchmark TUs and repeats the contract at `-O2`, where the rest
of the pipeline changes around the pair.

Run: `python3 tests/regression/check_peephole_pair_parallel.py [--json OUT]`.
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

# The wide fixture replicates three explored block shapes ten times, so every
# backend exercises ten independent pairs per compile rather than the single
# exploration a compact probe happens to trigger. x86-64 additionally runs real
# benchmark TUs, where pairs come from real code rather than a synthetic shape.
WIDE = "tests/regression/peephole_pair/chain_wide.c"

ARCHES = (
    ("x86-64", "", ["tests/benchmark/programs/expat_siphash24.c",
                    "tests/benchmark/programs/glibc_memcmp.c", WIDE]),
    ("i686", "-i686", [WIDE]),
    ("aarch64", "-arm", [WIDE]),
    ("riscv64", "-riscv", [WIDE]),
)

MODE = re.compile(r"mode=(\w+)")
COUNTERS = re.compile(r"parallel=(\d+) serial=(\d+) deduped=(\d+)")


def effective_threads():
    """Mirror `std::thread::available_parallelism` on Linux: affinity ∩ quota.

    `os.cpu_count()` reports the machine, not the sandbox, so a cgroup-limited
    CI runner would otherwise make the forced-mode contract flaky.
    """
    try:
        threads = len(os.sched_getaffinity(0))
    except AttributeError:
        threads = os.cpu_count() or 1
    quota = None
    try:  # cgroup v2: "<quota> <period>" or "max <period>"
        fields = Path("/sys/fs/cgroup/cpu.max").read_text().split()
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


def compile_asm(compiler, source, knob, workdir, tag, opt="-O1"):
    """Compile one source in one scheduling mode.

    Returns `(sha256, modes, counters)` where `modes` holds one entry per pair
    and `counters` holds the last `(parallel, serial, deduped)` triple seen.
    """
    out = workdir / f"{source.stem}-{tag}.s"
    env = dict(os.environ)
    env.pop("LCCC_PEEPHOLE_PAIR_PARALLEL", None)
    if knob is not None:
        env["LCCC_PEEPHOLE_PAIR_PARALLEL"] = knob
    env["CCC_TIME_PEEPHOLE"] = "1"
    result = subprocess.run(
        [str(compiler), opt, "-S", str(source), "-o", str(out)],
        capture_output=True, text=True, env=env,
    )
    if result.returncode != 0:
        raise ValueError(f"{source.name} [{tag}] failed: {result.stderr.strip()[:400]}")
    counters = COUNTERS.findall(result.stderr)
    return (
        hashlib.sha256(out.read_bytes()).hexdigest(),
        MODE.findall(result.stderr),
        tuple(int(value) for value in counters[-1]) if counters else None,
    )


def check_accounting(label, source_name, opt, modes, counters, failures):
    """`parallel + serial + deduped` must equal the number of reported pairs."""
    if not modes:
        return
    if counters is None:
        failures.append(f"{label}/{source_name} {opt}: telemetry without counters")
        return
    if sum(counters) != len(modes):
        failures.append(
            f"{label}/{source_name} {opt}: counters {counters} do not account for "
            f"{len(modes)} pairs"
        )


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

    threads = effective_threads()
    observed = {}
    total_pairs = 0
    failures = []
    with tempfile.TemporaryDirectory() as temp:
        workdir = Path(temp)
        for label, suffix, sources in ARCHES:
            binary = compiler.with_name(compiler.name + suffix)
            if not binary.exists():
                failures.append(f"{label}: missing backend binary {binary.name}")
                continue
            arch_pairs = 0
            rows = {}
            # -O1 is the primary level; x86-64 repeats the whole contract at -O2.
            levels = ("-O1", "-O2") if label == "x86-64" else ("-O1",)
            for name in sources:
                source = ROOT / name
                if not source.is_file():
                    failures.append(f"{label}: missing source {name}")
                    continue
                for opt in levels:
                    key = f"{name} {opt}"
                    hashes, modes, counters = {}, {}, {}
                    for tag, knob in (("serial", "0"), ("auto", None), ("forced", "1")):
                        hashes[tag], modes[tag], counters[tag] = compile_asm(
                            binary, source, knob, workdir, f"{opt}{tag}", opt
                        )
                    if len(set(hashes.values())) != 1:
                        failures.append(
                            f"{label}/{key}: scheduling mode changed the assembly {hashes}"
                        )
                    arch_pairs += len(modes["auto"])
                    if "parallel" in modes["serial"]:
                        failures.append(f"{label}/{key}: kill switch ignored {modes['serial']}")
                    if threads > 1 and "serial" in modes["forced"]:
                        failures.append(
                            f"{label}/{key}: forced parallel fell back on a "
                            f"{threads}-thread host {modes['forced']}"
                        )
                    if threads == 1 and "parallel" in modes["forced"]:
                        failures.append(
                            f"{label}/{key}: spawned on a single-thread host {modes['forced']}"
                        )
                    for tag in ("serial", "auto", "forced"):
                        check_accounting(label, name, opt, modes[tag], counters[tag], failures)
                    # Reproducibility: identical inputs, identical objects.
                    repeats = {
                        compile_asm(binary, source, None, workdir, f"{opt}repeat{index}", opt)[0]
                        for index in range(3)
                    }
                    if len(repeats) != 1:
                        failures.append(
                            f"{label}/{key}: not reproducible across runs "
                            f"({len(repeats)} distinct hashes); a randomized hash iteration in "
                            f"the peephole decides emission order"
                        )
                    rows[key] = {
                        "assembly_sha256": hashes["serial"],
                        "pairs_auto": modes["auto"],
                        "pairs_forced": modes["forced"],
                        "counters_auto": counters["auto"],
                    }
            if arch_pairs == 0:
                failures.append(f"{label}: no dual-layout pair measured; the contract is void")
            total_pairs += arch_pairs
            observed[label] = {"sources": rows, "pairs": arch_pairs}

    if failures:
        raise ValueError("pair contract failures:\n  " + "\n  ".join(failures))

    if args.json:
        Path(args.json).parent.mkdir(parents=True, exist_ok=True)
        Path(args.json).write_text(
            json.dumps(
                {"effective_threads": threads, "total_pairs": total_pairs, "arches": observed},
                indent=2, sort_keys=True,
            ) + "\n"
        )

    detail = ", ".join(f"{label} {row['pairs']}" for label, row in observed.items())
    print(
        f"PASS {len(observed)} backends ({detail} pairs), threads={threads}: "
        f"identical assembly across serial/auto/forced and across repeats, "
        f"counters account for every pair"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
