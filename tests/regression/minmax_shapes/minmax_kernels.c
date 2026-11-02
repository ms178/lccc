/* MINMAX-1 kernels: the min/max/sum shapes the differential harness
 * exercises.  No main - this file is only ever linked by
 * `check_minmax_reduction.sh` (and never by run_regression.py, which
 * requires a main), which is why it lives in a subdirectory.
 *
 * `mnmax_kernel`/`strict_kernel`/`ge_kernel` deliberately combine an
 * extreme with a second accumulator: MINMAX-1 refuses those (see the
 * gate's contract 3) and they must STAY correct when refused.
 */
// Min/max/sum reduction kernels in the shapes the vectorizer must handle
// (and, for from1_kernel, must refuse to vectorize: c == 1).
void mnmax_kernel(const int *a, int n, int *mn, int *mx, int *sum) {
    int lo = a[0], hi = a[0], s = 0;
    for (int i = 0; i < n; i++) {
        if (a[i] < lo) lo = a[i];
        if (a[i] > hi) hi = a[i];
        s += a[i];
    }
    *mn = lo; *mx = hi; *sum = s;
}
void from1_kernel(const int *a, int n, int *mn, int *mx) {
    int lo = a[0], hi = a[0];
    for (int i = 1; i < n; i++) {
        if (a[i] < lo) lo = a[i];
        if (a[i] > hi) hi = a[i];
    }
    *mn = lo; *mx = hi;
}
void strict_kernel(const int *a, int n, int *mn, int *mx) {
    int lo = a[0], hi = a[0];
    for (int i = 0; i < n; i++) {
        if (a[i] < lo) lo = a[i];
        if (hi < a[i]) hi = a[i];
    }
    *mn = lo; *mx = hi;
}
void ge_kernel(const int *a, int n, int *mn, int *mx) {
    int lo = a[0], hi = a[0];
    for (int i = 0; i < n; i++) {
        if (lo >= a[i]) lo = a[i];
        if (a[i] >= hi) hi = a[i];
    }
    *mn = lo; *mx = hi;
}
