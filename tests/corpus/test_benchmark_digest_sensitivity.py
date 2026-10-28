#!/usr/bin/env python3
"""Every benchmark digest must move when one input bit does.

WHY THIS GATE EXISTS
====================
The benchmark harness compares *printed checksums* between compilers: a
miscompile is caught only if the corrupted value reaches the number the program
prints. A digest can be structurally blind to the very data it is meant to
summarize, and both failure modes shipped in this corpus before this gate
existed:

  * `expat_utf8_content` folded its per-pass hashes with `digest ^= pass_hash`
    inside a pass loop whose body is periodic in `p & 7`. Every pass hash
    therefore recurred 32 times and the XOR cancelled: the program printed
    `0000000000000000` for every input, under every compiler, at every
    optimization level. GCC and LCCC agreeing on zero agreed on nothing.
  * `sqlite_vdbe_serial` summed decoded integer values and the *lengths* of the
    string/blob cells, so no text payload and no REAL payload ever reached the
    checksum: a miscompile that corrupts a string but keeps its length compared
    equal.

Neither is visible in a differential run -- both compilers produce the same
blind number -- so only a sensitivity probe finds them.

HOW IT WORKS
============
Each workload compiles its own input generator under `BENCH_DIGEST_PROBE` and
flips one bit of the generator's seed:

    #ifdef BENCH_DIGEST_PROBE
        s ^= 0x01U;
    #endif

The program is built twice with the host C compiler (the probe tests the
*fixture*, not the compiler under test), and the gate requires:

  1. exit status 0 both ways -- the probe may not turn a self-check red, so the
     perturbation stays inside the data the workload is allowed to interpret;
  2. different printed output -- the digest is a function of the input.

A workload that cannot satisfy (1) anywhere is asserting something about its
input's *validity* that its own self-check depends on; that is a fixture bug,
not a reason to weaken the gate.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PROGRAMS = ROOT / "tests" / "benchmark" / "programs"

# The workload-derived kernels added with this corpus round. Each one registers
# itself in tests/benchmark/run_benchmarks.py and carries the probe.
PROBED = (
    "gzip_crc32_slice8",
    "zlib_ng_slide_hash",
    "expat_utf8_content",
    "sqlite_vdbe_serial",
    "linux_xxhash64",
    "glibc_dl_hash",
)

CFLAGS = ("-O2", "-Wall", "-Wextra", "-std=c11")
RUN_TIMEOUT_S = 300


def compile_binary(cc: str, source: Path, out: Path, *, probe: bool) -> str | None:
    """Compile `source`; return the compiler's stderr on failure."""
    flags = list(CFLAGS)
    if probe:
        flags.append("-DBENCH_DIGEST_PROBE")
    result = subprocess.run(
        [cc, *flags, str(source), "-o", str(out)],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        return result.stderr.strip() or f"{cc} exited {result.returncode}"
    if result.stderr.strip():
        return f"warnings while building {source.name}: {result.stderr.strip()}"
    return None


def run_binary(binary: Path) -> tuple[int, str]:
    result = subprocess.run(
        [str(binary)],
        capture_output=True,
        text=True,
        timeout=RUN_TIMEOUT_S,
        check=False,
    )
    return result.returncode, result.stdout


def check(cc: str, name: str, workdir: Path) -> list[str]:
    source = PROGRAMS / f"{name}.c"
    if not source.is_file():
        return [f"{name}: missing {source.relative_to(ROOT)}"]
    problems: list[str] = []
    digests: list[str] = []
    for probe in (False, True):
        label = "probe" if probe else "plain"
        binary = workdir / f"{name}.{label}"
        error = compile_binary(cc, source, binary, probe=probe)
        if error is not None:
            problems.append(f"{name}: {label} build failed: {error}")
            break
        status, stdout = run_binary(binary)
        if status != 0:
            problems.append(f"{name}: {label} exited {status}; the probe must not"
                            " change a self-check verdict")
            break
        digests.append(stdout.strip())
    else:
        if len(set(digests)) != 2:
            problems.append(
                f"{name}: digest is insensitive to a one-bit input change "
                f"(both builds printed {digests[0]!r}); a miscompile in the "
                "data this workload reads would compare equal"
            )
        elif not digests[0] or digests[0] == "0" * len(digests[0]):
            problems.append(f"{name}: printed digest {digests[0]!r} is degenerate")
    return problems


class BenchmarkDigestSensitivity(unittest.TestCase):
    """Neither a cancelled digest nor a length-only digest can detect anything."""

    @classmethod
    def setUpClass(cls):
        cls.cc = os.environ.get("CC", "gcc")
        if shutil.which(cls.cc) is None:
            raise unittest.SkipTest(f"{cls.cc} not available to build the fixtures")

    def test_one_input_bit_moves_every_digest(self):
        with tempfile.TemporaryDirectory(prefix="lccc-bench-digest.") as tmp:
            workdir = Path(tmp)
            for name in PROBED:
                with self.subTest(workload=name):
                    problems = check(self.cc, name, workdir)
                    self.assertEqual(problems, [], "; ".join(problems))


if __name__ == "__main__":
    unittest.main()
