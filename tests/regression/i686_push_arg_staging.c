/* i686: outgoing stack arguments are staged with PUSHES, not with
 * `subl $N,%esp` + per-argument stores, when every stack argument is a
 * 4-byte scalar and the preferred stack boundary is 4
 * (`-mpreferred-stack-boundary=2`, the Linux boot configuration).
 *
 * The push model is byte-for-byte the cdecl layout the store model produced
 * (args right-to-left, slot 0 at the lowest address, callee-owned incoming
 * slots), but each argument costs one 2-byte `pushl %eax` / 3-byte
 * `pushl $imm8` instead of a 5-8-byte store, and the `subl` disappears when
 * the pushed total covers the area.  Two things can go wrong, and both are
 * observable here as wrong values rather than crashes:
 *
 *   * push ORDER: arguments must be pushed right-to-left; a left-to-right
 *     push sequence reverses the callee's view of its parameter list;
 *   * esp-relative staleness: every argument staged AFTER a push must read
 *     its source with the shifted %esp -- the esp_adjust bookkeeping must
 *     track each push exactly, or later arguments read the wrong local.
 *
 * The caller mixes constants (imm8-range, imm32-range, negative), register
 * values, alloca addresses, globals and sub-int types in every stack
 * position, with more than three stack arguments per call so several pushes
 * happen inside one marshalling sequence, and the callee reads its stack
 * parameters repeatedly and mutates one.  The flags force the boot-shaped
 * configuration: -Os for the size path, boundary 4 so the push eligibility
 * gate is open.
 *
 * Reference values are plain C semantics (gcc -m32 -O0..-O3 agree); build
 * with -DPRINT_REF to re-derive them. */
#include <stdio.h>

#ifndef EXPECT_A
#define EXPECT_A 3848315922u
#define EXPECT_B 3224632454u
#endif

static unsigned g_sink;
static int g_tbl[4] = {11, 22, 33, 44};

__attribute__((noinline)) static unsigned sum6(int a, int b, int c, int d,
                                               int e, int f) {
    /* Mutate one stack parameter and read each stack parameter more than
     * once, in two orders. */
    d = d * 2 + 1;
    return (unsigned)(a + b + c) ^ (unsigned)d ^ ((unsigned)e << 3) ^
           (unsigned)(f - 7);
}

__attribute__((noinline)) static unsigned
mix7(int a, char ch, unsigned u, int g, short sh, int big, int neg) {
    /* char/short travel as 4-byte stack slots here (regparm consumes a, ch
     * may stack depending on the count): the push path must store the same
     * four bytes the store path did. */
    unsigned acc = (unsigned)a;
    acc ^= (unsigned)ch;
    acc ^= u;
    acc ^= (unsigned)g * 3u;
    acc ^= (unsigned)(int)sh << 8;
    acc ^= (unsigned)big >> 2;
    acc ^= (unsigned)neg;
    g_sink = acc;
    return acc;
}

int main(void) {
    unsigned r = 0;
    int locals[4] = {5, -6, 700, -800};
    char buf[8] = {(char)'A', 0, 0, 0, 0, 0, 0, 0};

    /* 3 stack arguments (6 params, regparm 3): constants only. */
    r ^= sum6(1, 2, 3, 1000, 2000, 3000);
    /* Same shape, but the stack arguments come from locals and globals that
     * are read while the pushes have already shifted %esp. */
    r ^= sum6(locals[0], locals[1], locals[2], locals[3], g_tbl[2], -g_tbl[3]);
    /* Zero and negatives in every stack position. */
    r ^= sum6(0, 0, 0, 0, 0, -1) * 7u;
    r ^= sum6(-2147483647 - 1, 2147483647, -12345, 6789, -1, 1);
    /* Four stack arguments (7 params): several pushes per sequence. */
    r ^= mix7(9, buf[0], 0xDEADBEEFu, -13, -300, 1 << 28, -77);
    r ^= mix7(locals[1], 'Z', g_tbl[1], locals[0], 12345, -987654, 321);

#ifdef PRINT_REF
    printf("#define EXPECT_A %uu\n", r);
    printf("#define EXPECT_B %uu\n", g_sink);
#else
    if (r != EXPECT_A) {
        printf("FAIL: checksum %u != %u\n", r, EXPECT_A);
        return 1;
    }
    if (g_sink != EXPECT_B) {
        printf("FAIL: sink %u != %u\n", g_sink, EXPECT_B);
        return 1;
    }
    printf("OK i686_push_arg_staging\n");
#endif
    return 0;
}
