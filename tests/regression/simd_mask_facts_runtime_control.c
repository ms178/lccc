/* Runtime-valued control: the second operand is built from volatile reads,
 * so NO fold is legal.  A scalar C reference detects a wrong fold. */
#include <emmintrin.h>
#include <stdio.h>
static volatile unsigned char src[16];
int main(void) {
    static const unsigned char a[16] = {
        0x81, 0x02, 0xFF, 0x7F, 0x00, 0xC3, 0x80, 0x55,
        0xAA, 0x10, 0xFE, 0x01, 0x8F, 0x30, 0x00, 0xFF
    };
    for (int i = 0; i < 16; i++) src[i] = (unsigned char)(i * 17 + 3);
    __m128i va = _mm_setr_epi8(a[0],a[1],a[2],a[3],a[4],a[5],a[6],a[7],
                               a[8],a[9],a[10],a[11],a[12],a[13],a[14],a[15]);
    __m128i vb = _mm_setr_epi8(src[0],src[1],src[2],src[3],src[4],src[5],src[6],src[7],
                               src[8],src[9],src[10],src[11],src[12],src[13],src[14],src[15]);
    int got = _mm_movemask_epi8(_mm_and_si128(va, vb));
    int want = 0;
    for (int i = 0; i < 16; i++)
        if ((a[i] & src[i]) & 0x80) want |= 1 << i;
    printf("%d %d\n", got, want);
    return got == want ? 0 : 1;
}
