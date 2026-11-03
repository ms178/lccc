/* Quotient and remainder of the same operands at every integer width.
 *
 * AArch64 fuses `x / y` and `x % y` into one `udiv`/`sdiv` plus `msub`, the
 * remainder landing in x3.  Storing it to a register home used the W view
 * for 32-bit and narrower types -- `mov x6, w3` (no such instruction) or
 * `fmov d8, w3` for a home in an FP register -- so the unit failed to
 * assemble at -O1 and up (gcc.c-torture execute/arith-rand.c).  The 32-bit
 * `msub` already zeroed bits 63:32; the X view is the correct copy.
 *
 * Self-checking against the division identity on edge values and LCG
 * draws: exit status 0 is success; abort() otherwise. */
extern void abort(void);

static unsigned long long seed = 47114711;

static unsigned long long next(void)
{
    seed = seed * 6364136223846793005ull + 1442695040888963407ull;
    return seed ^ (seed >> 29);
}

#define ABS(x) ((x) >= 0 ? (x) : -(x))

#define UCHECK(T, x, y)                                         \
    do {                                                        \
        T xx = (T)(x), yy = (T)(y), q, r;                       \
        if (yy != 0) {                                          \
            q = xx / yy;                                        \
            r = xx % yy;                                        \
            if (r >= yy || (T)(q * yy + r) != xx)               \
                abort();                                        \
            acc += q ^ r;                                       \
        }                                                       \
    } while (0)

#define SCHECK(T, UT, x, y)                                     \
    do {                                                        \
        T xx = (T)(x), yy = (T)(y), q, r;                       \
        if (yy != 0 && !(yy == -1 && (UT)xx << 1 == 0)) {       \
            q = xx / yy;                                        \
            r = xx % yy;                                        \
            if ((UT)ABS(r) >= (UT)ABS(yy) || (T)(UT)((UT)q * (UT)yy + (UT)r) != xx) \
                abort();                                        \
            acc += (unsigned long long)(q ^ r);                 \
        }                                                       \
    } while (0)

__attribute__((noinline)) static unsigned long long all_widths(unsigned long long x,
                                                               unsigned long long y)
{
    unsigned long long acc = 0;
    UCHECK(unsigned long long, x, y);
    SCHECK(long long, unsigned long long, x, y);
    UCHECK(unsigned int, x, y);
    SCHECK(int, unsigned int, x, y);
    UCHECK(unsigned short, x, y);
    SCHECK(short, unsigned short, x, y);
    UCHECK(unsigned char, x, y);
    SCHECK(signed char, unsigned char, x, y);
    return acc;
}

static const unsigned long long edges[] = {
    0, 1, 2, 3, 7, 0x7f, 0x80, 0xff, 0x7fff, 0x8000, 0xffff, 0x7fffffff,
    0x80000000, 0xffffffff, 0x100000000ull, 0x7fffffffffffffffull,
    0x8000000000000000ull, 0xffffffffffffffffull, 0xfffffffffffffffeull,
};

int main(void)
{
    unsigned long long sum = 0;
    unsigned n = sizeof(edges) / sizeof(edges[0]);
    for (unsigned i = 0; i < n; i++)
        for (unsigned j = 0; j < n; j++)
            sum += all_widths(edges[i], edges[j]);
    for (int i = 0; i < 4000; i++) {
        unsigned long long x = next(), y = next() >> (next() & 63);
        sum += all_widths(x, y);
    }
    return sum == 0 ? 1 : 0;
}
