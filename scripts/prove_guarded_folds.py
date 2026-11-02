#!/usr/bin/env python3
"""Independent bit-vector checks, not a certificate for the compiler matchers."""
import argparse
import json
from pathlib import Path
import z3


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--json", type=Path, required=True)
    args = ap.parse_args()
    results = []

    def check(name, bits, constraints, different, expected):
        s = z3.Solver()
        s.set(timeout=10000)
        s.add(*constraints, different)
        got = str(s.check())
        row = {"name": name, "bits": bits, "expected": expected, "result": got}
        if got == "sat":
            row["witness"] = str(s.model())
        results.append(row)
        if got != expected:
            raise AssertionError(row)

    for w in (8, 16, 32, 64):
        x, b, y = z3.BitVecs(f"x{w} b{w} y{w}", w)
        old = (x ^ -b) + b
        new = z3.If(b != 0, -x, x)
        check("conditional-negate", w, [z3.ULE(b, 1)], old != new, "unsat")
        check("truthiness-is-not-boolean", w, [], old != new, "sat")
        lower_and_upper = z3.And(x >= 0, x < y)
        check("nonnegative-upper-range", w, [y >= 0], lower_and_upper != z3.ULT(x, y), "unsat")
        check("negative-upper-is-not-admitted", w, [], lower_and_upper != z3.ULT(x, y), "sat")
        edge = z3.Bool(f"edge{w}")
        pointer = z3.If(edge, z3.BitVecVal(0, w), x)
        condition = z3.If(edge, b, z3.BitVecVal(1, w))
        old_phi = z3.And(condition != 0, pointer != 0)
        new_phi = z3.If(edge, z3.BoolVal(False), x != 0)
        check("null-enabled-phi-observation", w, [], old_phi != new_phi, "unsat")
    src = z3.BitVec("src64", 64)
    mask = z3.BitVec("mask64", 64)
    masked = src & mask
    extended = z3.SignExt(32, z3.Extract(31, 0, masked))
    check("sign-extension-full-identity", 64, [z3.ULE(mask, 0x7fffffff)], masked != extended, "unsat")
    check("zero-upper32-alone-is-insufficient", 64, [z3.ULE(mask, 0xffffffff)], masked != extended, "sat")
    low_copy = z3.ZeroExt(32, z3.Extract(31, 0, src))
    check("movl-low-read-substitution", 32, [], z3.Extract(31, 0, low_copy) != z3.Extract(31, 0, src), "unsat")
    check("movl-address-substitution-needs-upper-proof", 64, [], low_copy != src, "sat")
    count = z3.BitVec("shift_count", 32)
    old_zf = z3.Bool("incoming_zf")
    value = z3.BitVec("shift_value", 32)
    output = z3.LShR(value, count & 31)
    flags = z3.If(count & 31 == 0, old_zf, output == 0)
    check("effective-zero-shift-needs-test", 32, [], flags != (output == 0), "sat")
    check("nonzero-effective-shift-zf", 32, [count & 31 != 0], flags != (output == 0), "unsat")
    args.json.parent.mkdir(parents=True, exist_ok=True)
    args.json.write_text(json.dumps({"solver": z3.get_version_string(), "scope": __doc__, "results": results}, indent=2) + "\n")
    print(f"PASS {len(results)} solver outcomes; no UNKNOWN")


if __name__ == "__main__":
    main()
