/* The guarded ffs shift-loop -> CtzNonZero rewrite (loop_idiom, #853) on
 * its edges: signed words (arithmetic shift), 8/16/32/64-bit words, narrow
 * and wrapping counters, nonzero counter starts, every guard spelling, and
 * words that are modified between the guard and the loop. Inputs: every
 * single bit, its neighbours, the sign bit, all-ones and an LCG sweep.
 * Prints a checksum; compared against GCC.
 */
#include <stdio.h>
#include <stdint.h>

#define FFS_BODY(x, r)            \
    while (!((x) & 1)) {          \
        (x) >>= 1;                \
        (r)++;                    \
    }

__attribute__((noinline)) static int ffs_u32(uint32_t x) {
    if (!x) return 0;
    int r = 1;
    FFS_BODY(x, r);
    return r;
}
__attribute__((noinline)) static int ffs_i32(int32_t x) {
    if (x == 0) return 0;
    int r = 1;
    FFS_BODY(x, r);
    return r;
}
__attribute__((noinline)) static int ffs_u64(uint64_t x) {
    if (!x) return 0;
    int r = 1;
    FFS_BODY(x, r);
    return r;
}
__attribute__((noinline)) static long ffs_i64(int64_t x) {
    if (!x) return 0;
    long r = 1;
    FFS_BODY(x, r);
    return r;
}
__attribute__((noinline)) static int ffs_u8(uint8_t x) {
    if (!x) return 0;
    int r = 1;
    FFS_BODY(x, r);
    return r;
}
__attribute__((noinline)) static int ffs_i16(int16_t x) {
    if (!x) return 0;
    int r = 1;
    FFS_BODY(x, r);
    return r;
}
/* Narrow counter starting high: wraps exactly like the loop does. */
__attribute__((noinline)) static unsigned char ffs_char_counter(uint64_t x) {
    if (!x) return 7;
    unsigned char r = 250;
    FFS_BODY(x, r);
    return r;
}
__attribute__((noinline)) static signed char ffs_schar_counter(uint32_t x) {
    if (!x) return -1;
    signed char r = 120;
    FFS_BODY(x, r);
    return r;
}
/* Counter start 0 (the __ffs shape) and a ternary guard. */
__attribute__((noinline)) static int ctz_ternary(uint32_t x) {
    int r = 0;
    if (x) {
        FFS_BODY(x, r);
    } else {
        r = 32;
    }
    return r;
}
/* Word modified between the guard and the loop: the guard says nothing
 * about the loop's word, so a rewrite must not rely on it. Only inputs
 * whose masked value is nonzero are fed (the loop never ends otherwise). */
__attribute__((noinline)) static int ffs_masked(uint32_t x, uint32_t m) {
    if (!x) return 0;
    x &= m;
    int r = 1;
    FFS_BODY(x, r);
    return r;
}
/* Guard on a wider value than the loop's word (truncation in between);
 * inputs keep the low word nonzero. */
__attribute__((noinline)) static int ffs_trunc(uint64_t y) {
    if (!y) return 0;
    uint32_t x = (uint32_t)y;
    int r = 1;
    FFS_BODY(x, r);
    return r;
}
/* The word escapes after the loop: the rewrite must not apply (or must keep
 * x's final value right). */
__attribute__((noinline)) static uint32_t ffs_escape(uint32_t x, int *rp) {
    if (!x) { *rp = 0; return 0; }
    int r = 1;
    FFS_BODY(x, r);
    *rp = r;
    return x;
}

static uint64_t h = 1469598103934665603ULL;
static void mix(uint64_t v) { h = (h ^ v) * 1099511628211ULL; }

static void feed(uint64_t v) {
    mix((uint64_t)ffs_u32((uint32_t)v));
    mix((uint64_t)ffs_i32((int32_t)v));
    mix((uint64_t)ffs_u64(v));
    mix((uint64_t)ffs_i64((int64_t)v));
    mix((uint64_t)ffs_u8((uint8_t)v));
    mix((uint64_t)ffs_i16((int16_t)v));
    mix((uint64_t)ffs_char_counter(v));
    mix((uint64_t)(int64_t)ffs_schar_counter((uint32_t)v));
    mix((uint64_t)ctz_ternary((uint32_t)v));
    if ((uint32_t)v & 0x00ff00ffu)
        mix((uint64_t)ffs_masked((uint32_t)v, 0x00ff00ffu));
    if ((uint32_t)v)
        mix((uint64_t)ffs_trunc(v));
    int r;
    mix(ffs_escape((uint32_t)v, &r));
    mix((uint64_t)r);
}

int main(void) {
    feed(0);
    feed(~0ULL);
    for (int b = 0; b < 64; b++) {
        uint64_t bit = 1ULL << b;
        feed(bit);
        feed(bit | (bit << 1));
        feed(~(bit - 1));        /* sign-extended style: bits b..63 set */
        feed(bit - 1 ? bit - 1 : 1);
    }
    uint64_t s = 0x9E3779B97F4A7C15ULL;
    for (int i = 0; i < 20000; i++) {
        s = s * 6364136223846793005ULL + 1442695040888963407ULL;
        feed(s);
        feed(s >> (s & 63));
        feed(s << (s & 63));
    }
    printf("ffs_ctz_idiom_edges %016llx\n", (unsigned long long)h);
    return 0;
}
