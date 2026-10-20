#!/usr/bin/env python3
"""Scoped bit-vector lemmas; not a certificate of the Rust implementation."""

import argparse
import json
from pathlib import Path
import z3


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--json", type=Path, required=True)
    args = parser.parse_args()
    results = []

    def prove(name, assumptions, proposition):
        solver = z3.Solver()
        solver.set(timeout=10000)
        solver.add(*assumptions, z3.Not(proposition))
        result = solver.check()
        assert result == z3.unsat, (name, result)
        results.append({"name": name, "negated_lemma": str(result)})

    for width in (32, 64):
        x, y = z3.BitVecs(f"x{width} y{width}", width)
        for k in range(1, width - 1):
            mask = (1 << k) - 1
            exact = [(x & mask) == 0]
            prove(f"i{width}/2^{k}", exact, x / (1 << k) == x >> k)
            prove(f"i{width}%2^{k}", exact, z3.SRem(x, 1 << k) == 0)
        for name, other in [("add", x + y), ("sub", x - y), ("xor", x ^ y)]:
            prove(
                f"i{width} opposite parity {name}",
                [(y & 1) == 1],
                ((x * other) & 1) == 0,
            )
        prove(f"i{width} complement product even", [], ((x * ~x) & 1) == 0)
        # Representative known-low-zero transfer lemmas, distinct from the
        # all-supported-divisor core proof above. Width and signedness matter.
        assumptions = [(x & 7) == 0, (y & 3) == 0]
        for name, value, mask in [
            ("and", x & y, 7),
            ("or", x | y, 3),
            ("xor", x ^ y, 3),
            ("add", x + y, 3),
            ("sub", x - y, 3),
            ("mul", x * y, 31),
            ("shl", x << 2, 31),
            ("ashr", x >> 1, 3),
            ("lshr", z3.LShR(x, 1), 3),
        ]:
            prove(f"i{width} transfer {name}", assumptions, (value & mask) == 0)
    args.json.write_text(
        json.dumps({"scope": __doc__, "checks": results, "complete": True}, indent=2)
        + "\n"
    )
    print(f"{len(results)} scoped bit-vector checks PASS")


if __name__ == "__main__":
    main()
