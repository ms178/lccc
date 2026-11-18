/* _mm_broadcastsd_pd under -mavx only (no AVX2).
 *
 * The intrinsic is AVX1 (GCC lowers it to VPERMILPD $0, the same as
 * _mm_movedup_pd).  It must be visible without __AVX2__.  The expected value
 * is the Intel definition: both lanes receive the low double of the source.
 * Exit status is nonzero on any mismatch.
 */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

static int failures;

__attribute__((noinline)) static void broadcast_low(const double *src, double *dst)
{
    __m128d v = _mm_loadu_pd(src);
    _mm_storeu_pd(dst, _mm_broadcastsd_pd(v));
}

int main(void)
{
    static const double cases[][2] = {
        {1.5, -2.25}, {0.0, 7.0}, {-0.0, 3.0}, {1e300, -1e-300},
    };
    for (size_t i = 0; i < sizeof cases / sizeof cases[0]; i++) {
        double got[2];
        /* Intel definition: result[0] = result[1] = src[0]. */
        double exp[2] = {cases[i][0], cases[i][0]};
        broadcast_low(cases[i], got);
        /* Compare bit patterns so -0.0 versus +0.0 is caught. */
        if (memcmp(got, exp, sizeof exp) != 0) {
            printf("FAIL broadcastsd_pd case %zu: got %g %g want %g %g\n", i,
                   got[0], got[1], exp[0], exp[1]);
            failures++;
        } else {
            printf("ok   broadcastsd_pd case %zu\n", i);
        }
    }
    if (failures) {
        printf("FAILURES: %d\n", failures);
        return 1;
    }
    printf("PASS\n");
    return 0;
}
