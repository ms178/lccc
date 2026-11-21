/* Shuffle masks built from compile-time constants with the header set
   intrinsics (_mm256_set_epi32, _mm256_setr_epi8, _mm_set_epi8), used as the
   control operand of byte shuffles inside a loop.

   The constant image must read back exactly as GCC 16.2 reads it, in every
   lane, and must survive being the control of several consumers in one
   iteration. Masks keep bit 7 clear and index within their own 128-bit lane,
   so each shuffle is fully defined. */
#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>

__attribute__((noinline)) static void run_masks(const uint32_t in[8], int rounds, uint32_t out[8]) {
    __m256i v = _mm256_loadu_si256((const __m256i *)in);
    /* Byte indices 0..15 repeated per lane, given as dwords (low byte first). */
    const __m256i m_rev = _mm256_set_epi32(
        0x0c0d0e0f, 0x08090a0b, 0x04050607, 0x00010203,
        0x0c0d0e0f, 0x08090a0b, 0x04050607, 0x00010203);
    const __m256i m_swap = _mm256_set_epi32(
        0x07060504, 0x03020100, 0x0f0e0d0c, 0x0b0a0908,
        0x07060504, 0x03020100, 0x0f0e0d0c, 0x0b0a0908);
    /* setr: element 0 first. Bytes within lane: 15..0 reversed pairs. */
    const __m256i m_pairs = _mm256_setr_epi8(
        1, 0, 3, 2, 5, 4, 7, 6, 9, 8, 11, 10, 13, 12, 15, 14,
        15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0);
    const __m128i m_low = _mm_set_epi8(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15);
    __m128i lo_acc = _mm_setzero_si128();
    for (int i = 0; i < rounds; i++) {
        __m256i a = _mm256_shuffle_epi8(v, m_rev);
        __m256i b = _mm256_shuffle_epi8(a, m_swap);
        __m256i c = _mm256_xor_si256(b, _mm256_shuffle_epi8(v, m_pairs));
        __m128i l = _mm_shuffle_epi8(_mm256_castsi256_si128(c), m_low);
        lo_acc = _mm_add_epi32(lo_acc, l);
        v = _mm256_add_epi32(c, _mm256_shuffle_epi8(b, m_pairs));
    }
    _mm256_storeu_si256((__m256i *)out, v);
    /* The 128-bit accumulator is observed through its two dwords of the high
       half of the result so no lane is dead code. */
    out[4] ^= (uint32_t)_mm_extract_epi32(lo_acc, 0);
    out[5] ^= (uint32_t)_mm_extract_epi32(lo_acc, 3);
}

int main(void) {
    for (int trial = 0; trial < 40; trial++) {
        uint32_t in[8];
        uint32_t out[8];
        for (int i = 0; i < 8; i++) in[i] = (uint32_t)(0x9e3779b9u * (uint32_t)(trial * 8 + i + 1));
        int rounds = trial % 13;
        run_masks(in, rounds, out);
        printf("trial %2d rounds %2d:", trial, rounds);
        for (int i = 0; i < 8; i++) printf(" %08x", out[i]);
        printf("\n");
    }
    return 0;
}
