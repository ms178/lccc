/* New-ISA intrinsic compile+encoding test (AVX-VNNI / INT8 / INT16 / GFNI /
 * VAES-256 / VPCLMUL-256). Compile-only + .text disassembly comparison against
 * GNU as; runtime requires a capable CPU (VNNI: Raptor Lake+). */
#include <immintrin.h>
__m128i v1(__m128i a, __m128i b, __m128i c) { return _mm_dpbusd_epi32(a, b, c); }
__m256i v2(__m256i a, __m256i b, __m256i c) { return _mm256_dpbusd_epi32(a, b, c); }
__m128i v3(__m128i a, __m128i b, __m128i c) { return _mm_dpbusds_epi32(a, b, c); }
__m256i v4(__m256i a, __m256i b, __m256i c) { return _mm256_dpwusd_epi32(a, b, c); }
__m128i v5(__m128i a, __m128i b, __m128i c) { return _mm_dpwusds_epi32(a, b, c); }
__m128i w1(__m128i a, __m128i b, __m128i c) { return _mm_dpbssd_epi32(a, b, c); }
__m256i w2(__m256i a, __m256i b, __m256i c) { return _mm256_dpbssds_epi32(a, b, c); }
__m128i w3(__m128i a, __m128i b, __m128i c) { return _mm_dpbsud_epi32(a, b, c); }
__m256i w4(__m256i a, __m256i b, __m256i c) { return _mm256_dpbuud_epi32(a, b, c); }
__m128i x1(__m128i a, __m128i b, __m128i c) { return _mm_dpwuud_epi32(a, b, c); }
__m256i x2(__m256i a, __m256i b, __m256i c) { return _mm256_dpwuuds_epi32(a, b, c); }
__m128i x3(__m128i a, __m128i b, __m128i c) { return _mm_dpwssd_epi32(a, b, c); }
__m256i x4(__m256i a, __m256i b, __m256i c) { return _mm256_dpwssds_epi32(a, b, c); }
__m128i g1(__m128i a, __m128i b) { return _mm_gf2p8mul_epi8(a, b); }
__m128i g2(__m128i a, __m128i b) { return _mm_gf2p8affine_epi64_epi8(a, b, 0x3f); }
__m128i g3(__m128i a, __m128i b) { return _mm_gf2p8affineinv_epi64_epi8(a, b, 0x1e); }
__m256i va1(__m256i a, __m256i b) { return _mm256_aesenc_epi128(a, b); }
__m256i va2(__m256i a, __m256i b) { return _mm256_aesenclast_epi128(a, b); }
__m256i va3(__m256i a, __m256i b) { return _mm256_aesdec_epi128(a, b); }
__m256i va4(__m256i a, __m256i b) { return _mm256_aesdeclast_epi128(a, b); }
__m256i pc(__m256i a, __m256i b) { return _mm256_clmulepi64_epi128(a, b, 0x11); }
