#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* Integer reduction shapes for vec_interleave: the x86-64 vectorizer turns
 * these into Vec* accumulator loops and vec_interleave splits them into
 * independent chains.  Integer accumulation is exact under reassociation
 * (modulo 2^n wrap), so the GCC-oracle byte compare stays exact. */

static int doti(const int *a, const int *b, int n) {
    int s = 0;
    for (int i = 0; i < n; i++) s += a[i] * b[i];
    return s;
}

static int sumi(const int *a, int n) {
    int s = 0;
    for (int i = 0; i < n; i++) s += a[i];
    return s;
}

static long long wsum(const int *a, int n) {
    long long s = 0;
    for (int i = 0; i < n; i++) s += a[i];
    return s;
}

static unsigned hash(unsigned h, int v) {
    return h * 131u + (unsigned)(v & 0x7fffffff);
}

int main(void) {
    int n = 40000;
    int *a = (int *)malloc((size_t)n * sizeof(int));
    int *b = (int *)malloc((size_t)n * sizeof(int));
    if (!a || !b) return 2;
    for (int i = 0; i < n; i++) {
        a[i] = (i * 2654435761u) % 2003 - 1001;
        b[i] = (i * 40503u) % 2003 - 1001;
    }
    unsigned h = 1;
    /* Sizes sweep 0..999: tiny loops never enter the interleaved main loop
     * (limit_main == 0) and fall straight to the epilogue; sizes around
     * multiples of the vector widths stress epilogue/remainder boundaries. */
    for (int k = 0; k < 1000; k++) {
        int m = k;
        h = hash(h, doti(a, b, m));
        h = hash(h, sumi(a, m));
        h = hash(h, (int)(wsum(a, m) >> 13));
    }
    h = hash(h, doti(a, b, n));
    h = hash(h, sumi(a, n));
    h = hash(h, (int)(wsum(a, n) >> 13));
    printf("hash %08x\n", h);
    free(a);
    free(b);
    return 0;
}
