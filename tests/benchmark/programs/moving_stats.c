/* Moving sum/min/max over int16 windows, checksum main. */
#include <stdio.h>

#define N 1024
#define WLEN 16

static void stats(const short *a, int *sums, short *mns, short *mxs) {
    for (int i = 0; i + WLEN <= N; i++) {
        int s = 0;
        short mn = a[i], mx = a[i];
        for (int j = 0; j < WLEN; j++) {
            short v = a[i + j];
            s += v;
            if (v < mn) mn = v;
            if (v > mx) mx = v;
        }
        sums[i] = s;
        mns[i] = mn;
        mxs[i] = mx;
    }
}

int main(void) {
    static short a[N], mns[N], mxs[N];
    static int sums[N];
    for (int i = 0; i < N; i++)
        a[i] = (short)(((i * 717 + 41) % 2001) - 1000);
    stats(a, sums, mns, mxs);
    unsigned long long h = 0;
    for (int i = 0; i + WLEN <= N; i++) {
        h = h * 31 + (unsigned)sums[i];
        h = h * 31 + (unsigned)(mns[i] + 1000);
        h = h * 31 + (unsigned)(mxs[i] + 1000);
    }
    printf("%llx\n", h);
    return 0;
}
