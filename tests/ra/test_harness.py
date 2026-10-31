#!/usr/bin/env python3
"""Determinism, assembly-parser and checksum tests for the RA research harness."""
from __future__ import annotations

import argparse
import importlib.util
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def load_module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--full", action="store_true", help="check GCC O0/O1/O2/O3 (slower)")
    args = parser.parse_args()
    if not __import__("shutil").which("gcc"):
        print("SKIP: gcc is required for the RA harness checksum test")
        return 0

    with tempfile.TemporaryDirectory(prefix="lccc-ra-harness-test-") as td:
        root = Path(td)
        a, b = root / "a", root / "b"
        subprocess.run([sys.executable, str(ROOT / "tests/ra/gen_rastress.py"), "--output-dir", str(a)], check=True, capture_output=True)
        subprocess.run([sys.executable, str(ROOT / "tests/ra/gen_rastress.py"), "--output-dir", str(b)], check=True, capture_output=True)
        files_a = sorted(a.glob("*.c"))
        files_b = sorted(b.glob("*.c"))
        assert len(files_a) == 20, f"expected 20 holdout kernels, got {len(files_a)}"
        assert [p.name for p in files_a] == [p.name for p in files_b]
        for x, y in zip(files_a, files_b):
            assert x.read_bytes() == y.read_bytes(), f"non-deterministic generated source: {x.name}"

        scorer = load_module("ra_spill_compare", ROOT / "tests/ra/spill_compare.py")
        assembly = """.type f,@function
f:
    pushq %rbx
.Lloop:
    movq -8(%rsp), %rax
    pushq %rax
    popq %rax
    addq $1, %rdi
    jne .Lloop
    popq %rbx
    ret
.size f,.-f
"""
        counts = scorer.instruction_counts(scorer.assembly_functions(assembly)["f"])
        assert counts["stack_operands"] == 1, counts
        assert counts["push"] == 2 and counts["pop"] == 2, counts
        assert counts["loop_push_pop"] == 2, counts

        levels = ("0", "1", "2", "3") if args.full else ("0", "2")
        for src in files_a:
            reference = scorer.check_correctness("gcc", src, "0", [])
            for opt in levels:
                result = scorer.check_correctness("gcc", src, opt, [], reference)
                assert result == reference
    print(f"PASS: deterministic 20-kernel corpus, loop push/pop census, GCC checksum matrix {levels}")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
