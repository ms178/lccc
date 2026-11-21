#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>

__attribute__((noinline)) static void run(const uint64_t *in, uint64_t *out) {
    __m256i t = _mm256_loadu_si256((const __m256i *)in);
    __m256i dead = _mm256_permute4x64_epi64(t, 27);
    __m256i w = _mm256_permute2x128_si256(t, t, 0x21);
    __m256i live = _mm256_xor_si256(w, t);
    _mm256_storeu_si256((__m256i *)out, live);
    (void)dead;
}

int main(void) {
    uint64_t in[4] __attribute__((aligned(32))) = {1, 2, 3, 4};
    uint64_t out[4] __attribute__((aligned(32))) = {0, 0, 0, 0};
    run(in, out);
    for (int i = 0; i < 4; i++) printf("%llu\n", (unsigned long long)out[i]);
    return 0;
}
