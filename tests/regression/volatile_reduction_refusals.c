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
