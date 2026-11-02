/* Integer prefix sum (scan) with checksum main. */
#include <stdio.h>

static void scan(unsigned *d, const unsigned short *a, int n) {
    unsigned s = 0;
    for (int i = 0; i < n; i++) {
        s += a[i];
        d[i] = s;
    }
}

int main(void) {
    static unsigned short a[1024];
    static unsigned d[1024];
    for (int i = 0; i < 1024; i++) a[i] = (unsigned short)(i * 31 + 7);
    scan(d, a, 1024);
    unsigned long long h = d[0] + d[511] + d[1023];
    for (int i = 0; i < 1024; i += 7) h = h * 33 + d[i];
    printf("%llx\n", h);
    return 0;
}
