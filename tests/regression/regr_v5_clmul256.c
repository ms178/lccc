/* Regression (v5): _mm256_clmulepi64_epi128 vs two _mm_clmulepi64_si128 lanes.
 *
 * The 256-bit VPCLMULQDQ intrinsic was lowered through the struct-copy-init
 * path, which misclassified it as a small packed return because the function
 * signature was never seeded (the system header gates the declaration behind
 * __VPCLMULQDQ__ && __AVX512F__, never defined by -mavx2). The result ADDRESS
 * was spilled as 8 bytes and the 32-byte init memcpy read "through" it —
 * the low qword of the destination became a stack address and the whole
 * 256-bit result shifted by one qword (clmul256 crash).
 *
 * Fix: declare the 256-bit CLMUL/AES intrinsics in the bundled wmmintrin.h
 * (seeding the sret signature), plus classify-by-return-type in the struct
 * copy-init fallback. Values verified against a software carry-less
 * multiply and against the 128-bit lane version. */
#include <immintrin.h>
#include <wmmintrin.h>
#include <stdio.h>
#include <stdint.h>
#include <string.h>

int main(void) {
    __m128i a0 = _mm_set_epi64x(0x1111111111111111ULL, 0x2222222222222222ULL);
    __m128i a1 = _mm_set_epi64x(0x3333333333333333ULL, 0x4444444444444444ULL);
    __m128i k0 = _mm_set_epi64x(0x0000000154442bd4ULL, 0x00000001c6e41596ULL);
    __m128i k1 = k0;

    __m256i a = _mm256_inserti128_si256(_mm256_castsi128_si256(a0), a1, 1);
    __m256i k = _mm256_inserti128_si256(_mm256_castsi128_si256(k0), k1, 1);

    __m256i r01 = _mm256_clmulepi64_epi128(a, k, 0x01);
    __m256i r10 = _mm256_clmulepi64_epi128(a, k, 0x10);

    __m128i e01_lo = _mm_clmulepi64_si128(a0, k0, 0x01);
    __m128i e01_hi = _mm_clmulepi64_si128(a1, k1, 0x01);
    __m128i e10_lo = _mm_clmulepi64_si128(a0, k0, 0x10);
    __m128i e10_hi = _mm_clmulepi64_si128(a1, k1, 0x10);

    uint8_t A[32], B[32];
    memcpy(A, &r01, 32); memcpy(B, &e01_lo, 16); memcpy(B+16, &e01_hi, 16);
    int bad = memcmp(A, B, 32) != 0;
    memcpy(A, &r10, 32); memcpy(B, &e10_lo, 16); memcpy(B+16, &e10_hi, 16);
    bad |= memcmp(A, B, 32) != 0;

    /* 256-bit AES lane check (same signature-seeding fix): each 128-bit lane
     * of _mm256_aesenc_epi128 must equal _mm_aesenc_si128 on the lane. */
    __m256i v = _mm256_inserti128_si256(_mm256_castsi128_si256(a0), a1, 1);
    __m256i rk = _mm256_inserti128_si256(_mm256_castsi128_si256(k0), k1, 1);
    __m256i enc = _mm256_aesenc_epi128(v, rk);
    __m128i enc_lo = _mm_aesenc_si128(a0, k0);
    __m128i enc_hi = _mm_aesenc_si128(a1, k1);
    memcpy(A, &enc, 32); memcpy(B, &enc_lo, 16); memcpy(B+16, &enc_hi, 16);
    bad |= memcmp(A, B, 32) != 0;

    if (bad) { printf("FAILED\n"); return 1; }
    printf("OK\n");
    return 0;
}
