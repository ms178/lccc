/* v4 regression: variadic functions + FP/GP ABI mixing (the classic x86-64
 * varargs hazard: %al = number of vector args), plus qsort callback ABI. */
#include <stdio.h>
#include <stdlib.h>
#include <stdarg.h>

static int vsum(int n, ...) {
    va_list ap;
    va_start(ap, n);
    int s = 0;
    for (int i = 0; i < n; i++) s += va_arg(ap, int);
    va_end(ap);
    return s;
}

static double vavg(int n, ...) {
    va_list ap;
    va_start(ap, n);
    double s = 0;
    for (int i = 0; i < n; i++) s += va_arg(ap, double);
    va_end(ap);
    return n ? s / n : 0;
}

static const char *vpick(int which, ...) {
    va_list ap;
    va_start(ap, which);
    const char *r = 0;
    for (int i = 0; i <= which; i++) r = va_arg(ap, const char*);
    va_end(ap);
    return r;
}

static long long vsumll(int n, ...) {
    va_list ap;
    va_start(ap, n);
    long long s = 0;
    for (int i = 0; i < n; i++) s += va_arg(ap, long long);
    va_end(ap);
    return s;
}

static int cmp_int(const void *a, const void *b) {
    int x = *(const int*)a, y = *(const int*)b;
    return (x > y) - (x < y);
}

int main(void) {
    if (vsum(4, 1, 2, 3, 4) != 10) return 1;
    if (vsum(0) != 0) return 2;

    /* mixed FP args after varargs — exercises %al/SSE regs */
    if (vavg(3, 1.0, 2.0, 3.0) != 2.0) return 3;
    if (vavg(2, 10.0, 20.0) != 15.0) return 4;
    if (vavg(8, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0) != 4.5) return 5;

    if (vpick(2, "zero", "one", "two", "three") != (const char*)"two") return 6;

    /* long long varargs (register pairs / stack) */
    if (vsumll(3, 10000000000LL, 20000000000LL, 30000000000LL) != 60000000000LL) return 7;
    if (vsumll(8, 1LL, 2LL, 3LL, 4LL, 5LL, 6LL, 7LL, 8LL) != 36LL) return 8;

    /* qsort callback ABI */
    int arr[10] = {9, 3, 7, 1, 8, 2, 6, 4, 0, 5};
    qsort(arr, 10, sizeof(int), cmp_int);
    for (int i = 0; i < 10; i++) if (arr[i] != i) return 9;

    /* printf varargs (various widths) */
    if (printf("%d %s %.2f %ld\n", 5, "hi", 3.5, 42L) < 0) return 10;

    printf("OK varargs_abi\n");
    return 0;
}
