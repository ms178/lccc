#!/usr/bin/env python3
"""Independent count identities and refusal witnesses, not a compiler certificate."""
import argparse
import json
from pathlib import Path
import z3


def count(x, width, leading):
    result = z3.BitVecVal(width, width)
    # Later alternatives win: low bits for trailing, high bits for leading.
    for bit in (range(width) if leading else reversed(range(width))):
        result = z3.If(z3.Extract(bit, bit, x) == 1,
                       z3.BitVecVal(width - 1 - bit if leading else bit, width), result)
    return result


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--json", type=Path, required=True)
    args = ap.parse_args()
    rows = []

    def check(name, width, constraints, different, expected):
        solver = z3.Solver()
        solver.set(timeout=10000)
        solver.add(*constraints, different)
        result = str(solver.check())
        row = dict(name=name, bits=width, expected=expected, result=result)
        if result == "sat":
            row["witness"] = str(solver.model())
        rows.append(row)
        if result != expected:
            raise AssertionError(row)

    for width in (8, 16, 32, 64):
        x, y, cap = z3.BitVecs(f"x{width} y{width} cap{width}", width)
        for leading in (False, True):
            a, b = count(x, width, leading), count(y, width, leading)
            minimum = z3.If(z3.ULT(a, b), a, b)
            check("leading-union" if leading else "trailing-union", width, [],
                  minimum != count(x | y, width, leading), "unsat")
            bounded = z3.If(z3.ULT(a, cap), a, cap)
            bit = width - 1 - cap if leading else cap
            check("leading-cap" if leading else "trailing-cap", width,
                  [z3.ULT(cap, width)], bounded != count(x | (1 << bit), width, leading), "unsat")
        leading = count(x, width, True)
        bounded = z3.If(z3.ULT(leading, cap), leading, cap)
        check("off-by-one-leading-sentinel", width, [z3.ULT(cap, width)],
              bounded != count(x | (1 << (width - cap)), width, True), "sat")
        trailing = count(x, width, False)
        check("hardware-masked-out-of-range-cap", width, [cap == width],
              z3.If(z3.ULT(trailing, cap), trailing, cap) != count(x | (1 << (cap & (width - 1))), width, False), "sat")
    args.json.parent.mkdir(parents=True, exist_ok=True)
    args.json.write_text(json.dumps(dict(solver=z3.get_version_string(), scope=__doc__, results=rows), indent=2) + "\n")
    print(f"PASS {len(rows)} expected outcomes; no UNKNOWN")


if __name__ == "__main__":
    main()
