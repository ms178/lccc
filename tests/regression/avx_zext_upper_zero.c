/* Regression: _mm256_zextsi128_si256 must zero bits 255:128 of its result for
 * every input, including inputs that are all-ones or carry a sign bit, and the
 * low 128 bits must be the input unchanged. The zero-extension is produced by
 * a VEX 128-bit register move; the upper half is checked lane by lane. */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

__attribute__((noinline)) static __m256i zx(__m128i x) {
    return _mm256_zextsi128_si256(x);
}

static unsigned fold(unsigned acc, unsigned w) { return acc * 1000003u ^ w; }

int main(void) {
    const unsigned src[8][4] = {
        {0x00000000u, 0x00000000u, 0x00000000u, 0x00000000u},
        {0xffffffffu, 0xffffffffu, 0xffffffffu, 0xffffffffu},
        {0x80000000u, 0x7fffffffu, 0x12345678u, 0x9abcdef0u},
        {0x00000001u, 0x00000002u, 0x00000003u, 0x00000004u},
        {0xdeadbeefu, 0xcafebabeu, 0x0badf00du, 0x8badf00du},
        {0x0000ffffu, 0xffff0000u, 0x55555555u, 0xaaaaaaaau},
        {0x01020304u, 0x05060708u, 0x090a0b0cu, 0x0d0e0f10u},
        {0xfffffffeu, 0x00000001u, 0x7ffffffeu, 0x80000001u},
    };
    unsigned acc = 0;
    int bad = 0;
    for (int i = 0; i < 8; ++i) {
        __m128i x;
        memcpy(&x, src[i], sizeof x);
        __m256i y = zx(x);
        unsigned w[8];
        memcpy(w, &y, sizeof w);
        for (int k = 0; k < 4; ++k) {
            if (w[k] != src[i][k]) bad = 1;
            if (w[k + 4] != 0u) bad = 1;
            acc = fold(acc, w[k]);
            acc = fold(acc, w[k + 4]);
        }
    }
    printf("avx_zext_upper_zero %08x %s\n", acc, bad ? "FAIL" : "ok");
    return bad;
}
