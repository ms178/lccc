#!/usr/bin/env python3
"""Scalar count composition: native, refusal, IR/RA and baseline mechanisms."""
import argparse
import os
from pathlib import Path
import re
import tempfile
from check_llvm_missed_zero_choice import body, run

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "tests/regression/llvm_word_counts.c"
POSITIVE = ("count_min_trailing", "count_min_leading", "count_cap_trailing", "count_cap_leading")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--compiler", default=os.environ.get("CCC", str(ROOT / "target/fastbuild/lccc")))
    ap.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    ap.add_argument("--clang")
    ap.add_argument("--baseline")
    args = ap.parse_args()
    banner = run([args.gcc, "--version"]).decode().splitlines()
    banner = banner[0] if banner else "<no output>"
    if not re.search(r"\b16\.2(?:\.0)?\b", banner):
        # Name the version that IS installed: "GCC 16.2 required" alone cannot
        # distinguish a lane provisioned with 16.3 from one whose `gcc-16` is
        # not on PATH at all, and the two have different fixes.
        raise ValueError(f"GCC 16.2 required, found: {banner!r} ({args.gcc})")
    if args.clang and not re.search(r"\b23\.1\.3\b", run([args.clang, "--version"]).decode()):
        raise ValueError("Clang 23.1.3 required")
    env = os.environ | {"CCC_VERIFY_IR": "abort", "CCC_VERIFY_REGALLOC": "1", "CCC_VALIDATE_SSA": "1"}
    with tempfile.TemporaryDirectory() as tmp:
        p = Path(tmp)
        run([args.gcc, "-O2", "-Wall", "-Wextra", "-Werror", str(SOURCE), "-o", str(p / "gcc")])
        expected = run([str(p / "gcc")])
        if args.clang:
            run([args.clang, "-O2", "-Wall", "-Wextra", "-Werror", str(SOURCE), "-o", str(p / "clang")])
            if run([str(p / "clang")]) != expected:
                raise ValueError("vendor disagreement")
        for flags in [[o] for o in ("-O0", "-O1", "-O2", "-O3", "-Os")] + [["-O3", "-g"], ["-O2", "-march=x86-64-v3"]]:
            run([args.compiler, *flags, str(SOURCE), "-o", str(p / "native")], env=env)
            if run([str(p / "native")]) != expected:
                raise ValueError(f"native mismatch: {flags}")
        off = env | {"CCC_DISABLE_PASSES": "bit_idioms"}
        run([args.compiler, "-O2", str(SOURCE), "-o", str(p / "disabled")], env=off)
        if run([str(p / "disabled")]) != expected:
            raise ValueError("disable control mismatch")
        asm = {}
        for label, compiler, settings in [("on", args.compiler, env), ("off", args.compiler, off)] + ([("before", args.baseline, env)] if args.baseline else []):
            run([compiler, "-O2", "-S", str(SOURCE), "-o", str(p / f"{label}.s")], env=settings)
            asm[label] = (p / f"{label}.s").read_text()
        for name in POSITIVE:
            current = body(asm["on"], name)
            if any(line.startswith("cmov") for line in current):
                raise ValueError(f"count minimum survives: {name}: {current}")
            for label in asm.keys() - {"on"}:
                previous = body(asm[label], name)
                if len(current) >= len(previous):
                    raise ValueError(f"no mechanism reduction: {name} versus {label}")
                print(f"{name}: {len(previous)} -> {len(current)} instructions versus {label}")
        for name in ("count_mixed", "count_max", "count_shared"):
            scans = sum(bool(re.match(r"(?:bsf|bsr|tzcnt|lzcnt)[lq]?\b", line)) for line in body(asm["on"], name))
            if scans < 2:
                raise ValueError(f"refusal control lost its distinct scans: {name}")
    print("PASS scalar count composition: native/IR/RA/disable/mechanism/refusal controls")


if __name__ == "__main__":
    main()
