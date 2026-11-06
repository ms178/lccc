/* Packed fused multiply-add families, executed on AArch64 and RV64.
 *
 * The vectorizer and the BB-SLP pass contract `a*b + c`, `c - a*b`,
 * `a*b - c` and `-(a*b) - c` into the packed FMA intrinsics
 * (`VecFma`/`VecFnma`/`VecFms`/`VecFnms`, 2 x F64 and 4 x F32) whenever the
 * target reports fused multiply-add, which AArch64 always does.  The AArch64
 * backend had no lowering for them, so every contracted loop executed the
 * `brk #0` of an unhandled operation at -O2 and -O3 (SIGTRAP, exit 133).
 *
 * Every product and sum below is exact in its type (small dyadic values), so
 * the fused and the unfused evaluation agree bit for bit and the expected
 * values do not depend on whether, or where, a compiler contracts.  The two
 * zero checks pin the sign of a zero result, which is where lowering
 * `a*b - c` as the negation of a fused `c - a*b` goes wrong:
 * 0*x - 0 is +0, while -(0 - 0*x) is -0.
 *
 * Expected values: GCC 16.2 (-O0, and -O2 -march=haswell
 * -ffp-contract=fast) and Clang 23.1.5 -O2 on x86-64 agree.
 * The cross harness runs this at -O0, -O1, -O2, -Os and -O3.
 */
#define NOINLINE __attribute__((noinline))
#define N 64
typedef unsigned long long u64;
typedef unsigned u32;

static double xd[N], yd[N], zd[N], rd[N];
static float xf[N], yf[N], zf[N], rf[N];

NOINLINE void fma_d(double *restrict r, const double *restrict a, const double *restrict b,
                    const double *restrict c, int n)
{
    for (int i = 0; i < n; i++)
        r[i] = a[i] * b[i] + c[i];
}
NOINLINE void fnma_d(double *restrict r, const double *restrict a, const double *restrict b,
                     const double *restrict c, int n)
{
    for (int i = 0; i < n; i++)
        r[i] = c[i] - a[i] * b[i];
}
NOINLINE void fms_d(double *restrict r, const double *restrict a, const double *restrict b,
                    const double *restrict c, int n)
{
    for (int i = 0; i < n; i++)
        r[i] = a[i] * b[i] - c[i];
}
NOINLINE void fnms_d(double *restrict r, const double *restrict a, const double *restrict b,
                     const double *restrict c, int n)
{
    for (int i = 0; i < n; i++)
        r[i] = -(a[i] * b[i]) - c[i];
}
NOINLINE void fma_f(float *restrict r, const float *restrict a, const float *restrict b,
                    const float *restrict c, int n)
{
    for (int i = 0; i < n; i++)
        r[i] = a[i] * b[i] + c[i];
}
NOINLINE void fnma_f(float *restrict r, const float *restrict a, const float *restrict b,
                     const float *restrict c, int n)
{
    for (int i = 0; i < n; i++)
        r[i] = c[i] - a[i] * b[i];
}
NOINLINE void fms_f(float *restrict r, const float *restrict a, const float *restrict b,
                    const float *restrict c, int n)
{
    for (int i = 0; i < n; i++)
        r[i] = a[i] * b[i] - c[i];
}
NOINLINE void fnms_f(float *restrict r, const float *restrict a, const float *restrict b,
                     const float *restrict c, int n)
{
    for (int i = 0; i < n; i++)
        r[i] = -(a[i] * b[i]) - c[i];
}
/* axpy with a scalar multiplier: the broadcast operand of the packed FMA. */
NOINLINE void axpy_d(double *restrict y, const double *restrict x, double a, int n)
{
    for (int i = 0; i < n; i++)
        y[i] = a * x[i] + y[i];
}
NOINLINE void axpy_f(float *restrict y, const float *restrict x, float a, int n)
{
    for (int i = 0; i < n; i++)
        y[i] = a * x[i] + y[i];
}
/* Straight-line pairs and quads: BB-SLP packs these without a loop. */
NOINLINE void slp_pair_d(double *restrict r, const double *restrict a, const double *restrict b,
                         const double *restrict c)
{
    r[0] = a[0] * b[0] + c[0];
    r[1] = a[1] * b[1] + c[1];
    r[2] = c[2] - a[2] * b[2];
    r[3] = c[3] - a[3] * b[3];
}
NOINLINE void slp_quad_f(float *restrict r, const float *restrict a, const float *restrict b,
                         const float *restrict c)
{
    r[0] = a[0] * b[0] - c[0];
    r[1] = a[1] * b[1] - c[1];
    r[2] = a[2] * b[2] - c[2];
    r[3] = a[3] * b[3] - c[3];
}

static u64 bits_d(double d)
{
    union { double d; u64 u; } v = { d };
    return v.u;
}
static u32 bits_f(float f)
{
    union { float f; u32 u; } v = { f };
    return v.u;
}

static void fill(void)
{
    for (int i = 0; i < N; i++) {
        xd[i] = (double)(i % 13) * 0.5 - 2.0;  /* -2 .. 4, step 0.5 */
        yd[i] = (double)(i % 7) + 0.25;        /* 0.25 .. 6.25 */
        zd[i] = (double)(i % 11) * 4.0 - 16.0; /* -16 .. 24 */
        xf[i] = (float)xd[i];
        yf[i] = (float)yd[i];
        zf[i] = (float)zd[i];
    }
}

/* The exact value of each form, computed in long double-free integer
 * arithmetic on the scaled operands: x = p/2, y = q/4, z = s, so
 * x*y = p*q/8 and every result is a multiple of 1/8. */
static long scaled_x(int i) { return (long)(i % 13) - 4; }   /* 2x */
static long scaled_y(int i) { return 4L * (i % 7) + 1; }     /* 4y */
static long scaled_z(int i) { return 4L * (i % 11) - 16; }   /* z  */
static double exact(int form, int i)
{
    long p8 = scaled_x(i) * scaled_y(i);   /* 8*x*y */
    long z8 = 8 * scaled_z(i);             /* 8*z */
    long r8 = form == 0 ? p8 + z8 : form == 1 ? z8 - p8 : form == 2 ? p8 - z8 : -p8 - z8;
    return (double)r8 / 8.0;
}

int main(void)
{
    fill();
    void (*dforms[4])(double *restrict, const double *restrict, const double *restrict,
                      const double *restrict, int) = { fma_d, fnma_d, fms_d, fnms_d };
    void (*fforms[4])(float *restrict, const float *restrict, const float *restrict,
                      const float *restrict, int) = { fma_f, fnma_f, fms_f, fnms_f };
    /* 1: the four double forms. */
    for (int form = 0; form < 4; form++) {
        dforms[form](rd, xd, yd, zd, N);
        for (int i = 0; i < N; i++)
            if (rd[i] != exact(form, i))
                return 1;
    }
    /* 2: the four float forms. */
    for (int form = 0; form < 4; form++) {
        fforms[form](rf, xf, yf, zf, N);
        for (int i = 0; i < N; i++)
            if (rf[i] != (float)exact(form, i))
                return 2;
    }
    /* 3: a trip count that leaves a scalar tail. */
    for (int i = 0; i < N; i++)
        rd[i] = -1.0;
    fms_d(rd, xd, yd, zd, 37);
    for (int i = 0; i < N; i++)
        if (rd[i] != (i < 37 ? exact(2, i) : -1.0))
            return 3;
    /* 4: axpy, double and float. */
    for (int i = 0; i < N; i++)
        rd[i] = zd[i];
    axpy_d(rd, xd, 0.75, N);
    for (int i = 0; i < N; i++)
        if (rd[i] != zd[i] + 0.75 * xd[i])
            return 4;
    for (int i = 0; i < N; i++)
        rf[i] = zf[i];
    axpy_f(rf, xf, -1.5f, N);
    for (int i = 0; i < N; i++)
        if (rf[i] != zf[i] + -1.5f * xf[i])
            return 5;
    /* 6: SLP pairs (fma, fnma) and a float quad (fms). */
    slp_pair_d(rd, xd + 3, yd + 3, zd + 3);
    for (int i = 0; i < 4; i++)
        if (rd[i] != exact(i < 2 ? 0 : 1, i + 3))
            return 6;
    slp_quad_f(rf, xf + 5, yf + 5, zf + 5);
    for (int i = 0; i < 4; i++)
        if (rf[i] != (float)exact(2, i + 5))
            return 7;
    /* 8: the sign of a zero result.  0*y - 0 = +0, -(0*y) - 0 = -0,
     * 0 - 0*y = +0 and 0*y + 0 = +0, in both widths. */
    for (int i = 0; i < N; i++) {
        xd[i] = 0.0;
        zd[i] = 0.0;
        xf[i] = 0.0f;
        zf[i] = 0.0f;
    }
    static const u64 dsign[4] = { 0, 0, 0, 0x8000000000000000ull };
    static const u32 fsign[4] = { 0, 0, 0, 0x80000000u };
    for (int form = 0; form < 4; form++) {
        dforms[form](rd, xd, yd, zd, N);
        fforms[form](rf, xf, yf, zf, N);
        for (int i = 0; i < N; i++)
            if (bits_d(rd[i]) != dsign[form] || bits_f(rf[i]) != fsign[form])
                return 8;
    }
    /* 9: aliasing operands -- x*x + x and x*x - x through one array. */
    for (int i = 0; i < N; i++)
        xd[i] = (double)(i % 9) - 4.0;
    fma_d(rd, xd, xd, xd, N);
    for (int i = 0; i < N; i++)
        if (rd[i] != xd[i] * xd[i] + xd[i])
            return 9;
    fms_d(rd, xd, xd, xd, N);
    for (int i = 0; i < N; i++)
        if (rd[i] != xd[i] * xd[i] - xd[i])
            return 9;
    return 0;
}
