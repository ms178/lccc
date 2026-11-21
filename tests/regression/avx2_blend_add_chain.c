#include <immintrin.h>
#include <stdio.h>
#include <string.h>

__attribute__((noinline)) static __m256 step(__m256 a, __m256 b, __m256 c) {
    __m256 x = _mm256_blend_ps(a, b, 0x5A);
    __m256 y = _mm256_blend_ps(x, c, 0x0F);
    return _mm256_add_ps(y, _mm256_blend_ps(c, a, 0xC3));
}

__attribute__((noinline)) static __m256i qstep(__m256i a, __m256i b) {
    __m256i s = _mm256_add_epi64(a, b);
    __m256i t = _mm256_add_epi64(s, _mm256_set1_epi64x(3));
    return _mm256_add_epi64(t, a);
}

int main(void) {
    float A[8] = {1, 2, 3, 4, 5, 6, 7, 8};
    float B[8] = {10, 20, 30, 40, 50, 60, 70, 80};
    float C[8] = {.5f, .25f, .125f, .0625f, 100, 200, 300, 400};
    __m256 acc = _mm256_loadu_ps(A);
    __m256 vb = _mm256_loadu_ps(B);
    __m256 vc = _mm256_loadu_ps(C);
    for (int i = 0; i < 1000; i++) {
        acc = step(acc, vb, vc);
        vb = _mm256_add_ps(vb, _mm256_set1_ps(0.5f));
    }
    float out[8];
    _mm256_storeu_ps(out, acc);
    for (int i = 0; i < 8; i++) printf("%.6g\n", out[i]);

    long long Q[4] = {1, -2, 3, 1LL << 40};
    long long R[4] = {7, 8, -9, 10};
    __m256i q = _mm256_loadu_si256((const __m256i *)Q);
    __m256i r = _mm256_loadu_si256((const __m256i *)R);
    for (int i = 0; i < 500; i++) q = qstep(q, r), r = _mm256_add_epi64(r, _mm256_set1_epi64x(1));
    long long o[4];
    _mm256_storeu_si256((__m256i *)o, q);
    for (int i = 0; i < 4; i++) printf("%lld\n", o[i]);
    return 0;
}
