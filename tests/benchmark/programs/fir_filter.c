/* 8-tap integer FIR filter with checksum main. */
#include <stdio.h>

static void fir(const short *a, const short *c, int *d, int n) {
    for (int i = 0; i < n; i++) {
        int s = 0;
        for (int j = 0; j < 8; j++) s += (int)a[i + j] * c[j];
        d[i] = s;
    }
}

int main(void) {
    static short a[264], c[8];
    static int d[256];
    for (int i = 0; i < 264; i++) a[i] = (short)((i * 257 + 13) & 1023) - 512;
    for (int j = 0; j < 8; j++) c[j] = (short)(j * j - 3 * j + 7);
    fir(a, c, d, 256);
    unsigned long long h = 146959ULL;
    for (int i = 0; i < 256; i++) {
        h ^= (unsigned)(d[i] & 0xffff);
        h *= 1099511628211ULL;
        h ^= (unsigned)((d[i] >> 16) & 0xffff);
        h *= 1099511628211ULL;
    }
    printf("%llx\n", h);
    return 0;
}
