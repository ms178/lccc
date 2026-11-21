/* 256-bit lane immediates at -O0.
 *
 * At -O0 the immediate of a 256-bit lane shuffle is not folded to a literal
 * before instruction selection. The lowering must pin it to a constant, or the
 * emitter encodes the instruction with immediate 0 (VPERMQ $0 for every control
 * word). Each call below uses a control word that is not zero, on data that is
 * not constant, so a wrong immediate changes the output. */
#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>

__attribute__((noinline)) static void lanes(const uint64_t in[8], uint64_t out[16]) {
    __m256i a = _mm256_loadu_si256((const __m256i *)in);
    __m256i b = _mm256_loadu_si256((const __m256i *)(in + 4));
    _mm256_storeu_si256((__m256i *)(out + 0), _mm256_permute4x64_epi64(a, 0x1b));
    _mm256_storeu_si256((__m256i *)(out + 4), _mm256_permute4x64_epi64(b, 0x4e));
    _mm256_storeu_si256((__m256i *)(out + 8), _mm256_permute2x128_si256(a, b, 0x21));
    _mm256_storeu_si256((__m256i *)(out + 12), _mm256_shuffle_epi32(b, 0x39));
}

int main(void) {
    uint64_t in[8] __attribute__((aligned(32)));
    uint64_t out[16] __attribute__((aligned(32)));
    uint64_t seed = 0x9e3779b97f4a7c15ull;
    for (int round = 0; round < 4; round++) {
        for (int i = 0; i < 8; i++) {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            in[i] = seed;
        }
        for (int i = 0; i < 16; i++) out[i] = 0;
        lanes(in, out);
        for (int i = 0; i < 16; i++) printf("%016llx ", (unsigned long long)out[i]);
        printf("\n");
    }
    return 0;
}
