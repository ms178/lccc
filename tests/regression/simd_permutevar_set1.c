/* HW emission for _mm256_permutevar8x32_epi32 (VPERMD) and
 * _mm256_set1_epi8 (VPBROADCASTB). zlib-ng chunkset_avx2 uses both. */
#include <stdio.h>
#include <stdint.h>
#include <immintrin.h>

int main(void) {
    int src[8] = {10, 20, 30, 40, 50, 60, 70, 80};
    int idx[8] = {7, 0, 6, 1, 5, 2, 4, 3};
    int out[8] = {0};
    __m256i a = _mm256_loadu_si256((const __m256i *)src);
    __m256i i = _mm256_loadu_si256((const __m256i *)idx);
    __m256i r = _mm256_permutevar8x32_epi32(a, i);
    _mm256_storeu_si256((__m256i *)out, r);
    /* Expected: src[idx[k] & 7] */
    if (out[0] != 80 || out[1] != 10 || out[2] != 70 || out[3] != 20 ||
        out[4] != 60 || out[5] != 30 || out[6] != 50 || out[7] != 40) {
        printf("FAIL permute");
        for (int k = 0; k < 8; k++)
            printf(" %d", out[k]);
        printf("\n");
        return 1;
    }

    __m256i s = _mm256_set1_epi8((char)0x5A);
    unsigned char b[32];
    _mm256_storeu_si256((__m256i *)b, s);
    for (int k = 0; k < 32; k++) {
        if (b[k] != 0x5A) {
            printf("FAIL set1_epi8 k=%d got=%02x\n", k, b[k]);
            return 1;
        }
    }
    printf("OK simd_permutevar_set1\n");
    return 0;
}
