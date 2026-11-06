/* Floating-point sign operations and the GNU-vector FMA families, executed
 * on AArch64 and RV64.
 *
 * 1-4  GNU vector types (`vector_size(16)`): `a*b + c`, `c - a*b`,
 *      `a*b - c` and `-(a*b) - c` contract into the packed FMA intrinsics
 *      (VecFma/VecFnma/VecFms/VecFnms) straight from the expression, with no
 *      vectorizer involved.  The AArch64 lowering of the two subtracting
 *      forms negates a vector register (`fneg v.2d` / `fneg v.4s`), and
 *      LCCC's built-in assembler encoded that instruction with size<1> = 0:
 *      an unallocated word, SIGILL under `-c`.  Every product and sum is
 *      exact (small dyadic values), so fused and unfused evaluation agree
 *      bit for bit, and the signed-zero checks pin `a*b - c` (0*y - 0 = +0)
 *      against the `-(c - a*b)` mis-lowering (-0).
 * 5    vector negation of signed zeros and infinities, bit for bit.
 * 6-7  scalar `-x`, `fabs`, `copysign` over +-0, +-inf, NaN and normals,
 *      compared as bit patterns (the sign of a NaN is part of the result).
 * 8-9  `fma`/`fmaf`: one rounding -- (1+2^-52)(1-2^-52) - 1 is -2^-104
 *      exactly, while a separate multiply rounds the product to 1.
 * 10   an FP constant reinterpreted through a union and compared as an
 *      integer (`bits(-3.0) == ...`): the comparison's constant operand is a
 *      float, and the AArch64 emitter used to stop with an internal error
 *      ("integer comparison constant must normalize") at -O1 and above.
 * 11   `-(x*y)` and `(-x)*y` with signed zeros and infinities.
 *
 * Expected values: GCC 16.2 and Clang 23.1.5 on x86-64 (-O0, -O2, -O3;
 * -fsanitize=undefined -fsanitize-trap=all clean) agree.  The cross harness
 * runs this at -O0, -O1, -O2, -Os and -O3.  Exit status: the number of the
 * first failing check, 0 when all pass.
 */
#define NOINLINE __attribute__((noinline))
typedef unsigned long long u64;
typedef unsigned u32;
typedef double v2d __attribute__((vector_size(16)));
typedef float v4f __attribute__((vector_size(16)));

NOINLINE v2d vfma_d(v2d a, v2d b, v2d c) { return a * b + c; }
NOINLINE v2d vfnma_d(v2d a, v2d b, v2d c) { return c - a * b; }
NOINLINE v2d vfms_d(v2d a, v2d b, v2d c) { return a * b - c; }
NOINLINE v2d vfnms_d(v2d a, v2d b, v2d c) { return -(a * b) - c; }
NOINLINE v4f vfma_f(v4f a, v4f b, v4f c) { return a * b + c; }
NOINLINE v4f vfnma_f(v4f a, v4f b, v4f c) { return c - a * b; }
NOINLINE v4f vfms_f(v4f a, v4f b, v4f c) { return a * b - c; }
NOINLINE v4f vfnms_f(v4f a, v4f b, v4f c) { return -(a * b) - c; }
NOINLINE v2d vneg_d(v2d a) { return -a; }
NOINLINE v4f vneg_f(v4f a) { return -a; }

/* A loop over vector values keeps several FMA results live at once. */
NOINLINE void vfms_loop(v2d *restrict r, const v2d *restrict a, const v2d *restrict b,
                        const v2d *restrict c, int n)
{
    for (int i = 0; i < n; i++)
        r[i] = a[i] * b[i] - c[i] + (-(a[i] * c[i]) - b[i]);
}

NOINLINE double negd(double x) { return -x; }
NOINLINE float negf(float x) { return -x; }
NOINLINE double absd(double x) { return __builtin_fabs(x); }
NOINLINE float absf(float x) { return __builtin_fabsf(x); }
NOINLINE double csd(double x, double y) { return __builtin_copysign(x, y); }
NOINLINE float csf(float x, float y) { return __builtin_copysignf(x, y); }
NOINLINE double fmad(double a, double b, double c) { return __builtin_fma(a, b, c); }
NOINLINE float fmaf_(float a, float b, float c) { return __builtin_fmaf(a, b, c); }
NOINLINE double nmuld(double x, double y) { return -(x * y); }
NOINLINE double mulnd(double x, double y) { return (-x) * y; }
NOINLINE float nmulf(float x, float y) { return -(x * y); }

static u64 bd(double d)
{
    union { double d; u64 u; } v = { d };
    return v.u;
}
static u32 bf(float f)
{
    union { float f; u32 u; } v = { f };
    return v.u;
}

/* 10: the constant operand of the integer compare is the FP constant. */
NOINLINE int pun_is_neg3(double x) { return bd(x) == bd(-3.0); }
NOINLINE int pun_lt(double x) { return bd(x) < bd(2.5); }
NOINLINE int punf_ne(float x) { return bf(x) != bf(-0.5f); }

static volatile double vd[6] = { 1.5, -2.25, 0.0, -0.0, __builtin_inf(), __builtin_nan("") };
static volatile float vf[6] = { 1.5f, -2.25f, 0.0f, -0.0f, __builtin_inff(), __builtin_nanf("") };
static volatile double small[8] = { -2.0, -0.5, 0.0, 0.25, 1.0, 1.5, 3.0, 4.0 };

int main(void)
{
    /* 1-2: double vector forms, lane by lane against the exact value. */
    for (int i = 0; i < 8; i++)
        for (int j = 0; j < 8; j++) {
            double x0 = small[i], x1 = small[7 - i], y0 = small[j], y1 = small[(j + 3) & 7];
            double z0 = small[(i + j) & 7] * 4.0, z1 = small[(i * 3 + j) & 7] * 2.0;
            v2d a = { x0, x1 }, b = { y0, y1 }, c = { z0, z1 };
            v2d r0 = vfma_d(a, b, c), r1 = vfnma_d(a, b, c);
            v2d r2 = vfms_d(a, b, c), r3 = vfnms_d(a, b, c);
            if (r0[0] != x0 * y0 + z0 || r0[1] != x1 * y1 + z1)
                return 1;
            if (r1[0] != z0 - x0 * y0 || r1[1] != z1 - x1 * y1)
                return 1;
            if (r2[0] != x0 * y0 - z0 || r2[1] != x1 * y1 - z1)
                return 2;
            if (r3[0] != -(x0 * y0) - z0 || r3[1] != -(x1 * y1) - z1)
                return 2;
        }
    /* 3: float vector forms. */
    for (int i = 0; i < 8; i++) {
        float x[4], y[4], z[4];
        for (int k = 0; k < 4; k++) {
            x[k] = (float)small[(i + k) & 7];
            y[k] = (float)small[(i * 5 + k) & 7];
            z[k] = (float)small[(i + 2 * k) & 7] * 8.0f;
        }
        v4f a = { x[0], x[1], x[2], x[3] }, b = { y[0], y[1], y[2], y[3] };
        v4f c = { z[0], z[1], z[2], z[3] };
        v4f r0 = vfma_f(a, b, c), r1 = vfnma_f(a, b, c);
        v4f r2 = vfms_f(a, b, c), r3 = vfnms_f(a, b, c);
        for (int k = 0; k < 4; k++) {
            if (r0[k] != x[k] * y[k] + z[k] || r1[k] != z[k] - x[k] * y[k])
                return 3;
            if (r2[k] != x[k] * y[k] - z[k] || r3[k] != -(x[k] * y[k]) - z[k])
                return 3;
        }
    }
    /* 4: signed zeros.  0*y - 0 = +0, -(0*y) - 0 = -0, 0 - 0*y = +0. */
    {
        v2d a = { 0.0, 0.0 }, b = { 2.0, 3.0 }, c = { 0.0, 0.0 };
        v2d r2 = vfms_d(a, b, c), r3 = vfnms_d(a, b, c), r1 = vfnma_d(a, b, c);
        if (bd(r2[0]) != 0 || bd(r2[1]) != 0)
            return 4;
        if (bd(r3[0]) != 0x8000000000000000ull || bd(r3[1]) != 0x8000000000000000ull)
            return 4;
        if (bd(r1[0]) != 0 || bd(r1[1]) != 0)
            return 4;
        v4f af = { 0.0f, 0.0f, 0.0f, 0.0f }, bf4 = { 1.0f, 2.0f, 3.0f, 4.0f };
        v4f r2f = vfms_f(af, bf4, af), r3f = vfnms_f(af, bf4, af);
        for (int k = 0; k < 4; k++)
            if (bf(r2f[k]) != 0 || bf(r3f[k]) != 0x80000000u)
                return 4;
    }
    /* 5: vector negation, bit for bit. */
    {
        v2d n = vneg_d((v2d){ 0.0, -__builtin_inf() });
        if (bd(n[0]) != 0x8000000000000000ull || bd(n[1]) != 0x7ff0000000000000ull)
            return 5;
        v4f nf = vneg_f((v4f){ -0.0f, 1.5f, __builtin_inff(), -2.25f });
        if (bf(nf[0]) != 0 || bf(nf[1]) != 0xbfc00000u || bf(nf[2]) != 0xff800000u ||
            bf(nf[3]) != 0x40100000u)
            return 5;
        v2d ra[5], aa[5], ba[5], ca[5];
        for (int i = 0; i < 5; i++) {
            aa[i] = (v2d){ small[i], small[i + 1] };
            ba[i] = (v2d){ small[7 - i], small[6 - i] };
            ca[i] = (v2d){ small[(i + 2) & 7], small[(i + 5) & 7] };
        }
        vfms_loop(ra, aa, ba, ca, 5);
        for (int i = 0; i < 5; i++)
            for (int k = 0; k < 2; k++) {
                double a = aa[i][k], b = ba[i][k], c = ca[i][k];
                if (ra[i][k] != a * b - c + (-(a * c) - b))
                    return 5;
            }
    }
    /* 6: scalar double sign ops over the special values. */
    for (int i = 0; i < 6; i++) {
        u64 x = bd(vd[i]);
        if (bd(negd(vd[i])) != (x ^ 0x8000000000000000ull))
            return 6;
        if (bd(absd(vd[i])) != (x & 0x7fffffffffffffffull))
            return 6;
        for (int j = 0; j < 6; j++) {
            u64 y = bd(vd[j]);
            if (bd(csd(vd[i], vd[j])) !=
                ((x & 0x7fffffffffffffffull) | (y & 0x8000000000000000ull)))
                return 6;
        }
    }
    /* 7: scalar float sign ops. */
    for (int i = 0; i < 6; i++) {
        u32 x = bf(vf[i]);
        if (bf(negf(vf[i])) != (x ^ 0x80000000u) || bf(absf(vf[i])) != (x & 0x7fffffffu))
            return 7;
        for (int j = 0; j < 6; j++) {
            u32 y = bf(vf[j]);
            if (bf(csf(vf[i], vf[j])) != ((x & 0x7fffffffu) | (y & 0x80000000u)))
                return 7;
        }
    }
    /* 8-9: one rounding. */
    {
        volatile double e = 0x1p-52;
        if (fmad(1.0 + e, 1.0 - e, -1.0) != -0x1p-104)
            return 8;
        volatile float ef = 0x1p-23f;
        if (fmaf_(1.0f + ef, 1.0f - ef, -1.0f) != -0x1p-46f)
            return 9;
    }
    /* 10: FP constants compared as integer bit patterns. */
    if (!pun_is_neg3(negd(3.0)) || pun_is_neg3(3.0))
        return 10;
    if (!pun_lt(1.0) || pun_lt(3.0))
        return 10;
    if (punf_ne(-0.5f) || !punf_ne(0.5f))
        return 10;
    /* 11: negated products, signed zeros and infinities included. */
    if (bd(nmuld(0.0, 2.0)) != 0x8000000000000000ull || bd(nmuld(-0.0, 2.0)) != 0)
        return 11;
    if (bd(mulnd(0.0, 2.0)) != 0x8000000000000000ull || nmuld(1.5, -4.0) != 6.0)
        return 11;
    if (bd(nmuld(__builtin_inf(), 2.0)) != 0xfff0000000000000ull)
        return 11;
    if (bf(nmulf(0.0f, 3.0f)) != 0x80000000u || nmulf(0.5f, 0.5f) != -0.25f)
        return 11;
    return 0;
}
