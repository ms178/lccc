/* Multiply-by-constant and shifted-operand fusion, FP-to-integer
 * conversions, and the RV64 FP slot/copy forwarding, executed on AArch64
 * and RV64.
 *
 * 1  x*k + c and c - x*k for the multipliers AArch64 lowers to one shifted
 *    add/sub (3, 5, 9, 17, 33, 65: x + (x << s); 7, 15, 31, 63:
 *    (x << s) - x) or to that plus a shift (6, 10, 12, 24), the powers of
 *    two, and a negative multiplier, with the constant addend at the 12-bit
 *    immediate limit (+-4095) and past it (+-4096): 32 and 64 bits, signed
 *    (in range) and unsigned (wrapping).
 * 2  a + x*k and a - x*k with a register addend, and the accumulator loops
 *    r = r*k + a[i] / r = a[i] - r*k whose destination is their addend.
 * 3  a + (b >> s), (b << s) + a, a - (b << s), a - (b >> s): arithmetic and
 *    logical shifts, 32 and 64 bits, counts 1..63; two results also feed a
 *    bit test or a branch, where a second fusion wants the same add.
 * 4  (long), (int), (unsigned), (unsigned long), (short), (unsigned char)
 *    of a double and (long), (int), (unsigned), (signed char),
 *    (unsigned short) of a float: truncation toward zero at the in-range
 *    boundaries; results returned, stored, compared, added to and widened
 *    (an int must arrive sign-extended in a long, an unsigned
 *    zero-extended).
 * 5  FP leaves whose RV64 frame slots, copies and frames the peephole
 *    removes: a+b in both precisions, a<b, -a, selects, a*b+c, a*b + c*a
 *    (a temporary spilled across another product), a straight-line
 *    polynomial, a temporary that crosses a join, and identity functions --
 *    every copy checked bit for bit (NaN payloads, signed zeros).
 * 6  bit casts between the register files: double <-> long, float -> int
 *    (sign-extended), float -> unsigned (zero-extended), unsigned -> float.
 * 7  narrow results: AAPCS64 leaves the bits of x0 above an int, short or
 *    char result unspecified, and LCCC's AArch64 callee no longer extends
 *    them on the way to `ret` -- so every caller must: each result is used
 *    as a 64-bit value and as a table index, with arguments whose upper
 *    bits differ from the result's extension.  The RV64 psABI instead
 *    returns an int sign-extended: an int truncated from a 64-bit constant
 *    must arrive as the sign-extended word, not as the constant.
 * 8  multipliers inside loops, where the loop-invariant constant hoist
 *    decides whether the backend still sees the factor: one-instruction
 *    AArch64 factors (3, 5, 9, 17, 33, -3) and the RV64 shift-and-add
 *    factors (also 7, 15, 31) stay constants, the others (6, 10, 1000) are
 *    hoisted into a register -- djb2 and x31 string hashes, accumulations,
 *    an in-place scale, a wrapping recurrence, a constant that is also a
 *    divisor and a remainder modulus in the same loop, and a nested loop.
 *
 * Expected values are computed by the generator (exact integer and IEEE
 * arithmetic), never by the compiler under test.  GCC 16.2 and Clang
 * 23.1.5 on x86-64 (-O0, -O2, -O3; -fsanitize=undefined
 * -fsanitize-trap=all clean) agree.  The cross harness runs this at -O0,
 * -O1, -O2, -Os and -O3.  Exit status: the number of the first failing
 * check, 0 when all pass.
 */
#define NOINLINE __attribute__((noinline))
#define N(a) ((int)(sizeof(a) / sizeof((a)[0])))
typedef unsigned long u64;
typedef unsigned u32;

/* 1 */
NOINLINE int mc3p7(int x) { return x * 3 + 7; }
NOINLINE int mc5p4095(int x) { return x * 5 + 4095; }
NOINLINE int mc9p4096(int x) { return x * 9 + 4096; }
NOINLINE int mc7m4095(int x) { return x * 7 - 4095; }
NOINLINE int mc15m4096(int x) { return x * 15 - 4096; }
NOINLINE int c100m3(int x) { return 100 - x * 3; }
NOINLINE int c4095m17(int x) { return 4095 - x * 17; }
NOINLINE int cm4096m5(int x) { return -4096 - x * 5; }
NOINLINE int mc6p1(int x) { return x * 6 + 1; }
NOINLINE int mc10m3(int x) { return x * 10 - 3; }
NOINLINE int mc12p4095(int x) { return x * 12 + 4095; }
NOINLINE int mc24m1(int x) { return x * 24 - 1; }
NOINLINE int mcn3p2(int x) { return x * -3 + 2; }
NOINLINE int cn7(int x) { return 9 - x * -7; }
NOINLINE int mc2p3(int x) { return x * 2 + 3; }
NOINLINE int mc4m1(int x) { return x * 4 - 1; }
NOINLINE long lmc33p1(long x) { return x * 33 + 1; }
NOINLINE long lmc63m9(long x) { return x * 63 - 9; }
NOINLINE long lmc6p5(long x) { return x * 6 + 5; }
NOINLINE long lc12(long x) { return -4095 - x * 12; }
NOINLINE long lmc65p4096(long x) { return x * 65 + 4096; }
NOINLINE long lmc31m4095(long x) { return x * 31 - 4095; }
NOINLINE long lc3(long x) { return 4095 - x * 3; }
NOINLINE long lmc5(long x) { return x * 5; }
NOINLINE u32 umc65p3(u32 x) { return x * 65u + 3u; }
NOINLINE u32 umc31m1(u32 x) { return x * 31u - 1u; }
NOINLINE u32 uc7(u32 x) { return 4095u - x * 7u; }
NOINLINE u32 umc17p4096(u32 x) { return x * 17u + 4096u; }
NOINLINE u32 umc10(u32 x) { return x * 10u + 4095u; }
NOINLINE u64 ulmc10p7(u64 x) { return x * 10ul + 7ul; }
NOINLINE u64 ulc24(u64 x) { return 4000ul - x * 24ul; }
NOINLINE u64 ulmc9m4095(u64 x) { return x * 9ul - 4095ul; }
NOINLINE u64 ulmc63p1(u64 x) { return x * 63ul + 1ul; }
/* 2 */
NOINLINE int madd6(int a, int x) { return a + x * 6; }
NOINLINE int msub7(int a, int x) { return a - x * 7; }
NOINLINE int madd3l(int a, int x) { return x * 3 + a; }
NOINLINE int msub9(int a, int x) { return a - x * 9; }
NOINLINE int madd12(int a, int x) { return x * 12 + a; }
NOINLINE long lmadd9(long a, long x) { return x * 9 + a; }
NOINLINE long lmsub5(long a, long x) { return a - x * 5; }
NOINLINE long lmadd15(long a, long x) { return a + x * 15; }
NOINLINE long lmsub10(long a, long x) { return a - x * 10; }
NOINLINE u32 poly3(const u32 *a, int n)
{
    u32 r = 0;
    for (int i = 0; i < n; i++)
        r = r * 3u + a[i];
    return r;
}
NOINLINE u64 poly7(const u64 *a, int n)
{
    u64 r = 1;
    for (int i = 0; i < n; i++)
        r = a[i] - r * 7ul;
    return r;
}
/* 3 */
NOINLINE int add_asr3(int a, int b) { return a + (b >> 3); }
NOINLINE int add_lsl4_lhs(int a, int b) { return (b << 4) + a; }
NOINLINE int sub_lsl4(int a, int b) { return a - (b << 4); }
NOINLINE int sub_asr31(int a, int b) { return a - (b >> 31); }
NOINLINE int add_asr1_lhs(int a, int b) { return (b >> 1) + a; }
NOINLINE u32 uadd_lsr1(u32 a, u32 b) { return a + (b >> 1); }
NOINLINE u32 usub_lsr31(u32 a, u32 b) { return a - (b >> 31); }
NOINLINE u32 uadd_lsl31(u32 a, u32 b) { return a + (b << 31); }
NOINLINE u32 usub_lsl5(u32 a, u32 b) { return a - (b << 5); }
NOINLINE long ladd_asr63(long a, long b) { return a + (b >> 63); }
NOINLINE long lsub_lsl3(long a, long b) { return a - (b << 3); }
NOINLINE long ladd_asr7_lhs(long a, long b) { return (b >> 7) + a; }
NOINLINE u64 uladd_lsr7(u64 a, u64 b) { return (b >> 7) + a; }
NOINLINE u64 ulsub_lsl63(u64 a, u64 b) { return a - (b << 63); }
NOINLINE int bit_of_sum(int a, int b)
{
    int s = a + (b >> 2);
    return (s >> 5) & 1 ? s : -s;
}
NOINLINE int add_lsl_branch(int a, int b, int c)
{
    int s = a + (b << 2);
    if (s & 16)
        return s + c;
    return s - c;
}
/* 4 */
NOINLINE long cvt_l(double x) { return (long)x; }
NOINLINE int cvt_i(double x) { return (int)x; }
NOINLINE u32 cvt_ud(double x) { return (u32)x; }
NOINLINE u64 cvt_ul(double x) { return (u64)x; }
NOINLINE short cvt_s(double x) { return (short)x; }
NOINLINE unsigned char cvt_uc(double x) { return (unsigned char)x; }
NOINLINE long cvt_fl(float x) { return (long)x; }
NOINLINE int cvt_fi(float x) { return (int)x; }
NOINLINE u32 cvt_u(float x) { return (u32)x; }
NOINLINE signed char cvt_sc(float x) { return (signed char)x; }
NOINLINE unsigned short cvt_us(float x) { return (unsigned short)x; }
NOINLINE long cvt_add(double d, long x) { return (long)d + x; }
NOINLINE long cvt_i_widen(double d) { int i = (int)d; return i; }
NOINLINE u64 cvt_u_widen(float f) { u32 u = (u32)f; return u; }
NOINLINE int cvt_lt(double d, int k) { return (int)d < k; }
NOINLINE void halve_add(int *restrict p, const double *restrict d, int n)
{
    for (int i = 0; i < n; i++) {
        int v = (int)d[i];
        p[i] = v + (v >> 1);
    }
}
NOINLINE long sum_cvt(const float *f, int n)
{
    long s = 0;
    for (int i = 0; i < n; i++)
        s += (long)f[i] - (int)f[i] + (u32)(f[i] < 0 ? -f[i] : f[i]);
    return s;
}
/* 5 */
NOINLINE double addd(double a, double b) { return a + b; }
NOINLINE float addf(float a, float b) { return a + b; }
NOINLINE int cmpd(double a, double b) { return a < b; }
NOINLINE int cmpf(float a, float b) { return a <= b; }
NOINLINE double negd(double a) { return -a; }
NOINLINE float negf(float a) { return -a; }
NOINLINE double seld(double a, double b, int c) { return c ? a : b; }
NOINLINE float self(float a, float b, int c) { return c ? a : b; }
NOINLINE double fmad(double a, double b, double c) { return a * b + c; }
NOINLINE double muladd(double a, double b, double c) { double t = a * b; return t + c * a; }
NOINLINE float muladdf(float a, float b, float c) { float t = a * b; return t + c * a; }
NOINLINE double identd(double a) { return a; }
NOINLINE float identf(float a) { return a; }
NOINLINE double swapsub(double a, double b) { return b - a; }
NOINLINE float addf3(float a, float b, float c) { return (a + b) * c - a; }
NOINLINE double poly5(double x)
{
    double x2 = x * x, x3 = x2 * x, x4 = x2 * x2;
    return 1.0 + 2.0 * x + 3.0 * x2 + 4.0 * x3 + 5.0 * x4 - x4 * x;
}
NOINLINE double join_tmp(double a, double b, double c)
{
    double t = a * b;
    if (c > 0)
        t += c;
    return t * a;
}
NOINLINE double pick3(double a, double b, double c, int k)
{
    double r = k > 0 ? a : b;
    return k < -1 ? c : r;
}
/* 6 */
NOINLINE long dbits(double d) { union { double d; long l; } v = { d }; return v.l; }
NOINLINE double bitsd(long l) { union { long l; double d; } v = { l }; return v.d; }
NOINLINE int fbits(float f) { union { float f; int i; } v = { f }; return v.i; }
NOINLINE u32 fbitsu(float f) { union { float f; u32 u; } v = { f }; return v.u; }
NOINLINE float bitsf(u32 u) { union { u32 u; float f; } v = { u }; return v.f; }
NOINLINE long fbits_wide(float f) { return fbits(f); }
NOINLINE u64 fbitsu_wide(float f) { return fbitsu(f); }

/* 7 */
NOINLINE int narrow(long x) { return (int)x; }
NOINLINE unsigned char lowb(unsigned x) { return (unsigned char)x; }
NOINLINE short mul16(int a, int b) { return (short)(a * b); }
NOINLINE signed char sc8(long x) { return (signed char)x; }
NOINLINE unsigned short us16(long x) { return (unsigned short)x; }
NOINLINE unsigned u32_of(long x) { return (unsigned)x; }
NOINLINE _Bool nonzero(long x) { return x != 0; }
NOINLINE int trunc_wide(void) { long c = 0x100000001L; return (int)c; }
NOINLINE int trunc_wide_neg(void) { long c = 0x1fffffffeL; return (int)c; }
NOINLINE int sum_narrow(const long *p, int n)
{
    int s = 0;
    for (int i = 0; i < n; i++)
        s += narrow(p[i]);
    return s;
}

/* 8 */
NOINLINE u32 djb2(const unsigned char *s, int n)
{
    u32 h = 5381;
    for (int i = 0; i < n; i++)
        h = h * 33u + s[i];
    return h;
}
NOINLINE u32 hash31(const unsigned char *s, int n)
{
    u32 h = 0;
    for (int i = 0; i < n; i++)
        h = h * 31u + s[i];
    return h;
}
NOINLINE int sum5(const int *a, int n)
{
    int s = 0;
    for (int i = 0; i < n; i++)
        s += a[i] * 5;
    return s;
}
NOINLINE long sum6l(const long *a, int n)
{
    long s = 0;
    for (int i = 0; i < n; i++)
        s += a[i] * 6;
    return s;
}
NOINLINE long sum1000(const int *a, int n)
{
    long s = 0;
    for (int i = 0; i < n; i++)
        s += a[i] * 1000;
    return s;
}
NOINLINE void scale_m3(long *a, int n)
{
    for (int i = 0; i < n; i++)
        a[i] *= -3;
}
NOINLINE u64 mix9(const u64 *a, int n)
{
    u64 r = 7;
    for (int i = 0; i < n; i++)
        r = r * 9ul - a[i];
    return r;
}
NOINLINE u32 xor7(const u32 *a, int n)
{
    u32 r = 0;
    for (int i = 0; i < n; i++)
        r ^= a[i] * 7u + (a[i] >> 3) * 10u;
    return r;
}
NOINLINE int mul_div3(const int *a, int n)
{
    int s = 0;
    for (int i = 0; i < n; i++)
        s += a[i] * 3 + a[i] / 3 + a[i] % 3;
    return s;
}
NOINLINE u64 nested17(const u64 *a, int n)
{
    u64 s = 0;
    for (int j = 0; j < 3; j++)
        for (int i = 0; i < n; i++)
            s = s * 17ul + a[i] * 15ul;
    return s;
}

static u64 bd(double d) { union { double d; u64 u; } v = { d }; return v.u; }
static u32 bf(float f) { union { float f; u32 u; } v = { f }; return v.u; }
static double db(u64 u) { union { u64 u; double d; } v = { u }; return v.d; }
static float fb(u32 u) { union { u32 u; float f; } v = { u }; return v.f; }

static const int INT_XS[] = { 0, 1, -1, 2, 7, -7, 1000, -1000, 123456, -123456, 4095, -4096, 46340, -46341, 10000000, -10000000 };
static const long LONG_XS[] = { 0L, 1L, -1L, 3L, -3L, 4096L, -4097L, 8589934592L, -8589934592L, 123456789012L, -123456789012L, 1099511627781L, -1099511627781L };
static const u32 U32_XS[] = { 0U, 1U, 2U, 2147483647U, 2147483648U, 4294967295U, 305419896U, 3735928559U, 66076419U, 66076419U };
static const u64 U64_XS[] = { 0UL, 1UL, 2UL, 9223372036854775808UL, 18446744073709551615UL, 81985529216486895UL, 18364758544493064720UL, 4294967296UL, 4294967295UL };
static const int E_mc3p7[] = { 7, 10, 4, 13, 28, -14, 3007, -2993, 370375, -370361, 12292, -12281, 139027, -139016, 30000007, -29999993 };
static const int E_mc5p4095[] = { 4095, 4100, 4090, 4105, 4130, 4060, 9095, -905, 621375, -613185, 24570, -16385, 235795, -227610, 50004095, -49995905 };
static const int E_mc9p4096[] = { 4096, 4105, 4087, 4114, 4159, 4033, 13096, -4904, 1115200, -1107008, 40951, -32768, 421156, -412973, 90004096, -89995904 };
static const int E_mc7m4095[] = { -4095, -4088, -4102, -4081, -4046, -4144, 2905, -11095, 860097, -868287, 24570, -32767, 320285, -328482, 69995905, -70004095 };
static const int E_mc15m4096[] = { -4096, -4081, -4111, -4066, -3991, -4201, 10904, -19096, 1847744, -1855936, 57329, -65536, 691004, -699211, 149995904, -150004096 };
static const int E_c100m3[] = { 100, 97, 103, 94, 79, 121, -2900, 3100, -370268, 370468, -12185, 12388, -138920, 139123, -29999900, 30000100 };
static const int E_c4095m17[] = { 4095, 4078, 4112, 4061, 3976, 4214, -12905, 21095, -2094657, 2102847, -65520, 73727, -783685, 791892, -169995905, 170004095 };
static const int E_cm4096m5[] = { -4096, -4101, -4091, -4106, -4131, -4061, -9096, 904, -621376, 613184, -24571, 16384, -235796, 227609, -50004096, 49995904 };
static const int E_mc6p1[] = { 1, 7, -5, 13, 43, -41, 6001, -5999, 740737, -740735, 24571, -24575, 278041, -278045, 60000001, -59999999 };
static const int E_mc10m3[] = { -3, 7, -13, 17, 67, -73, 9997, -10003, 1234557, -1234563, 40947, -40963, 463397, -463413, 99999997, -100000003 };
static const int E_mc12p4095[] = { 4095, 4107, 4083, 4119, 4179, 4011, 16095, -7905, 1485567, -1477377, 53235, -45057, 560175, -551997, 120004095, -119995905 };
static const int E_mc24m1[] = { -1, 23, -25, 47, 167, -169, 23999, -24001, 2962943, -2962945, 98279, -98305, 1112159, -1112185, 239999999, -240000001 };
static const int E_mcn3p2[] = { 2, -1, 5, -4, -19, 23, -2998, 3002, -370366, 370370, -12283, 12290, -139018, 139025, -29999998, 30000002 };
static const int E_cn7[] = { 9, 16, 2, 23, 58, -40, 7009, -6991, 864201, -864183, 28674, -28663, 324389, -324378, 70000009, -69999991 };
static const int E_mc2p3[] = { 3, 5, 1, 7, 17, -11, 2003, -1997, 246915, -246909, 8193, -8189, 92683, -92679, 20000003, -19999997 };
static const int E_mc4m1[] = { -1, 3, -5, 7, 27, -29, 3999, -4001, 493823, -493825, 16379, -16385, 185359, -185365, 39999999, -40000001 };
static const long E_lmc33p1[] = { 1L, 34L, -32L, 100L, -98L, 135169L, -135200L, 283467841537L, -283467841535L, 4074074037397L, -4074074037395L, 36283883716774L, -36283883716772L };
static const long E_lmc63m9[] = { -9L, 54L, -72L, 180L, -198L, 258039L, -258120L, 541165879287L, -541165879305L, 7777777707747L, -7777777707765L, 69269232550194L, -69269232550212L };
static const long E_lmc6p5[] = { 5L, 11L, -1L, 23L, -13L, 24581L, -24577L, 51539607557L, -51539607547L, 740740734077L, -740740734067L, 6597069766691L, -6597069766681L };
static const long E_lc12[] = { -4095L, -4107L, -4083L, -4131L, -4059L, -53247L, 45069L, -103079219199L, 103079211009L, -1481481472239L, 1481481464049L, -13194139537467L, 13194139529277L };
static const long E_lmc65p4096[] = { 4096L, 4161L, 4031L, 4291L, 3901L, 270336L, -262209L, 558345752576L, -558345744384L, 8024691289876L, -8024691281684L, 71468255809861L, -71468255801669L };
static const long E_lmc31m4095[] = { -4095L, -4064L, -4126L, -4002L, -4188L, 122881L, -131102L, 266287968257L, -266287976447L, 3827160455277L, -3827160463467L, 34084860457116L, -34084860465306L };
static const long E_lc3[] = { 4095L, 4092L, 4098L, 4086L, 4104L, -8193L, 16386L, -25769799681L, 25769807871L, -370370362941L, 370370371131L, -3298534879248L, 3298534887438L };
static const long E_lmc5[] = { 0L, 5L, -5L, 15L, -15L, 20480L, -20485L, 42949672960L, -42949672960L, 617283945060L, -617283945060L, 5497558138905L, -5497558138905L };
static const u32 E_umc65p3[] = { 3U, 68U, 133U, 2147483586U, 2147483651U, 4294967234U, 2672424059U, 2317187762U, 4294967238U, 4294967238U };
static const u32 E_umc31m1[] = { 4294967295U, 30U, 61U, 2147483616U, 2147483647U, 4294967264U, 878082183U, 4144635632U, 2048368988U, 2048368988U };
static const u32 E_uc7[] = { 4095U, 4088U, 4081U, 2147487750U, 2147487743U, 4102U, 2157032119U, 3913275254U, 3832436458U, 3832436458U };
static const u32 E_umc17p4096[] = { 4096U, 4113U, 4130U, 2147487727U, 2147487744U, 4079U, 897175032U, 3381247455U, 1123303219U, 1123303219U };
static const u32 E_umc10[] = { 4095U, 4105U, 4115U, 4085U, 4095U, 4085U, 3054203055U, 2999551317U, 660768285U, 660768285U };
static const u64 E_ulmc10p7[] = { 7UL, 17UL, 27UL, 7UL, 18446744073709551613UL, 819855292164868957UL, 17626888781544682663UL, 42949672967UL, 42949672957UL };
static const u64 E_ulc24[] = { 4000UL, 3976UL, 3952UL, 4000UL, 4024UL, 16479091372513870136UL, 1967652701195689504UL, 18446743970630340512UL, 18446743970630340536UL };
static const u64 E_ulmc9m4095[] = { 18446744073709547521UL, 18446744073709547530UL, 18446744073709547539UL, 9223372036854771713UL, 18446744073709547512UL, 737869762948377960UL, 17708874310761165457UL, 38654701569UL, 38654701560UL };
static const u64 E_ulmc63p1[] = { 1UL, 64UL, 127UL, 9223372036854775809UL, 18446744073709551554UL, 5165088340638674386UL, 13281655733070877169UL, 270582939649UL, 270582939586UL };
static const int PA_I[] = { 0, 1, -5, 100000, -2147480000, 2147480000, 17 };
static const int PX_I[] = { 0, 1, 3, -2000, 400, -300, 123456 };
static const long PA_L[] = { 0L, -1L, 1099511627776L, -1125899906842624L, 123L };
static const long PX_L[] = { 0L, 1L, -34359738368L, 35184372088832L, -456L };
static const int E_madd6[] = { 0, 7, 13, 88000, -2147477600, 2147478200, 740753 };
static const int E_msub7[] = { 0, -6, -26, 114000, -2147482800, 2147482100, -864175 };
static const int E_madd3l[] = { 0, 4, 4, 94000, -2147478800, 2147479100, 370385 };
static const int E_msub9[] = { 0, -8, -32, 118000, -2147483600, 2147482700, -1111087 };
static const int E_madd12[] = { 0, 13, 31, 76000, -2147475200, 2147476400, 1481489 };
static const long E_lmadd9[] = { 0L, 8L, 790273982464L, -809240558043136L, -3981L };
static const long E_lmsub5[] = { 0L, -6L, 1271310319616L, -1301821767286784L, 2403L };
static const long E_lmadd15[] = { 0L, 14L, 584115552256L, -598134325510144L, -6717L };
static const long E_lmsub10[] = { 0L, -11L, 1443109011456L, -1477743627730944L, 4683L };
static const u32 POLY_U32[] = { 2654435769U, 1013904242U, 3668340011U, 2027808484U, 387276957U, 3041712726U, 1401181199U, 4055616968U, 2415085441U, 774553914U, 3428989683U, 1788458156U, 147926629U, 2802362398U, 1161830871U, 3816266640U, 2175735113U, 535203586U, 3189639355U, 1549107828U, 4203543597U, 2563012070U, 922480543U, 3576916312U, 1936384785U, 295853258U, 2950289027U, 1309757500U, 3964193269U, 2323661742U, 683130215U, 3337565984U, 1697034457U, 56502930U, 2710938699U, 1070407172U, 3724842941U };
static const u64 POLY_U64[] = { 15755400384260043839UL, 8709371129873690708UL, 1663341875487337577UL, 13064056694810536062UL, 6018027440424182931UL, 17418742259747381416UL, 10372713005361028285UL, 3326683750974675154UL, 14727398570297873639UL, 7681369315911520508UL, 635340061525167377UL, 12036054880848365862UL, 4990025626462012731UL, 16390740445785211216UL, 9344711191398858085UL, 2298681937012504954UL, 13699396756335703439UL, 6653367501949350308UL, 18054082321272548793UL, 11008053066886195662UL, 3962023812499842531UL, 15362738631823041016UL, 8316709377436687885UL, 1270680123050334754UL, 12671394942373533239UL, 5625365687987180108UL, 17026080507310378593UL, 9980051252924025462UL, 2934021998537672331UL };
static const unsigned char HSTR[] = { 84, 104, 101, 32, 113, 117, 105, 99, 107, 32, 98, 114, 111, 119, 110, 32, 102, 111, 120, 32, 106, 117, 109, 112, 115, 32, 111, 118, 101, 114, 32, 116, 104, 101, 32, 108, 97, 122, 121, 32, 100, 111, 103, 255, 128, 1, 127 };
static const int LM_I[] = { 0, 1, -1, 7, -7, 1000000, -1000000, 2, -2, 999999, -999998, 123456, -654321, 3, -3, 5 };
static const long LM_L[] = { 0L, 1L, -1L, 1000000000000000L, -1000000000000000L, 123456789012345L, -98765432109876L, 7L, -7L, 3L, 1099511627776L, -2199023255552L };
static const long E_SCALE[] = { 0L, -3L, 3L, -3000000000000000L, 3000000000000000L, -370370367037035L, 296296296329628L, -21L, 21L, -9L, -3298534883328L, 6597069766656L };
static const u64 LM_U[] = { 1663341875487337577UL, 13064056694810536062UL, 6018027440424182931UL, 17418742259747381416UL, 10372713005361028285UL, 3326683750974675154UL, 14727398570297873639UL, 7681369315911520508UL, 635340061525167377UL, 12036054880848365862UL, 4990025626462012731UL, 16390740445785211216UL, 9344711191398858085UL, 2298681937012504954UL, 13699396756335703439UL, 6653367501949350308UL, 18054082321272548793UL, 11008053066886195662UL, 3962023812499842531UL };
static const u32 LM_U32[] = { 2246822519U, 198677742U, 2445500261U, 397355484U, 2644178003U, 596033226U, 2842855745U, 794710968U, 3041533487U, 993388710U, 3240211229U, 1192066452U, 3438888971U, 1390744194U, 3637566713U, 1589421936U, 3836244455U, 1788099678U, 4034922197U, 1986777420U, 4233599939U, 2185455162U, 137310385U };
#define E_DJB2 3228003741U
#define E_HASH31 1425917842U
#define E_SUM5 -2654295
#define E_SUM6L 141551071648176L
#define E_SUM1000 -530859000L
#define E_MIX9 17691480131576641885UL
#define E_XOR7 2130647050U
#define E_MULDIV3 -1769530
#define E_NESTED17 2507293121995481802UL
static const int SI_A[] = { 0, 5, -5, 1000, -1000, 2000000000, -7 };
static const int SI_B[] = { 0, 9, -9, 100000, -100000, -2000000000, 134217727 };
static const int SIL_A[] = { 0, 5, -5, 1000, -1000, 7, -16 };
static const int SIL_B[] = { 0, 9, 9, 100000, 1000000, 134217727, 134217727 };
static const long SLL_A[] = { 0L, -1L, 1125899906842624L, -1125899906842624L, 99L };
static const long SLL_B[] = { 0L, 1L, 1099511627776L, 576460752303423487L, 12345678901L };
static const u32 SU_A[] = { 0U, 1U, 4294967295U, 2147483648U, 12345U };
static const u32 SU_B[] = { 0U, 1U, 4294967295U, 3U, 3735928559U };
static const long SL_A[] = { 0L, -1L, 1125899906842624L, -1125899906842624L, 99L };
static const long SL_B[] = { 0L, -1L, 1099511627776L, -1099511627776L, -12345678901L };
static const u64 SUL_A[] = { 0UL, 1UL, 18446744073709551615UL, 9223372036854775808UL, 81985529216486895UL };
static const u64 SUL_B[] = { 0UL, 1UL, 18446744073709551615UL, 3UL, 18364758544493064720UL };
static const int E_add_asr3[] = { 0, 6, -7, 13500, -13500, 1750000000, 16777208 };
static const int E_add_lsl4_lhs[] = { 0, 149, 139, 1601000, 15999000, 2147483639, 2147483616 };
static const int E_sub_lsl4[] = { 0, -139, -149, -1599000, -16001000, -2147483625, (-2147483647 - 1) };
static const int E_sub_asr31[] = { 0, 5, -4, 1000, -999, 2000000001, -7 };
static const int E_add_asr1_lhs[] = { 0, 9, -10, 51000, -51000, 1000000000, 67108856 };
static const u32 E_uadd_lsr1[] = { 0U, 1U, 2147483646U, 2147483649U, 1867976624U };
static const u32 E_usub_lsr31[] = { 0U, 1U, 4294967294U, 2147483648U, 12344U };
static const u32 E_uadd_lsl31[] = { 0U, 2147483649U, 2147483647U, 0U, 2147495993U };
static const u32 E_usub_lsl5[] = { 0U, 4294967265U, 31U, 2147483552U, 709382745U };
static const long E_ladd_asr63[] = { 0L, -2L, 1125899906842624L, -1125899906842625L, 98L };
static const long E_lsub_lsl3[] = { 0L, -9L, 1117103813820416L, -4612811918334230520L, -98765431109L };
static const long E_ladd_asr7_lhs[] = { 0L, -2L, 1125908496777216L, -1125908496777216L, -96450518L };
static const u64 E_uladd_lsr7[] = { 0UL, 1UL, 144115188075855870UL, 9223372036854775808UL, 225460205345338963UL };
static const u64 E_ulsub_lsl63[] = { 0UL, 9223372036854775809UL, 9223372036854775807UL, 0UL, 81985529216486895UL };
static const int BOS_A[] = { 0, 31, 32, -32, 100, -100, 12345 };
static const int BOS_B[] = { 0, 4, 0, 0, 1000, -1000, 67 };
static const int E_BOS[] = { 0, 32, 32, -32, -350, -350, -12361 };
static const int ALB_A[] = { 0, 16, 0, 3, -100, 1048576 };
static const int ALB_B[] = { 0, 0, 4, 1, 50, 3 };
static const int ALB_C[] = { 1, 5, 5, -9, 7, 2 };
static const int E_ALB[] = { -1, 21, 21, 16, 93, 1048586 };
static const double D_IN[] = { 0.0, -0.0, 0.9, -0.9, 1.0, -1.0, 2.5, -2.5, 127.99, -128.5, 255.75, 32767.9, -32768.9, 65535.5, 2147483647.9, -2147483648.9, 4294967295.5, 1000000000000.5, -1000000000000.75, 9007199254740992.0, -4503599627370498.0, 1.844674407370955e+19 };
static const float F_IN[] = { 0.0f, -0.0f, 0.75f, -0.75f, 1.5f, -1.5f, 127.5f, -128.75f, 255.5f, 65535.0f, 32767.0f, -32768.0f, 16777216.0f, -16777216.0f, 2147483520.0f, -2147483648.0f, 4294967040.0f, 10000000000.0f };
static const int I_cvt_l[] = { 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20 };
static const long E_cvt_l[] = { 0L, 0L, 0L, 0L, 1L, -1L, 2L, -2L, 127L, -128L, 255L, 32767L, -32768L, 65535L, 2147483647L, -2147483648L, 4294967295L, 1000000000000L, -1000000000000L, 9007199254740992L, -4503599627370498L };
static const int I_cvt_i[] = { 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15 };
static const int E_cvt_i[] = { 0, 0, 0, 0, 1, -1, 2, -2, 127, -128, 255, 32767, -32768, 65535, 2147483647, (-2147483647 - 1) };
static const int I_cvt_ud[] = { 0, 1, 2, 3, 4, 6, 8, 10, 11, 13, 14, 16 };
static const u32 E_cvt_ud[] = { 0U, 0U, 0U, 0U, 1U, 2U, 127U, 255U, 32767U, 65535U, 2147483647U, 4294967295U };
static const int I_cvt_ul[] = { 0, 1, 2, 3, 4, 6, 8, 10, 11, 13, 14, 16, 17, 19, 21 };
static const u64 E_cvt_ul[] = { 0UL, 0UL, 0UL, 0UL, 1UL, 2UL, 127UL, 255UL, 32767UL, 65535UL, 2147483647UL, 4294967295UL, 1000000000000UL, 9007199254740992UL, 18446744073709549568UL };
static const int I_cvt_s[] = { 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12 };
static const int E_cvt_s[] = { 0, 0, 0, 0, 1, -1, 2, -2, 127, -128, 255, 32767, -32768 };
static const int I_cvt_uc[] = { 0, 1, 2, 3, 4, 6, 8, 10 };
static const int E_cvt_uc[] = { 0, 0, 0, 0, 1, 2, 127, 255 };
static const int I_cvt_fl[] = { 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17 };
static const long E_cvt_fl[] = { 0L, 0L, 0L, 0L, 1L, -1L, 127L, -128L, 255L, 65535L, 32767L, -32768L, 16777216L, -16777216L, 2147483520L, -2147483648L, 4294967040L, 10000000000L };
static const int I_cvt_fi[] = { 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15 };
static const int E_cvt_fi[] = { 0, 0, 0, 0, 1, -1, 127, -128, 255, 65535, 32767, -32768, 16777216, -16777216, 2147483520, (-2147483647 - 1) };
static const int I_cvt_u[] = { 0, 1, 2, 3, 4, 6, 8, 9, 10, 12, 14, 16 };
static const u32 E_cvt_u[] = { 0U, 0U, 0U, 0U, 1U, 127U, 255U, 65535U, 32767U, 16777216U, 2147483520U, 4294967040U };
static const int I_cvt_sc[] = { 0, 1, 2, 3, 4, 5, 6, 7 };
static const int E_cvt_sc[] = { 0, 0, 0, 0, 1, -1, 127, -128 };
static const int I_cvt_us[] = { 0, 1, 2, 3, 4, 6, 8, 9, 10 };
static const int E_cvt_us[] = { 0, 0, 0, 0, 1, 127, 255, 65535, 32767 };
static const double CA_D[] = { 2.9, -2.9, 1000000000000.25, -0.5 };
static const long CA_X[] = { 10L, -10L, -1000000000000L, 0L };
static const long E_CA[] = { 12L, -12L, 0L, 0L };
static const double WI_D[] = { -1.5, -2147483648.0, 2147483647.0, -0.25, 77.7 };
static const long E_WI[] = { -1L, -2147483648L, 2147483647L, 0L, 77L };
static const float WU_F[] = { 4294967040.0f, 2147483648.0f, 3.75f, 0.0f };
static const u64 E_WU[] = { 4294967040UL, 2147483648UL, 3UL, 0UL };
static const double LT_D[] = { -0.5, -1.5, -2.5, 5.99, 6.0 };
static const int LT_K[] = { 0, -1, -1, 6, 6 };
static const int E_LT[] = { 0, 0, 1, 1, 0 };
static const double HA_D[] = { 0.0, 1.9, -1.9, 3.5, -3.5, 1000001.0, -1000001.0, 99.99, -0.1 };
static const int E_HA[] = { 0, 1, -2, 4, -5, 1500001, -1500002, 148, 0 };
static const float SC_F[] = { 1.5f, -2.5f, 100.25f, -100.75f, 0.0f, -0.0f, 65536.5f, -65536.5f, 2147483520.0f };
#define E_POLY3 1758667371U
#define E_POLY7 11336505976122475274UL
#define E_SC 2147614795L
#define NAN_PAYLOAD 0x7ff4000000000123UL
#define QNAN_NEG 0xfff8000000000456UL
#define FNAN 0x7fa00123U

int main(void)
{
    int check = 0;
#define CHECK(cond) do { if (!(cond)) return check; } while (0)
#define FOR1(fn, ty, xs, e) do { for (int i = 0; i < N(xs); i++) CHECK(fn(xs[i]) == e[i]); } while (0)
#define FOR2(fn, as, xs, e) do { for (int i = 0; i < N(as); i++) CHECK(fn(as[i], xs[i]) == e[i]); } while (0)

    check = 1;  /* multiply by constant, constant addend */
    FOR1(mc3p7, int, INT_XS, E_mc3p7);
    FOR1(mc5p4095, int, INT_XS, E_mc5p4095);
    FOR1(mc9p4096, int, INT_XS, E_mc9p4096);
    FOR1(mc7m4095, int, INT_XS, E_mc7m4095);
    FOR1(mc15m4096, int, INT_XS, E_mc15m4096);
    FOR1(c100m3, int, INT_XS, E_c100m3);
    FOR1(c4095m17, int, INT_XS, E_c4095m17);
    FOR1(cm4096m5, int, INT_XS, E_cm4096m5);
    FOR1(mc6p1, int, INT_XS, E_mc6p1);
    FOR1(mc10m3, int, INT_XS, E_mc10m3);
    FOR1(mc12p4095, int, INT_XS, E_mc12p4095);
    FOR1(mc24m1, int, INT_XS, E_mc24m1);
    FOR1(mcn3p2, int, INT_XS, E_mcn3p2);
    FOR1(cn7, int, INT_XS, E_cn7);
    FOR1(mc2p3, int, INT_XS, E_mc2p3);
    FOR1(mc4m1, int, INT_XS, E_mc4m1);
    check = 2;
    FOR1(lmc33p1, long, LONG_XS, E_lmc33p1);
    FOR1(lmc63m9, long, LONG_XS, E_lmc63m9);
    FOR1(lmc6p5, long, LONG_XS, E_lmc6p5);
    FOR1(lc12, long, LONG_XS, E_lc12);
    FOR1(lmc65p4096, long, LONG_XS, E_lmc65p4096);
    FOR1(lmc31m4095, long, LONG_XS, E_lmc31m4095);
    FOR1(lc3, long, LONG_XS, E_lc3);
    FOR1(lmc5, long, LONG_XS, E_lmc5);
    check = 3;
    FOR1(umc65p3, u32, U32_XS, E_umc65p3);
    FOR1(umc31m1, u32, U32_XS, E_umc31m1);
    FOR1(uc7, u32, U32_XS, E_uc7);
    FOR1(umc17p4096, u32, U32_XS, E_umc17p4096);
    FOR1(umc10, u32, U32_XS, E_umc10);
    check = 4;
    FOR1(ulmc10p7, u64, U64_XS, E_ulmc10p7);
    FOR1(ulc24, u64, U64_XS, E_ulc24);
    FOR1(ulmc9m4095, u64, U64_XS, E_ulmc9m4095);
    FOR1(ulmc63p1, u64, U64_XS, E_ulmc63p1);
    check = 5;  /* register addends */
    FOR2(madd6, PA_I, PX_I, E_madd6);
    FOR2(msub7, PA_I, PX_I, E_msub7);
    FOR2(madd3l, PA_I, PX_I, E_madd3l);
    FOR2(msub9, PA_I, PX_I, E_msub9);
    FOR2(madd12, PA_I, PX_I, E_madd12);
    FOR2(lmadd9, PA_L, PX_L, E_lmadd9);
    FOR2(lmsub5, PA_L, PX_L, E_lmsub5);
    FOR2(lmadd15, PA_L, PX_L, E_lmadd15);
    FOR2(lmsub10, PA_L, PX_L, E_lmsub10);
    check = 6;  /* accumulator loops */
    CHECK(poly3(POLY_U32, N(POLY_U32)) == E_POLY3);
    CHECK(poly7(POLY_U64, N(POLY_U64)) == E_POLY7);
    CHECK(poly3(POLY_U32, 0) == 0 && poly7(POLY_U64, 0) == 1);
    check = 7;  /* shifted operands */
    FOR2(add_asr3, SI_A, SI_B, E_add_asr3);
    FOR2(add_lsl4_lhs, SIL_A, SIL_B, E_add_lsl4_lhs);
    FOR2(sub_lsl4, SIL_A, SIL_B, E_sub_lsl4);
    FOR2(sub_asr31, SI_A, SI_B, E_sub_asr31);
    FOR2(add_asr1_lhs, SI_A, SI_B, E_add_asr1_lhs);
    FOR2(uadd_lsr1, SU_A, SU_B, E_uadd_lsr1);
    FOR2(usub_lsr31, SU_A, SU_B, E_usub_lsr31);
    FOR2(uadd_lsl31, SU_A, SU_B, E_uadd_lsl31);
    FOR2(usub_lsl5, SU_A, SU_B, E_usub_lsl5);
    check = 8;
    FOR2(ladd_asr63, SL_A, SL_B, E_ladd_asr63);
    FOR2(lsub_lsl3, SLL_A, SLL_B, E_lsub_lsl3);
    FOR2(ladd_asr7_lhs, SL_A, SL_B, E_ladd_asr7_lhs);
    FOR2(uladd_lsr7, SUL_A, SUL_B, E_uladd_lsr7);
    FOR2(ulsub_lsl63, SUL_A, SUL_B, E_ulsub_lsl63);
    check = 9;  /* a shifted add another fusion competes for */
    FOR2(bit_of_sum, BOS_A, BOS_B, E_BOS);
    for (int i = 0; i < N(ALB_A); i++) CHECK(add_lsl_branch(ALB_A[i], ALB_B[i], ALB_C[i]) == E_ALB[i]);
    check = 10;  /* FP to integer */
    for (int i = 0; i < N(I_cvt_l); i++) CHECK((long)cvt_l(D_IN[I_cvt_l[i]]) == E_cvt_l[i]);
    for (int i = 0; i < N(I_cvt_i); i++) CHECK((int)cvt_i(D_IN[I_cvt_i[i]]) == E_cvt_i[i]);
    for (int i = 0; i < N(I_cvt_ud); i++) CHECK((u32)cvt_ud(D_IN[I_cvt_ud[i]]) == E_cvt_ud[i]);
    for (int i = 0; i < N(I_cvt_ul); i++) CHECK((u64)cvt_ul(D_IN[I_cvt_ul[i]]) == E_cvt_ul[i]);
    for (int i = 0; i < N(I_cvt_s); i++) CHECK((int)cvt_s(D_IN[I_cvt_s[i]]) == E_cvt_s[i]);
    for (int i = 0; i < N(I_cvt_uc); i++) CHECK((int)cvt_uc(D_IN[I_cvt_uc[i]]) == E_cvt_uc[i]);
    for (int i = 0; i < N(I_cvt_fl); i++) CHECK((long)cvt_fl(F_IN[I_cvt_fl[i]]) == E_cvt_fl[i]);
    for (int i = 0; i < N(I_cvt_fi); i++) CHECK((int)cvt_fi(F_IN[I_cvt_fi[i]]) == E_cvt_fi[i]);
    for (int i = 0; i < N(I_cvt_u); i++) CHECK((u32)cvt_u(F_IN[I_cvt_u[i]]) == E_cvt_u[i]);
    for (int i = 0; i < N(I_cvt_sc); i++) CHECK((int)cvt_sc(F_IN[I_cvt_sc[i]]) == E_cvt_sc[i]);
    for (int i = 0; i < N(I_cvt_us); i++) CHECK((int)cvt_us(F_IN[I_cvt_us[i]]) == E_cvt_us[i]);
    check = 11;  /* conversions feeding arithmetic, widening, compares, loops */
    for (int i = 0; i < N(CA_D); i++) CHECK(cvt_add(CA_D[i], CA_X[i]) == E_CA[i]);
    for (int i = 0; i < N(WI_D); i++) CHECK(cvt_i_widen(WI_D[i]) == E_WI[i]);
    for (int i = 0; i < N(WU_F); i++) CHECK(cvt_u_widen(WU_F[i]) == E_WU[i]);
    for (int i = 0; i < N(LT_D); i++) CHECK(cvt_lt(LT_D[i], LT_K[i]) == E_LT[i]);
    {
        int out[N(HA_D)];
        halve_add(out, HA_D, N(HA_D));
        for (int i = 0; i < N(HA_D); i++) CHECK(out[i] == E_HA[i]);
    }
    CHECK(sum_cvt(SC_F, N(SC_F)) == E_SC);
    check = 12;  /* FP leaves */
    CHECK(addd(1.5, 2.25) == 3.75 && bd(addd(-0.0, -0.0)) == bd(-0.0) && bd(addd(0.0, -0.0)) == 0);
    CHECK(addf(1.5f, -2.25f) == -0.75f && bf(addf(-0.0f, -0.0f)) == 0x80000000u);
    CHECK(cmpd(1.0, 2.0) == 1 && cmpd(2.0, 1.0) == 0 && cmpd(db(NAN_PAYLOAD), 1.0) == 0);
    CHECK(cmpf(2.0f, 2.0f) == 1 && cmpf(3.0f, 2.0f) == 0 && cmpf(fb(FNAN), 1.0f) == 0);
    CHECK(bd(negd(0.0)) == 0x8000000000000000UL && bd(negd(db(NAN_PAYLOAD))) == (NAN_PAYLOAD ^ 0x8000000000000000UL));
    CHECK(bf(negf(-0.0f)) == 0 && bf(negf(fb(FNAN))) == (FNAN ^ 0x80000000u));
    check = 13;  /* selects and identities keep every bit */
    CHECK(bd(seld(db(NAN_PAYLOAD), 1.0, 1)) == NAN_PAYLOAD && bd(seld(1.0, db(QNAN_NEG), 0)) == QNAN_NEG);
    CHECK(bd(seld(-0.0, 0.0, 1)) == bd(-0.0) && bd(seld(-0.0, 0.0, 0)) == 0);
    CHECK(bf(self(fb(FNAN), 1.0f, 7)) == FNAN && bf(self(1.0f, -0.0f, 0)) == 0x80000000u);
    CHECK(bd(identd(db(NAN_PAYLOAD))) == NAN_PAYLOAD && bd(identd(-0.0)) == bd(-0.0));
    CHECK(bf(identf(fb(FNAN))) == FNAN && bf(identf(-0.0f)) == 0x80000000u);
    CHECK(bd(pick3(1.0, 2.0, db(QNAN_NEG), -2)) == QNAN_NEG && pick3(1.0, 2.0, 3.0, 1) == 1.0 && pick3(1.0, 2.0, 3.0, 0) == 2.0);
    check = 14;  /* temporaries across products, joins, polynomials */
    CHECK(fmad(1.5, 2.0, 0.25) == 3.25 && muladd(1.5, 2.0, 4.0) == 9.0);
    CHECK(muladdf(1.5f, 2.0f, 4.0f) == 9.0f && addf3(1.0f, 2.0f, 4.0f) == 11.0f);
    CHECK(swapsub(1.0, 0.25) == -0.75 && bd(swapsub(1.0, 1.0)) == 0);
    CHECK(poly5(0.5) == 3.53125 && poly5(-2.0) == 89.0 && poly5(0.0) == 1.0);
    CHECK(join_tmp(1.5, 2.0, 1.0) == 6.0 && join_tmp(1.5, 2.0, -1.0) == 4.5);
    check = 15;  /* bit casts between the register files */
    CHECK(dbits(-2.0) == (long)0xc000000000000000UL && bd(bitsd((long)NAN_PAYLOAD)) == NAN_PAYLOAD);
    CHECK(fbits(-1.0f) == (int)0xbf800000u && fbitsu(-1.0f) == 0xbf800000u);
    CHECK(fbits_wide(-1.0f) == -1082130432L && fbitsu_wide(-1.0f) == 0xbf800000UL);
    CHECK(bf(bitsf(FNAN)) == FNAN && bitsf(0x3fc00000u) == 1.5f);
    check = 16;  /* narrow results, extended by the caller */
    {
        static unsigned char table[256];
        static const long wide[] = { 0x1ffffffffL, 0x123456789L, -0x100000007L, 5 };
        for (int i = 0; i < 256; i++)
            table[i] = (unsigned char)(i ^ 0x5a);
        long r = narrow(0x1ffffffffL);
        CHECK(r == -1L);
        r = narrow(0x123456789L);
        CHECK(r == 0x23456789L);
        CHECK(narrow(-0x80000001L) == 0x7fffffffL);
        u64 b = lowb(0x1ffu);
        CHECK(b == 0xffu && table[lowb(0x1ffu)] == (0xff ^ 0x5a) && table[lowb(0x80u)] == (0x80 ^ 0x5a));
        long m = mul16(300, 300);
        CHECK(m == 24464L && mul16(-300, 300) == -24464 && mul16(0x7fff, 3) == 0x7ffd);
        CHECK((long)sc8(0x180L) == -128L && (long)sc8(0x7fffffffffffff7fL) == 127L);
        CHECK((u64)us16(-1L) == 0xffffUL && table[us16(0x10042L) & 0xff] == (0x42 ^ 0x5a));
        CHECK((u64)u32_of(-1L) == 0xffffffffUL && (long)(int)u32_of(0x180000000L) == (long)(-2147483647 - 1));
        CHECK(nonzero(0x100000000L) == 1 && nonzero(0) == 0 && (long)nonzero(-1L) == 1L);
        CHECK(sum_narrow(wide, 4) == 0x23456786);
        /* A 64-bit constant truncated to int: the RV64 psABI returns the
         * word sign-extended, which a caller may compare as a whole. */
        CHECK(trunc_wide() == 1 && trunc_wide_neg() == -2);
        long tw = trunc_wide();
        CHECK(tw == 1L && (long)trunc_wide_neg() == -2L);
    }
    check = 17;  /* multipliers inside loops */
    CHECK(djb2(HSTR, N(HSTR)) == E_DJB2 && djb2(HSTR, 0) == 5381u);
    CHECK(hash31(HSTR, N(HSTR)) == E_HASH31 && hash31(HSTR, 0) == 0u);
    CHECK(sum5(LM_I, N(LM_I)) == E_SUM5 && sum6l(LM_L, N(LM_L)) == E_SUM6L);
    CHECK(sum1000(LM_I, N(LM_I)) == E_SUM1000 && sum1000(LM_I, 0) == 0);
    {
        long v[N(LM_L)];
        for (int i = 0; i < N(LM_L); i++)
            v[i] = LM_L[i];
        scale_m3(v, N(LM_L));
        for (int i = 0; i < N(LM_L); i++)
            CHECK(v[i] == E_SCALE[i]);
    }
    CHECK(mix9(LM_U, N(LM_U)) == E_MIX9 && mix9(LM_U, 0) == 7u);
    CHECK(xor7(LM_U32, N(LM_U32)) == E_XOR7);
    CHECK(mul_div3(LM_I, N(LM_I)) == E_MULDIV3);
    CHECK(nested17(LM_U, N(LM_U)) == E_NESTED17 && nested17(LM_U, 0) == 0u);
    return 0;
}
