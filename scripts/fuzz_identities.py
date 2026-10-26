#!/usr/bin/env python3
"""Randomised differential red-team for the scalar ring identities.

Manual evidence producer, not a CI gate: one run compiles and executes hundreds
of programs with three compilers. Run it after touching any rule in
`src/passes/bit_idioms_identities.rs` or the power-of-two paths of
`src/passes/div_by_const.rs`:

    python3 scripts/fuzz_identities.py 100

Exit status is non-zero if any program's digest differs between the three
builds; the failing program is written to `research/fuzz_fail_<seed>.c`.

Each generated program draws from the idiom templates below and is compiled three
ways -- folds on, folds off (kill switches), GCC 16.2 -- then executed over a
value matrix. Any disagreement is a miscompile.

Shapes: remainder window compares, sign-mask shifts (both mask forms, runtime
counts), complement of negation, saturated min/max selects, zero-test selects in
all four arm orders, or-chain selects, boolean products, and power-of-two
remainder and division in both widths.
"""
import os
import random
import subprocess
import sys
import tempfile
from pathlib import Path

LCCC = "/home/user/lccc/target/fastbuild/lccc"
GCC = "/home/user/.local/gcc16/gcc-16/bin/gcc-16"

WIDTHS = {"int8_t": 8, "uint8_t": 8, "int16_t": 16, "uint16_t": 16, "int32_t": 32,
          "uint32_t": 32, "int64_t": 64, "uint64_t": 64}
SIGNED = ["int8_t", "int16_t", "int32_t", "int64_t"]
UNSIGNED = ["uint8_t", "uint16_t", "uint32_t", "uint64_t"]

DIVISORS = {
    8: [2, 3, 7, 100, 127, -2, -7, -128],
    16: [2, 3, 250, 1000, 32767, -2, -250, -32768],
    32: [2, 3, 7, 250, 1000000, 2147483647, -2, -250, -2147483648],
    64: [2, 3, 7, 250, 1000000, 9223372036854775807, -250],
}

EXTREMUM = {
    ("min", 8, True): "127", ("max", 8, True): "(-128)" if False else "-128",
    ("min", 16, True): "32767", ("max", 16, True): "-32768",
    ("min", 32, True): "2147483647", ("max", 32, True): "(-2147483648)",
    ("min", 64, True): "9223372036854775807LL", ("max", 64, True): "(-9223372036854775807LL - 1)",
    ("min", 8, False): "255u", ("max", 8, False): "0",
    ("min", 16, False): "65535u", ("max", 16, False): "0",
    ("min", 32, False): "4294967295u", ("max", 32, False): "0",
    ("min", 64, False): "18446744073709551615ULL", ("max", 64, False): "0",
}

# (signature, body) with `f` as the placeholder name.
def remainder_case(ty, divisor, cmp, swapped):
    lhs = "x"
    rhs = f"({ty})(x % ({ty})({divisor}))"
    if swapped:
        lhs, rhs = rhs, lhs
    op = "==" if cmp == "eq" else "!="
    return ("remainder", ty, f"{ty} f({ty} x)", f"return ({lhs}) {op} ({rhs});")


def shift_case(ty, asr_mask, swapped):
    w = WIDTHS[ty]
    mask = f"({ty})(x >> {w - 1})" if asr_mask else f"({ty})(-(x >> {w - 1}))"
    inner = f"(({mask}) ^ x)" if swapped else f"(x ^ ({mask}))"
    return (
        "shift",
        ty,
        f"{ty} f({ty} x, int n)",
        f"{ty} m = {mask}; return ({ty})((({ty})({inner}) >> n) ^ m);",
    )


def complement_case(ty, shape):
    expr = "(~(-x))" if shape == 0 else "(-(~x))"
    return ("complement", ty, f"{ty} f({ty} x)", f"return ({ty}){expr};")


def bound_case(ty, kind, x_first):
    signed = ty.startswith("int")
    extremum = EXTREMUM[(kind, WIDTHS[ty], signed)]
    cond = "<" if kind == "min" else ">"
    arms = ("x", extremum) if x_first else (extremum, "x")
    return (
        "bound",
        ty,
        f"{ty} f({ty} x)",
        f"return ({ty})x {cond} ({ty})({extremum}) ? ({ty})({arms[0]}) : ({ty})({arms[1]});",
    )


ZERO_TESTS = [
    "return x != 0 ? x : 0;",
    "return x == 0 ? 0 : x;",
    "return x != 0 ? 0 : x;",
    "return x == 0 ? x : 0;",
]


def zero_test_case(ty, order):
    return ("zero", ty, f"{ty} f({ty} x)", ZERO_TESTS[order])


def or_chain_case(ty, arm):
    body = "return (x | y | z) ? x : 0;" if arm == 0 else "return (x | y | z) ? 0 : x;"
    return ("orchain", ty, f"{ty} f({ty} x, {ty} y, {ty} z)", body)


def nested_select_case(ty):
    """`(a != 0 || b) ? a : 0`, whose condition the front end flattens into
    selects: the surviving arm is the value and the other arm is zero."""
    return ("nselect", ty, f"{ty} f({ty} a, {ty} b)",
            f"return ({ty})((a != 0 || b) ? a : 0);")


def bool_product_case(ty, shape):
    body = [
        "return (ty_)(x * (x || (y != 0)));",
        "return (ty_)(x * (y != 0));",
        "return (ty_)((x < y) * x);",
        "return (ty_)(x * ((x != 0) | ~(y < y + 1)));",
    ][shape].replace("ty_", ty)
    return ("boolmul", ty, f"{ty} f({ty} x, {ty} y)", body)


def bool_product_and_case(ty):
    """Three-operand product whose boolean factor is a nested and/or tree."""
    return ("boolmul", ty, f"{ty} f({ty} x, {ty} y, {ty} z)",
            f"return ({ty})(x * (x || (y && z)));")


def pow2_case(ty, k, rem, negate):
    d = (1 << k) * (-1 if negate else 1)
    op = "%" if rem else "/"
    return ("pow2", ty, f"{ty} f({ty} x)", f"return ({ty})(x {op} ({ty})({d}));")


def build(seed):
    rng = random.Random(seed)
    cases = []
    for _ in range(3):
        ty = rng.choice(SIGNED)
        cases.append(remainder_case(ty, rng.choice(DIVISORS[WIDTHS[ty]]),
                                    rng.choice(["eq", "ne"]), rng.random() < 0.5))
    for _ in range(3):
        ty = rng.choice(UNSIGNED)
        cases.append(shift_case(ty, rng.random() < 0.5, rng.random() < 0.5))
    for _ in range(2):
        cases.append(complement_case(rng.choice(UNSIGNED), rng.randrange(2)))
    for _ in range(2):
        ty = rng.choice(SIGNED + UNSIGNED)
        cases.append(bound_case(ty, rng.choice(["min", "max"]), rng.random() < 0.5))
    for _ in range(2):
        cases.append(zero_test_case(rng.choice(SIGNED + UNSIGNED), rng.randrange(4)))
    cases.append(or_chain_case(rng.choice(SIGNED + UNSIGNED), rng.randrange(2)))
    for _ in range(2):
        ty = rng.choice(SIGNED)
        cases.append(pow2_case(ty, rng.randrange(1, WIDTHS[ty] - 1),
                               rng.random() < 0.5, rng.random() < 0.5))
    for _ in range(2):
        ty = rng.choice(SIGNED + UNSIGNED)
        cases.append(nested_select_case(ty))
    for shape in range(4):
        ty = rng.choice(UNSIGNED if shape != 3 else SIGNED + UNSIGNED)
        cases.append(bool_product_case(ty, shape))
    cases.append(bool_product_and_case(rng.choice(SIGNED + UNSIGNED)))
    return cases


VALUES = {
    8: ["0", "1", "2", "-1", "127", "-128", "-127", "100", "-100", "3", "-3"],
    16: ["0", "1", "-1", "32767", "-32768", "-32767", "250", "-250", "1000", "-1000"],
    32: ["0", "1", "-1", "2147483647", "-2147483647", "-2147483648", "250", "-250",
         "1000000", "-12345"],
    64: ["0", "1", "-1", "9223372036854775807LL", "-9223372036854775807LL",
         "250", "-250", "1000000", "-1099511627776LL"],
}
COUNTS = [0, 1, 3, 7, 15, 31, 33, 63, 65]


def safe_values(kind, ty):
    """Values that keep the generated program free of undefined behaviour.

    Signed negation, signed multiply and division by `-1` are only defined away
    from the INT_MIN edge, and a shift count must stay below the width, so each
    kind receives its own matrix.
    """
    width = WIDTHS[ty]
    values = VALUES[width]
    if kind in {"complement", "mulbool"} or ty.startswith("int"):
        values = [v for v in values if "2147483648" not in v and "9223372036854775808" not in v
                  and v not in {"-9223372036854775807LL"}]
    return values


def program(cases):
    lines = [
        "#include <stdio.h>",
        "#include <stdint.h>",
        "static unsigned long long h = 1469598103934665603ULL;",
        "static void mix(unsigned long long v){ h ^= v; h *= 1099511628211ULL; }",
    ]
    for index, (_, _, signature, body) in enumerate(cases):
        lines.append(signature.replace("f(", f"f_{index}(") + " { " + body + " }")
    lines.append("int main(void){")
    for index, (kind, case_ty, signature, _) in enumerate(cases):
        params = signature.split("(")[1].rstrip(")").split(", ")
        ty = params[0].split()[0]
        values = safe_values(kind, ty)
        if len(params) == 2 and params[1] == "int n":
            for value in values:
                for count in [c for c in COUNTS if c < WIDTHS[ty]]:
                    lines.append(f"  mix((unsigned long long)f_{index}(({ty}){value}, {count}));")
        else:
            if len(params) == 1:
                combos = [(v,) for v in values]
            elif len(params) == 2:
                combos = [(v, w) for v in values for w in values]
            else:
                combos = [(v, w, z) for v in values for w in values[:3] for z in values[:2]]
            for combo in combos:
                args = ", ".join(f"({ty}){part}" for part in combo)
                lines.append(f"  mix((unsigned long long)f_{index}({args}));")
    lines.append('  printf("%016llx\\n", h); return 0; }')
    return "\n".join(lines) + "\n"


def main():
    rounds = int(sys.argv[1]) if len(sys.argv) > 1 else 40
    failures = 0
    off = {"CCC_NO_RING_IDENTITIES": "1", "CCC_DISABLE_PASSES": "bit_idioms,div_by_const"}
    with tempfile.TemporaryDirectory() as tmpdir:
        tmp = Path(tmpdir)
        for seed in range(rounds):
            source = program(build(seed))
            path = tmp / "fuzz.c"
            path.write_text(source)
            outputs = {}
            for label, compiler, extra in (("on", LCCC, {}), ("off", LCCC, off), ("gcc", GCC, {})):
                binary = tmp / f"{label}.bin"
                build_result = subprocess.run(
                    [compiler, "-O2", "-w", str(path), "-o", str(binary)],
                    capture_output=True, text=True, env={**os.environ, **extra},
                )
                if build_result.returncode:
                    print(f"seed {seed}: {label} build failed: {build_result.stderr.strip()[:300]}")
                    failures += 1
                    outputs = {}
                    break
                run = subprocess.run([str(binary)], capture_output=True, text=True)
                outputs[label] = run.stdout.strip() if run.returncode == 0 else f"rc={run.returncode}"
            if not outputs:
                continue
            if len(set(outputs.values())) != 1:
                print(f"seed {seed}: MISMATCH {outputs}")
                Path(f"/home/user/research/fuzz_fail_{seed}.c").write_text(source)
                failures += 1
    print(f"{rounds} programs, {failures} failures")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
