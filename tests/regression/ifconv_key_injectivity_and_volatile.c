/*
 * Address-key injectivity and volatile-access preservation in if-conversion,
 * plus the backend's SIB index cast peel.
 *
 * Five defects, all measured live on the unfixed compiler at -O1..-Os and all
 * silent (wrong values / wrong addresses, no diagnostic):
 *
 *  1. SEXT vs ZEXT collapsed to one address key. `canonical_addr_key_impl`
 *     descended through every widening cast as "value-preserving", so with
 *     `signed char i = -1`, `d[i]` (sext -> d-1) and `d[(unsigned char) i]`
 *     (zext -> d+255) produced the SAME key. Measured: `ext_diamond` returned
 *     11 for the zext arm instead of 22.
 *  2. The same collision made `rewrite_covered_arm_loads` forward a pred load
 *     to an arm load 256 bytes away (`ext_covered` -> 11, want 22).
 *  3. ...and made `sink_conditional_stores` merge two stores to DIFFERENT
 *     addresses: `ext_store`'s false arm wrote `d[-1]` instead of `d[255]`.
 *     A wrong-address write.
 *  4. The backend's SIB index cast peel walked `I32 -> U32 -> I64` down to the
 *     I32 root and then SIGN-extended it, so `t[(unsigned char) c]` with
 *     `c = -1` addressed `t[-1]` instead of `t[255]` — every character, CRC
 *     and tolower table lookup. Measured: `peel_idx` returned 0, want 7255.
 *  5. The covered-arm load rewrite matched `Load { .. }`, blind to the
 *     volatile flag, so `volatile int g; if (g > 0) return g;` performed ONE
 *     read where C11 5.1.2.3 requires two. Measured in the assembly: one
 *     `g(%rip)` for lccc, two for GCC.
 *
 * The volatile read count is additionally asserted in the assembly by the
 * companion Rust unit tests; here the value is checked so a dropped access
 * that also changes the result is caught at run time. Every index pair used
 * below lives inside one allocation, so a wrong-address access reads or
 * writes a defined neighbouring element rather than faulting — which is
 * exactly what makes these defects silent and worth pinning.
 */
#include <stdio.h>
#include <string.h>

volatile int g;
int sink;

/* 1. volatile: pred loads *g for the compare, arm re-loads it. Must be 2 reads. */
__attribute__((noinline)) int vol_twice(void) {
    if (g > 0)
        return g;
    return -1;
}

/* 2. sext vs zext of the same signed char index. */
__attribute__((noinline)) int ext_diamond(const char *d, signed char i, int c) {
    return c ? d[i] : d[(unsigned char) i];
}
__attribute__((noinline)) int ext_covered(const char *d, signed char i) {
    if (d[i] > 0)
        return d[(unsigned char) i];
    return -100;
}
__attribute__((noinline)) void ext_store(char *d, signed char i, int c, char x, char y) {
    if (c)
        d[i] = x;
    else
        d[(unsigned char) i] = y;
}

/* 3. index peel through an unsigned cast: table[(unsigned char) c] with c < 0. */
__attribute__((noinline)) int peel_idx(const int *t, signed char c) {
    return t[(unsigned char) c];
}
__attribute__((noinline)) int peel_idx_u(const int *t, signed char c) {
    return t[(unsigned) (unsigned char) c];
}

int main(void) {
    int bad = 0;
    static char buf[1024];
    char *d = buf + 512;
    memset(buf, 0, sizeof buf);
    d[-1] = 11;
    d[255] = 22;

    if (ext_diamond(d, -1, 1) != 11) { printf("ext_diamond sext = %d want 11\n", ext_diamond(d,-1,1)); bad = 1; }
    if (ext_diamond(d, -1, 0) != 22) { printf("ext_diamond zext = %d want 22\n", ext_diamond(d,-1,0)); bad = 1; }
    if (ext_covered(d, -1) != 22)    { printf("ext_covered = %d want 22\n", ext_covered(d,-1)); bad = 1; }

    memset(buf, 0, sizeof buf);
    ext_store(d, -1, 1, 44, 55);
    if (d[-1] != 44 || d[255] != 0) { printf("ext_store true arm: d[-1]=%d d[255]=%d\n", d[-1], d[255]); bad = 1; }
    memset(buf, 0, sizeof buf);
    ext_store(d, -1, 0, 44, 55);
    if (d[-1] != 0 || d[255] != 55) { printf("ext_store false arm: d[-1]=%d d[255]=%d\n", d[-1], d[255]); bad = 1; }

    {
        static int t[256];
        for (int k = 0; k < 256; k++) t[k] = 7000 + k;
        if (peel_idx(t, -1) != 7255)   { printf("peel_idx = %d want 7255\n", peel_idx(t, -1)); bad = 1; }
        if (peel_idx_u(t, -1) != 7255) { printf("peel_idx_u = %d want 7255\n", peel_idx_u(t, -1)); bad = 1; }
        if (peel_idx(t, -128) != 7128) { printf("peel_idx(-128) = %d want 7128\n", peel_idx(t, -128)); bad = 1; }
    }

    g = 7;
    if (vol_twice() != 7) { printf("vol_twice value wrong\n"); bad = 1; }

    printf("ifconv_key_injectivity_and_volatile: %s\n", bad ? "FAIL" : "OK");
    return bad;
}
