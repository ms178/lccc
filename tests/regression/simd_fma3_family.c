/* FMA3 family: every spelling must be ONE fused instruction.
 *
 * The bundled <fmaintrin.h> used to implement `_mm256_fmsub_ps` and
 * `_mm256_fnmadd_ps` as lane loops (`__r[i] = __a[i] * __b[i] - __c[i]`), and
 * did not provide `_mm_fmsub_*`, `_mm_fnmadd_*`, `_mm_fnmsub_*`,
 * `_mm_fmaddsub_*`, `_mm_fmsubadd_*`, `_mm256_fmsub_pd`, `_mm256_fnmadd_pd`,
 * `_mm256_fmaddsub_*`, `_mm256_fmsubadd_*` or any of the scalar forms at all.
 *
 * The loop is not merely slower, it is WRONG: an FMA intrinsic promises a
 * single rounding of the product, and the loop rounds twice.  It only agreed
 * with the reference compilers because the default `-ffp-contract=fast`
 * contracted the loop back into one instruction.  With `-ffp-contract=off`
 * `_mm256_fmsub_ps(1.0000001f, 1-1.1920929e-7f, 1.0f)` returned 0 where gcc-16
 * returns -1.42108547e-14 (bits 0xa8800000).
 *
 * The operands below are chosen so that a multiply-then-add implementation
 * cannot produce the fused result: `a` and `b` are adjacent floats, so their
 * product needs the bits that rounding to float discards, and the addend is
 * small enough that the discarded bits survive into the result.  Every result
 * is printed as raw bits, so a one-ulp difference fails the differential.
 *
 * The gate (check_fma3_family.sh) counts instructions per function AND runs
 * this program against a reference compiler at -O2, -O1 and with
 * -ffp-contract=off, because the whole point is that the answer does not
 * depend on the contraction setting.
 */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

static unsigned fbits(float f)
{
    unsigned u;
    memcpy(&u, &f, sizeof(u));
    return u;
}

static unsigned long long dbits(double d)
{
    unsigned long long u;
    memcpy(&u, &d, sizeof(u));
    return u;
}

static void dumpf(const char *tag, const float *v, int n)
{
    printf("%s", tag);
    for (int i = 0; i < n; i++)
        printf(" %08x", fbits(v[i]));
    printf("\n");
}

static void dumpd(const char *tag, const double *v, int n)
{
    printf("%s", tag);
    for (int i = 0; i < n; i++)
        printf(" %016llx", dbits(v[i]));
    printf("\n");
}

/* a*b is not representable in float: |a*b - round(a*b)| ~ 2^-46, and the
 * addend of order 1 keeps that difference in the low bits of the result. */
static const float af[8] = { 1.0000001f, -3.25f, 5.5f, 7.125f, -9.0f, 11.0f, 1.0000001f, 1e30f };
static const float bf[8] = { 1.0f - 1.1920929e-7f, 2.0f, -2.5f, 3.0f, 0.5f, -0.0f,
                             1.0f - 1.1920929e-7f, 1e-30f };
static const float cf[8] = { 1.0f, -4.0f, 8.0f, 10.0f, 14.0f, 18.0f, 1.0f, -1e30f };

static const double ad[4] = { 1.0 + 1e-10, -2.5, 3.0, 1e300 };
static const double bd[4] = { 1.0 - 1e-10, 1.5, -3.0, 1e-300 };
static const double cd[4] = { 1.0, 0.0, -7.0, 1.0 };

/* `noinline`: the gate counts instructions per function, so the operations
 * must not be inlined into `main` where their bodies merge. */
/* ── 256-bit packed ─────────────────────────────────────────────────────── */
static __attribute__((noinline)) void ops256_ps(float *out)
{
    __m256 a = _mm256_loadu_ps(af), b = _mm256_loadu_ps(bf), c = _mm256_loadu_ps(cf);
    _mm256_storeu_ps(out, _mm256_fmsub_ps(a, b, c));
    dumpf("256 fmsub_ps", out, 8);
    _mm256_storeu_ps(out, _mm256_fnmadd_ps(a, b, c));
    dumpf("256 fnmadd_ps", out, 8);
    _mm256_storeu_ps(out, _mm256_fnmsub_ps(a, b, c));
    dumpf("256 fnmsub_ps", out, 8);
    _mm256_storeu_ps(out, _mm256_fmaddsub_ps(a, b, c));
    dumpf("256 fmaddsub_ps", out, 8);
    _mm256_storeu_ps(out, _mm256_fmsubadd_ps(a, b, c));
    dumpf("256 fmsubadd_ps", out, 8);
    _mm256_storeu_ps(out, _mm256_fmadd_ps(a, b, c));
    dumpf("256 fmadd_ps", out, 8);
}

static __attribute__((noinline)) void ops256_pd(double *out)
{
    __m256d a = _mm256_loadu_pd(ad), b = _mm256_loadu_pd(bd), c = _mm256_loadu_pd(cd);
    _mm256_storeu_pd(out, _mm256_fmsub_pd(a, b, c));
    dumpd("256 fmsub_pd", out, 4);
    _mm256_storeu_pd(out, _mm256_fnmadd_pd(a, b, c));
    dumpd("256 fnmadd_pd", out, 4);
    _mm256_storeu_pd(out, _mm256_fnmsub_pd(a, b, c));
    dumpd("256 fnmsub_pd", out, 4);
    _mm256_storeu_pd(out, _mm256_fmaddsub_pd(a, b, c));
    dumpd("256 fmaddsub_pd", out, 4);
    _mm256_storeu_pd(out, _mm256_fmsubadd_pd(a, b, c));
    dumpd("256 fmsubadd_pd", out, 4);
    _mm256_storeu_pd(out, _mm256_fmadd_pd(a, b, c));
    dumpd("256 fmadd_pd", out, 4);
}

/* ── 128-bit packed ─────────────────────────────────────────────────────── */
static __attribute__((noinline)) void ops128_ps(float *out)
{
    __m128 a = _mm_loadu_ps(af), b = _mm_loadu_ps(bf), c = _mm_loadu_ps(cf);
    _mm_storeu_ps(out, _mm_fmsub_ps(a, b, c));
    dumpf("128 fmsub_ps", out, 4);
    _mm_storeu_ps(out, _mm_fnmadd_ps(a, b, c));
    dumpf("128 fnmadd_ps", out, 4);
    _mm_storeu_ps(out, _mm_fnmsub_ps(a, b, c));
    dumpf("128 fnmsub_ps", out, 4);
    _mm_storeu_ps(out, _mm_fmaddsub_ps(a, b, c));
    dumpf("128 fmaddsub_ps", out, 4);
    _mm_storeu_ps(out, _mm_fmsubadd_ps(a, b, c));
    dumpf("128 fmsubadd_ps", out, 4);
    _mm_storeu_ps(out, _mm_fmadd_ps(a, b, c));
    dumpf("128 fmadd_ps", out, 4);
}

static __attribute__((noinline)) void ops128_pd(double *out)
{
    __m128d a = _mm_loadu_pd(ad), b = _mm_loadu_pd(bd), c = _mm_loadu_pd(cd);
    _mm_storeu_pd(out, _mm_fmsub_pd(a, b, c));
    dumpd("128 fmsub_pd", out, 2);
    _mm_storeu_pd(out, _mm_fnmadd_pd(a, b, c));
    dumpd("128 fnmadd_pd", out, 2);
    _mm_storeu_pd(out, _mm_fnmsub_pd(a, b, c));
    dumpd("128 fnmsub_pd", out, 2);
    _mm_storeu_pd(out, _mm_fmaddsub_pd(a, b, c));
    dumpd("128 fmaddsub_pd", out, 2);
    _mm_storeu_pd(out, _mm_fmsubadd_pd(a, b, c));
    dumpd("128 fmsubadd_pd", out, 2);
    _mm_storeu_pd(out, _mm_fmadd_pd(a, b, c));
    dumpd("128 fmadd_pd", out, 2);
}

/* ── 128-bit scalar ─────────────────────────────────────────────────────── */
/* Lane 0 is fused and the upper lanes come from `a`, which is what the 132
 * placement of the ss/sd encoding does.  The upper lanes are printed too: a
 * scalar implementation that clobbered them would pass a lane-0-only check. */
static __attribute__((noinline)) void ops_scalar(float *out, double *outd)
{
    __m128 a = _mm_loadu_ps(af), b = _mm_loadu_ps(bf), c = _mm_loadu_ps(cf);
    _mm_storeu_ps(out, _mm_fmsub_ss(a, b, c));
    dumpf("ss fmsub", out, 4);
    _mm_storeu_ps(out, _mm_fnmadd_ss(a, b, c));
    dumpf("ss fnmadd", out, 4);
    _mm_storeu_ps(out, _mm_fnmsub_ss(a, b, c));
    dumpf("ss fnmsub", out, 4);
    _mm_storeu_ps(out, _mm_fmadd_ss(a, b, c));
    dumpf("ss fmadd", out, 4);

    __m128d da = _mm_loadu_pd(ad), db = _mm_loadu_pd(bd), dc = _mm_loadu_pd(cd);
    _mm_storeu_pd(outd, _mm_fmsub_sd(da, db, dc));
    dumpd("sd fmsub", outd, 2);
    _mm_storeu_pd(outd, _mm_fnmadd_sd(da, db, dc));
    dumpd("sd fnmadd", outd, 2);
    _mm_storeu_pd(outd, _mm_fnmsub_sd(da, db, dc));
    dumpd("sd fnmsub", outd, 2);
    _mm_storeu_pd(outd, _mm_fmadd_sd(da, db, dc));
    dumpd("sd fmadd", outd, 2);
}

/* ── fused identity that a split implementation cannot reproduce ─────────── */
/* (x + 1)*(x - 1) - x^2 = -1 exactly in the fused form for every normal x;
 * split into mul/sub it is off by up to one ulp of x^2, i.e. nonzero. */
static __attribute__((noinline)) double fma_identity(unsigned seed)
{
    double acc = 0.0;
    for (int i = 0; i < 64; i++) {
        seed = seed * 1103515245u + 12345u;
        double x = 1.0 + (double)(seed >> 11) / 8388608.0;
        __m256d vx = _mm256_set1_pd(x);
        __m256d one = _mm256_set1_pd(1.0);
        __m256d up = _mm256_add_pd(vx, one);
        __m256d down = _mm256_sub_pd(vx, one);
        __m256d sq = _mm256_mul_pd(vx, vx);
        __m256d r = _mm256_fmsub_pd(up, down, sq);
        double lanes[4];
        _mm256_storeu_pd(lanes, r);
        acc += lanes[0];
    }
    return acc;
}

/* A loop that uses an FMA must keep it fused and in-register. */
static __attribute__((noinline)) float loop_kernel(const float *src, int n)
{
    __m256 acc = _mm256_setzero_ps();
    for (int i = 0; i < n; i += 8) {
        __m256 x = _mm256_loadu_ps(src + i);
        acc = _mm256_fmsub_ps(acc, _mm256_set1_ps(1.0000001f), x);
    }
    float lanes[8];
    _mm256_storeu_ps(lanes, acc);
    return lanes[0] + lanes[3] + lanes[7];
}

int main(void)
{
    float out8[8];
    double out4[4];
    float buf[64];
    for (int i = 0; i < 64; i++)
        buf[i] = (float)(i * 7 - 32) * 0.37f;

    ops256_ps(out8);
    ops256_pd(out4);
    ops128_ps(out8);
    ops128_pd(out4);
    ops_scalar(out8, out4);
    printf("identity %016llx\n", dbits(fma_identity(20261112u)));
    printf("loop %08x\n", fbits(loop_kernel(buf, 64)));
    return 0;
}
