// Integer madd/msub fusion (levkropp 9f064faa, audited port): on AArch64,
// madd/msub have the same latency as the mul they replace, so integer
// Mul;Add fuses even for register-homed temps, and `acc - a*b` fuses to
// msub. `a*b - acc` has NO msub form and must stay split (operand-order
// gate). rem10 is the magic-number-division shape (strlen/itoa).
int printf(const char *, ...);
unsigned rem10(unsigned n) { unsigned q = n / 10u; return n - q * 10u; }
long dot3(long a, long b, long c, long d) { long t = a * b; return t + c * d; }
int msub_ok(int a, int b, int c) { return c - a * b; }
int no_msub(int a, int b, int c) { return a * b - c; }
int main(void) {
    volatile unsigned n = 12347;
    volatile long p = 3, q = 4, r = 5, s = 6;
    volatile int x = 3, y = 4, z = 100;
    // Overflow-wrap sanity: fused and split must agree on wrapping.
    volatile int big = 0x7fffffff;
    printf("%u %ld %d %d %d\n", rem10(n), dot3(p, q, r, s), msub_ok(x, y, z),
           no_msub(x, y, z), msub_ok(big, 2, 5));
    return 0;
}
