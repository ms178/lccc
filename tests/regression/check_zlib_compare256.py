#!/usr/bin/env python3
"""Differential correctness and negative control for the zlib-ng match kernel.

This is not a performance benchmark. GCC 16.2 and LCCC must agree at four
optimization levels; --m32 also requires the executable i686 multilib. The
kernel itself checks every mismatch offset and alignment against a byte-wise
reference. A deliberately broken GCC build must trip that self-check.
"""
from __future__ import annotations

import argparse
import re
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "tests/benchmark/programs/zlib_ng_compare256.c"
EXPECTED = b"zlib_ng_compare256 checksum=10322246725779703789\n"


def run(command: list[str], *, timeout: int = 30) -> subprocess.CompletedProcess:
    try:
        return subprocess.run(command, capture_output=True, timeout=timeout, check=False)
    except (OSError, subprocess.TimeoutExpired) as error:
        raise RuntimeError(f"cannot execute {command!r}: {error}") from error


def compile_one(compiler: str, source: Path, output: Path, flags: list[str]) -> None:
    result = run([compiler, *flags, "-DPASSES=2", str(source), "-o", str(output)])
    if result.returncode or not output.is_file():
        raise RuntimeError(f"{compiler} {flags}: compile failed ({result.returncode}): "
                           f"{result.stderr.decode(errors='replace')}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lccc", default="target/fastbuild/lccc")
    parser.add_argument("--gcc", default="gcc-16")
    parser.add_argument("--clang", help="optional Clang 23.1.4 executable")
    parser.add_argument("--m32", action="store_true", help="require executable i686 multilib")
    args = parser.parse_args()
    version = run([args.gcc, "-dumpfullversion"])
    if version.returncode or version.stdout.strip() != b"16.2.0":
        raise RuntimeError(f"expected GCC 16.2.0, got {version.stdout!r} {version.stderr!r}")
    compilers = [("gcc", args.gcc), ("lccc", str(Path(args.lccc).resolve()))]
    if args.clang:
        version = run([args.clang, "--version"])
        if version.returncode or not re.search(rb"clang version 23\.1\.4\b", version.stdout):
            raise RuntimeError(f"expected Clang 23.1.4, got {version.stdout!r}")
        compilers.append(("clang", args.clang))
    count = 0
    with tempfile.TemporaryDirectory(prefix="lccc-zlib-compare256-") as directory:
        work = Path(directory)
        for arch in ([], ["-m32"]) if args.m32 else ([],):
            for label, compiler in compilers:
                for opt in ("-O0", "-O2", "-O3", "-Os"):
                    binary = work / f"{label}-{opt[1:]}-{'32' if arch else '64'}"
                    compile_one(compiler, SOURCE, binary, [*arch, opt])
                    result = run([str(binary)])
                    if result.returncode or result.stdout != EXPECTED:
                        raise RuntimeError(f"{label} {arch} {opt}: execution mismatch: "
                                           f"rc={result.returncode}, stdout={result.stdout!r}, "
                                           f"stderr={result.stderr!r}")
                    count += 1
        original = SOURCE.read_text()
        needle = "    return MATCH_LENGTH;\n}"
        if original.count(needle) != 1:
            raise RuntimeError("negative-control mutation no longer identifies the kernel return")
        mutant = work / "mutant.c"
        mutant.write_text(original.replace(needle, "    return MATCH_LENGTH - 1;\n}", 1))
        binary = work / "mutant"
        compile_one(args.gcc, mutant, binary, ["-O2"])
        result = run([str(binary)])
        if result.returncode != 2 or result.stdout:
            raise RuntimeError(f"negative control escaped self-check: rc={result.returncode}, "
                               f"stdout={result.stdout!r}, stderr={result.stderr!r}")
    print(f"PASS: zlib-ng compare256: {count} cross-compiler/optimization/ABI runs; "
          "exhaustive offset/alignment checks and one detected mutation")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except RuntimeError as error:
        raise SystemExit(f"FAIL: {error}")
