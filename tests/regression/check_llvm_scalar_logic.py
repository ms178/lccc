#!/usr/bin/env python3
"""Closed scalar logic: fifty native forms, defined shifts and disable controls."""
import argparse
import json
import os
from pathlib import Path
import re
import tempfile
from check_llvm_missed_zero_choice import body, run

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "tests/regression/llvm_scalar_logic.c"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", default=os.environ.get("CCC", str(ROOT / "target/fastbuild/lccc")))
    parser.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    parser.add_argument("--clang", default=os.environ.get("CLANG"))
    parser.add_argument("--baseline")
    parser.add_argument("--json")
    args = parser.parse_args()
    if not run([args.gcc, "-dumpfullversion"]).decode().strip().startswith("16."):
        raise ValueError("GCC 16 oracle required")
    if args.clang and run([args.clang, "-dumpversion"]).decode().strip() != "23.1.3":
        raise ValueError("Clang 23.1.3 oracle required")
    env = os.environ | {"CCC_VERIFY_IR": "abort", "CCC_VERIFY_REGALLOC": "1", "CCC_VALIDATE_SSA": "1"}
    references = [("gcc", args.gcc)] + ([("clang", args.clang)] if args.clang else [])
    measurements = []
    with tempfile.TemporaryDirectory() as tmp:
        directory = Path(tmp)
        expected = None
        for label, compiler in references:
            run([compiler, "-O2", "-Wall", "-Wextra", "-Werror", str(SOURCE), "-o", str(directory / label)])
            output = run([str(directory / label)])
            if expected is not None and output != expected:
                raise ValueError("reference disagreement")
            expected = output
        for flags in [[level] for level in ("-O0", "-O1", "-O2", "-O3", "-Os")] + [["-O3", "-g"], ["-O2", "-march=x86-64-v3"], ["-O2", "-m32"]]:
            run([args.compiler, *flags, str(SOURCE), "-o", str(directory / "native")], env=env)
            if run([str(directory / "native")]) != expected:
                raise ValueError(f"native mismatch: {flags}")
        off = env | {"CCC_DISABLE_PASSES": "bit_idioms"}
        run([args.compiler, "-O2", str(SOURCE), "-o", str(directory / "disabled")], env=off)
        if run([str(directory / "disabled")]) != expected:
            raise ValueError("disable control mismatch")
        modes = [("enabled", args.compiler, env), ("disabled", args.compiler, off)]
        if args.baseline:
            modes.append(("baseline", args.baseline, env))
        assembly = {}
        for label, compiler, settings in modes:
            path = directory / f"{label}.s"
            run([compiler, "-O2", "-S", str(SOURCE), "-o", str(path)], env=settings)
            assembly[label] = path.read_text()
        names = re.findall(r"uint(?:32|64)_t (logic_\w+)\(", SOURCE.read_text())
        if len(names) != 50 or len(set(names)) != 50:
            raise ValueError("native contract inventory is not fifty distinct width-specific forms")
        for name in names:
            counts = {label: len(body(text, name)) for label, text in assembly.items()}
            measurements.append({"name": name, "instructions": counts})
        for width in (32, 64):
            for stem in ("shift_left_bit", "shift_right_bit", "shift_low_bits"):
                instructions = body(assembly["enabled"], f"logic_{stem}_{width}")
                if any(re.match(r"(?:sal|shl|shr)[bwlq]?\b", instruction) for instruction in instructions):
                    raise ValueError(f"bounded shift survives: {stem}/{width}: {instructions}")
        improved = [row for row in measurements if row["instructions"]["enabled"] < row["instructions"]["disabled"]]
        if not improved:
            raise ValueError("no instruction reduction against the disabled-pass control")
        report = {"native_forms": len(names), "correct": True, "timing_claim": False,
                  "comparisons": measurements, "improved_vs_disabled": len(improved),
                  "note": "The disabled-pass control is not an immutable before-implementation baseline."}
        if args.json:
            Path(args.json).write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps(report))
    print("PASS scalar logic: fifty native forms, vendor agreement, IR/SSA/RA, m32 and disable controls")


if __name__ == "__main__":
    main()
