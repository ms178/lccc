/* Bit-exact semantics for the fourteen scalar SSE lane intrinsics.
 *
 * These wrappers used to be written as C struct arithmetic
 * (`__a.__val[0] > __b.__val[0] ? ... : ...`), which is cheap to read and
 * expensive to compile: every call copied the 16-byte aggregate in and out of
 * memory and the compare turned into `ucomisd`+`cmov`, so a single `maxsd`
 * cost ~84 instructions in a loop. They now lower to the SSE instruction
 * itself. This fixture is the contract that the rewrite must preserve, because
 * "looks equivalent" is exactly the reasoning that ships sign-of-zero bugs.
 *
 * The whole point is the awkward inputs:
 *   * NaN in either operand (SSE max/min return the SECOND operand),
 *   * +0.0 against -0.0 in both orders (a comparison cannot see the difference;
 *     the instruction can),
 *   * denormals, infinities, and the sqrt of a negative (returns NaN),
 *   * lanes above the low one, which scalar ops must copy from the first
 *     operand and never recompute.
 *
 * Output is one hex word per case, so any divergence from the oracle's
 * instruction semantics changes a byte rather than a printed decimal.
 */
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <xmmintrin.h>
#include <emmintrin.h>

static uint64_t bits_d(double d) { uint64_t u; memcpy(&u, &d, 8); return u; }
static uint32_t bits_f(float f) { uint32_t u; memcpy(&u, &f, 4); return u; }

/* Lane access goes through the portable set/store intrinsics so the same
 * source compiles against a struct-backed __m128d (LCCC) and a native vector
 * type (GCC/Clang), which is what makes the differential meaningful. */
static __m128d mk_d(double lo, double hi) { return _mm_set_pd(hi, lo); }

static __m128 mk_f(float lo, float h0, float h1, float h2)
{
    return _mm_set_ps(h2, h1, h0, lo);
}

static void st_d(__m128d v, double out[2]) { _mm_storeu_pd(out, v); }
static void st_f(__m128 v, float out[4]) { _mm_storeu_ps(out, v); }

#define NAN_D (__builtin_nan(""))
#define INF_D (__builtin_inf())

int main(void)
{
    const double dn[] = { 0.0, -0.0, 1.0, -1.0, 2.5, -2.5, NAN_D, INF_D, -INF_D,
                          1e-310 /* denormal */, 1e308, -1e308, 3.0, -3.0 };
    const int nd = (int)(sizeof dn / sizeof dn[0]);
    unsigned long long h = 1469598103934665603ULL;

    for (int i = 0; i < nd; i++) {
        for (int j = 0; j < nd; j++) {
            /* Distinct upper lanes prove they are copied from `a`, not redone. */
            __m128d a = mk_d(dn[i], 7.25);
            __m128d b = mk_d(dn[j], -9.5);
            __m128d r[7];
            r[0] = _mm_add_sd(a, b);
            r[1] = _mm_sub_sd(a, b);
            r[2] = _mm_mul_sd(a, b);
            r[3] = _mm_div_sd(a, b);
            r[4] = _mm_max_sd(a, b);
            r[5] = _mm_min_sd(a, b);
            r[6] = _mm_sqrt_sd(a, b);
            for (int k = 0; k < 7; k++) {
                double lane[2];
                st_d(r[k], lane);
                (void)printf("%016llx%016llx\n",
                             (unsigned long long)bits_d(lane[0]),
                             (unsigned long long)bits_d(lane[1]));
                h ^= bits_d(lane[0]);
                h *= 1099511628211ULL;
                h ^= bits_d(lane[1]);
                h *= 1099511628211ULL;
            }
        }
    }

    const float fn[] = { 0.0f, -0.0f, 1.0f, -1.0f, 2.5f, -2.5f,
                         __builtin_nanf(""), __builtin_inff(), -__builtin_inff(),
                         1e-40f, 3.0f, -3.0f };
    const int nf = (int)(sizeof fn / sizeof fn[0]);
    for (int i = 0; i < nf; i++) {
        for (int j = 0; j < nf; j++) {
            __m128 a = mk_f(fn[i], 5.5f, 6.5f, 7.5f);
            __m128 b = mk_f(fn[j], -1.5f, -2.5f, -3.5f);
            __m128 r[7];
            r[0] = _mm_add_ss(a, b);
            r[1] = _mm_sub_ss(a, b);
            r[2] = _mm_mul_ss(a, b);
            r[3] = _mm_div_ss(a, b);
            r[4] = _mm_max_ss(a, b);
            r[5] = _mm_min_ss(a, b);
            r[6] = _mm_sqrt_ss(b);
            for (int k = 0; k < 7; k++) {
                float lane[4];
                st_f(r[k], lane);
                (void)printf("%08x%08x%08x%08x\n",
                             bits_f(lane[0]), bits_f(lane[1]),
                             bits_f(lane[2]), bits_f(lane[3]));
                for (int l = 0; l < 4; l++) {
                    h ^= bits_f(lane[l]);
                    h *= 1099511628211ULL;
                }
            }
        }
    }

    (void)printf("hash %016llx\n", h);
    return 0;
}
