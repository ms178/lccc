/* Narrow (8/16-bit) integer vector division and remainder.
 *
 * Scalarizing a narrow vector divide hands a value from a register home
 * straight to the 64-bit `idivq`. A home may hold only the producer's
 * 32-bit-canonical form: `movswl` gives `%eax` the right low 16 bits while the
 * upper half of `%rax` stays zero, so -1 reaches `idivq` as 0x0000FFFF and
 * `-1 / -8` comes back as 0 remainder 65535 instead of 0 remainder -1.
 *
 * The checker below never divides: it verifies the defining properties of
 * truncated division, so it is an oracle independent of the compiler's own
 * division lowering --
 *     q * d + r == n      (reconstruction, in exact integer arithmetic)
 *     |r| < |d|           (remainder magnitude)
 *     r == 0 || sign(r) == sign(n)   (C truncation keeps the dividend's sign)
 *
 * The sweeps are exhaustive over the representable dividends of each width for
 * a divisor set that exercises the trapping and width-crossing shapes (-1,
 * INT_MIN / -1, even powers of two, odd divisors, both signs). Lanes are split
 * across the vector so the scalarizer's lane loop runs to completion.
 */
#include <stdio.h>

typedef signed char v16qi __attribute__((vector_size(16)));
typedef unsigned char v16uqi __attribute__((vector_size(16)));
typedef short v8hi __attribute__((vector_size(16)));
typedef unsigned short v8uhi __attribute__((vector_size(16)));

static const short divisors16[] = {-8, -7, -3, -2, -1, 1, 2, 3, 7, 8, 15, 16, -16, -32768};
static const signed char divisors8[] = {-8, -7, -3, -2, -1, 1, 2, 3, 7, 8, 15, 16};

static unsigned long long checks;
static unsigned long long failures;

static void report(const char *what, long long n, long long d, long long q, long long r)
{
    ++failures;
    if (failures <= 16)
        printf("FAIL %s: %lld / %lld = %lld rem %lld (want %lld rem %lld)\n", what, n, d, q, r,
               n / d, n % d);
}

/* INT_MIN / -1 is the one quotient that does not fit the dividend's type: C
 * leaves it undefined, and a narrow `idiv` raises #DE on it -- GCC 16 and
 * Clang 23 both die with SIGFPE on a sweep that computes it. The dividend is
 * therefore substituted with the nearest DEFINED value before the divide, so
 * every vector actually computed is well-defined and every lane is checkable.
 * (Found by running the same source through all three compilers.) */
static short defined_dividend16(short v, short d)
{
    return (d == -1 && v == -32768) ? -32767 : v;
}

static signed char defined_dividend8(signed char v, signed char d)
{
    return (d == -1 && v == -128) ? -127 : v;
}

/* Truncated-division identities, checked without dividing. */
static int truncated_ok(long long n, long long d, long long q, long long r)
{
    if (d == 0)
        return 0;
    if (q * d + r != n)
        return 0;
    long long ad = d < 0 ? -d : d;
    long long ar = r < 0 ? -r : r;
    if (ar >= ad)
        return 0;
    if (r != 0 && ((r < 0) != (n < 0)))
        return 0;
    return 1;
}

static int unsigned_ok(unsigned long long n, unsigned long long d, unsigned long long q,
                       unsigned long long r)
{
    if (d == 0)
        return 0;
    return q * d + r == n && r < d;
}

__attribute__((noinline)) static v8hi sdiv16(v8hi x, v8hi y) { return x / y; }
__attribute__((noinline)) static v8hi srem16(v8hi x, v8hi y) { return x % y; }
__attribute__((noinline)) static v8uhi udiv16(v8uhi x, v8uhi y) { return x / y; }
__attribute__((noinline)) static v8uhi urem16(v8uhi x, v8uhi y) { return x % y; }
__attribute__((noinline)) static v16qi sdiv8(v16qi x, v16qi y) { return x / y; }
__attribute__((noinline)) static v16qi srem8(v16qi x, v16qi y) { return x % y; }
__attribute__((noinline)) static v16uqi udiv8(v16uqi x, v16uqi y) { return x / y; }
__attribute__((noinline)) static v16uqi urem8(v16uqi x, v16uqi y) { return x % y; }

static void sweep_signed16(void)
{
    for (unsigned long d = 0; d < sizeof divisors16 / sizeof divisors16[0]; ++d) {
        short div = divisors16[d];
        v8hi y = {div, div, div, div, div, div, div, div};
        for (long base = -32768; base <= 32760; base += 8) {
            v8hi x;
            for (int lane = 0; lane < 8; ++lane) {
                /* -32768 .. 32767, wrapped through the lane's own type. */
                x[lane] = defined_dividend16((short)(base + lane), div);
            }
            v8hi q = sdiv16(x, y);
            v8hi r = srem16(x, y);
            for (int lane = 0; lane < 8; ++lane) {
                ++checks;
                if (!truncated_ok(x[lane], div, q[lane], r[lane]))
                    report("i16", x[lane], div, q[lane], r[lane]);
            }
        }
    }
}

static void sweep_signed8(void)
{
    for (unsigned long d = 0; d < sizeof divisors8 / sizeof divisors8[0]; ++d) {
        signed char div = divisors8[d];
        v16qi y = {div, div, div, div, div, div, div, div, div, div, div, div, div, div, div, div};
        for (int base = -128; base <= 112; base += 16) {
            v16qi x;
            for (int lane = 0; lane < 16; ++lane)
                x[lane] = defined_dividend8((signed char)(base + lane), div);
            v16qi q = sdiv8(x, y);
            v16qi r = srem8(x, y);
            for (int lane = 0; lane < 16; ++lane) {
                ++checks;
                if (!truncated_ok(x[lane], div, q[lane], r[lane]))
                    report("i8", x[lane], div, q[lane], r[lane]);
            }
        }
    }
}

static void sweep_unsigned16(void)
{
    /* Every unsigned dividend, positive divisors only (C unsigned division is
     * defined for d != 0, and a negative literal is the same divisor here). */
    static const unsigned short udivisors[] = {1, 2, 3, 7, 8, 15, 16, 255, 256, 4095, 0x7fff,
                                               0x8000, 0x8001, 0xffff};
    for (unsigned long d = 0; d < sizeof udivisors / sizeof udivisors[0]; ++d) {
        unsigned short div = udivisors[d];
        v8uhi y = {div, div, div, div, div, div, div, div};
        for (unsigned long base = 0; base <= 0xffff; base += 8) {
            v8uhi x;
            for (int lane = 0; lane < 8; ++lane)
                x[lane] = (unsigned short)(base + (unsigned long)lane);
            v8uhi q = udiv16(x, y);
            v8uhi r = urem16(x, y);
            for (int lane = 0; lane < 8; ++lane) {
                ++checks;
                if (!unsigned_ok(x[lane], div, q[lane], r[lane]))
                    report("u16", x[lane], div, q[lane], r[lane]);
            }
        }
    }
}

static void sweep_unsigned8(void)
{
    static const unsigned char udivisors[] = {1, 2, 3, 7, 8, 15, 16, 127, 128, 129, 255};
    for (unsigned long d = 0; d < sizeof udivisors / sizeof udivisors[0]; ++d) {
        unsigned char div = udivisors[d];
        v16uqi y = {div, div, div, div, div, div, div, div, div, div, div, div, div, div, div, div};
        for (int base = 0; base <= 240; base += 16) {
            v16uqi x;
            for (int lane = 0; lane < 16; ++lane)
                x[lane] = (unsigned char)(base + lane);
            v16uqi q = udiv8(x, y);
            v16uqi r = urem8(x, y);
            for (int lane = 0; lane < 16; ++lane) {
                ++checks;
                if (!unsigned_ok(x[lane], div, q[lane], r[lane]))
                    report("u8", x[lane], div, q[lane], r[lane]);
            }
        }
    }
}

int main(void)
{
    sweep_signed16();
    sweep_signed8();
    sweep_unsigned16();
    sweep_unsigned8();
    printf("vector narrow div/rem: %llu checks, %llu failures\n", checks, failures);
    return failures != 0;
}
