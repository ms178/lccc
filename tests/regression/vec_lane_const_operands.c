/* Packed-operand slots may hold a scalar lane constant.
 *
 * The BB-SLP vectorizer keeps `v * 1664525` literal instead of forcing a
 * materialised splat register, so every emitter that needs that operand in a
 * register must BROADCAST it. The generic load path used to hand the constant
 * to `operand_to_reg` and dereference it as an address:
 *
 *     movl $1664525, %eax
 *     movdqu (%rax), %xmm1
 *
 * which faulted at run time in bb_slp_v6/v7, outer_loop_shapes,
 * sha256_transform and regress_lea_self_scaled_after_lea_producer. This
 * fixture pins the whole class: each kernel is four straight-line lanes over
 * consecutive dwords (the SLP seed shape) whose non-load operand is a scalar
 * constant, and the result is checked against an independent scalar
 * recomputation — the values differ per lane, so a broadcast that collapsed
 * into "constant in lane 0 only" fails here even if it does not fault.
 *
 * The SSE2 multiply is the sharpest leg: it is reconstructed from two
 * `pmuludq`s plus a shuffle, and each of those reads the masked operand in a
 * register, so it exercises the const materialisation three times. */
#include <stdio.h>
#include <string.h>

typedef unsigned u32;

static u32 src[16];
static u32 out[16];
static u32 ref[16];

__attribute__((noinline)) void k_mul(const u32 *restrict a, u32 *restrict o) {
    o[0] = a[0] * 1664525u;
    o[1] = a[1] * 1664525u;
    o[2] = a[2] * 1664525u;
    o[3] = a[3] * 1664525u;
}

__attribute__((noinline)) void k_add(const u32 *restrict a, u32 *restrict o) {
    o[0] = a[0] + 7u;
    o[1] = a[1] + 7u;
    o[2] = a[2] + 7u;
    o[3] = a[3] + 7u;
}

__attribute__((noinline)) void k_sub(const u32 *restrict a, u32 *restrict o) {
    o[0] = a[0] - 9u;
    o[1] = a[1] - 9u;
    o[2] = a[2] - 9u;
    o[3] = a[3] - 9u;
}

__attribute__((noinline)) void k_and(const u32 *restrict a, u32 *restrict o) {
    o[0] = a[0] & 0x00ff00ffu;
    o[1] = a[1] & 0x00ff00ffu;
    o[2] = a[2] & 0x00ff00ffu;
    o[3] = a[3] & 0x00ff00ffu;
}

__attribute__((noinline)) void k_xor(const u32 *restrict a, u32 *restrict o) {
    o[0] = a[0] ^ 0x5a5a5a5au;
    o[1] = a[1] ^ 0x5a5a5a5au;
    o[2] = a[2] ^ 0x5a5a5a5au;
    o[3] = a[3] ^ 0x5a5a5a5au;
}

/* Constant on the LEFT of a non-commutative op: the vectorizer builds the same
 * Splat pack, but the emitter now sees it in the src1 position, so the
 * materialisation has to be right on both sides. */
__attribute__((noinline)) void k_sub_const_lhs(const u32 *restrict a, u32 *restrict o) {
    o[0] = 100u - a[0];
    o[1] = 100u - a[1];
    o[2] = 100u - a[2];
    o[3] = 100u - a[3];
}

/* Two different constants feeding one lane run: distinct .LCVEC slots, and a
 * mixed splat/gather graph rather than a single broadcast. */
__attribute__((noinline)) void k_mixed(const u32 *restrict a, const u32 *restrict b,
                                       u32 *restrict o) {
    o[0] = a[0] * 3u + b[0];
    o[1] = a[1] * 5u + b[1];
    o[2] = a[2] * 3u + b[2];
    o[3] = a[3] * 5u + b[3];
}

static int check(const char *name, const u32 *expect) {
    if (memcmp(out, expect, 4 * sizeof(u32)) != 0) {
        printf("FAIL %s: %08x %08x %08x %08x want %08x %08x %08x %08x\n", name,
               out[0], out[1], out[2], out[3], expect[0], expect[1], expect[2],
               expect[3]);
        return 1;
    }
    return 0;
}

int main(void) {
    int bad = 0;
    for (int round = 0; round < 64; round++) {
        for (int i = 0; i < 16; i++) {
            src[i] = 0x9e3779b9u * (u32)(i + 1) + (u32)round * 0x85ebca6bu;
        }
        /* Every constant is odd/zero-free so no lane can pass by accident
         * through a zero-extended materialisation. */
        src[0] |= 1u;

        k_mul(src, out);
        for (int i = 0; i < 4; i++) ref[i] = src[i] * 1664525u;
        bad |= check("mul", ref);

        k_add(src, out);
        for (int i = 0; i < 4; i++) ref[i] = src[i] + 7u;
        bad |= check("add", ref);

        k_sub(src, out);
        for (int i = 0; i < 4; i++) ref[i] = src[i] - 9u;
        bad |= check("sub", ref);

        k_and(src, out);
        for (int i = 0; i < 4; i++) ref[i] = src[i] & 0x00ff00ffu;
        bad |= check("and", ref);

        k_xor(src, out);
        for (int i = 0; i < 4; i++) ref[i] = src[i] ^ 0x5a5a5a5au;
        bad |= check("xor", ref);

        k_sub_const_lhs(src, out);
        for (int i = 0; i < 4; i++) ref[i] = 100u - src[i];
        bad |= check("sub_const_lhs", ref);

        k_mixed(src, src + 4, out);
        for (int i = 0; i < 4; i++) ref[i] = src[i] * (i & 1 ? 5u : 3u) + src[4 + i];
        bad |= check("mixed", ref);

        if (bad) return 1;
    }
    printf("PASS vec lane constants 7 kernels 64 rounds\n");
    return 0;
}
