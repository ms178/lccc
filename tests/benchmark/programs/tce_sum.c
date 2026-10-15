// Benchmark 6: Tail-recursive accumulator sum (TCE demonstration)
// sum(n, acc) is a self-recursive tail call -> converted to a loop by TCE.
// Without TCE: 10M stack frames. With TCE: a single tight loop.
#include <stdio.h>

#ifndef TCE_N
#define TCE_N 10000000
#endif
#define TCE_STRINGIFY_INNER(x) #x
#define TCE_STRINGIFY(x) TCE_STRINGIFY_INNER(x)

static long sum(int n, long acc) {
    if (n <= 0) return acc;
    return sum(n - 1, acc + n);
}

int main(void) {
    volatile long result = sum(TCE_N, 0);
    printf("sum(" TCE_STRINGIFY(TCE_N) ") = %ld\n", result);
    return 0;
}
