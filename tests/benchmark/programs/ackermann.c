/* Optional bounded instrumentation size; default workload is unchanged. */
#ifndef ACK_N
#define ACK_N 11
#endif
// Ackermann function - deep recursion stress test (stack, call overhead)
#include <stdio.h>

static int ackermann(int m, int n) {
    if (m == 0) return n + 1;
    if (n == 0) return ackermann(m - 1, 1);
    return ackermann(m - 1, ackermann(m, n - 1));
}

int main(void) {
    // ack(3,11) = 16381 - deep enough to stress call overhead
    int result = ackermann(3, ACK_N);
    printf("ackermann(3,%d) = %d\n", ACK_N, result);
    return 0;
}
