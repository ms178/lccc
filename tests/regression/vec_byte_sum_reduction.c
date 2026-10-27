/*
 * Byte-sum reductions (psadbw / vpsadbw path) -- end-to-end battery.
 *
 * The vectorizer sums bytes with `psadbw(lane, 0)`: unsigned byte values
 * directly, signed ones through the `^ 0x80` bias (-128 per consumed byte at
 * the exit), 0/1 predicates through `& 1`.  This battery runs every admitted
 * shape and every shape that must be REFUSED through the real backend:
 *
 *   - every length 0..299 at all 16 start misalignments (the legacy-SSE
 *     encoding of psadbw faults on an unaligned memory operand, so a wrong
 *     load fold would crash here);
 *   - bytes at both extremes (0x00, 0x7f, 0x80, 0xff) and a random stream;
 *   - accumulators of every width, seeded right below their wrap point;
 *   - the affine-exit form `for (i = 0; i + 4 < N; i++)` over a global array
 *     (constant trip count after the pre-vectorizer affine fold);
 *   - refusals: a compare used as a value, a two-stream difference and a
 *     signed value plus one (no byte domain fits), stride 2, shifted base.
 *
 * References run through `volatile` scalars so they can never be vectorized
 * into the same (possibly wrong) form.  Output is a checksum plus "OK", and
 * the binary exits non-zero on the first mismatch.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define MAXN 300
#define PAD 16

static unsigned char ubuf[MAXN + PAD];
static unsigned char vbuf[MAXN + PAD];
static unsigned char gbuf[4096];

typedef unsigned long long u64;

/* ---- kernels (vectorized or refused; never miscompiled) --------------- */
__attribute__((noinline)) long k_plain(const unsigned char *p, int n) {
    long s = 0;
    for (int i = 0; i < n; i++) s += p[i];
    return s;
}
__attribute__((noinline)) unsigned k_plain_u32(const unsigned char *p, unsigned n, unsigned seed) {
    unsigned s = seed;
    for (unsigned i = 0; i < n; i++) s += p[i];
    return s;
}
__attribute__((noinline)) int k_plain_int(const unsigned char *p, long n, int seed) {
    int s = seed;
    for (long i = 0; i < n; i++) s += p[i];
    return s;
}
__attribute__((noinline)) u64 k_plain_u64(const unsigned char *p, unsigned long n, u64 seed) {
    u64 s = seed;
    for (unsigned long i = 0; i < n; i++) s += p[i];
    return s;
}
__attribute__((noinline)) long k_mask(const unsigned char *p, int n) {
    long s = 0;
    for (int i = 0; i < n; i++) s += p[i] & 0x0f;
    return s;
}
__attribute__((noinline)) long k_xor(const unsigned char *p, const unsigned char *q, int n) {
    long s = 0;
    for (int i = 0; i < n; i++) s += p[i] ^ q[i];
    return s;
}
__attribute__((noinline)) long k_select(const unsigned char *p, int n) {
    long s = 0;
    for (int i = 0; i < n; i++) s += p[i] > 100 ? p[i] : 0;
    return s;
}
__attribute__((noinline)) long k_trunc(const unsigned char *p, int n) {
    long s = 0;
    for (int i = 0; i < n; i++) s += (unsigned char)(p[i] + 3);
    return s;
}
__attribute__((noinline)) long k_signed(const signed char *p, int n) {
    long s = 0;
    for (int i = 0; i < n; i++) s += p[i];
    return s;
}
__attribute__((noinline)) int k_signed_int(const signed char *p, int n, int seed) {
    int s = seed;
    for (int i = 0; i < n; i++) s += p[i];
    return s;
}
__attribute__((noinline)) long k_signed_trunc(const unsigned char *p, int n) {
    long s = 0;
    for (int i = 0; i < n; i++) s += (signed char)p[i];
    return s;
}
__attribute__((noinline)) long k_pred(const unsigned char *p, int n) {
    long s = 0;
    for (int i = 0; i < n; i++) s += (p[i] == 7);
    return s;
}
/* must be refused: compare used as a VALUE (0xFF packed vs 1 scalar) */
__attribute__((noinline)) long k_zero_minus_cmp(const unsigned char *p, int n) {
    long s = 0;
    for (int i = 0; i < n; i++) s += 0 - (p[i] == 7);
    return s;
}
/* must be refused: [-255, 255] fits neither byte domain */
__attribute__((noinline)) long k_diff(const unsigned char *p, const unsigned char *q, int n) {
    long s = 0;
    for (int i = 0; i < n; i++) s += p[i] - q[i];
    return s;
}
/* must be refused: [-127, 128] fits neither byte domain */
__attribute__((noinline)) long k_signed_plus1(const signed char *p, int n) {
    long s = 0;
    for (int i = 0; i < n; i++) s += p[i] + 1;
    return s;
}
/* must stay scalar: wrong elements if packed */
__attribute__((noinline)) long k_stride2(const unsigned char *p, int n) {
    long s = 0;
    for (int i = 0; i < n / 2; i++) s += p[2 * i];
    return s;
}
__attribute__((noinline)) long k_shift3(const unsigned char *p, int n) {
    long s = 0;
    for (int i = 0; i + 3 < n; i++) s += p[i + 3];
    return s;
}
/* the affine_countdown shape: constant trip after the affine fold */
__attribute__((noinline)) long k_affine_global(void) {
    long s = 0;
    for (int i = 0; i + 4 < 4096; i++) s += gbuf[i];
    return s;
}

/* ---- volatile references ---------------------------------------------- */
static u64 ref(const unsigned char *p, const unsigned char *q, long n, int which) {
    u64 s = 0;
    for (long i = 0; i < n; i++) {
        volatile unsigned char a = p[i];
        volatile unsigned char b = q[i];
        long v;
        switch (which) {
        case 0: v = a; break;
        case 1: v = a & 0x0f; break;
        case 2: v = a ^ b; break;
        case 3: v = a > 100 ? a : 0; break;
        case 4: v = (unsigned char)(a + 3); break;
        case 5: v = (signed char)a; break;
        case 6: v = (a == 7); break;
        case 7: v = 0 - (a == 7); break;
        case 8: v = (long)a - (long)b; break;
        case 9: v = (signed char)a + 1; break;
        default: abort();
        }
        s += (u64)v;
    }
    return s;
}

static int failures;
static u64 checksum;

static void expect(const char *what, int n, int off, u64 got, u64 want) {
    checksum = checksum * 1000003u + got;
    if (got != want) {
        if (failures++ < 10)
            printf("FAIL %s n=%d off=%d got %llu want %llu\n", what, n, off, got, want);
    }
}

static void fill(int pattern) {
    unsigned x = 12345u + (unsigned)pattern;
    for (int i = 0; i < MAXN + PAD; i++) {
        x = x * 1103515245u + 12345u;
        unsigned char r = (unsigned char)(x >> 16);
        switch (pattern) {
        case 0: ubuf[i] = 0x00; vbuf[i] = 0xff; break;
        case 1: ubuf[i] = 0xff; vbuf[i] = 0x00; break;
        case 2: ubuf[i] = 0x80; vbuf[i] = 0x7f; break;
        case 3: ubuf[i] = 0x7f; vbuf[i] = 0x80; break;
        case 4: ubuf[i] = (i % 5 == 0) ? 7 : 200; vbuf[i] = r; break;
        default: ubuf[i] = r; vbuf[i] = (unsigned char)(r * 7u + 1u); break;
        }
    }
}

int main(void) {
    for (int pattern = 0; pattern < 6; pattern++) {
        fill(pattern);
        for (int off = 0; off < PAD; off++) {
            const unsigned char *p = ubuf + off, *q = vbuf + off;
            const signed char *sp = (const signed char *)p;
            for (int n = 0; n < MAXN; n++) {
                expect("plain", n, off, (u64)k_plain(p, n), ref(p, q, n, 0));
                expect("mask", n, off, (u64)k_mask(p, n), ref(p, q, n, 1));
                expect("xor", n, off, (u64)k_xor(p, q, n), ref(p, q, n, 2));
                expect("select", n, off, (u64)k_select(p, n), ref(p, q, n, 3));
                expect("trunc", n, off, (u64)k_trunc(p, n), ref(p, q, n, 4));
                expect("signed", n, off, (u64)k_signed(sp, n), ref(p, q, n, 5));
                expect("signed_trunc", n, off, (u64)k_signed_trunc(p, n), ref(p, q, n, 5));
                expect("pred", n, off, (u64)k_pred(p, n), ref(p, q, n, 6));
                expect("zero_minus_cmp", n, off, (u64)k_zero_minus_cmp(p, n), ref(p, q, n, 7));
                expect("diff", n, off, (u64)k_diff(p, q, n), ref(p, q, n, 8));
                expect("signed_plus1", n, off, (u64)k_signed_plus1(sp, n), ref(p, q, n, 9));
                /* wrapping accumulators, seeded just below their wrap */
                expect("u32_wrap", n, off, k_plain_u32(p, (unsigned)n, 0xFFFFFF00u),
                       (u64)(unsigned)(0xFFFFFF00u + (unsigned)ref(p, q, n, 0)));
                expect("int_seed", n, off,
                       (u64)(unsigned)k_plain_int(p, n, -1000),
                       (u64)(unsigned)(-1000 + (int)ref(p, q, n, 0)));
                expect("u64_wrap", n, off, k_plain_u64(p, (unsigned long)n, ~0ull - 50),
                       ~0ull - 50 + ref(p, q, n, 0));
                expect("signed_int_seed", n, off,
                       (u64)(unsigned)k_signed_int(sp, n, 77),
                       (u64)(unsigned)(77 + (int)ref(p, q, n, 5)));
                {
                    u64 w = 0;
                    for (int i = 0; i < n / 2; i++) { volatile unsigned char a = p[2 * i]; w += a; }
                    expect("stride2", n, off, (u64)k_stride2(p, n), w);
                    w = 0;
                    for (int i = 0; i + 3 < n; i++) { volatile unsigned char a = p[i + 3]; w += a; }
                    expect("shift3", n, off, (u64)k_shift3(p, n), w);
                }
            }
        }
    }
    for (int i = 0; i < 4096; i++) gbuf[i] = (unsigned char)(i * 31 + 7);
    {
        u64 w = 0;
        for (int i = 0; i + 4 < 4096; i++) { volatile unsigned char a = gbuf[i]; w += a; }
        expect("affine_global", 4092, 0, (u64)k_affine_global(), w);
    }
    printf("checksum %llu\n", checksum);
    if (failures) {
        printf("%d failures\n", failures);
        return 1;
    }
    printf("OK vec_byte_sum_reduction\n");
    return 0;
}
