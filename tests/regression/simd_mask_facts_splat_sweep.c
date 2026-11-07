/* Exhaustive runtime differential over all 256 byte splats: no image here
 * is provably constant, so an unsound "always fold" would diverge from the
 * scalar reference.  Prints a checksum of all 256 differences. */
#include <emmintrin.h>
#include <stdio.h>
int main(void) {
    unsigned long long acc = 0;
    for (int i = 0; i < 256; i++) {
        __m128i v = _mm_set1_epi8((char)i);
        int got = _mm_movemask_epi8(v);
        int want = (i & 0x80) ? 0xFFFF : 0;
        acc = acc * 131u + (unsigned)(got ^ want);
    }
    printf("%llu\n", acc);
    return 0;
}
