/* Loads of several scalar widths from ONE proven constant vector image.
 * Exercises unsigned zero-extension spellings, -0.0 / NaN bit patterns and
 * byte/word/dword/qword windows.  Byte-identical stdout is required under
 * LCCC, GCC 16.2 and Clang 23.1.5. */
#include <emmintrin.h>
#include <stdio.h>
int main(void) {
    union {
        __m128i v;
        unsigned char b[16];
        unsigned w[4];
        unsigned long long q[2];
    } u;
    u.v = _mm_setr_epi8(
        (char)0xFF, (char)0xFF, (char)0xFF, (char)0xFF,
        (char)0x00, (char)0x00, (char)0x00, (char)0x80,
        (char)0x01, (char)0x02, (char)0x03, (char)0x04,
        (char)0xF8, (char)0xFF, (char)0xFF, (char)0xFF);
    printf("%u %u %u %u %llu %llu %u %u\n",
           u.w[0], u.w[1], u.w[2], u.w[3],
           u.q[0], u.q[1],
           (unsigned)u.b[0], (unsigned)u.b[15]);
    return 0;
}
