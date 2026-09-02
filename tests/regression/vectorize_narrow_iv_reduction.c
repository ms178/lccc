/* Vectorizer guards: reduction loops whose induction variable is NARROW
 * (8/16-bit) or does not start at a constant zero.
 *
 * (1) NARROW IV — reduction vectorization of a loop whose induction
 *     variable is 8/16-bit (`unsigned char`, `signed char`,
 *     `unsigned short`, `signed short`) used to retype the narrow counter
 *     into a vectorized byte/word step of 16 against a byte-scaled trip
 *     limit — e.g. `unsigned char i; for (i = 0; i < n; i++) s += a[i]`
 *     with n = 120 became a U8 counter stepping 16 toward byte-limit 480,
 *     wrapping at 256 and looping forever at -O2 (reproduced on main @
 *     dcf673d; GCC terminates and prints 7140).
 *
 * (2) NON-ZERO IV START — both reduction addressing schemes rescale the
 *     trip limit but keep the IV's original preheader value, so a start of
 *     `c != 0` made the vector loop cover [c, c + w*floor((n-c')/w)) with a
 *     remainder resuming at a rescaled IV — silently dropping or misreading
 *     elements. Reproduced on main @ dcf673d, -O2 (GCC prints 1760):
 *       `int`  accumulator, i = 5, n = 60 -> 31708938240 (AVX2 4-wide path)
 *       `long` accumulator, i = 5, n = 60 -> 30702305280 (SSE2 2-wide path)
 *     The SSE2 breakage is why the guard lives in the reduction DISPATCHER
 *     (both transforms are gated), not only inside the AVX2 transform.
 *
 * The vectorizer now requires the loop's induction phi to be I32/U32-wide
 * AND to start at a constant 0 before it applies either the SSE2 or the
 * AVX2 reduction transform. These loops must compile, terminate, and stay
 * bit-exact against GCC.
 *
 * Trip counts stay inside each narrow type's domain (where the C program is
 * defined and terminating): 8-bit counters below 128/256, 16-bit below
 * 32768/65536.
 */
#include <stdio.h>

__attribute__((noinline)) static long u8red(const int *a, unsigned char n) {
    long s = 0;
    unsigned char i;
    for (i = 0; i < n; i++) {
        s += a[i];
    }
    return s;
}

__attribute__((noinline)) static long s8red(const int *a, signed char n) {
    long s = 0;
    signed char i;
    for (i = 0; i < n; i++) {
        s += a[i];
    }
    return s;
}

__attribute__((noinline)) static long u16red(const int *a, unsigned short n) {
    long s = 0;
    unsigned short i;
    for (i = 0; i < n; i++) {
        s += a[i];
    }
    return s;
}

__attribute__((noinline)) static long s16red(const int *a, short n) {
    long s = 0;
    short i;
    for (i = 0; i < n; i++) {
        s += a[i];
    }
    return s;
}

__attribute__((noinline)) static long u8off(const int *a, unsigned char n) {
    long s = 0;
    unsigned char i;
    for (i = 0; i < n; i++) {
        s += a[i + 1];
    }
    return s;
}

/* (2) Non-zero constant start, I32 accumulator (AVX2 4-wide path). */
__attribute__((noinline)) static long start5_int(const int *a, int n) {
    long s = 0;
    for (int i = 5; i < n; i++) {
        s += a[i];
    }
    return s;
}

/* (2) Non-zero constant start, I64 accumulator + elements (SSE2 2-wide
 * path — the transform the AVX2-only guard would have missed). */
__attribute__((noinline)) static long start5_long(const long *a, int n) {
    long s = 0;
    for (int i = 5; i < n; i++) {
        s += a[i];
    }
    return s;
}

/* (2) Non-zero start with a dot-product body (two-array reduction). */
__attribute__((noinline)) static long start3_dot(const int *a, const int *b,
                                                 int n) {
    long s = 0;
    for (int i = 3; i < n; i++) {
        s += a[i] * b[i];
    }
    return s;
}

int main(void) {
    enum { N = 300 };
    enum { NBIG = 66000 };
    static int a[N + 4];
    static long al[N];
    static int big[NBIG];
    for (int i = 0; i < N + 4; i++) {
        a[i] = (i * 17 - 5) % 700 - 350;
    }
    for (int i = 0; i < N; i++) {
        al[i] = (long)(i * 31 - 11) % 9000 - 4500;
    }
    for (int i = 0; i < NBIG; i++) {
        big[i] = (i * 13 - 7) % 900 - 450;
    }
    long h = 0;
    for (unsigned n = 1; n <= 250; n++) {
        unsigned char n8 = (unsigned char)(n & 0x7f);
        signed char s8 = (signed char)((n & 0x7f) - 64);
        h = h * 31 + u8red(a, n8);
        h = h * 31 + s8red(a, s8);
        h = h * 31 + u8off(a, n8);
    }
    /* 16-bit counters need a matching big array. */
    for (unsigned n = 1; n <= 1000; n++) {
        unsigned short n16 = (unsigned short)((n * 37) & 0x7fff);
        h = h * 31 + u16red(big, n16);
        h = h * 31 + s16red(big, (short)((n * 37) & 0x7fff));
    }
    h = h * 31 + u8red(a, 120);    /* the wrap repro (byte limit 480) */
    h = h * 31 + u8red(a, 250);    /* inside 8-bit domain but > 128 */
    h = h * 31 + u16red(big, 60000);
    h = h * 31 + s16red(big, 30000);
    /* Non-zero-start reductions (must stay scalar, stay exact). */
    h = h * 31 + start5_int(a, 60);
    h = h * 31 + start5_int(a, 297);
    h = h * 31 + start5_long(al, 60);
    h = h * 31 + start5_long(al, 297);
    h = h * 31 + start3_dot(a, big, 297);
    printf("%ld\n", h);
    return 0;
}
