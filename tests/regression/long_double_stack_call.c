/* Regression: direct long double stack argument and x87 return obey SysV ABI. */
#include <stdio.h>

static long double add_scaled(long double x, int scale) {
    return x + (long double)scale / 4.0L;
}

int main(void) {
    long double result = add_scaled(1.5L, 6);
    printf("%.1Lf\n", result);
    return result != 3.0L;
}
