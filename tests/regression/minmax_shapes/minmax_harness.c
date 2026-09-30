// Differential harness: min/max/sum reductions over randomly shaped arrays.
// Exits non-zero on the first disagreement with the reference implementation.
#include <stdio.h>
#include <limits.h>
#include <stdlib.h>

#define MAXN 4096
static int a[MAXN];

/* Kernels under test: `minmax_shapes/minmax_kernels.c`.  They are compiled
 * separately (by lccc AND by the oracle) so a miscompile can be localised to
 * the vectorized side instead of being masked by the harness's own codegen. */
void mnmax_kernel(const int *a, int n, int *mn, int *mx, int *sum);
void from1_kernel(const int *a, int n, int *mn, int *mx);
void strict_kernel(const int *a, int n, int *mn, int *mx);
void ge_kernel(const int *a, int n, int *mn, int *mx);

static unsigned long long rs = 88172645463325252ULL;
static unsigned long long rnd(void) {
    rs ^= rs << 13; rs ^= rs >> 7; rs ^= rs << 17; return rs;
}

// min/max/sum over a[0..n), with the accumulator seeded from a[0] (the shape
// the vectorizer turns into a lane-wise vpmin/vpmax reduction).
static void ref(int n, int *mn, int *mx, int *sum) {
    int lo = a[0], hi = a[0]; long long s = 0;
    for (int i = 0; i < n; i++) {
        if (a[i] < lo) lo = a[i];
        if (a[i] > hi) hi = a[i];
        s += a[i];
    }
    *mn = lo; *mx = hi; *sum = (int)s;
}
// Same, but the loop starts at 1 with the accumulator seeded from a[0]:
// this is the c == 1 shape the detector must REJECT (and stay scalar/correct).
static void ref_from1(int n, int *mn, int *mx) {
    int lo = a[0], hi = a[0];
    for (int i = 1; i < n; i++) {
        if (a[i] < lo) lo = a[i];
        if (a[i] > hi) hi = a[i];
    }
    *mn = lo; *mx = hi;
}
// Strict-inequality and swapped-operand spellings of the same reductions.
static void ref_strict(int n, int *mn, int *mx) {
    int lo = a[0], hi = a[0];
    for (int i = 0; i < n; i++) {
        if (a[i] < lo) lo = a[i];          // take x when x < acc  -> min
        if (hi < a[i]) hi = a[i];          // operands swapped     -> max
    }
    *mn = lo; *mx = hi;
}
static void ref_ge(int n, int *mn, int *mx) {
    int lo = a[0], hi = a[0];
    for (int i = 0; i < n; i++) {
        if (lo >= a[i]) lo = a[i];         // <= spelling -> min
        if (a[i] >= hi) hi = a[i];         // >= spelling -> max
    }
    *mn = lo; *mx = hi;
}

int main(void) {
    int fails = 0;
    // 1) every length around the vector width, plus large/random lengths
    for (int n = 1; n <= 80; n++) {
        for (int kind = 0; kind < 4; kind++) {
            for (int t = 0; t < 3; t++) {
                for (int i = 0; i < n; i++) {
                    unsigned long long r = rnd();
                    switch (kind) {
                    case 0: a[i] = (int)(r & 0xffff) - 32768; break;         // small
                    case 1: a[i] = (int)r; break;                            // full range
                    case 2: a[i] = (r & 1) ? INT_MIN : INT_MAX; break;       // extremes
                    case 3: a[i] = (int)(r % 3) - 1; break;                  // duplicates
                    }
                }
                if (t == 1 && n > 1) a[n / 2] = INT_MIN;   // interior extreme
                if (t == 2 && n > 1) a[n - 1] = INT_MAX;   // tail extreme
                int mn, mx, sum; ref(n, &mn, &mx, &sum);
                int gmn, gmx, gsum; mnmax_kernel(a, n, &gmn, &gmx, &gsum);
                if (gmn != mn || gmx != mx || gsum != sum) {
                    printf("MISMATCH mnmax n=%d kind=%d t=%d: got (%d,%d,%d) want (%d,%d,%d)\n",
                           n, kind, t, gmn, gmx, gsum, mn, mx, sum);
                    if (++fails > 8) return 1;
                }
                int s1mn, s1mx, f1mn, f1mx;
                ref_from1(n, &s1mn, &s1mx); from1_kernel(a, n, &f1mn, &f1mx);
                if (s1mn != f1mn || s1mx != f1mx) {
                    printf("MISMATCH from1 n=%d kind=%d t=%d: got (%d,%d) want (%d,%d)\n",
                           n, kind, t, f1mn, f1mx, s1mn, s1mx);
                    if (++fails > 8) return 1;
                }
                int stmn, stmx, gmn2, gmx2;
                ref_strict(n, &stmn, &stmx); strict_kernel(a, n, &gmn2, &gmx2);
                if (stmn != gmn2 || stmx != gmx2) {
                    printf("MISMATCH strict n=%d kind=%d t=%d: got (%d,%d) want (%d,%d)\n",
                           n, kind, t, gmn2, gmx2, stmn, stmx);
                    if (++fails > 8) return 1;
                }
                int gemn, gemx, gmn3, gmx3;
                ref_ge(n, &gemn, &gemx); ge_kernel(a, n, &gmn3, &gmx3);
                if (gemn != gmn3 || gemx != gmx3) {
                    printf("MISMATCH ge n=%d kind=%d t=%d: got (%d,%d) want (%d,%d)\n",
                           n, kind, t, gmn3, gmx3, gemn, gemx);
                    if (++fails > 8) return 1;
                }
            }
        }
    }
    // 2) large lengths
    for (int n = 1000; n <= MAXN; n += 97) {
        for (int i = 0; i < n; i++) a[i] = (int)rnd();
        int mn, mx, sum; ref(n, &mn, &mx, &sum);
        int gmn, gmx, gsum; mnmax_kernel(a, n, &gmn, &gmx, &gsum);
        if (gmn != mn || gmx != mx || gsum != sum) {
            printf("MISMATCH big n=%d: got (%d,%d,%d) want (%d,%d,%d)\n",
                   n, gmn, gmx, gsum, mn, mx, sum);
            if (++fails > 8) return 1;
        }
    }
    printf("minmax differential: %s (%d failures)\n", fails ? "FAIL" : "PASS", fails);
    return fails != 0;
}
