#!/usr/bin/env python3
"""Reproduce the integer observations' static code-size table, never a timing claim."""

import argparse
import json
import os
from pathlib import Path
import re
import sys
import tempfile
import hashlib
import shutil

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tests/regression"))
from check_llvm_missed_zero_choice import run, body  # noqa: E402


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--suite", choices=("integer-observations", "bit-demand", "scalar-facts"), default="integer-observations"
    )
    parser.add_argument(
        "--wide", action="store_true", help="bit-demand 64-bit companion source"
    )
    parser.add_argument("--compiler", default=str(ROOT / "target/fastbuild/lccc"))
    parser.add_argument(
        "--baseline", required=True, help="saved compiler from before this batch"
    )
    parser.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    parser.add_argument("--clang", required=True, help="exact Clang 23.1.3 executable")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    versions = {
        name: run([tool, "--version"]).decode().splitlines()[0]
        for name, tool in (("gcc", args.gcc), ("clang", args.clang))
    }
    if not re.search(r"\b16\.2(?:\.0)?\b", versions["gcc"]) or not re.search(
        r"\b23\.1\.3\b", versions["clang"]
    ):
        raise ValueError("requires GCC 16.2 and Clang 23.1.3")
    if args.wide and args.suite != "bit-demand":
        raise ValueError("--wide requires --suite=bit-demand")
    stem = {
        "integer-observations": "llvm_integer_observations",
        "bit-demand": "llvm_bit_demand",
        "scalar-facts": "llvm_scalar_facts",
    }[args.suite]
    contracts = ROOT / "tests/regression" / (stem + ".json")
    source = ROOT / "tests/regression" / (stem + ("_wide" if args.wide else "") + ".c")
    rows = {c["name"]: {} for c in json.loads(contracts.read_text())}
    flags = ["-O3", "-march=x86-64-v3", "-S"]
    tools = (
        ("baseline", args.baseline),
        ("lccc", args.compiler),
        ("gcc", args.gcc),
        ("clang", args.clang),
    )
    if args.suite == "scalar-facts":
        flags.insert(-1, "-fPIE")
    with tempfile.TemporaryDirectory() as directory:
        for name, tool in tools:
            output = Path(directory) / (name + ".s")
            run([tool, *flags, str(source), "-o", str(output)])
            text = re.sub(r"#[^\n]*", "", output.read_text())
            for n in rows:
                rows[n][name] = len(body(text, n))
    report = {
        "flags": flags,
        "measurement": "static instruction count including RET, no timing inference",
        "versions": versions,
        "compiler_binary_sha256": {
            name: hashlib.sha256(
                Path(shutil.which(tool) or tool).read_bytes()
            ).hexdigest()
            for name, tool in tools
        },
        "source_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
        "counts": rows,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    for compiler in ("baseline", "gcc", "clang"):
        print(
            compiler,
            {
                result: sum(
                    (
                        r["lccc"] < r[compiler]
                        if result == "fewer"
                        else (
                            r["lccc"] == r[compiler]
                            if result == "equal"
                            else r["lccc"] > r[compiler]
                        )
                    )
                    for r in rows.values()
                )
                for result in ("fewer", "equal", "more")
            },
        )


if __name__ == "__main__":
    main()
