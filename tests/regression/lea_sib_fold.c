#include <stdio.h>

static int sum_indexed(const int *p, int n) {
    int sum = 0;
    for (int i = 0; i < n; ++i)
        sum += p[i];
    return sum;
}

int main(void) {
    int values[9] = {1, 2, 3, 4, 5, 6, 7, 8, 9};
    printf("%d %d\n", sum_indexed(values, 9), sum_indexed(values + 2, 5));
    return 0;
}
