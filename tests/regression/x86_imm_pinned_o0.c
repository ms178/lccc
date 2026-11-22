/* Regression: immediate-taking SSE/AES/CLMUL intrinsics at -O0.
 *
 * At -O0 a constant immediate reaches the backend as an SSA value, not a
 * literal.  The x86 emitter used to read every such operand as 0, so
 * _mm_blend_epi16, _mm_clmulepi64_si128, _mm_aeskeygenassist_si128 and
 * _mm_sha1rnds4_epu32 silently ignored their immediate at -O0.  The lowering
 * now pins each range-checked immediate as an IrConst at every optimisation
 * level.  Every line below must match GCC byte for byte.
 */
#include <immintrin.h>
#include <wmmintrin.h>
#include <stdio.h>

static void pr(const char *name, __m128i v)
{
    unsigned w[4];
    _mm_storeu_si128((__m128i *)w, v);
    printf("%s %08x %08x %08x %08x\n", name, w[0], w[1], w[2], w[3]);
}

int main(void)
{
    __m128i a = _mm_set_epi32(0x44444444, 0x33333333, 0x22222222, 0x11111111);
    __m128i b = _mm_set_epi32(0x88888888, 0x77777777, 0x66666666, 0x55555555);

    /* PBLENDW: 8-bit immediate selects lanes from b. */
    pr("blend_c3", _mm_blend_epi16(a, b, 0xC3));
    pr("blend_f0", _mm_blend_epi16(a, b, 0xF0));
    pr("blend_0f", _mm_blend_epi16(a, b, 0x0F));

    /* PCLMULQDQ: 8-bit immediate selects the 64-bit halves. */
    pr("clmul_00", _mm_clmulepi64_si128(a, b, 0x00));
    pr("clmul_11", _mm_clmulepi64_si128(a, b, 0x11));
    pr("clmul_10", _mm_clmulepi64_si128(a, b, 0x10));

    /* AESKEYGENASSIST: 8-bit round constant. */
    pr("aeskeygen_01", _mm_aeskeygenassist_si128(a, 0x01));
    pr("aeskeygen_36", _mm_aeskeygenassist_si128(a, 0x36));

    /* SHA1RNDS4: 2-bit function selector (lane immediate), all four values. */
    pr("sha1rnds4_0", _mm_sha1rnds4_epu32(a, b, 0));
    pr("sha1rnds4_1", _mm_sha1rnds4_epu32(a, b, 1));
    pr("sha1rnds4_2", _mm_sha1rnds4_epu32(a, b, 2));
    pr("sha1rnds4_3", _mm_sha1rnds4_epu32(a, b, 3));

    /* Immediates that already worked, kept as a guard against regressions. */
    pr("shuffle_1b", _mm_shuffle_epi32(a, 0x1B));
    pr("shufflelo", _mm_shufflelo_epi16(a, 0x1B));
    pr("insert_epi16", _mm_insert_epi16(a, 7, 5));
    pr("alignr_8", _mm_alignr_epi8(a, b, 8));

    return 0;
}
