#include <stdio.h>

static long sum_to(int n, long acc) {
    if (n <= 0) return acc;
    return sum_to(n - 1, acc + n);
}

int main(void) {
    printf("%ld %ld %ld %ld\n",
           sum_to(-3, 11),
           sum_to(0, 11),
           sum_to(1, 11),
           sum_to(100, 7));
    return 0;
}
