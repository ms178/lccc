// Compile-time recursive specialization benchmark.
#include <stdio.h>

static int ackermann(int m, int n) {
    if (m == 0) return n + 1;
    if (n == 0) return ackermann(m - 1, 1);
    return ackermann(m - 1, ackermann(m, n - 1));
}

int main(void) {
    volatile int result = ackermann(3, 11);
    printf("constant ackermann: %d\n", result);
    return 0;
}
