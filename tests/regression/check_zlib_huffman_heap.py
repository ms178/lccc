#!/usr/bin/env python3
"""Check the Huffman workload and its independent edge oracle, without timing."""
from __future__ import annotations

import argparse
from pathlib import Path
import re
import shlex
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "tests/benchmark/programs/zlib_ng_huffman_heap.c"
FIXTURE = ROOT / "tests/regression/fixtures/zlib/huffman_heap_contract.c"
EXPECTED = b"de85215d22629785\n"  # PASSES=8; GCC 16.2 and Clang 23.1.5 agree.
LEVELS = ("-O0", "-O1", "-O2", "-O3", "-Os")
INVALID_PASSES = ("0", "-1", "4294967296ULL", "2.5")


def checked(argv):
    result = subprocess.run(argv, capture_output=True, timeout=45)
    if result.returncode or result.stderr:
        raise ValueError(f"{shlex.join(argv)}: exit {result.returncode}\n"
                         f"{result.stderr.decode(errors='replace').rstrip()}")
    return result.stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lccc", default=str(ROOT / "target/fastbuild/lccc"))
    parser.add_argument("--gcc", default="gcc-16")
    parser.add_argument("--clang", default="clang-23")
    parser.add_argument("--icx", help="authenticated ICX 2026.2 snapshot (x86-64 only)")
    args = parser.parse_args()
    compilers = [(args.lccc, ("-m64", "-m32")), (args.gcc, ("-m64", "-m32")),
                 (args.clang, ("-m64", "-m32"))]
    try:
        if checked([args.gcc, "-dumpfullversion"]).strip() != b"16.2.0":
            raise ValueError("GCC 16.2.0 is required")
        if checked([args.clang, "-dumpversion"]).strip() != b"23.1.5":
            raise ValueError("Clang 23.1.5 is required")
        if args.icx:
            banner = checked([args.icx, "--version"]).partition(b"\n")[0]
            if not re.search(rb"\b2026\.2\.\d+\b", banner):
                raise ValueError(f"ICX 2026.2 is required, got {banner!r}")
            compilers.append((args.icx, ("-m64",)))
        count = 0
        with tempfile.TemporaryDirectory(prefix="lccc-huffman-contract-") as work:
            out = Path(work) / "contract"
            for compiler, widths in compilers:
                for passes in INVALID_PASSES:
                    result = subprocess.run([compiler, "-m64", "-std=c11", "-fsyntax-only",
                                             f"-DPASSES={passes}", str(SOURCE)],
                                            capture_output=True, timeout=45)
                    if result.returncode <= 0 or b"PASSES must be a positive 32-bit integer" not in result.stderr:
                        raise ValueError(f"{compiler}: PASSES={passes} was not diagnosed: {result.stderr!r}")
                for width in widths:
                    for level in LEVELS:
                        out.unlink(missing_ok=True)
                        checked([compiler, width, level, "-std=c11", "-DPASSES=8",
                                 "-Wall", "-Wextra", "-Werror", str(FIXTURE), "-o", str(out)])
                        stdout = checked([str(out)])
                        if stdout != EXPECTED:
                            raise ValueError(f"{compiler} {width} {level}: got {stdout!r}, "
                                             f"expected {EXPECTED!r}")
                        count += 1
    except (OSError, ValueError, subprocess.TimeoutExpired) as error:
        print(f"Huffman heap contract: FAIL: {error}")
        return 1
    print(f"Huffman heap contract: {count} warning-free build/run rows; exact checksum, "
          "285 saturated histograms and structural negatives per row")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
