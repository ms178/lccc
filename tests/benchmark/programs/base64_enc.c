/* Base64 encoder with checksum main. */
#include <stdio.h>

static const char tab[64] =
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

static int enc(char *d, const unsigned char *a, int n) {
    int o = 0;
    for (int i = 0; i < n; i += 3) {
        unsigned t = (unsigned)a[i] << 16;
        int rem = n - i;
        if (rem > 1) t |= (unsigned)a[i + 1] << 8;
        if (rem > 2) t |= a[i + 2];
        d[o++] = tab[(t >> 18) & 63];
        d[o++] = tab[(t >> 12) & 63];
        d[o++] = rem > 1 ? tab[(t >> 6) & 63] : '=';
        d[o++] = rem > 2 ? tab[t & 63] : '=';
    }
    return o;
}

int main(void) {
    static unsigned char a[300];
    static char d[404];
    for (int i = 0; i < 300; i++) a[i] = (unsigned char)(i * 37 + 11);
    int n = enc(d, a, 300);
    unsigned long long h = (unsigned)n;
    for (int i = 0; i < n; i++) h = h * 33 + (unsigned char)d[i];
    printf("%llx\n", h);
    return 0;
}
