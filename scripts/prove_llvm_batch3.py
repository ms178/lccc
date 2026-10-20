#!/usr/bin/env python3
"""Optional Z3 cross-checks, not a proof of the Rust implementation.

Word-level demanded-bit laws use exact 32/64-bit bitvectors. Cardinality
lemmas are per-bit truth tables, whose sums establish the stated word laws.
Remainder lemmas use mathematical integer remainders. Count-position laws
use 8-bit exhaustive symbolic domains (not a 32/64-bit formal certificate).
Every UNKNOWN is reported and fails the command; no timeout is called a proof.
Install z3-solver separately; this is deliberately not a compiler dependency.
"""

import argparse
import ast
import re
import json
from pathlib import Path
import z3 as z


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout-ms", type=int, default=5000)
    args = parser.parse_args()
    results = []

    def check(name, claim, expected="unsat"):
        solver = z.Solver()
        solver.set(timeout=args.timeout_ms)
        solver.add(z.Not(claim))
        status = str(solver.check())
        record = {"law": name, "expected": expected, "result": status}
        if status == "sat":
            record["counterexample"] = str(solver.model())
        if status == "unknown":
            record["reason"] = solver.reason_unknown()
        results.append(record)
        print(name, status, flush=True)

    for w in (32, 64):
        x, y = z.BitVecs(f"x{w} y{w}", w)
        a, b = x & 255, y & 255
        laws = {
            "add": ((a + b) & 15) == ((x + y) & 15),
            "sub": ((a - b) & 15) == ((x - y) & 15),
            "mul": ((a * b) & 15) == ((x * y) & 15),
            "mul-trailing-zero-factor": (((x & 15) * 1536) & 1536)
            == ((x * 1536) & 1536),
            "left-shift": (((x & 255) << 7) & 1536) == ((x << 7) & 1536),
            "logical-right": (z.LShR(x & 0xFF00, 8) & 15) == (z.LShR(x, 8) & 15),
            "arithmetic-right": (((x & ~127) >> 7) & 15) == ((x >> 7) & 15),
            "rotate": (z.RotateLeft(x & 65535, 8) & 0xF00)
            == (z.RotateLeft(x, 8) & 0xF00),
            "not": (~a & 15) == (~x & 15),
            "negate": (-a & 15) == (-x & 15),
            "xor": ((a ^ b) & 15) == ((x ^ y) & 15),
            "or": ((a | b) & 15) == ((x | y) & 15),
            "and": ((a & b) & 15) == ((x & y) & 15),
        }
        for name, law in laws.items():
            check(f"word{w}/{name}", law)
        check(
            f"word{w}/required-sign-demand",
            (((x & 255) >> 7) & (1 << (w - 1))) == ((x >> 7) & (1 << (w - 1))),
            "sat",
        )
    n = z.Int("n")
    for divisor, modulus in ((48, 16), (80, 16), (768, 256), (48, 4)):
        check(
            f"integer/remainder-{divisor}-mod-{modulus}",
            z.Implies(n >= 0, (n % divisor) % modulus == n % modulus),
        )
    check(
        "guard/nondivisible-remainder", z.Implies(n >= 0, (n % 49) % 4 == n % 4), "sat"
    )

    x, y = z.BitVecs("x8 y8", 8)
    low = lambda a: a & -a
    trailing = {
        "odd-constant": x * 3,
        "odd-variable": x * (y | 1),
        "negate": -x,
        "isolate": x & -x,
        "or-negate": x | -x,
        "or-left": x | (x << 3),
        "xor-left": x ^ (x << 3),
        "add-left": x + (x << 3),
        "sub-left": x - (x << 3),
        "and-not-left": x & ~(x << 3),
    }
    for name, expr in trailing.items():
        check("domain8/ctz/" + name, low(expr) == low(x))

    def clz(a):
        result = z.IntVal(8)
        for i in range(8):
            result = z.If(z.Extract(i, i, a) == 1, 7 - i, result)
        return result

    for name, expr in {
        "or-right": x | z.LShR(x, 3),
        "xor-right": x ^ z.LShR(x, 3),
        "and-not-right": x & ~z.LShR(x, 3),
    }.items():
        check("domain8/clz/" + name, clz(expr) == clz(x))
    check("guard/even-factor", low(x * 6) == low(x), "sat")
    check("guard/zero-shift", low(x ^ (x << 0)) == low(x), "sat")
    check("guard/wrong-direction", clz(x | (x << 3)) == clz(x), "sat")

    # A sum of these single-bit identities proves the cardinality laws.
    a, b = z.Bools("a b")
    bit = lambda p: z.If(p, 1, 0)
    check("bit/complement", bit(a) + bit(z.Not(a)) == 1)
    check("bit/partition", bit(z.And(a, b)) + bit(z.And(a, z.Not(b))) == bit(a))
    check(
        "bit/symmetric-difference",
        bit(z.Or(a, b)) - bit(z.And(a, b)) == bit(z.Xor(a, b)),
    )
    check("bit/parity", (bit(a) + bit(b)) % 2 == bit(z.Xor(a, b)))
    # The extra cardinality witnesses are verified as exact per-bit integer
    # identities. Parse only a tiny arithmetic/Boolean grammar, never eval C.
    x1, y1 = z.BitVecs("x1 y1", 1)

    def expression(text):
        text = re.sub(r"\b32u\b", "1", text)
        text = re.sub(r"\b([0-9]+)u\b", r"\1", text)

        def visit(node):
            if isinstance(node, ast.Name) and node.id in ("x", "y"):
                return x1 if node.id == "x" else y1
            if isinstance(node, ast.Constant) and isinstance(node.value, int):
                return node.value
            if isinstance(node, ast.UnaryOp) and isinstance(node.op, ast.Invert):
                return ~visit(node.operand)
            if (
                isinstance(node, ast.Call)
                and isinstance(node.func, ast.Name)
                and node.func.id == "__builtin_popcount"
                and len(node.args) == 1
                and not node.keywords
            ):
                return z.If(visit(node.args[0]) == 1, 1, 0)
            if isinstance(node, ast.BinOp):
                a, b = visit(node.left), visit(node.right)
                operations = {
                    ast.Add: lambda: a + b,
                    ast.Sub: lambda: a - b,
                    ast.Mult: lambda: a * b,
                    ast.BitAnd: lambda: a & b,
                    ast.BitOr: lambda: a | b,
                    ast.BitXor: lambda: a ^ b,
                }
                if type(node.op) in operations:
                    return operations[type(node.op)]()
            raise ValueError(f"unsupported proof expression: {ast.dump(node)}")

        return visit(ast.parse(text, mode="eval").body)

    contracts = json.loads(
        (
            Path(__file__).resolve().parents[1]
            / "tests/regression/llvm_missed_batch3.json"
        ).read_text()
    )
    for case in contracts[42:]:
        check(
            "bit/cardinality/" + case["name"],
            expression(case["expression"]) == expression(case["expected"]),
        )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(results, indent=2) + "\n")
    failed = [r for r in results if r["result"] != r["expected"]]
    if failed:
        raise SystemExit(f"FAIL: {len(failed)} unknown/unexpected results")
    print(f"PASS {len(results)} checks, including essential-guard counterexamples")


if __name__ == "__main__":
    main()
