/* Truncating float/double -> integer conversions at the edges of every
 * destination's DEFINED domain (C23 6.3.1.4p1: the value truncates toward
 * zero and must be representable).  Every input is exactly representable in
 * its source type and every result is defined, so this compares against GCC
 * without depending on what any compiler does for out-of-range inputs.
 *
 * Regression: float -> unsigned long long for values >= 2^63 took the plain
 * signed `cvttss2siq` path on x86-64 and produced 0x8000000000000000; the
 * double path had the 2^63 bias but the float path did not.  The narrow
 * destinations pin the 32-bit / 16-bit conversion forms (x86-64 cvtts?2si,
 * i686 fisttps/fisttpl/fisttpq). */
#include <stdio.h>

#define NI __attribute__((noinline))

#define CONV(name, S, D) \
    NI static D name(S x) { return (D)x; }

CONV(f_u64, float, unsigned long long)
CONV(d_u64, double, unsigned long long)
CONV(f_i64, float, long long)
CONV(d_i64, double, long long)
CONV(f_u32, float, unsigned)
CONV(d_u32, double, unsigned)
CONV(f_i32, float, int)
CONV(d_i32, double, int)
CONV(f_u16, float, unsigned short)
CONV(d_u16, double, unsigned short)
CONV(f_i16, float, short)
CONV(d_i16, double, short)
CONV(f_u8, float, unsigned char)
CONV(d_u8, double, unsigned char)
CONV(f_i8, float, signed char)
CONV(d_i8, double, signed char)
CONV(f_b, float, _Bool)
CONV(d_b, double, _Bool)

/* volatile: the conversions must run on the target, not in the folder. */
static volatile float vf[] = {
    0.0f, -0.0f, 0.75f, -0.75f, 1.5f, -1.5f, 127.75f, -128.5f, 255.5f,
    32767.5f, -32768.75f, 65535.5f,
    2147483520.0f,          /* largest float below 2^31 */
    -2147483648.0f,         /* -2^31 */
    4294967040.0f,          /* largest float below 2^32 */
    9223371487098961920.0f, /* largest float below 2^63 */
    9223372036854775808.0f, /* 2^63: u64 only */
    9.3e18f,                /* in (2^63, 2^64) */
    18446742974197923840.0f /* largest float below 2^64 */
};
static volatile double vd[] = {
    0.0, -0.0, 0.999999999, -0.999999999, 1.5, -1.5, 127.99, -128.99, 255.99,
    32767.99, -32768.99, 65535.99,
    2147483647.9,                 /* INT_MAX + 0.9 */
    -2147483648.9,                /* INT_MIN - 0.9 */
    4294967295.5,                 /* UINT_MAX + 0.5 */
    9223372036854774784.0,        /* largest double below 2^63 */
    9223372036854775808.0,        /* 2^63 */
    9223372036854777856.0,        /* 2^63 + 2^11 */
    18446744073709549568.0        /* largest double below 2^64 */
};

/* Which destinations each input is defined for (index-aligned). */
enum { U64 = 1, I64 = 2, U32 = 4, I32 = 8, U16 = 16, I16 = 32, U8 = 64, I8 = 128 };
static const int f_ok[] = {
    255, 255, 255, 255, 255, 170, 255, 170, 1 | 2 | 4 | 8 | 16 | 32 | 64,
    1 | 2 | 4 | 8 | 16 | 32, 2 | 8 | 32, 1 | 2 | 4 | 8 | 16,
    1 | 2 | 4 | 8, 2 | 8, 1 | 2 | 4, 1 | 2, 1, 1, 1
};
static const int d_ok[] = {
    255, 255, 255, 255, 255, 170, 255, 170, 1 | 2 | 4 | 8 | 16 | 32 | 64,
    1 | 2 | 4 | 8 | 16 | 32, 2 | 8 | 32, 1 | 2 | 4 | 8 | 16,
    1 | 2 | 4 | 8, 2 | 8, 1 | 2 | 4, 1 | 2, 1, 1, 1
};

#define N(a) ((int)(sizeof(a) / sizeof((a)[0])))
_Static_assert(N(vf) == N(f_ok) && N(vd) == N(d_ok), "tables aligned");

int main(void) {
    for (int i = 0; i < N(vf); i++) {
        float x = vf[i];
        int ok = f_ok[i];
        printf("f[%d] b=%d", i, (int)f_b(x));
        if (ok & U64) printf(" u64=%llu", f_u64(x));
        if (ok & I64) printf(" i64=%lld", f_i64(x));
        if (ok & U32) printf(" u32=%u", f_u32(x));
        if (ok & I32) printf(" i32=%d", f_i32(x));
        if (ok & U16) printf(" u16=%u", (unsigned)f_u16(x));
        if (ok & I16) printf(" i16=%d", (int)f_i16(x));
        if (ok & U8) printf(" u8=%u", (unsigned)f_u8(x));
        if (ok & I8) printf(" i8=%d", (int)f_i8(x));
        printf("\n");
    }
    for (int i = 0; i < N(vd); i++) {
        double x = vd[i];
        int ok = d_ok[i];
        printf("d[%d] b=%d", i, (int)d_b(x));
        if (ok & U64) printf(" u64=%llu", d_u64(x));
        if (ok & I64) printf(" i64=%lld", d_i64(x));
        if (ok & U32) printf(" u32=%u", d_u32(x));
        if (ok & I32) printf(" i32=%d", d_i32(x));
        if (ok & U16) printf(" u16=%u", (unsigned)d_u16(x));
        if (ok & I16) printf(" i16=%d", (int)d_i16(x));
        if (ok & U8) printf(" u8=%u", (unsigned)d_u8(x));
        if (ok & I8) printf(" i8=%d", (int)d_i8(x));
        printf("\n");
    }
    return 0;
}
