/* copy_prop::forward_memcpy_chains rewrote
 *   memcpy tmp, src; memcpy dst, tmp   ->   memcpy dst, src
 * checking only SAME-BLOCK uses of tmp. Loop bodies in OTHER blocks reading
 * the variable slot then observed a deleted store: the slot was never
 * updated (zlib-ng adler32's vs1 slot stale while both loops kept reading
 * it -> wide-loop sums corrupted).
 * Fix: the rewrite now requires a FUNCTION-WIDE use count of exactly 2
 * (the first memcpy dest + the consumer's src). */
#include <immintrin.h>
#include <stdint.h>

static __m128i loop_chain(__m128i x, __m128i y) {
    __m128i s = y;
    __m128i o = _mm_setzero_si128();
    __m128i r = _mm_setzero_si128();
    for (int i = 0; i < 64; i++) {
        s = _mm_add_epi32(s, x);   /* s = s + x  (chain of slot copies) */
        o = s;                     /* copy o <- s (forwarded through tmp) */
        r = _mm_add_epi32(r, o);   /* adjacent consumer */
    }
    /* cross-block read of s's slot: the chain rewrite must NOT have deleted
     * the store that keeps s live across this second loop's iterations. */
    for (int i = 0; i < 64; i++) {
        r = _mm_add_epi32(r, s);
    }
    return r;
}

int main(void) {
    __m128i x = _mm_set1_epi32(1);
    __m128i y = _mm_set1_epi32(0);
    __m128i r = loop_chain(x, y);
    /* loop1: s = 1..64, o = s, r = sum(1..64) = 2080.
     * loop2: r += s (64) each of 64 iters -> +4096.  Total 6176 lane0. */
    if (_mm_cvtsi128_si32(r) != 6176) return 1;
    return 0;
}
