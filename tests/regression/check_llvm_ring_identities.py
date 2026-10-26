#!/usr/bin/env python3
"""Ring identities: native oracle, per-idiom mechanism, disable controls."""

import argparse
import json
import os
from pathlib import Path
import re
import tempfile

from check_llvm_missed_zero_choice import body, gcc_oracle_identity, report_gcc_oracle, run

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "tests/regression/llvm_ring_identities.c"

# Assembly the fold must leave behind for each idiom, as (required, forbidden)
# regular expressions over the function body.
# Which control each form is compared against: the ring-identity sub-pass, or
# the division strength reduction for the power-of-two forms.
CONTROL = {
    "pow2rem": "disabled-div",
    "pow2div": "disabled-div",
}

MECHANISM = {
    # One wrapping add plus one unsigned compare; no division sequence.
    "srem": (r"\b(?:add|lea)[bwlq]?\b", r"\b(?:idiv|div|imul)[bwlq]?\b"),
    # A single arithmetic shift carries the sign; no xor/neg re-synthesis.
    "shift": (r"\bsar[bwlq]?\b", r"\b(?:neg|not)[bwlq]?\b"),
    # The complement of a negation is a unit offset; the register allocator may
    # pick `lea` or an immediate add for it.
    "complement": (r"\b(?:lea|add|sub)[bwlq]?\b", r"\b(?:neg|not)[bwlq]?\b"),
    # A saturated bound selects the other arm for every pattern.
    "bound": (r"\bmov[bwlq]?\b", r"\bcmov\w*\b"),
    # A provably constant arm collapses the whole select to that constant.
    "constant": (r"\bxor[bwlq]?\b", r"\b(?:cmov|j\w+)\b"),
    # An implied-condition select is the surviving arm, with no compare left.
    "orchain": (r"\b(?:mov|xor)[bwlq]?\b", r"\bcmov\w*\b"),
    # A boolean product is a conditional move, never a multiply.
    "boolmul": (r"\bcmov\w*\b", r"\bimul\w*\b"),
    # Power-of-two remainder keeps no magic constant and no multiply.
    "pow2rem": (r"\b(?:sar|shr|and)[bwlq]?\b", r"\b(?:imul|idiv|div)\w*\b"),
    # Power-of-two signed division is a bias add and an arithmetic shift.
    "pow2div": (r"\bsar[bwlq]?\b", r"\b(?:imul|idiv|div)\w*\b"),
}

FORMS = {
    "ring_srem_eq_250": "srem",
    "ring_srem_ne_250": "srem",
    "ring_srem_eq_min": "srem",
    "ring_srem_ne_min": "srem",
    "ring_srem_eq_neg7": "srem",
    "ring_srem_eq32": "srem",
    "ring_srem_ne32": "srem",
    "ring_sign_shift": "shift",
    "ring_sign_shift64": "shift",
    "ring_not_neg": "complement",
    "ring_neg_not": "complement",
    "ring_not_neg16": "complement",
    "ring_neg_not64": "complement",
    "ring_min_imax": "bound",
    "ring_max_imin": "bound",
    "ring_min_umax": "bound",
    "ring_max_u0": "bound",
    "ring_min_imax64": "bound",
    "ring_max_imin64": "bound",
    "ring_zero_select": "bound",
    "ring_zero_constant_eq": "bound",
    "ring_or_chain_value": "orchain",
    "ring_or_chain_zero": "orchain",
    "ring_nested_select": "orchain",
    "ring_bool_product_self": "orchain",
    "ring_bool_product_test": "boolmul",
    "ring_bool_product_cmp": "boolmul",
    "ring_srem_pow2_32": "pow2rem",
    "ring_srem_pow2_64": "pow2rem",
    "ring_srem_pow2_neg": "pow2rem",
    "ring_sdiv_pow2_32": "pow2div",
}

# Forms that must keep a real compare: the neighbouring clamp is not an
# identity and the fold has to leave it alone.
# The power-of-two forms trade a handful of ALU instructions for the hardware
# divider, so the win is latency and port pressure rather than instruction count:
# `idiv` is ~20-40 cycles and occupies the divider for many cycles, while the
# five ALU ops retire in about two. They are measured on divider removal, not on
# length, and the control must still contain the divider for that to mean
# anything.
NO_COUNT_REDUCTION = {"pow2rem", "pow2div"}
DIVIDER = r"\b(?:idiv|div)[bwlq]?\b"

SURVIVORS = {"ring_clamp_imax": r"\bcmov\w*\b"}

# Narrow remainders are promoted to `int` before the remainder is formed, so the
# dividend is a sign-extended copy and the compared value is not the same SSA
# operand. The fold refuses that shape by construction; these forms only guard
# that the refusal stays correct, they are not expected to shrink.
# `x % 256u` is a mask in the backend's own strength reduction and never reaches
# this pass, so it is a boundary rather than a fold of this batch.
PROMOTED = {"ring_srem_eq16", "ring_srem_ne16", "ring_srem_eq8", "ring_urem_pow2"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", default=os.environ.get("CCC", str(ROOT / "target/fastbuild/lccc")))
    parser.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    parser.add_argument("--clang", default=os.environ.get("CLANG"))
    parser.add_argument("--baseline")
    parser.add_argument("--json")
    args = parser.parse_args()
    version = run([args.gcc, "-dumpfullversion"]).decode().strip()
    report_gcc_oracle(args.gcc, version, gcc_oracle_identity(version, allow_snapshot=True))
    if args.clang and run([args.clang, "-dumpversion"]).decode().strip() != "23.1.4":
        raise ValueError("Clang 23.1.4 oracle required")
    source = SOURCE.read_text()
    env = os.environ | {
        "CCC_VERIFY_IR": "abort",
        "CCC_VERIFY_REGALLOC": "1",
        "CCC_VALIDATE_SSA": "1",
    }
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
        levels = [["-O0"], ["-O1"], ["-O2"], ["-O3"], ["-Os"], ["-O3", "-g"], ["-O2", "-march=x86-64-v3"]]
        for flags in levels:
            run([args.compiler, *flags, str(SOURCE), "-o", str(directory / "native")], env=env)
            if run([str(directory / "native")]) != expected:
                raise ValueError(f"native mismatch: {flags}")
        controls = {
            "disabled-subpass": env | {"CCC_NO_RING_IDENTITIES": "1"},
            "disabled-pass": env | {"CCC_DISABLE_PASSES": "bit_idioms"},
            "disabled-div": env | {"CCC_DISABLE_PASSES": "bit_idioms,divconst"},
        }
        for label, settings in controls.items():
            run([args.compiler, "-O2", str(SOURCE), "-o", str(directory / label)], env=settings)
            if run([str(directory / label)]) != expected:
                raise ValueError(f"{label} control mismatch")
        modes = [("enabled", env)] + list(controls.items())
        if args.baseline:
            modes.append(("baseline", env))
        assembly = {}
        for label, settings in modes:
            compiler = args.baseline if label == "baseline" else args.compiler
            path = directory / f"{label}.s"
            run([compiler, "-O2", "-S", str(SOURCE), "-o", str(path)], env=settings)
            assembly[label] = path.read_text()
        inventory = re.findall(r"^(?:unsigned\s+)?(?:int|long|u?int\d+_t)\s+(ring_\w+)\(", source, re.M)
        if sorted(inventory) != sorted(list(FORMS) + list(SURVIVORS) + list(PROMOTED)):
            raise ValueError(
                "inventory drift: "
                f"{sorted(set(inventory) ^ set(list(FORMS) + list(SURVIVORS) + list(PROMOTED)))}"
            )
        improved = []
        for name, kind in FORMS.items():
            required, forbidden = MECHANISM[kind]
            enabled = body(assembly["enabled"], name)
            probe = " ".join(enabled)
            if not re.search(required, probe):
                raise ValueError(f"{name}: expected {required!r} in {enabled}")
            if re.search(forbidden, probe):
                raise ValueError(f"{name}: forbidden {forbidden!r} in {enabled}")
            control = CONTROL.get(kind, "disabled-subpass")
            off = body(assembly[control], name)
            if enabled == off:
                raise ValueError(f"{name}: the {control} control changed nothing")
            if kind in NO_COUNT_REDUCTION:
                if not re.search(DIVIDER, " ".join(off)):
                    raise ValueError(f"{name}: control still has no divider to remove")
            elif len(enabled) >= len(off):
                raise ValueError(f"{name}: no instruction reduction ({len(enabled)} vs {len(off)})")
            improved.append(name)
            measurements.append(
                {
                    "name": name,
                    "idiom": kind,
                    "divider_in_control": bool(re.search(DIVIDER, " ".join(off))),
                    "instructions": {
                        label: len(body(text, name)) for label, text in assembly.items()
                    },
                }
            )
        for name, pattern in SURVIVORS.items():
            probe = " ".join(body(assembly["enabled"], name))
            if not re.search(pattern, probe):
                raise ValueError(f"{name}: clamp must survive, found {probe!r}")
        for name in sorted(PROMOTED):
            if body(assembly["enabled"], name) != body(assembly["disabled-subpass"], name):
                raise ValueError(f"{name}: documented promotion boundary started folding")
        report = {
            "forms": len(FORMS),
            "survivors": len(SURVIVORS),
            "promotion_boundary": sorted(PROMOTED),
            "correct": True,
            "timing_claim": False,
            "gcc_oracle": version,
            "comparisons": measurements,
            "improved_vs_disabled": len(improved),
            "note": "The disabled sub-pass control is a policy switch, not an immutable baseline.",
        }
        if args.json:
            Path(args.json).write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps(report))
    print(
        f"PASS ring identities: {len(FORMS)} folded forms, {len(SURVIVORS)} survivor, "
        f"{len(PROMOTED)} documented promotion boundary, vendor agreement, IR/SSA/RA, "
        "seven optimisation levels and three disable controls"
    )


if __name__ == "__main__":
    main()
