#!/usr/bin/env python3
"""Packed integer relations: native oracles, mechanism and disable controls."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
from check_llvm_missed_zero_choice import run, body
from check_llvm_predicate_relations import gcc_oracle_identity

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "tests/regression/packed_vector_relations.c"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", default=os.environ.get("CCC", str(ROOT / "target/fastbuild/lccc")))
    parser.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    parser.add_argument("--clang", default=os.environ.get("CLANG"))
    parser.add_argument("--allow-gcc16-snapshot", action="store_true")
    parser.add_argument("--json")
    args = parser.parse_args()
    version = run([args.gcc, "-dumpfullversion"]).decode().strip()
    normative = gcc_oracle_identity(version, args.allow_gcc16_snapshot)
    if args.clang and run([args.clang, "-dumpversion"]).decode().strip() != "23.1.3":
        raise ValueError("exact Clang 23.1.3 required")
    references = [("gcc", args.gcc)] + ([("clang", args.clang)] if args.clang else [])
    env = os.environ | {"CCC_VERIFY_IR": "abort", "CCC_VERIFY_REGALLOC": "1", "CCC_VALIDATE_SSA": "1"}
    off = env | {"CCC_DISABLE_PASSES": "vecrelations"}
    names = re.findall(r"void (packed_\w+)\(", SOURCE.read_text())
    if len(names) != 120 or len(set(names)) != 120:
        raise ValueError("expected 15 contracts x 4 lane types x 2 register widths")
    with tempfile.TemporaryDirectory() as tmp:
        directory = Path(tmp)
        for width, base_flags in [(128, ["-msse2", "-DONLY128"]), (256, ["-march=x86-64-v3"])]:
            expected = None
            for label, compiler in references:
                executable = directory / f"{label}-{width}"
                run([compiler, "-O2", *base_flags, "-Wall", "-Wextra", "-Werror", str(SOURCE), "-o", str(executable)])
                output = run([str(executable)])
                if expected is not None and expected != output:
                    raise ValueError(f"vendor disagreement at {width}")
                expected = output
            modes = [[level, *base_flags] for level in ("-O0", "-O1", "-O2", "-O3", "-Os")]
            modes += [["-O2", "-g", *base_flags]]
            if width == 128:
                modes += [["-O2", "-m32", *base_flags]]
            for flags in modes:
                executable = directory / "native"
                run([args.compiler, *flags, str(SOURCE), "-o", str(executable)], env=env)
                if run([str(executable)]) != expected:
                    raise ValueError(f"native mismatch: {flags}")
            executable = directory / "disabled"
            run([args.compiler, "-O2", *base_flags, str(SOURCE), "-o", str(executable)], env=off)
            if run([str(executable)]) != expected:
                raise ValueError(f"disabled-control mismatch at {width}")
        assembly = {}
        for label, settings in [("enabled", env), ("disabled", off)]:
            path = directory / f"{label}.s"
            run([args.compiler, "-O2", "-march=x86-64-v3", "-S", str(SOURCE), "-o", str(path)], env=settings)
            assembly[label] = path.read_text()
        rows = [{"name": name, "enabled": len(body(assembly["enabled"], name)),
                 "disabled": len(body(assembly["disabled"], name))} for name in names]
        improved = [r for r in rows if r["enabled"] < r["disabled"]]
        if len(improved) < 8:
            raise ValueError(f"fewer than eight improvements against isolated vecrelations control: {improved}")
        for bits in (8, 16, 32, 64):
            for size in (16, 32):
                name = f"packed_bounded_mask_u{bits}_{size}"
                row = next(r for r in rows if r["name"] == name)
                if row["enabled"] >= row["disabled"]:
                    diagnostic = subprocess.run(
                        [args.compiler, "-O2", "-march=x86-64-v3", "-S", str(SOURCE), "-o", str(directory / "diagnostic.s")],
                        env=env | {"CCC_DUMP_EACH_PASS": "1", "CCC_DUMP_FUNC": name},
                        capture_output=True, timeout=60)
                    raise ValueError(f"bounded-mask mechanism did not improve: {row}; enabled={body(assembly['enabled'], name)}; IR={diagnostic.stderr.decode(errors='replace')[-14000:]}")
        for bits in (8, 16, 32, 64):
            for size in (16, 32):
                name = f"packed_xor_cancel_u{bits}_{size}"
                if any(re.match(r"v?pxor\b|v?xorps\b|v?xorpd\b", line) for line in body(assembly["enabled"], name)):
                    raise ValueError(f"XOR cancellation did not fire: {name}")
        report = {"correct": True, "native_forms": 120, "gcc_version": version,
                  "gcc_16_2_normative": normative, "clang_version": "23.1.3" if args.clang else None,
                  "improved_vs_isolated_disable": len(improved), "comparisons": rows,
                  "timing_claim": False, "control": "only vecrelations disabled; existing vector SSA remains enabled"}
        if args.json:
            Path(args.json).write_text(json.dumps(report, indent=2) + "\n")
        summary={key:report[key] for key in ("correct","native_forms","gcc_version","gcc_16_2_normative","clang_version","improved_vs_isolated_disable","timing_claim")}
        print("::notice title=Packed vector relations receipt::"+json.dumps(summary))
        print(json.dumps(report))
    print("PASS packed vector relations: 120 forms, baseline SSE2, AVX2, i386, verifiers and isolated disable control")


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        detail = str(error).replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
        print(f"::error title=Packed vector relations::{detail}")
        raise
