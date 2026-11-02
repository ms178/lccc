/* Regression: vectorization must not misaddress a runtime-sized stack array. */
#include <stdio.h>

static int sum_vla(int n) {
    int a[n];
    int sum = 0;
    for (int i = 0; i < n; ++i) {
        a[i] = i + 1;
        sum += a[i];
    }
    return sum;
}

int main(void) {
    int got = sum_vla(100);
    printf("%d\n", got);
    return got != 5050;
}
