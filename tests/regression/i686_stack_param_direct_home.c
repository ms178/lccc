/* i686: stack parameters get DIRECT HOMES -- the callee reads and mutates
 * them in the caller's outgoing area instead of copying them into local
 * slots in the prologue, with the slot references redirected through the
 * incoming offsets (esp_adjust-aware under FPO).
 *
 * The win: the prologue homing copy disappears (one `movl K(%esp),%reg`
 * per stack parameter), and with it the local slot.  Two things can go
 * wrong, and both are observable as wrong values:
 *
 *   * REDIRECTION: every read/write of the parameter's slot must resolve
 *     to the incoming offset (plus the CURRENT esp delta when the frame
 *     is rsp-addressed and a call moved %esp); a missed redirect reads a
 *     local slot that no longer exists or a stale frame offset;
 *   * COALESCING: wide locals (i64/u64/f64) must never share or absorb a
 *     redirected 4-byte parameter slot -- the width-class eligibility
 *     scan must refuse the webs (the 05c9a6ba hazard shape).
 *
 * Freestanding on purpose: the flags pin -mregparm=3 (the kernel boot
 * regime), which passes arguments in registers -- calling libc under it
 * is an ABI violation (glibc is cdecl), so the checksum is observed
 * through the exit status and a volatile sink instead of printf.  The
 * EXPECT_* constants are pure C semantics of the arithmetic below (every
 * oracle at -O0..-Os agrees; regenerate with
 * `gcc -m32 -O2 -DPRINT_REF` which is regparm-free). */
#include <stdint.h>

#ifndef EXPECT_A
#define EXPECT_A 952530524u
#define EXPECT_B 4294967227u
#define EXPECT_C 2678713873u
#define EXPECT_D 4294964015u
#endif

volatile uint32_t g_sink;

/* 6 int params: 3 in regparm registers, 3 on the stack.  Mutates stack
 * parameters, reads every parameter both before and after the mutation,
 * and interleaves wide locals for coalescing/spill pressure. */
__attribute__((noinline)) static uint32_t take6(int a, int b, int c, int d,
                                                int e, int f) {
    int64_t wide1 = (int64_t)d * e + 1;
    int64_t wide2 = (int64_t)f * 3 - 7;
    uint32_t pre = (uint32_t)(a + d) ^ ((uint32_t)e * 7u) ^ (uint32_t)(f + c);

    /* Wide-local arithmetic between parameter reads. */
    wide1 += (int32_t)pre;
    wide2 ^= wide1 << 3;

    /* In-place mutation of stack parameters: the write-back goes through
     * the same redirected location the reads used. */
    f = -f;
    e = e * 3 + b;

    uint32_t post = (uint32_t)(a + d) ^ ((uint32_t)e * 7u) ^ (uint32_t)(f + c);
    return pre ^ post ^ (uint32_t)(wide1 >> 1) ^ (uint32_t)wide2;
}

/* 5 params, 2 on the stack, with an i64 local adjacent to the parameter
 * reads under maximum pressure -- the wide-slot-sharing hazard. */
__attribute__((noinline)) static uint32_t take5(int a, int b, int c, int d,
                                                int e) {
    int64_t w = (int64_t)a * d;
    int m1 = d + 11;
    int m2 = e - 13;
    w ^= (int64_t)m1 * m2;
    return (uint32_t)(a + b + c) ^ (uint32_t)m1 ^ ((uint32_t)m2 << 4) ^
           (uint32_t)w;
}

#ifdef PRINT_REF
/* Derivation build: regparm-free so libc printf is a legal call.  Every
 * row is emitted by the same reference run, so the baked constants can
 * never drift from one arm's semantics. */
#include <stdio.h>
int main(void) {
    printf("#define EXPECT_A %uu\n",
           take6(1, 2, 3, 100, 200, -300)
               ^ take6(-7, 9, 11, -1000000, 12345, 0) * 3u
               ^ take6(41, 42, 43, 41000, -3157, -958) ^ take5(5, 6, 7, 100000, -200000) * 5u
               ^ take5(-3, -4, -5, 7, 8));
    printf("#define EXPECT_B %uu\n", take6(0, 0, 0, 0, 0, 0) ^ take5(0, 0, 0, 0, 0));
    printf("#define EXPECT_C %uu\n",
           take6(1000000, 2000000, 3000000, -4000000, 5000000, -6000000));
    printf("#define EXPECT_D %uu\n", take6(-300, 200, 100, 3, 2, 1));
    return 0;
}
#else
int main(void) {
    uint32_t r = 0;
    /* Constants in every stack position, both polarities of zero, negatives
     * (the -f path must round-trip the sign), and register-file values. */
    r ^= take6(1, 2, 3, 100, 200, -300);
    r ^= take6(-7, 9, 11, -1000000, 12345, 0) * 3u;
    int v = 41;
    r ^= take6(v, v + 1, v + 2, v * 1000, -v * 77, v - 999);
    r ^= take5(5, 6, 7, 100000, -200000) * 5u;
    r ^= take5(-3, -4, -5, 7, 8);
    if (r != EXPECT_A)
        return 1;
    g_sink = r;

    /* Zero-arg row: every parameter defaults; the checksum mixes the wide
     * locals only (a corrupted adjacent slot shows up here too). */
    uint32_t t = take6(0, 0, 0, 0, 0, 0);
    uint32_t u = take5(0, 0, 0, 0, 0);
    if ((t ^ u) != EXPECT_B)
        return 2;
    g_sink = t ^ u;

    /* Wide magnitudes: the i64 products overflow 32 bits deliberately. */
    if (take6(1000000, 2000000, 3000000, -4000000, 5000000, -6000000) !=
        EXPECT_C)
        return 3;

    /* Mutation round-trip: the callee's write-back must be visible to a
     * second call reading the same outgoing layout (fresh frame each
     * time, so this is a determinism check, not aliasing). */
    if (take6(-300, 200, 100, 3, 2, 1) != EXPECT_D)
        return 4;

    return 0;
}
#endif
