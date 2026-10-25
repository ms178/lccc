#!/usr/bin/env python3
"""Fixed SSE2 shuffle webs: independent native oracle and isolated controls."""
import argparse
import json
import os
from pathlib import Path
import re
import tempfile
from check_llvm_missed_zero_choice import run, body
from check_llvm_predicate_relations import gcc_oracle_identity

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "tests/regression/shuffle_composition.c"


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--compiler", default=os.environ.get("CCC", str(ROOT / "target/fastbuild/lccc")))
    p.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    p.add_argument("--clang", default=os.environ.get("CLANG"))
    p.add_argument("--allow-gcc16-snapshot", action="store_true")
    p.add_argument("--json")
    args = p.parse_args()
    version = run([args.gcc, "-dumpfullversion"]).decode().strip()
    normative = gcc_oracle_identity(version, args.allow_gcc16_snapshot)
    if args.clang and run([args.clang, "-dumpversion"]).decode().strip() != "23.1.3":
        raise ValueError("exact Clang 23.1.3 required")
    env = os.environ | {"CCC_VERIFY_IR": "abort", "CCC_VERIFY_REGALLOC": "1", "CCC_VALIDATE_SSA": "1"}
    off = env | {"CCC_DISABLE_PASSES": "vecrelations"}
    names = re.findall(r"void (shuffle_\d+)\(", SOURCE.read_text())
    if len(set(names)) != 26:
        raise ValueError("expected 26 distinct kernels")
    with tempfile.TemporaryDirectory() as tmp:
        directory = Path(tmp)
        expected = None
        for label, compiler in [("gcc", args.gcc)] + ([("clang", args.clang)] if args.clang else []):
            exe = directory / label
            run([compiler, "-O2", "-msse2", "-Wall", "-Wextra", "-Wno-unused-variable", "-Werror", str(SOURCE), "-o", str(exe)])
            result = run([str(exe)])
            if expected is not None and result != expected:
                raise ValueError("vendor disagreement")
            expected = result
        for flags in [[level, "-msse2"] for level in ("-O0", "-O1", "-O2", "-O3", "-Os")] + [["-O2", "-g", "-msse2"], ["-O2", "-march=x86-64-v3"], ["-O2", "-mno-sse2", "-msse"]]:
            exe = directory / "native"
            run([args.compiler, *flags, str(SOURCE), "-o", str(exe)], env=env)
            if run([str(exe)]) != expected:
                raise ValueError(f"native mismatch: {flags}")
        exe = directory / "disabled"
        run([args.compiler, "-O2", "-msse2", str(SOURCE), "-o", str(exe)], env=off)
        if run([str(exe)]) != expected:
            raise ValueError("isolated-disable mismatch")
        # Compile-only i686 coverage does not depend on host multilib startup.
        run([args.compiler, "-O2", "-m32", "-msse2", "-DSHUFFLE_KERNELS_ONLY", "-S", str(SOURCE), "-o", str(directory / "i686.s")], env=env)
        asm = {}
        for label, settings in [("enabled", env), ("disabled", off)]:
            path = directory / f"{label}.s"
            run([args.compiler, "-O2", "-msse2", "-S", str(SOURCE), "-o", str(path)], env=settings)
            asm[label] = path.read_text()
        rows = [{"name": name, "enabled": len(body(asm["enabled"], name)), "disabled": len(body(asm["disabled"], name))} for name in names]
        # Inverse composition must erase both permutations; nonidentity chains
        # retain at most one. Same-mask private binops need one, not two.
        for i in range(21):
            code = body(asm["enabled"], f"shuffle_{i}")
            count = sum(bool(re.match(r"v?pshufd\b", line)) for line in code)
            if count > (0 if i == 0 else 1):
                raise ValueError(f"shuffle web not reduced: shuffle_{i}: {code}")
        improved = sum(row["enabled"] < row["disabled"] for row in rows)
        for i in range(16, 21):
            row = next(r for r in rows if r["name"] == f"shuffle_{i}")
            disabled_shuffles = sum(bool(re.match(r"v?pshufd\b", line)) for line in body(asm["disabled"], f"shuffle_{i}"))
            if row["enabled"] >= row["disabled"] or disabled_shuffles != 2:
                raise ValueError(f"private-binop mechanism did not improve: {row}")
        report = {"correct": True, "native_kernels": 26, "lane_checks": 425984,
                  "checksum": expected.decode().strip(), "gcc_version": version,
                  "gcc_16_2_normative": normative, "clang_version": "23.1.3" if args.clang else None,
                  "improved_vs_isolated_disable": improved, "comparisons": rows,
                  "timing_claim": False, "control": "only vecrelations disabled; typed lowering and vector SSA remain enabled"}
        if args.json:
            Path(args.json).write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps({k: v for k, v in report.items() if k != "comparisons"}, sort_keys=True))


if __name__ == "__main__":
    main()
