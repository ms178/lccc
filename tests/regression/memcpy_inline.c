/* constant-size __builtin_memcpy/memmove inlining
 * — exact copies at 1,2,4,8,16,24,32 bytes and non-multiple sizes, plus
 * overlapping memmove. */
#include <stdio.h>
#include <string.h>
#include <stdint.h>

int main(void) {
    uint8_t src[64], dst[64];
    for (int i = 0; i < 64; i++) src[i] = (uint8_t)(i * 3 + 1);

    static const int sizes[] = {1, 2, 3, 4, 5, 7, 8, 9, 12, 15, 16, 17, 24, 31, 32, 33, 48, 63, 64};
    for (unsigned s = 0; s < sizeof(sizes)/sizeof(sizes[0]); s++) {
        int n = sizes[s];
        memset(dst, 0xEE, sizeof dst);
        memcpy(dst, src, (size_t)n);
        for (int i = 0; i < n; i++) if (dst[i] != src[i]) { printf("FAIL copy n=%d i=%d\n", n, i); return 1; }
        if (n < 64) for (int i = n; i < 64; i++) if (dst[i] != 0xEE) return 2; /* untouched */
    }

    /* src=src self-copy must not corrupt */
    memcpy(src, src, 32);
    if (src[0] != 1 || src[31] != 94) return 3;

    /* memmove overlap: shift right by 4 */
    uint8_t buf[32];
    for (int i = 0; i < 32; i++) buf[i] = (uint8_t)i;
    memmove(buf + 4, buf, 28);
    if (buf[4] != 0 || buf[5] != 1 || buf[31] != 27) return 4;
    if (buf[0] != 0 || buf[3] != 3) return 5;

    /* memmove overlap: shift left by 4 */
    for (int i = 0; i < 32; i++) buf[i] = (uint8_t)i;
    memmove(buf, buf + 4, 28);
    if (buf[0] != 4 || buf[1] != 5 || buf[27] != 31) return 6;

    /* struct copy through memcpy */
    struct { uint64_t a; uint32_t b; uint16_t c; uint8_t d; } x = {0x1122334455667788ULL, 0xAABBCCDD, 0xEEFF, 0x10};
    struct { uint64_t a; uint32_t b; uint16_t c; uint8_t d; } y;
    __builtin_memcpy(&y, &x, sizeof x);
    if (y.a != x.a || y.b != x.b || y.c != x.c || y.d != x.d) return 7;

    /* __builtin___memcpy_chk forwarding */
    __builtin___memcpy_chk(dst, src, 16, 64);
    if (memcmp(dst, src, 16) != 0) return 8;

    printf("OK memcpy_inline\n");
    return 0;
}
