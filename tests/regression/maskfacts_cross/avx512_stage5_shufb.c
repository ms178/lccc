/* Round-8 stage-5 three-way differential: LCCC / GCC 16.2 / Clang 23.1.5.
 *
 * The two AVX-512VL/BW masked byte-shuffle intrinsics had a lowering entry,
 * an x86 emitter and a mask-facts fold in the merged tree, but NO usable
 * header wrapper: `_mm_mask_shuffle_epi8` was a 16-iteration SCALAR loop and
 * `_mm_maskz_shuffle_epi8` did not exist at all.  Their declared operand
 * counts were also one short of what the emitter reads.  Stage 5 wires both
 * ends:
 *
 *   _mm_maskz_shuffle_epi8(k, a, b)     -> [mask, a, b]      (3 args)
 *   _mm_mask_shuffle_epi8(src, k, a, b) -> [mask, src, a, b] (4 args)
 *
 * CONSTANT masks are written literally in separate noinline functions so
 * the constant-mask decision procedure can fire (a runtime parameter would
 * be `Unknown`).  `CCC_DISABLE_PASSES=maskfacts` is the A/B control:
 * all-off -> exact 16-byte zero, all-on -> unmasked `vpshufb`.
 *
 * Build: -O2 -mavx512f -mavx512bw -mavx512vl -I include (LCCC only)
 */
#include <stdio.h>
#include <immintrin.h>

static unsigned char out[16];

static unsigned h(int n) {
    unsigned x = 2166136261u;
    for (int i = 0; i < n; i++) x = (x ^ out[i]) * 16777619u;
    return x;
}

static __m128i A(void) {
    return _mm_setr_epi8(0, 1, 2, 3, 4, 5, 6, 7,
                         8, 9, 10, 11, 12, 13, 14, 15);
}
static __m128i B(void) {
    return _mm_setr_epi8(15, 14, 13, 12, 11, 10, 9, 8,
                         7, 6, 5, 4, 3, 2, 1, 0);
}
static __m128i S(void) { return _mm_set1_epi8((char)0xA5); }

#define CASE(NAME, EXPR)                                        \
    __attribute__((noinline)) static unsigned NAME(void) {     \
        __m128i r = (EXPR);                                    \
        _mm_storeu_si128((__m128i_u *)out, r);                  \
        return h(16);                                           \
    }

CASE(z_mixed_lo,  _mm_maskz_shuffle_epi8((__mmask16)0x00FF, A(), B()))
CASE(z_mixed_hi,  _mm_maskz_shuffle_epi8((__mmask16)0xFF00, A(), B()))
CASE(z_off,       _mm_maskz_shuffle_epi8((__mmask16)0x0000, A(), B()))
CASE(z_on,        _mm_maskz_shuffle_epi8((__mmask16)0xFFFF, A(), B()))
CASE(m_mixed_lo,  _mm_mask_shuffle_epi8(S(), (__mmask16)0x00FF, A(), B()))
CASE(m_mixed_hi,  _mm_mask_shuffle_epi8(S(), (__mmask16)0xFF00, A(), B()))
CASE(m_off,       _mm_mask_shuffle_epi8(S(), (__mmask16)0x0000, A(), B()))
CASE(m_on,        _mm_mask_shuffle_epi8(S(), (__mmask16)0xFFFF, A(), B()))

int main(void) {
    printf("%u %u %u %u %u %u %u %u\n",
           z_mixed_lo(), z_mixed_hi(), z_off(), z_on(),
           m_mixed_lo(), m_mixed_hi(), m_off(), m_on());
    return 0;
}
