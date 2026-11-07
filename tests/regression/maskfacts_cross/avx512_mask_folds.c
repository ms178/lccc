/* Three-way differential witness for the ISPC `IntrinsicsOpt` transplants.
 *
 * Kept in a SUBDIRECTORY on purpose: `run_regression.py` globs only the top
 * level, so an AVX-512 EXECUTABLE can never SIGILL on a hosted runner whose
 * CPU lacks AVX-512.  The IR-level folds are covered in hosted CI by 8
 * exhaustive unit tests instead; this file is the local, executed proof
 * against GCC 16.2 and Clang 23.1.5 on an AVX-512BW/VL host.
 *
 * Build: -O2 -mavx512f -mavx512bw -mavx512vl
 */
#include <stdio.h>
#include <immintrin.h>

static unsigned char buf[64];

/* Constant mask 0: ISPC's all-off masked load must vanish. */
__attribute__((noinline)) static int zero_load(void) {
    __m128i z = _mm_maskz_loadu_epi8((__mmask16)0, buf);
    return _mm_cvtsi128_si32(z);
}

/* Constant mask 0xFFFF: all lanes active, still a masked load today. */
__attribute__((noinline)) static int full_load(void) {
    __m128i f = _mm_maskz_loadu_epi8((__mmask16)0xFFFF, buf);
    return _mm_cvtsi128_si32(f);
}

/* Constant operands: the mask compare is a compile-time scalar constant. */
__attribute__((noinline)) static unsigned const_compare(void) {
    __m128i a = _mm_setr_epi8(1, 2, 3, 4, 5, 6, 7, 8,
                              9, 10, 11, 12, 13, 14, 15, 16);
    __m128i b = _mm_setr_epi8(1, 0, 3, 0, 5, 0, 7, 0,
                              9, 0, 11, 0, 13, 0, 15, 0);
    return (unsigned)_mm_cmp_epu8_mask(a, b, 0);
}

/* Constant mask 0: an all-off masked store performs no memory access. */
__attribute__((noinline)) static int zero_store(void) {
    __m128i v = _mm_set1_epi8((char)0x7F);
    _mm_mask_storeu_epi8(buf + 32, (__mmask16)0, v);
    return buf[32] + buf[33];
}

int main(void) {
    for (int i = 0; i < 64; i++) buf[i] = (unsigned char)(i * 3 + 1);
    printf("%d %d %04x %d\n", zero_load(), full_load(),
           const_compare() & 0xFFFF, zero_store());
    return 0;
}
