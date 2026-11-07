/* Differential regression for the target-neutral 16-byte sign-bit extractor.
 *
 * LCCC calls __lccc_simd128_i_pmovmskb128, which `maskfacts` folds to one
 * immediate for a constant image and leaves to the real backend reduction
 * for a volatile-derived image.  GCC 16.2 cannot know that helper, so the
 * reference side computes the same 16-bit mask in ordinary portable C; the
 * stdout comparison is therefore an INDEPENDENT oracle, not a self-check.
 *
 * The guard follows public_simd_native_types.c: LCCC's bundled xmmintrin.h
 * defines _LCCCSIMD_H_INCLUDED, the vendor header does not. */
#include <stdio.h>
#include <immintrin.h>

typedef long long v2i64 __attribute__((vector_size(16)));
union U { v2i64 v; unsigned char b[16]; };

#ifdef _LCCCSIMD_H_INCLUDED
extern int __lccc_simd128_i_pmovmskb128(long, v2i64);
#endif

volatile unsigned char seed = 0x01;

static int portable_mask(const unsigned char *b) {
    int m = 0;
    for (int i = 0; i < 16; i++) {
        if (b[i] & 0x80u) m |= 1 << i;
    }
    return m;
}

static int checksum(int m) {
    return ((m >> 8) ^ m) & 0xFF;
}

static void fill_constant(union U *u) {
    u->b[0] = 0x80; u->b[1] = 0x00; u->b[2] = 0x80; u->b[3] = 0x00;
    u->b[4] = 0x80; u->b[5] = 0x00; u->b[6] = 0x80; u->b[7] = 0x00;
    u->b[8] = 0x00; u->b[9] = 0x80; u->b[10] = 0x00; u->b[11] = 0x80;
    u->b[12] = 0x00; u->b[13] = 0x80; u->b[14] = 0x00; u->b[15] = 0x80;
}

static void fill_runtime(union U *u) {
    unsigned char s = seed; /* volatile: no compiler may fold this image */
    for (int i = 0; i < 16; i++) {
        u->b[i] = (unsigned char)(s * (unsigned)(i + 1) * 37u + 0x40u);
    }
}

static int extract(const union U *u) {
#ifdef _LCCCSIMD_H_INCLUDED
    return __lccc_simd128_i_pmovmskb128(0, u->v);
#else
    return portable_mask(u->b);
#endif
}

int main(void) {
    union U c, r;
    fill_constant(&c);
    fill_runtime(&r);
    printf("%04x %04x\n", extract(&c) & 0xFFFF, extract(&r) & 0xFFFF);
    printf("%d %d\n", checksum(extract(&c)), checksum(extract(&r)));
    printf("%d %d\n", checksum(portable_mask(c.b)), checksum(portable_mask(r.b)));
    return 0;
}
