#!/usr/bin/env python3
"""Differential and independent-model checks for affine addresses across calls.

GCC 16.2 is mandatory; pass --clang/--icx to add exact-version vendor oracles.
Every subprocess is bounded. The expected output is computed independently in
Python, including unsigned truncation and every untouched destination/guard.
"""
import argparse
import os
from pathlib import Path
import re
import subprocess
import tempfile

SOURCE = Path(__file__).with_name("affine_scaled_index_matrix.c").resolve()
MASK = (1 << 64) - 1


def run(args, env=None):
    result = subprocess.run(args, env=env, text=True, capture_output=True,
                            timeout=120, check=True)
    # A zero exit code does not excuse diagnostics from these warning-clean
    # fixtures. Keep runtime stdout for exact comparison with the model.
    if result.stderr:
        raise AssertionError(f"unexpected stderr from {args}:\n{result.stderr}")
    return result.stdout


def expected_output():
    rows = []
    for n in (0, 1, 2, 3, 7, 15, 31, 63, 64):
        hashes = []
        for bits in (8, 16, 32, 64):
            h = 0
            for j in range(65):
                value = 0xa5
                if 1 <= j <= n:
                    value = (j * 17 + 3 + (j - 1) * 0x9e3779b9 + 0x123456789) & ((1 << bits) - 1)
                h = (h * 131 + value) & MASK
            hashes.append(h)
        h = 0
        for j in range(65):
            value = 0xa5 if not 1 <= j <= n else j * 17 + 3 + (j - 1) * 0x9e3779b9 + 0x123456789
            h = (h * 131 + value) & MASK
            h = (h * 131 + (0xdeadbeef ^ j)) & MASK
        hashes.append(h)
        rows.append(" ".join(map(str, [n, *hashes])))
    return "\n".join(rows) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lccc", default=os.environ.get("CCC", "target/fastbuild/lccc"))
    parser.add_argument("--gcc", default="gcc-16")
    parser.add_argument("--clang")
    parser.add_argument("--icx")
    args = parser.parse_args()
    gcc_version = run([args.gcc, "-dumpfullversion"]).strip()
    if gcc_version != "16.2.0":
        raise AssertionError(f"requires GCC 16.2.0, got {gcc_version}")
    oracles = [args.gcc]
    for compiler, pattern in ((args.clang, r"clang version 23\.1\.3\b"),
                              (args.icx, r"\b2026\.2(?:\.|\b)")):
        if compiler:
            banner = run([compiler, "--version"])
            if not re.search(pattern, banner):
                raise AssertionError(f"unexpected oracle identity: {banner}")
            oracles.append(compiler)
    expected = expected_output()
    # Remove ambient optimization switches from the measured default leg.
    env = {k: v for k, v in os.environ.items() if not k.startswith(("CCC_", "LCCC_"))}
    env["CCC_VALIDATE_SSA"] = "1"
    modes = ({}, {"CCC_NO_IVSR": "1"}, {"CCC_NO_IV_WIDEN": "1"},
             {"CCC_IVSR_SCALAR_DERIVED": "1"})
    cases = 0
    with tempfile.TemporaryDirectory(prefix="lccc-affine-matrix-") as directory:
        binary = str(Path(directory) / "test")
        for opt in ("-O0", "-O2", "-O3", "-Os"):
            for compiler, policies in [(cc, ({},)) for cc in oracles] + [(args.lccc, modes)]:
                for policy in policies:
                    case_env = {**env, **policy}
                    run([compiler, opt, "-Wall", "-Wextra", "-Werror", str(SOURCE), "-o", binary], case_env)
                    actual = run([binary], case_env)
                    if actual != expected:
                        raise AssertionError(f"mismatch: {compiler} {opt} {policy}\n{actual}\nexpected:\n{expected}")
                    cases += 1
    print(f"PASS: affine scaled-index matrix: {cases} compiler/policy builds, {cases * 45} checked hashes")


if __name__ == "__main__":
    main()
