/* BUG-004: inlined fold_state_1 must not lose the CLMUL result.
 *
 * emit_memcpy of __m128i (*crc0 = *crc1; ...) goes through %xmm0.
 * The vector last-store peephole then treated %xmm0 as still holding
 * x_high and skipped the reload, so
 *   *crc3 = xor(x_low, x_high)
 * became xor(x_low, 0). zlib-ng CRC32 failed iff (n % 64) ∈ [16,31]. */
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <wmmintrin.h>
#include <emmintrin.h>
#include <smmintrin.h>

static inline void fold_state_1(__m128i *xmm_crc0, __m128i *xmm_crc1,
                                __m128i *xmm_crc2, __m128i *xmm_crc3,
                                const __m128i xmm_fold4) {
    __m128i x_low  = _mm_clmulepi64_si128(*xmm_crc0, xmm_fold4, 0x01);
    __m128i x_high = _mm_clmulepi64_si128(*xmm_crc0, xmm_fold4, 0x10);
    *xmm_crc0 = *xmm_crc1;
    *xmm_crc1 = *xmm_crc2;
    *xmm_crc2 = *xmm_crc3;
    *xmm_crc3 = _mm_xor_si128(x_low, x_high);
}

__attribute__((noinline))
static uint32_t crc16_path(const uint8_t *src) {
    const __m128i xmm_fold4 = _mm_set_epi32(0x00000001, 0x54442bd4, 0x00000001, 0xc6e41596);
    __m128i xmm_crc0 = _mm_cvtsi32_si128(0x9db42487);
    __m128i xmm_crc1 = _mm_setzero_si128();
    __m128i xmm_crc2 = _mm_setzero_si128();
    __m128i xmm_crc3 = _mm_setzero_si128();
    __m128i xmm_t0 = _mm_load_si128((const __m128i *)src);
    fold_state_1(&xmm_crc0, &xmm_crc1, &xmm_crc2, &xmm_crc3, xmm_fold4);
    xmm_crc3 = _mm_xor_si128(xmm_crc3, xmm_t0);

    const __m128i k12 = _mm_set_epi32(0x00000001, 0x751997d0, 0x00000000, 0xccaa009e);
    const __m128i barrett_k = _mm_set_epi32(0x00000001, 0xdb710640, 0xb4e5b025, 0xf7011641);

    __m128i x_low0  = _mm_clmulepi64_si128(xmm_crc0, k12, 0x01);
    __m128i x_high0 = _mm_clmulepi64_si128(xmm_crc0, k12, 0x10);
    xmm_crc1 = _mm_xor_si128(_mm_xor_si128(xmm_crc1, x_low0), x_high0);
    __m128i x_low1  = _mm_clmulepi64_si128(xmm_crc1, k12, 0x01);
    __m128i x_high1 = _mm_clmulepi64_si128(xmm_crc1, k12, 0x10);
    xmm_crc2 = _mm_xor_si128(_mm_xor_si128(xmm_crc2, x_low1), x_high1);
    __m128i x_low2  = _mm_clmulepi64_si128(xmm_crc2, k12, 0x01);
    __m128i x_high2 = _mm_clmulepi64_si128(xmm_crc2, k12, 0x10);
    xmm_crc3 = _mm_xor_si128(_mm_xor_si128(xmm_crc3, x_low2), x_high2);

    __m128i x_tmp0 = _mm_clmulepi64_si128(xmm_crc3, barrett_k, 0x00);
    __m128i x_tmp1 = _mm_clmulepi64_si128(x_tmp0, barrett_k, 0x10);
    x_tmp1 = _mm_blend_epi16(x_tmp1, _mm_setzero_si128(), 0xcf);
    x_tmp0 = _mm_xor_si128(x_tmp1, xmm_crc3);
    __m128i x_res_a = _mm_clmulepi64_si128(x_tmp0, barrett_k, 0x01);
    __m128i x_res_b = _mm_clmulepi64_si128(x_res_a, barrett_k, 0x10);
    return ~((uint32_t)_mm_extract_epi32(x_res_b, 2));
}

int main(void) {
    unsigned char buf[16] __attribute__((aligned(16)));
    for (int i = 0; i < 16; i++)
        buf[i] = (unsigned char)(i * 131u + 17u);
    uint32_t got = crc16_path(buf);
    /* IEEE CRC-32 of that 16-byte pattern (matches zlib crc32()). */
    const uint32_t want = 0xbf22721d;
    if (got != want) {
        printf("FAIL crc_fold_state1 got=%08x want=%08x\n", got, want);
        return 1;
    }
    printf("OK crc_fold_state1\n");
    return 0;
}
