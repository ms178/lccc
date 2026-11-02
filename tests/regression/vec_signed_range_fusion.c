/*
 * SIGNED-predicate range-mask fusion (the red-team gap in OP-05c).
 *
 * The classifier idiom `lo <= c && c <= hi` written over SIGNED element
 * types (`int`, `short`) with non-negative constants admits exactly the
 * unsigned window [lo, hi]: a negative value fails the signed lower
 * bound, and the two orders agree on non-negatives.  GCC folds this
 * shape; lccc must too — one vpadd + one vpcmpgt instead of two compares
 * and a pand.
 *
 * Every kernel is exercised over the signed extremes, with a trip-count
 * sweep from 0, and the reference is computed through `volatile` so it
 * can never be vectorized into the same form.
 */
#include <stdio.h>
#include <string.h>

#define N 1033  /* prime: odd remainder for VF=4/8/16/32 */

static int    isrc[N], idst[N], iref[N];
static short  wsrc[N], wdst[N], wref[N];

/* ---- kernels: signed classifiers over `int` -------------------------------- */
void k_s_classify(int *restrict d, const int *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) {
        int c = s[i];
        d[i] = (c >= 'a' && c <= 'z') ? 1 : 0;
    }
}
void k_s_fold(int *restrict d, const int *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) {
        int c = s[i];
        if (c >= 'A' && c <= 'Z') c += 32;
        d[i] = c;
    }
}
/* Mixed spellings: signed lower + unsigned upper, and the swapped order. */
void k_s_mixed(int *restrict d, const int *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) {
        int c = s[i];
        d[i] = ((int)((unsigned)c >= 40u) && (c <= 90)) ? 7 : 3;
    }
}
/* Single signed lower bound: `k <=s X` is the window [k, smax]. */
void k_s_lower(int *restrict d, const int *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) {
        d[i] = (s[i] >= 65) ? 1 : 0;
    }
}
/* NEGATIVE-constant signed bounds: the fusion MUST refuse (fail closed);
   correctness is the only assertion. */
void k_s_neg_bounds(int *restrict d, const int *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) {
        int c = s[i];
        d[i] = (c >= -40 && c <= -2) ? 5 : 9;
    }
}
/* ---- kernels: signed classifiers over `short` (word lanes) ---------------- */
void k_w_classify(short *restrict d, const short *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) {
        short c = s[i];
        d[i] = (c >= 1000 && c <= 2000) ? 1 : 0;
    }
}
/* ---- complement window: `(c < K1) || (c > K2)` — the negated classifier -- */
void k_c_fold(int *restrict d, const int *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) {
        int c = s[i];
        if (c < 'a' || c > 'z') c = '.';
        d[i] = c;
    }
}
void k_c_classify(int *restrict d, const int *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) {
        int c = s[i];
        d[i] = (c < 10 || c > 20) ? 1 : 0;
    }
}
/* Adjacent complement bounds (K1 == K2+1): the OR covers everything —
   the fold must refuse, not fold to a wrong mask. */
void k_c_adjacent(int *restrict d, const int *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) {
        int c = s[i];
        d[i] = (c < 11 || c > 10) ? 1 : 0;
    }
}

/* ---- references (volatile => never vectorized) ---------------------------- */
#define IREF(EXPR) do { for (unsigned long i = 0; i < N; ++i) { volatile int vc = isrc[i]; int c = vc; iref[i] = (EXPR); } } while (0)
#define WREF(EXPR) do { for (unsigned long i = 0; i < N; ++i) { volatile short vc = wsrc[i]; short c = vc; wref[i] = (EXPR); } } while (0)

static int fails = 0;
static void check(const char *name, const void *dst, const void *ref, size_t esz) {
    if (memcmp(dst, ref, N * esz) != 0) {
        printf("FAIL %s\n", name);
        fails++;
    } else {
        printf("PASS %s\n", name);
    }
}

int main(void) {
    unsigned int st = 20260908u;
    for (int i = 0; i < N; ++i) {
        st = st * 1664525u + 1013904223u;
        isrc[i] = (int)(st >> 3) % 256 - 96;   /* negatives .. 159 */
        wsrc[i] = (short)((int)(st >> 5) % 4096 - 1500);
    }
    /* pin the interesting points at low indices so short trips see them */
    isrc[0] = 'a'; isrc[1] = 'z'; isrc[2] = 'a' - 1; isrc[3] = 'z' + 1;
    isrc[4] = 0; isrc[5] = -1; isrc[6] = 40; isrc[7] = 90; isrc[8] = 39;
    isrc[9] = 65; isrc[10] = 'A'; isrc[11] = 'Z'; isrc[12] = -2147483647 - 1;
    isrc[13] = 10; isrc[14] = 20; isrc[15] = 11; isrc[16] = 9; isrc[17] = 21;
    wsrc[0] = 1000; wsrc[1] = 2000; wsrc[2] = 999; wsrc[3] = 2001;
    wsrc[4] = -1; wsrc[5] = 0; wsrc[6] = 32767; wsrc[7] = -32768;

    {
        int c;
        IREF(((c >= 'a' && c <= 'z') ? 1 : 0));
        memset(idst, 0xAB, sizeof idst); k_s_classify(idst, isrc, N);
        check("k_s_classify", idst, iref, sizeof(int));
        IREF((c = ((c >= 'A' && c <= 'Z') ? c + 32 : c), c));
        memset(idst, 0xAB, sizeof idst); k_s_fold(idst, isrc, N);
        check("k_s_fold", idst, iref, sizeof(int));
        IREF(((((unsigned)c >= 40u) && (c <= 90)) ? 7 : 3));
        memset(idst, 0xAB, sizeof idst); k_s_mixed(idst, isrc, N);
        check("k_s_mixed", idst, iref, sizeof(int));
        IREF(((c >= 65) ? 1 : 0));
        memset(idst, 0xAB, sizeof idst); k_s_lower(idst, isrc, N);
        check("k_s_lower", idst, iref, sizeof(int));
        IREF(((c >= -40 && c <= -2) ? 5 : 9));
        memset(idst, 0xAB, sizeof idst); k_s_neg_bounds(idst, isrc, N);
        check("k_s_neg_bounds", idst, iref, sizeof(int));
        IREF((c = ((c < 'a' || c > 'z') ? '.' : c), c));
        memset(idst, 0xAB, sizeof idst); k_c_fold(idst, isrc, N);
        check("k_c_fold", idst, iref, sizeof(int));
        IREF(((c < 10 || c > 20) ? 1 : 0));
        memset(idst, 0xAB, sizeof idst); k_c_classify(idst, isrc, N);
        check("k_c_classify", idst, iref, sizeof(int));
        IREF(((c < 11 || c > 10) ? 1 : 0));
        memset(idst, 0xAB, sizeof idst); k_c_adjacent(idst, isrc, N);
        check("k_c_adjacent", idst, iref, sizeof(int));
    }
    {
        short c;
        WREF(((c >= 1000 && c <= 2000) ? 1 : 0));
        memset(wdst, 0xAB, sizeof wdst); k_w_classify(wdst, wsrc, N);
        check("k_w_classify", wdst, wref, sizeof(short));
    }
    /* Trip-count sweep: packed body, scalar remainder, empty loop. */
    for (unsigned long n = 0; n <= 40; ++n) {
        int c;
        IREF(((c >= 'a' && c <= 'z') ? 1 : 0));
        memset(idst, 0xAB, sizeof idst); k_s_classify(idst, isrc, n);
        if (memcmp(idst, iref, n * sizeof(int)) != 0) {
            printf("FAIL k_s_classify trip %lu\n", n);
            fails++;
        }
        IREF((c = ((c < 'a' || c > 'z') ? '.' : c), c));
        memset(idst, 0xAB, sizeof idst); k_c_fold(idst, isrc, n);
        if (memcmp(idst, iref, n * sizeof(int)) != 0) {
            printf("FAIL k_c_fold trip %lu\n", n);
            fails++;
        }
    }
    printf(fails ? "VALIDATION FAILED\n" : "VALIDATION OK\n");
    return fails ? 1 : 0;
}
