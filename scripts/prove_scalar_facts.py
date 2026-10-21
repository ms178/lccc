#!/usr/bin/env python3
"""Optional independent algebra checks, not a certificate of the Rust matcher.

All scalar word laws use exact bitvectors at 8/16/32/64 bits. The division
recombination law uses bounded mathematical integers and modular output to
avoid an expensive divider circuit; that distinction is retained in the report.
Expected counterexamples protect the refusal conditions. UNKNOWN always fails.
Install z3-solver separately; it is not a compiler dependency.
"""

import argparse
import json
from pathlib import Path
import z3 as z


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout-ms", type=int, default=10000)
    args = parser.parse_args()
    results = []

    def check(name, law, expected="unsat", domain="bitvector"):
        solver = z.Solver()
        solver.set(timeout=args.timeout_ms)
        solver.add(z.Not(law))
        actual = str(solver.check())
        row = {"law": name, "domain": domain, "expected": expected, "result": actual}
        if actual == "sat":
            row["counterexample"] = str(solver.model())
        if actual == "unknown":
            row["reason"] = solver.reason_unknown()
        results.append(row)
        print(name, actual, flush=True)

    for width in (8, 16, 32, 64):
        x, y, other = z.BitVecs(f"x{width} y{width} other{width}", width)
        p = z.Bool(f"p{width}")
        laws = {
            "231586/mask-implies-different": z.And(z.Or(p, x != 13), x & ~1 == 42)
            == (x & ~1 == 42),
            "231063/guarded-add": z.And(x == 0, x + y == other)
            == z.And(x == 0, y == other),
            "231063/guarded-sub": z.And(y == 0, x - y == 1) == z.And(y == 0, x == 1),
            "230800/modular-range-mask": z.And(
                z.ULT(x + 1, 3),
                x & ((1 << (width - 1)) | 1) == ((1 << (width - 1)) | 1),
            )
            == (x == (1 << width) - 1),
            "230800/even-two": z.And(x & 1 == 0, x - 2 == 0) == (x == 2),
            "229537/joint-guard-select": z.If(
                z.And(x == 1, y == 0), y != x, z.UGT(x, y)
            )
            == z.UGT(x, y),
            "229537/signed-zero-select": z.If(x < 0, False, x == 0) == (x == 0),
            "229332/available-inverse": z.Not(x == 0) == (x != 0),
            "217230/masked-trip": (z.LShR(x + 12, 2) + 1) & 3 == z.LShR(x, 2) & 3,
            "214397/range-bit": z.If(z.ULT(x, 13), z.If(x & 16 == 0, 1, 2), 0)
            == z.If(z.ULT(x, 13), 1, 0),
            "213708/lowbit-zero": ((x & -x) == 0) == (x == 0),
            "213708/lowbit-one": ((x & -x) == 1) == ((x & 1) == 1),
            "210131/boolean-or": (
                (z.If(x != 0, z.BitVecVal(1, width), z.BitVecVal(0, width)) | x) != 0
            )
            == (x != 0),
        }
        for name, law in laws.items():
            check(f"u{width}/{name}", law)
        for name, law in {
            "trip-nonmultiple": ((z.LShR(x + 11, 2) + 1) & 3) == (z.LShR(x, 2) & 3),
            "trip-visible-carry": ((z.LShR(x + 12, 2) + 1) & 7) == (z.LShR(x, 2) & 7),
            "nonboolean-select": z.If(p, 2, 3) == 3,
        }.items():
            check(f"u{width}/reject/{name}", law, "sat")
        n = z.Int(f"n{width}")
        check(
            f"u{width}/230021/shared-divrem",
            z.Implies(
                z.And(n >= 0, n < 1 << width),
                ((n / 6) * 12 + (n % 6) * 2) % (1 << width) == (n * 2) % (1 << width),
            ),
            domain="bounded unsigned integer / modular output",
        )
        if width >= 32:
            old = z.LShR(x, 4) * 24576 + ((x & 15) * 1536 & 30720)
            check(f"u{width}/230021/masked-divrem", old == (x * 1536 & -2048))
            bad = z.LShR(x, 4) * 24576 + ((x & 15) * 1536 & 28672)
            check(f"u{width}/reject/masked-divrem", bad == (x * 1536 & -2048), "sat")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps({"solver": z.get_version_string(), "results": results}, indent=2)
        + "\n"
    )
    if any(r["expected"] != r["result"] for r in results):
        raise SystemExit("proof mismatch or UNKNOWN")


if __name__ == "__main__":
    main()
