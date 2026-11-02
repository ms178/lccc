/* Valid frontend IR negative cases: no malformed SSA is needed to show refusal. */
typedef unsigned long long U;
__attribute__((noinline)) U atomic_product(volatile U *p, U n, U seed) {
    for (U i=0;i<n;++i) seed *= __atomic_load_n(p, __ATOMIC_RELAXED);
    return seed;
}
__attribute__((noinline)) long long signed_sum(volatile long long *p, U n, long long seed) {
    for (U i=0;i<n;++i) seed += *p;
    return seed;
}
__attribute__((noinline)) double fp_sum(volatile double *p, U n, double seed) {
    for (U i=0;i<n;++i) seed += *p;
    return seed;
}
__attribute__((noinline)) U stride_sum(volatile U *p, U n, U seed) {
    for (U i=0;i<n;i+=2) seed += *p;
    return seed;
}
__attribute__((noinline)) U dynamic_address(volatile U *p, U n, U seed) {
    for (U i=0;i<n;++i) seed += p[i];
    return seed;
}
__attribute__((noinline)) U intermediate_store(volatile U *p, volatile U *out, U n, U seed) {
    for (U i=0;i<n;++i) { seed += *p; *out=seed; }
    return seed;
}
__attribute__((noinline)) U two_reads(volatile U *p, volatile U *q, U n, U seed) {
    for (U i=0;i<n;++i) { seed += *p; seed ^= *q; }
    return seed;
}

/* Also a standalone corpus executable: all root-level .c files are linked and
 * run by run_regression.py, not just inspected by the mechanism contract. */
int main(void) {
    volatile U p = 2, q = 3, out = 0;
    volatile U array[4] = {2,3,4,5};
    volatile long long signed_value = 3;
    volatile double floating_value = 0.5;
    if (atomic_product(&p,4,3) != 48 || signed_sum(&signed_value,4,7) != 19 ||
        fp_sum(&floating_value,4,1.25) != 3.25 || stride_sum(&p,5,7) != 13 ||
        dynamic_address(array,4,7) != 21 || intermediate_store(&p,&out,4,7) != 15 ||
        out != 15 || two_reads(&p,&q,2,7) != 15) return 1;
    return 0;
}
