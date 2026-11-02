/*
 * Signed 32-bit shifts by an immediate and the consumers that read the
 * result 64 bits wide.
 *
 * A 32-bit `sarl`/`shll` zero-fills the upper half of the register.  The
 * x86 backend used to append `cltq` after every signed one; it now extends
 * only when the value-level demand analysis (`needs_sext_values`) reports a
 * consumer that reads the 8-byte home raw: address arithmetic, 64-bit
 * arguments, x87 `long double` conversion, 64-bit selects/phis.  Explicit
 * widening casts extend on their own.  Every consumer kind below sees
 * negative, positive and boundary results; a missing extension shows up
 * as a huge positive value (or a wild address) instead of a negative one.
 */
#include <stdio.h>

static long sink[64];

__attribute__((noinline)) long take_long(long v) { return v * 3 + 1; }

__attribute__((noinline)) long idx_sar(const long *base, int x) {
    return base[(x >> 2) + 32];
}
__attribute__((noinline)) long idx_shl(const long *base, int x) {
    return base[(x << 1) + 32];
}
__attribute__((noinline)) long arg_sar(int x) { return take_long(x >> 3); }
__attribute__((noinline)) long double ld_sar(int x) { return (long double)(x >> 1); }
__attribute__((noinline)) double d_shl(int x) { return (double)(x << 4); }
__attribute__((noinline)) long cast_sar(int x) { return (long)(x >> 5) * 7; }
__attribute__((noinline)) long sel_sar(int x, int c) {
    long a = x >> 2, b = x << 3;
    return c ? a : b;
}
__attribute__((noinline)) long phi_acc(int x, int n) {
    long s = 0;
    for (int i = 0; i < n; i++) {
        int t = (x - i) >> 1;
        s += t;
    }
    return s;
}
__attribute__((noinline)) void store_wide(int x, int k) {
    long v = x >> 4;
    sink[k & 63] = v;
}
__attribute__((noinline)) unsigned long cmp_wide(int x) {
    long v = x >> 6;
    return v < -1L ? 1UL : 2UL;
}

int main(void) {
    static const int xs[] = {0, 1, -1, 7, -7, 64, -64, 123456, -123456,
                             0x3fffffff, -0x40000000, 0x7fffffff, (int)0x80000000u};
    long base[64];
    unsigned long h = 1469598103934665603ull;
    for (int i = 0; i < 64; i++) base[i] = i * 1000 + 17;
    for (unsigned k = 0; k < sizeof xs / sizeof xs[0]; k++) {
        int x = xs[k];
        int small = x % 64; /* keeps the indexes in bounds */
        long r[11];
        r[0] = idx_sar(base, small);
        r[1] = idx_shl(base, small % 16);
        r[2] = arg_sar(x);
        r[3] = (long)(ld_sar(x) * 2);
        r[4] = (long)d_shl(x % 0x8000000);
        r[5] = cast_sar(x);
        r[6] = sel_sar(x, 1);
        r[7] = sel_sar(x % 0x10000000, 0);
        r[8] = phi_acc(x % 0x1000000, 37);
        store_wide(x, k);
        r[9] = sink[k & 63];
        r[10] = (long)cmp_wide(x);
        for (int j = 0; j < 11; j++) {
            h = (h ^ (unsigned long)r[j]) * 1099511628211ull;
            printf("%ld%c", r[j], j == 10 ? '\n' : ' ');
        }
    }
    printf("hash %lu\n", h);
    return 0;
}
