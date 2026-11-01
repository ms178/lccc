#!/usr/bin/env python3
"""Emit a C stress program for the strided-2 i32 sum-reduction vectorizer.

Every (trip count, lane expression, base) triple is a separate noinline
function in the exact form `src/passes/vectorize_strided_i32_reduce.rs`
admits (`for (uint64_t i = 0; i < CONST; i += 8) acc += f(dword(b + i), i)`).
The trip counts straddle every boundary the transform reasons about: the
four-vector admission floor, the AVX2 4x and SSE2 2x unroll, and the scalar
remainder.  `fg*` reads a locally binding global (the whole vector prefix is
provably in bounds); `fp*` reads a run-time pointer at two misalignments (only
the scalar loop's own bytes may be read); `fm*` combines both streams in one
expression, so a proven wide load and an exact gather share the lane layout.  The program prints a running hash
per trip count, so a single wrong lane anywhere changes the output that the
gate compares against GCC.

Usage: strided_reduce_stress.py OUT.c
"""
import sys

TRIPS = (1, 3, 4, 5, 7, 8, 9, 12, 13, 15, 16, 17, 31, 32, 33, 64, 65, 511, 513)

# Lane expressions: the xor-with-IV shape of byte_load_reassembly, a
# displaced load with an invariant and a multiply by 3 (a shift and an add),
# shifts/sub/or, and/arithmetic-shift with an invariant, and a multiply by a
# constant with no shift-add form (the generic lane multiply: pmuludq pairs
# on SSE2, vpmulld on AVX2).
EXPRS = {
    "x": "(*(const uint32_t *)(b + i) ^ (uint32_t)i)",
    "m": "((*(const uint32_t *)(b + i + 4) + k) * 3u)",
    "s": "(((*(const uint32_t *)(b + i) << 3) - (uint32_t)i)"
         " | (*(const uint32_t *)(b + i) >> 5))",
    "a": "((*(const uint32_t *)(b + i) & k)"
         " ^ (uint32_t)((int32_t) * (const uint32_t *)(b + i) >> 7))",
    "h": "((*(const uint32_t *)(b + i) * 2654435761u) ^ k)",
}


def main() -> None:
    lines = [
        "#include <stdint.h>",
        "#include <stdio.h>",
        "static uint32_t g[2100];",
        "unsigned char *volatile gp = (unsigned char *)g;",
    ]
    for n in TRIPS:
        for tag, expr in EXPRS.items():
            lines.append(
                f"__attribute__((noinline)) static uint32_t fg{tag}{n}(uint32_t k) {{"
                " const unsigned char *b = (const unsigned char *)g; uint32_t acc = 5;"
                f" for (uint64_t i = 0; i < {8 * n}U; i += 8) acc += {expr};"
                " return acc; }")
            lines.append(
                f"__attribute__((noinline)) static uint32_t fp{tag}{n}"
                "(const unsigned char *b, uint32_t k) { uint32_t acc = 0;"
                f" for (uint64_t i = 0; i < {8 * n}U; i += 8) acc += {expr};"
                " return acc; }")
        lines.append(
            f"__attribute__((noinline)) static uint32_t fm{n}"
            "(const unsigned char *b, uint32_t k) {"
            " const unsigned char *gb = (const unsigned char *)g; uint32_t acc = k;"
            f" for (uint64_t i = 0; i < {8 * n}U; i += 8)"
            " acc += (*(const uint32_t *)(gb + i + 4) ^ *(const uint32_t *)(b + i))"
            " - (uint32_t)i;"
            " return acc; }")
    lines.append("int main(void) {")
    lines.append("    uint32_t h = 0;")
    lines.append("    for (unsigned i = 0; i < 2100; i++) g[i] = i * 2654435761u ^ (i >> 3);")
    for n in TRIPS:
        for tag in EXPRS:
            lines.append(f"    h = h * 31 + fg{tag}{n}(h | 1);")
            lines.append(f"    h = h * 31 + fp{tag}{n}(gp + 4, h);")
            lines.append(f"    h = h * 31 + fp{tag}{n}(gp + 12, ~h);")
        lines.append(f"    h = h * 31 + fm{n}(gp + 4, h);")
        lines.append(f"    h = h * 31 + fm{n}(gp + 12, ~h);")
        lines.append(f'    printf("%u %x\\n", {n}u, h);')
    lines.append("    return 0;")
    lines.append("}")
    with open(sys.argv[1], "w", encoding="utf-8") as out:
        out.write("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
