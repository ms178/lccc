/* Regression (red-team): the scalar read address is chosen at run time through
 * a volatile index, so the compiler cannot forward the stored value. The read
 * must observe the vector store's value, both as a float and as raw bytes
 * read through a character pointer. */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

static volatile int pick = 3;

__attribute__((noinline)) static unsigned run(const float *in, float *out, int n) {
    unsigned acc = 0;
    for (int i = 0; i < n; i += 8) {
        __m256 v = _mm256_loadu_ps(in + i);
        __m256 w = _mm256_mul_ps(v, _mm256_set1_ps(3.0f));
        _mm256_storeu_ps(out + i, w);
        float got = out[i + pick];
        unsigned b;
        memcpy(&b, &got, sizeof b);
        acc = acc * 1000003u ^ b;
        unsigned char raw[4];
        memcpy(raw, (const unsigned char *)(out + i + 5), 4);
        for (int k = 0; k < 4; ++k) acc = acc * 131u ^ raw[k];
    }
    return acc;
}

int main(void) {
    float in[64], out[64];
    for (int i = 0; i < 64; ++i) in[i] = (float)(i * 7 % 13) - 4.0f;
    memset(out, 0, sizeof out);
    printf("avx_deferred_opaque_read %08x\n", run(in, out, 64));
    return 0;
}
