#!/usr/bin/env python3
"""Predicate relations: sixty-two native forms and conditional substitution controls."""
import argparse
import json
import os
from pathlib import Path
import re
import tempfile
from check_llvm_missed_zero_choice import body, run

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "tests/regression/llvm_predicate_relations.c"


def gcc_oracle_identity(version, allow_snapshot=False):
    """Only the Debian exact-version gate supplies normative GCC evidence."""
    normative = bool(re.fullmatch(r"16\.2(?:\.0)?", version))
    if not normative and not (allow_snapshot and version.startswith("16.")):
        raise ValueError(f"GCC 16.2 oracle required, found {version!r}")
    return normative


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", default=os.environ.get("CCC", str(ROOT / "target/fastbuild/lccc")))
    parser.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    parser.add_argument("--clang", default=os.environ.get("CLANG"))
    parser.add_argument("--baseline")
    parser.add_argument("--allow-gcc16-snapshot", action="store_true",
                        help="explicit supplemental snapshot oracle; never normative GCC 16.2 evidence")
    parser.add_argument("--json")
    args = parser.parse_args()
    gcc_version = run([args.gcc, "-dumpfullversion"]).decode().strip()
    gcc_normative = gcc_oracle_identity(gcc_version, args.allow_gcc16_snapshot)
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
        names = re.findall(r"uint(?:32|64)_t (predicate_\w+)\(", SOURCE.read_text())
        if len(names) != 62 or len(set(names)) != 62:
            raise ValueError("native contract inventory is not sixty-two distinct width-specific forms")
        for name in names:
            counts = {label: len(body(text, name)) for label, text in assembly.items()}
            measurements.append({"name": name, "instructions": counts})
        for width in (32, 64):
            for stem in ("high_mask_equal", "high_mask_unequal", "low_mask_equal", "low_mask_unequal"):
                instructions = body(assembly["enabled"], f"predicate_{stem}_{width}")
                if any(re.match(r"(?:and|or)[bwlq]?\b", instruction) for instruction in instructions):
                    raise ValueError(f"contiguous mask survives: {stem}/{width}: {instructions}")
        improved = [row for row in measurements if row["instructions"]["enabled"] < row["instructions"]["disabled"]]
        if not improved:
            raise ValueError("no instruction reduction against the disabled-pass control")
        report = {"gcc_version": gcc_version, "gcc_16_2_normative": gcc_normative,
                  "clang_version": "23.1.3" if args.clang else None, "native_forms": len(names), "correct": True, "timing_claim": False,
                  "comparisons": measurements, "improved_vs_disabled": len(improved),
                  "note": "The disabled-pass control is not an immutable before-implementation baseline."}
        if args.json:
            Path(args.json).write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps(report))
    print("PASS predicate relations: sixty-two native forms, vendor agreement, IR/SSA/RA, m32 and disable controls")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, RuntimeError) as error:
        detail = str(error).replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
        print(f"::error title=Predicate relations::{detail}")
        raise
