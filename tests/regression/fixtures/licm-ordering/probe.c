/* Plain global accesses are protected by a release/acquire handshake in driver.c.
   In the second iteration even a load textually BEFORE the acquire is after
   the first iteration's acquire. Ordinal-only LICM across the backedge is wrong. */
extern int ordering_g0, ordering_g1, ordering_g2, ordering_g3;
extern unsigned ordering_ready, ordering_done;
__attribute__((noinline)) int ordering_before_acquire(int n) {
    int sum = 0;
    for (int i = 0; i < n; ++i) {
        sum += ordering_g0 + ordering_g1 + ordering_g2 + ordering_g3;
        if (i == 0) {
            __atomic_store_n(&ordering_ready, 1, __ATOMIC_RELEASE);
            while (!__atomic_load_n(&ordering_done, __ATOMIC_ACQUIRE)) {}
        }
    }
    return sum;
}
/* Shape-only cost witnesses; not concurrent performance measurements. */
__attribute__((noinline)) unsigned ordering_acquire_shape(unsigned n) {
    unsigned sum = 0;
    if (!n) return 0;
    unsigned i = 0;
    do {
        __atomic_load_n(&ordering_done, __ATOMIC_ACQUIRE);
        sum += (unsigned)ordering_g0 + ordering_g1 + ordering_g2 + ordering_g3;
    } while (++i < n);
    return sum;
}
__attribute__((noinline)) unsigned ordering_plain_shape(unsigned n) {
    unsigned sum = 0;
    if (!n) return 0;
    unsigned i = 0;
    do {
        sum += (unsigned)ordering_g0 + ordering_g1 + ordering_g2 + ordering_g3;
    } while (++i < n);
    return sum;
}
