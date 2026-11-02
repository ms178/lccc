/* C23 alignas/alignof keyword spellings (gzip 1.14 builds with -std=gnu23 and
 * uses `alignas (4096)` as a declaration specifier via the DECLARE macro).
 * LCCC previously only knew the C11 _Alignas/_Alignof spellings. */
#include <stdio.h>

#define BUFFER_ALIGNED alignas (4096)
typedef unsigned char uch;
#define DECLARE(type, array, size) type array[size]
DECLARE(uch BUFFER_ALIGNED, inbuf, 16384 + 64);

int main(void) {
    if (((unsigned long)inbuf & 0xFFF) != 0) { printf("FAIL align\n"); return 1; }
    if (alignof(int) != 4) { printf("FAIL alignof\n"); return 1; }
    inbuf[0] = 7;
    if (inbuf[0] != 7) { printf("FAIL store\n"); return 1; }
    printf("OK\n");
    return 0;
}
