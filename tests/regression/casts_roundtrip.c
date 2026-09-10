// Numeric conversion battery: signedness changes, float<->int boundaries,
// identity-cast representation preservation, and constant-fold correctness.
// Regression for the i686 audit (constant-fold drop of float->int cast
// chains, canonical subword register forms, wide identity truncation).
#include <stdint.h>
#include <string.h>

typedef long double f128;
typedef unsigned long long u64;
typedef long long i64;

static int fails = 0;
#define CHECK(cond) do { if (!(cond)) { fails++; } } while (0)

static uint32_t f32_bits(float f) { uint32_t b; memcpy(&b, &f, 4); return b; }
static uint64_t f64_bits(double d) { uint64_t b; memcpy(&b, &d, 8); return b; }

/* constant fold through a float->int cast chain must not collapse to 0 */
static unsigned fold_u32_of_int_of_ld(void) { return (uint32_t)(int)3.999L; }
static unsigned fold_u32_of_int_of_dbl(void) { return (uint32_t)(int)(double)3.999L; }
static int fold_i32_of_ld(void) { return (int)3.999L; }
static unsigned fold_via_f128_ident(void) { return (uint32_t)(int)(f128)3.999L; }
static u64 fold_u64_of_ld(void) { return (u64)(f128)123.75L; }

static double ident_f64(double a) { return (double)(double)a; }
static f128 ident_f128(f128 a) { return (f128)(f128)a; }

int main(void) {
    /* constant folding (regression: folded to 0 before the fix) */
    CHECK(fold_u32_of_int_of_ld() == 3);
    CHECK(fold_u32_of_int_of_dbl() == 3);
    CHECK(fold_i32_of_ld() == 3);
    CHECK(fold_via_f128_ident() == 3);
    CHECK(fold_u64_of_ld() == 123);
    CHECK((uint32_t)(int)(-3.999L) == 0xfffffffdU);
    CHECK((int)(f128)3.999L == 3);

    /* signedness changes re-canonicalize (U8->I8, U16->I16, I8->U16) */
    for (int v = 0; v < 256; v++) {
        signed char sc = (signed char)(unsigned char)v;
        CHECK((int)sc == (v < 128 ? v : v - 256));
        CHECK(sc < 0 == (v >= 128));
        CHECK((int)(unsigned short)(signed char)v == (unsigned short)sc);
    }
    CHECK((int)(unsigned short)(signed char)(-1) == 65535);
    CHECK((int)(unsigned short)(signed char)(-128) == 65408);
    CHECK((short)(unsigned short)65535 == -1);
    CHECK((signed char)(unsigned char)255 == -1);
    CHECK((unsigned char)(short)(-1) == 255);

    /* float -> unsigned boundaries (runtime values, not folded) */
    {
        volatile float a = 3221225472.0f;   /* 3*2^30 */
        CHECK((uint32_t)a == 0xc0000000u);
        volatile float b = 2147483648.0f;   /* 2^31 */
        CHECK((uint32_t)b == 0x80000000u);
        volatile double c = 13835058055282163712.0; /* 3*2^62 */
        CHECK((u64)c == 0xc000000000000000ull);
        volatile f128 d = 18446744073709551615.0L;  /* 2^64-1 */
        CHECK((u64)d == 0xffffffffffffffffull);
        volatile f128 e = 9223372036854775809.0L;   /* 2^63+1 */
        CHECK((u64)e == 0x8000000000000001ull);
    }

    /* unsigned -> float round trips */
    {
        volatile u64 v = (1ull << 63) + 1;
        CHECK((u64)(f128)v == v);
        /* double cannot represent 2^63+1; test an exactly representable one */
        volatile u64 v2 = (1ull << 63) + 2048;
        CHECK((u64)(double)v2 == v2);
        volatile u64 w = 0xffffffffffffffffull;
        CHECK((u64)(f128)w == w);
        volatile uint32_t u = 0xffffffffu;
        CHECK((uint32_t)(f128)u == u);
        CHECK(f32_bits((float)3000000000u) == 0x4f32d05e); /* exact single rounding */
    }

    /* signed 64-bit round trips */
    {
        volatile i64 s = -9223372036854775807ll - 1;
        CHECK((i64)(f128)s == s);
        CHECK((i64)(double)s == s);
    }

    /* identity casts preserve the full representation */
    {
        double a; uint64_t p = 0x7ff8000000001234ull; memcpy(&a, &p, 8);
        CHECK(f64_bits(ident_f64(a)) == p);
        double z = -0.0;
        CHECK(f64_bits(ident_f64(z)) == 0x8000000000000000ull);
        f128 q = 3.999L;
        CHECK(ident_f128(q) == q);
    }

    return fails;
}
