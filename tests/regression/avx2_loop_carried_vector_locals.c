/* Loop-carried 256-bit and 128-bit vector locals consumed by pure register
   operands (pshufb data and control, permutes, lane extract, blend).

   Each local is reassigned inside the loop, so the front end keeps it in a
   stack object and every consumer reads that object by address. Promotion
   must turn those reads into register uses without changing a single bit.
   The program prints a digest of every intermediate so any wrong byte shows
   up in the comparison against GCC 16.2. */
#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>

static uint64_t rng_state = 0x243F6A8885A308D3ull;

static uint64_t rng(void) {
    rng_state ^= rng_state << 13;
    rng_state ^= rng_state >> 7;
    rng_state ^= rng_state << 17;
    return rng_state;
}

__attribute__((noinline)) static void run_chain(const uint64_t in[8], int rounds, uint64_t out[8]) {
    __m256i s0 = _mm256_loadu_si256((const __m256i *)in);
    __m256i s1 = _mm256_loadu_si256((const __m256i *)(in + 4));
    __m128i acc = _mm_setzero_si128();
    /* Byte-permutation control: every byte selects within its own 128-bit
       lane (bit 7 clear, low nibble 0..15), so the shuffle is well defined. */
    const __m256i ctl = _mm256_set_epi8(
        1, 0, 3, 2, 5, 4, 7, 6, 9, 8, 11, 10, 13, 12, 15, 14,
        14, 15, 12, 13, 10, 11, 8, 9, 6, 7, 4, 5, 2, 3, 0, 1);
    for (int i = 0; i < rounds; i++) {
        __m256i p = _mm256_permute2x128_si256(s0, s1, 0x21);
        __m256i q = _mm256_shuffle_epi8(p, ctl);
        __m256i r = _mm256_permute4x64_epi64(q, 0x4e);
        __m128i hi = _mm256_extracti128_si256(r, 1);
        acc = _mm_add_epi64(acc, hi);
        __m256i t = _mm256_blend_epi32(r, s0, 0x33);
        /* Float-domain blend (VBLENDPS): a register-homed result feeding a
           later pure consumer. Same bytes, different encoding. */
        __m256 fb = _mm256_blend_ps(_mm256_castsi256_ps(t), _mm256_castsi256_ps(r), 0xa5);
        t = _mm256_castps_si256(fb);
        s1 = _mm256_add_epi64(t, s1);
        s0 = _mm256_xor_si256(s0, _mm256_shuffle_epi8(s1, ctl));
        s0 = _mm256_shuffle_epi32(s0, 0x1b);
        s1 = _mm256_permute2x128_si256(s1, s0, 0x03);
    }
    _mm256_storeu_si256((__m256i *)out, s0);
    _mm256_storeu_si256((__m256i *)(out + 4), s1);
    /* Fold the 128-bit accumulator into the output so its last value counts. */
    out[0] ^= (uint64_t)_mm_cvtsi128_si64(acc);
    out[1] ^= (uint64_t)_mm_extract_epi64(acc, 1);
}

int main(void) {
    for (int trial = 0; trial < 48; trial++) {
        uint64_t in[8];
        uint64_t out[8];
        for (int i = 0; i < 8; i++) in[i] = rng();
        int rounds = trial % 17;
        run_chain(in, rounds, out);
        printf("trial %2d rounds %2d:", trial, rounds);
        for (int i = 0; i < 8; i++) printf(" %016llx", (unsigned long long)out[i]);
        printf("\n");
    }
    return 0;
}
