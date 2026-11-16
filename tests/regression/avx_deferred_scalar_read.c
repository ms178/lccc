/* Regression (red-team): a single-use 256-bit producer whose store to a home
 * is deferred, followed in the same block by scalar reads of that home. The
 * scalar reads must observe the vector store, never a stale home. */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

__attribute__((noinline)) static float run(const float *in, float *out, int n) {
    float acc = 0.0f;
    for (int i = 0; i < n; i += 8) {
        __m256 v = _mm256_loadu_ps(in + i);
        __m256 w = _mm256_add_ps(v, _mm256_set1_ps(1.25f));
        _mm256_storeu_ps(out + i, w);
        acc += out[i + 3];
        acc += out[i + 7];
    }
    return acc;
}

int main(void) {
    float in[64], out[64];
    for (int i = 0; i < 64; ++i) in[i] = (float)i * 0.5f - 7.0f;
    memset(out, 0, sizeof out);
    float r = run(in, out, 64);
    unsigned u;
    memcpy(&u, &r, sizeof u);
    printf("avx_deferred_scalar_read %08x\n", u);
    return 0;
}
