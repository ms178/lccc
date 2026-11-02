#!/usr/bin/env python3
"""Emit a C stress program for the vector-fill pass (src/passes/vector_fill.rs).

Every (element type, value kind, exit test) triple is a noinline fill
function; main calls each one for trip counts straddling every vector step
boundary (0 .. 3 steps + remainder, plus large counts), at three
misalignments and a nonzero displacement, inside a guard-filled arena.  It
then hashes the WHOLE arena, so a store past either end of the fill window,
a wrong lane pattern, or a skipped remainder element changes the printed
hash that the gate compares against GCC.

Usage: vector_fill_stress.py OUT.c
"""
import sys

# (C type, constant literal, runtime-value expression from `seed`)
TYPES = [
    ("uint16_t", "0xBEEF", "(uint16_t)(seed * 40503u)"),
    ("int16_t", "-2", "(int16_t)(seed ^ 0x8001u)"),
    ("uint32_t", "0xFFFFu", "seed * 2654435761u"),
    ("int32_t", "-123456", "(int32_t)(seed ^ 0x80000001u)"),
    ("uint64_t", "0x0123456789ABCDEFull", "((uint64_t)seed << 32) ^ 0x9E3779B97F4A7C15ull"),
    ("int64_t", "-0x7EDCBA9876543210ll", "-(int64_t)seed * 1000003ll"),
    ("float", "1.5f", None),
    ("double", "-0.25", None),
]

# Exit tests: (name, index type, loop header given bound expression `n`).
LOOPS = [
    ("ult", "unsigned", "for (unsigned i = 0; i < n; i++)"),
    ("slt", "int", "for (int i = 0; i < n; i++)"),
    ("szt", "size_t", "for (size_t i = 0; i < n; i++)"),
    ("ne", "unsigned", "for (unsigned i = 0; i != n; i++)"),
]

COUNTS = [0, 1, 2, 3, 4, 5, 7, 8, 9, 15, 16, 17, 23, 24, 25, 31, 32, 33, 63, 64, 65, 100, 1000]
CONST_COUNTS = [16, 17, 40, 333, 1024]


def main() -> None:
    out = [
        "#include <stddef.h>",
        "#include <stdint.h>",
        "#include <stdio.h>",
        "#include <string.h>",
        "static unsigned char arena[16384] __attribute__((aligned(64)));",
        "static uint64_t hash_arena(uint64_t h) {",
        "    for (size_t i = 0; i < sizeof arena; i++) h = (h ^ arena[i]) * 0x100000001B3ull;",
        "    return h;",
        "}",
    ]
    calls = []
    for ti, (cty, lit, rt) in enumerate(TYPES):
        for lname, ity, head in LOOPS:
            vals = [("c", lit)] + ([("r", rt)] if rt else [])
            for vk, vexpr in vals:
                fn = f"f{ti}_{lname}_{vk}"
                out.append(
                    f"__attribute__((noinline)) static void {fn}({cty} *p, {ity} n, uint32_t seed) {{"
                    f" (void)seed; {head} p[i] = {vexpr}; }}")
                calls.append((fn, cty, ity, lname))
            # A compile-time bound and a displaced window: p[i + 3].
            for n in CONST_COUNTS:
                fn = f"k{ti}_{n}"
                out.append(
                    f"__attribute__((noinline)) static void {fn}({cty} *p) {{"
                    f" for (unsigned i = 0; i < {n}u; i++) p[i + 3] = {lit}; }}")
                calls.append((fn, cty, None, None))
            break  # constant-bound functions once per type, not per loop
    for ti, (cty, lit, rt) in enumerate(TYPES):
        for lname, ity, head in LOOPS[1:]:
            vals = [("c", lit)] + ([("r", rt)] if rt else [])
            for vk, vexpr in vals:
                fn = f"f{ti}_{lname}_{vk}"
                if not any(c[0] == fn for c in calls):
                    out.append(
                        f"__attribute__((noinline)) static void {fn}({cty} *p, {ity} n, uint32_t seed) {{"
                        f" (void)seed; {head} p[i] = {vexpr}; }}")
                    calls.append((fn, cty, ity, lname))
    out.append("int main(void) {")
    out.append("    uint64_t h = 0xCBF29CE484222325ull;")
    out.append("    uint32_t seed = 12345;")
    for fn, cty, ity, lname in calls:
        if ity is None:
            out.append("    memset(arena, 0xA5, sizeof arena);")
            out.append(f"    {fn}(({cty} *)(arena + 64));")
            out.append(f"    {fn}(({cty} *)(arena + 4096 + 1));")
            out.append("    h = hash_arena(h);")
            continue
        out.append("    for (unsigned k = 0; k < %du; k++) {" % len(COUNTS))
        out.append("        static const unsigned counts[] = {%s};" % ", ".join(map(str, COUNTS)))
        out.append("        memset(arena, 0x5A, sizeof arena);")
        out.append("        seed = seed * 1103515245u + 12345u;")
        out.append(f"        {fn}(({cty} *)(arena + 128), ({ity})counts[k], seed);")
        out.append(f"        {fn}(({cty} *)(arena + 4096 + 3), ({ity})counts[k], seed);")
        out.append(f"        {fn}(({cty} *)(arena + 9000 + 2), ({ity})counts[k], seed);")
        out.append("        h = hash_arena(h);")
        out.append("    }")
        if lname == "slt":
            # A negative signed bound runs zero iterations.
            out.append("    memset(arena, 0x3C, sizeof arena);")
            out.append(f"    {fn}(({cty} *)(arena + 256), -5, seed);")
            out.append("    h = hash_arena(h);")
        out.append(f'    printf("{fn} %016llx\\n", (unsigned long long)h);')
    out.append("    return 0;")
    out.append("}")
    with open(sys.argv[1], "w", encoding="utf-8") as f:
        f.write("\n".join(out) + "\n")


if __name__ == "__main__":
    main()
