/* Regression: loop_memory_promote must not invent faulting memory accesses on
 * a zero-trip path. The first case uses an out-of-bounds alloca-derived
 * address; the second executes the load but not the store and points at a
 * read-only global. Both programs are valid on the exercised zero-trip path. */

static const int read_only_value = 7;

__attribute__((noinline)) static int out_of_bounds_alloca(int n) {
    int local = 7;
    unsigned long bits = (unsigned long)&local + (1UL << 47);
    int *bad = (int *)bits;
    for (int i = 0; i < n; ++i) {
        int value = *bad;
        *bad = value + 1;
    }
    return local != 7;
}

__attribute__((noinline)) static int store_does_not_execute(int n) {
    int *p = (int *)(unsigned long)&read_only_value;
    for (;;) {
        int value = *p;
        if (n-- <= 0)
            return value != 7;
        *p = value + 1;
    }
}

int main(void) {
    if (out_of_bounds_alloca(0) != 0)
        return 1;
    if (store_does_not_execute(0) != 0)
        return 2;
    return 0;
}
