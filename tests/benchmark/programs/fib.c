/* Optional bounded instrumentation size; default workload is unchanged. */
#ifndef FIB_N
#define FIB_N 40
#endif
// Benchmark 2: Recursive Fibonacci (function call overhead)
#include <stdio.h>

long fib(int n) {
    if (n <= 1) return n;
    return fib(n - 1) + fib(n - 2);
}

int main(void) {
    volatile long result = fib(FIB_N);
    printf("fib(%d) = %ld\n", FIB_N, result);
    return 0;
}
