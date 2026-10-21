#!/usr/bin/env python3
"""Dual-layout metric pairs: scheduling contract and reproducible output.

Two properties are checked for **every backend** (x86-64, i686, AArch64,
RISC-V), because all four run a text peephole over both exploration
candidates:

1. *Scheduling* — `LCCC_PEEPHOLE_PAIR_PARALLEL=0`, unset (heuristic) and `=1`
   must emit byte-identical assembly. The split is latency-only, so no
   assembly-level difference is permitted; the only observable proof that the
   concurrent branch ran is the `[pair] … mode=…` telemetry, which also pins
   the kill switch and the single-thread fallback.

2. *Reproducibility* — repeated compilation of one source must produce one
   assembly hash. This is what caught RS-1: the RISC-V identical-block pass
   iterated a `std::collections::HashMap`, so its per-process hash seed decided
   `.Lskip_N` labels and block placement, and five serial compilations produced
   five different objects.

The cross backends are driven with header-free fixtures
(`peephole_pair_chain*.c`), so the gate needs no cross sysroot; x86-64 also
runs real benchmark sources, where pairs are produced by real code.

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

# (arch label, binary suffix, sources). x86-64 additionally uses real TUs.
ARCHES = (
    ("x86-64", "", ["tests/benchmark/programs/expat_siphash24.c",
                    "tests/benchmark/programs/glibc_memcmp.c",
                    "tests/regression/peephole_pair_chain.c"]),
    ("i686", "-i686", ["tests/regression/peephole_pair_chain.c"]),
    ("aarch64", "-arm", ["tests/regression/peephole_pair_chain.c"]),
    ("riscv64", "-riscv", ["tests/regression/peephole_pair_chain.c"]),
)

MODE = re.compile(r"mode=(\w+)")


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
            for name in sources:
                source = ROOT / name
                if not source.is_file():
                    failures.append(f"{label}: missing source {name}")
                    continue
                hashes, modes = {}, {}
                for tag, knob in (("serial", "0"), ("auto", None), ("forced", "1")):
                    hashes[tag], modes[tag] = compile_asm(binary, source, knob, workdir, tag)
                if len(set(hashes.values())) != 1:
                    failures.append(
                        f"{label}/{source.name}: scheduling mode changed the assembly {hashes}"
                    )
                arch_pairs += len(modes["auto"])
                if "parallel" in modes["serial"]:
                    failures.append(f"{label}/{source.name}: kill switch ignored {modes['serial']}")
                if threads > 1 and "serial" in modes["forced"]:
                    failures.append(
                        f"{label}/{source.name}: forced parallel fell back on a "
                        f"{threads}-thread host {modes['forced']}"
                    )
                if threads == 1 and "parallel" in modes["forced"]:
                    failures.append(
                        f"{label}/{source.name}: spawned on a single-thread host {modes['forced']}"
                    )
                # Reproducibility: identical inputs, identical objects.
                repeats = {
                    compile_asm(binary, source, None, workdir, f"repeat{index}")[0]
                    for index in range(3)
                }
                if len(repeats) != 1:
                    failures.append(
                        f"{label}/{source.name}: not reproducible across runs ({len(repeats)} "
                        f"distinct hashes); a randomized hash iteration in the peephole decides "
                        f"emission order"
                    )
                rows[source.name] = {
                    "assembly_sha256": hashes["serial"],
                    "pairs_auto": modes["auto"],
                    "pairs_forced": modes["forced"],
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
        f"identical assembly across serial/auto/forced and across repeats"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
