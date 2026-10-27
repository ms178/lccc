/* Guarded EQUAL-WIDTH int sums (`int s; if (a[i] > K) s += a[i]`), the
 * late-rerun admission lowered to VecMaskedAddI32x8 on AVX2: constant
 * guards fold a rodata splat into vpcmpgtd, `> 0` is one vpmaxsd, run-time
 * guards broadcast. Every vector-width boundary trip count (0..4096),
 * misaligned bases, the >=/mirrored spellings, the INT_MIN/INT_MAX guard
 * and element edges, and the widening sibling. UB-free: the data are
 * partitioned so no int accumulator can overflow; output must equal GCC's.
 */
#include <stdio.h>
#include <limits.h>
#define NI __attribute__((noinline))
NI int g0(const int *a, int n)   { int s = 0; for (int i = 0; i < n; i++) if (a[i] > 0) s += a[i]; return s; }
NI int g1000(const int *a, int n){ int s = 0; for (int i = 0; i < n; i++) if (a[i] > 1000) s += a[i]; return s; }
NI int gneg(const int *a, int n) { int s = 0; for (int i = 0; i < n; i++) if (a[i] > -5) s += a[i]; return s; }
NI int gge(const int *a, int n)  { int s = 0; for (int i = 0; i < n; i++) if (a[i] >= 7) s += a[i]; return s; }
NI int gmir(const int *a, int n) { int s = 0; for (int i = 0; i < n; i++) if (-3 < a[i]) s += a[i]; return s; }
NI int gmax(const int *a, int n) { int s = 0; for (int i = 0; i < n; i++) if (a[i] > INT_MAX - 1) s += a[i]; return s; }
NI int gmin(const int *a, int n) { int s = 0; for (int i = 0; i < n; i++) if (a[i] > INT_MIN) s += a[i]; return s; }
NI int gt(const int *a, int n, int t) { int s = 0; for (int i = 0; i < n; i++) if (a[i] > t) s += a[i]; return s; }
NI long wl(const int *a, int n)  { long s = 0; for (int i = 0; i < n; i++) if (a[i] > 1000) s += a[i]; return s; }
int A[4100], M[4100], X[4100];
int main(void) {
    unsigned x = 12345;
    for (int i = 0; i < 4100; i++) { x = x * 1103515245u + 12345u; A[i] = (int)(x >> 9) % 4001 - 2000; M[i] = (i % 5 == 0) ? INT_MIN : (int)(x >> 12) % 2001 - 1000; X[i] = (i == 3) ? INT_MAX : -(int)(x % 1000); /* one INT_MAX: no overflow */ }
    static const int ns[] = {0,1,2,3,7,8,9,15,16,17,31,32,33,63,64,65,100,1000,4095,4096};
    unsigned long long h = 1469598103934665603ull;
#define MIX(v) (h = (h ^ (unsigned long long)(long long)(v)) * 1099511628211ull)
    for (int off = 0; off < 4; off += 1) for (unsigned k = 0; k < sizeof ns / sizeof *ns; k++) {
        int n = ns[k]; if (n + off > 4100) n = 4100 - off;
        const int *a = A + off;
        MIX(g0(a, n)); MIX(g1000(a, n)); MIX(gneg(a, n)); MIX(gge(a, n)); MIX(gmir(a, n)); MIX(wl(a, n));
        for (int t = -2001; t <= 2001; t += 667) MIX(gt(a, n, t));
        MIX(gt(a, n, INT_MIN)); MIX(gt(a, n, INT_MAX));
        MIX(gmin(M + off, n)); MIX(gt(M + off, n, INT_MIN));
        MIX(gmax(X + off, n)); MIX(gt(X + off, n, INT_MAX - 1));
    }
    printf("%016llx g0=%d gt=%d gmin=%d gmax=%d\n", h, g0(A, 4096), gt(A, 4096, 17), gmin(M, 4096), gmax(X, 4096));
    return 0;
}
