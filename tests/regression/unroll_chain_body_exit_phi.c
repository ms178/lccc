#include <stdio.h>
/* Red-team cfg 1765 (unroll_stress seed=1): runtime-trip loop whose body is a
 * straight-line chain left behind by the complete unroller of an inner loop.
 * do_unroll's early exit-check edges used to read the header phi's PREHEADER
 * value (0 instead of 30) because the Step-5b reader rewrite covered only
 * instructions, never the RETURN terminator. */
__attribute__((noinline)) unsigned long long f0000(long lim) {
    unsigned long long acc = 0;
    long i;
    for (i = ((long)5l); (i <= lim); i += 7) {
        { int j; for (j = 0; j < 3; j++) acc += (unsigned long long)i * (j + 1); }
    }
    return acc;
}
int main(void) {
    volatile int zero = 0; (void)zero;
    printf("f0000 %llu\n", f0000((long)(((long)11l) + zero)));
    return 0;
}
