/* Guarded widening sums `long s += a[i] if (guard)` across the guard
 * spellings the vectorizer canonicalises to `a[i] > K` (`>=`, mirrored
 * `<`/`<=`) and the constants on both sides of the zero-extension proof
 * (K >= -1: masked lanes are non-negative, split with pand/psrlq; K < -1:
 * negative lanes survive, sign-unpack required), plus INT_MIN/INT_MAX
 * data, a run-time guard, and every trip count 0..37 at three alignments
 * so the 4-lane body and the scalar remainder are both exercised. */
#include <limits.h>
#include <stdio.h>

#define SUM(name, cond)                                                     \
    __attribute__((noinline)) long name(const int *a, int n, int t) {       \
        long s = 0;                                                         \
        for (int i = 0; i < n; i++)                                         \
            if (cond)                                                       \
                s += a[i];                                                  \
        return s + (t & 0);                                                 \
    }

SUM(gt0, a[i] > 0)
SUM(gtm1, a[i] > -1)
SUM(gtm2, a[i] > -2)
SUM(gt1000, a[i] > 1000)
SUM(ge0, a[i] >= 0)
SUM(ge1, a[i] >= 1)
SUM(gem1, a[i] >= -1)
SUM(mirror_lt, 7 < a[i])
SUM(mirror_le, -3 <= a[i])
SUM(gt_min, a[i] > INT_MIN)
SUM(ge_minp1, a[i] >= INT_MIN + 1)
SUM(gt_maxm1, a[i] > INT_MAX - 1)
SUM(gt_t, a[i] > t)

int main(void) {
    static int buf[48];
    unsigned x = 2463534242u;
    unsigned long long h = 1469598103934665603ULL;
    const int ts[] = {0, -1, INT_MIN, INT_MAX, 5};
    for (int round = 0; round < 6; round++) {
        for (int i = 0; i < 48; i++) {
            x ^= x << 13; x ^= x >> 17; x ^= x << 5;
            int v = (int)x;
            switch (x >> 29) {
            case 0: v = INT_MIN; break;
            case 1: v = INT_MAX; break;
            case 2: v = (int)(x >> 22) - 512; break;
            case 3: v = -1 + (int)(x & 1); break;
            default: break;
            }
            buf[i] = v;
        }
        for (int off = 0; off < 3; off++)
            for (int n = 0; n < 38; n++) {
                int t = ts[(n + off) % 5];
                long r[] = {gt0(buf + off, n, t),      gtm1(buf + off, n, t),
                            gtm2(buf + off, n, t),     gt1000(buf + off, n, t),
                            ge0(buf + off, n, t),      ge1(buf + off, n, t),
                            gem1(buf + off, n, t),     mirror_lt(buf + off, n, t),
                            mirror_le(buf + off, n, t), gt_min(buf + off, n, t),
                            ge_minp1(buf + off, n, t), gt_maxm1(buf + off, n, t),
                            gt_t(buf + off, n, t)};
                for (unsigned k = 0; k < sizeof r / sizeof r[0]; k++)
                    h = (h ^ (unsigned long long)r[k]) * 1099511628211ULL;
            }
    }
    printf("%016llx\n", h);
    return 0;
}
