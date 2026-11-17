/* AVX1-only 256-bit float logic.  There is no 256-bit integer logic under
 * -mavx, so every operation here must lower to a float-domain instruction
 * (vandpd/vandnpd/vorps/vxorps).  A VEX-256 `vp{and,andn,or,xor}` in this
 * file under -mavx is an AVX2 leak onto AVX1 hardware.  The file has no
 * integer intrinsics, so it compiles under -mavx alone.  Checked by
 * check_avx256_float_bitwise.sh against GCC 16.2 at the same flags. */
#include <immintrin.h>
#include <stdio.h>

__attribute__((noinline)) static __m256d pd_and(__m256d a, __m256d b) { return _mm256_and_pd(a, b); }
__attribute__((noinline)) static __m256d pd_andn(__m256d a, __m256d b) { return _mm256_andnot_pd(a, b); }
__attribute__((noinline)) static __m256 ps_or(__m256 a, __m256 b) { return _mm256_or_ps(a, b); }
__attribute__((noinline)) static __m256 ps_xor(__m256 a, __m256 b) { return _mm256_xor_ps(a, b); }

int main(void) {
    double x[4] = {1, 2, 3, 4}, y[4] = {5, 6, 7, 8}, o[4];
    _mm256_storeu_pd(o, pd_and(_mm256_loadu_pd(x), _mm256_loadu_pd(y)));
    printf("%f %f %f %f\n", o[0], o[1], o[2], o[3]);
    _mm256_storeu_pd(o, pd_andn(_mm256_loadu_pd(x), _mm256_loadu_pd(y)));
    printf("%f %f %f %f\n", o[0], o[1], o[2], o[3]);
    float fx[8] = {1, 2, 3, 4, 5, 6, 7, 8}, fy[8] = {8, 7, 6, 5, 4, 3, 2, 1}, fo[8];
    _mm256_storeu_ps(fo, ps_or(_mm256_loadu_ps(fx), _mm256_loadu_ps(fy)));
    printf("%f %f %f %f\n", fo[0], fo[1], fo[2], fo[3]);
    _mm256_storeu_ps(fo, ps_xor(_mm256_loadu_ps(fx), _mm256_loadu_ps(fy)));
    printf("%f %f %f %f\n", fo[0], fo[1], fo[2], fo[3]);
    return 0;
}
