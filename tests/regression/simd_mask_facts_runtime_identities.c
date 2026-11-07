/* Algebraic mask identities on a RUNTIME-valued vector.
 *
 * `mk` is noinline, so the vector's bytes are not compile-time constants.
 * `x^x = 0`, `x==x = -1`, `x&0 = 0` and `x|-1 = -1` still hold for every
 * possible x.  GCC 16.2 and Clang 23.1.5 fold all four movemasks to
 * immediates; before the algebraic lattice LCCC emitted four `pmovmskb`.
 * The fifth value is the vector's own mask, which proves the runtime operand
 * really is opaque (its high bit is set) and that the identities were not
 * "proved" by silently assuming a constant zero vector. */
#include <emmintrin.h>
#include <stdio.h>

__attribute__((noinline)) static __m128i mk(int x) { return _mm_set1_epi8((char)x); }

int main(int argc, char **argv) {
    (void)argv;
    __m128i v = mk(argc + 0x7F); /* 0x80 for argc == 1 -> mask 0xFFFF */
    int xor_self = _mm_movemask_epi8(_mm_xor_si128(v, v));
    int eq_self = _mm_movemask_epi8(_mm_cmpeq_epi8(v, v));
    int and_zero = _mm_movemask_epi8(_mm_and_si128(v, _mm_setzero_si128()));
    int or_ones = _mm_movemask_epi8(_mm_or_si128(v, _mm_set1_epi8((char)-1)));
    int raw = _mm_movemask_epi8(v);
    printf("%d %d %d %d %d\n", xor_self, eq_self, and_zero, or_ones, raw);
    return 0;
}
