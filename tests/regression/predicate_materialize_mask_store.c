/*
 * A stored comparison is 0/1, a stored lane MASK is 0/-1.
 *
 * `d[i] = lo <= c && c <= hi ? 1 : 0` and its select-free spellings are the
 * same program, and both reach the vectorizer as a comparison feeding a store.
 * The packed integer compare produces -1 in a lane that matches, so a
 * vectorized body has to materialize `& 1`; storing the raw mask writes
 * 0xFFFFFFFF where the C program writes 1.  The scalar remainder loop is
 * correct either way, which is what makes the bug invisible unless the
 * vector body actually runs -- hence the trip-count sweep: the wrapped body
 * only executes for n >= one vector step, and the failure below reproduces at
 * n == 37 for a 4-lane step.
 *
 * The reference is computed through `volatile` so it can never be vectorized
 * into the same form, and the arrays are re-filled between trip counts so a
 * short trip cannot inherit a long trip's leftovers.
 */
#include <stdio.h>
#include <string.h>

#define N 1033 /* prime: odd remainder at every vector width */

static int src[N], dst[N], ref[N];
static short wsrc[N], wdst[N], wref[N];

void classify_int(int *restrict d, const int *restrict s, unsigned long n)
{
    for (unsigned long i = 0; i < n; ++i) {
        int c = s[i];
        d[i] = (c >= 'a' && c <= 'z') ? 1 : 0;
    }
}

/* The same predicate without the ternary: the comparison IS the value. */
void classify_bare(int *restrict d, const int *restrict s, unsigned long n)
{
    for (unsigned long i = 0; i < n; ++i) {
        int c = s[i];
        d[i] = c >= 'a' && c <= 'z';
    }
}

/* Complement window: `c < lo || c > hi`. */
void classify_complement(int *restrict d, const int *restrict s, unsigned long n)
{
    for (unsigned long i = 0; i < n; ++i) {
        int c = s[i];
        d[i] = (c < 10 || c > 20) ? 1 : 0;
    }
}

void classify_short(short *restrict d, const short *restrict s, unsigned long n)
{
    for (unsigned long i = 0; i < n; ++i) {
        short c = s[i];
        d[i] = (c >= 1000 && c <= 2000) ? 1 : 0;
    }
}

static int fails;

static void check(const char *name, const void *dst, const void *ref, size_t bytes)
{
    if (memcmp(dst, ref, bytes) != 0) {
        printf("FAIL %s\n", name);
        fails++;
    }
}

int main(void)
{
    unsigned int st = 20261101u;
    for (int i = 0; i < N; ++i) {
        st = st * 1664525u + 1013904223u;
        src[i] = (int)(st >> 7) % 200;          /* 0..199, straddles the windows */
        wsrc[i] = (short)((int)(st >> 5) % 4096 - 1500);
    }
    /* Pin the window edges at the low indices every short trip reaches. */
    src[0] = 'a'; src[1] = 'z'; src[2] = 'a' - 1; src[3] = 'z' + 1;
    src[4] = 0; src[5] = -1; src[6] = 10; src[7] = 20; src[8] = 9; src[9] = 21;
    wsrc[0] = 1000; wsrc[1] = 2000; wsrc[2] = 999; wsrc[3] = 2001;
    wsrc[4] = -1; wsrc[5] = 32767; wsrc[6] = -32768;

    for (unsigned long n = 0; n <= 96; ++n) {
        for (unsigned long i = 0; i < N; ++i) {
            volatile int vc = src[i];
            int c = vc;
            ref[i] = (c >= 'a' && c <= 'z') ? 1 : 0;
        }
        memset(dst, 0xAB, sizeof dst);
        classify_int(dst, src, n);
        if (memcmp(dst, ref, n * sizeof(int)) != 0) {
            printf("FAIL classify_int trip %lu\n", n);
            fails++;
        }
        memset(dst, 0xAB, sizeof dst);
        classify_bare(dst, src, n);
        if (memcmp(dst, ref, n * sizeof(int)) != 0) {
            printf("FAIL classify_bare trip %lu\n", n);
            fails++;
        }
        for (unsigned long i = 0; i < N; ++i) {
            volatile int vc = src[i];
            int c = vc;
            ref[i] = (c < 10 || c > 20) ? 1 : 0;
        }
        memset(dst, 0xAB, sizeof dst);
        classify_complement(dst, src, n);
        if (memcmp(dst, ref, n * sizeof(int)) != 0) {
            printf("FAIL classify_complement trip %lu\n", n);
            fails++;
        }
        for (unsigned long i = 0; i < N; ++i) {
            volatile short vc = wsrc[i];
            short c = vc;
            wref[i] = (short)((c >= 1000 && c <= 2000) ? 1 : 0);
        }
        memset(wdst, 0xAB, sizeof wdst);
        classify_short(wdst, wsrc, n);
        if (memcmp(wdst, wref, n * sizeof(short)) != 0) {
            printf("FAIL classify_short trip %lu\n", n);
            fails++;
        }
    }
    /* One full-length run past the end of the sweep. */
    for (unsigned long i = 0; i < N; ++i) {
        volatile int vc = src[i];
        int c = vc;
        ref[i] = (c >= 'a' && c <= 'z') ? 1 : 0;
    }
    memset(dst, 0xAB, sizeof dst);
    classify_int(dst, src, N);
    memset(ref + N, 0xAB, 0);
    check("classify_int full", dst, ref, N * sizeof(int));

    printf(fails ? "VALIDATION FAILED\n" : "VALIDATION OK\n");
    return fails ? 1 : 0;
}
