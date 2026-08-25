/*
 * Masked widening I32->I64 reduction (conditional sums, v8):
 *   long s = 0; for (...) if (a[i] > K) s += a[i];
 * Verified elementwise against volatile-anchored references for const,
 * variable, and negative guard thresholds, plus a small tail (16 = 4*4,
 * exercising the masked vector body and the scalar remainder).
 */
#include <stdio.h>
static int a[64];
static long cs(int *arr, int n, int t) {
    long s = 0;
    for (int i = 0; i < n; i++) if (arr[i] > t) s += arr[i];
    return s;
}
int main(void) {
    for (int i = 0; i < 64; i++) a[i] = (i * 37 % 211) - 105;
    int bad = 0;
    for (int n = 1; n <= 64; n *= 4) {
        long r0 = 0, r5 = 0, rn = 0, rv = 0;
        for (int i = 0; i < n; i++) {
            volatile int x = a[i];
            if (x > 0) r0 += x;
            if (x > 5) r5 += x;
            if (x > -100) rn += x;
            if (x > 30) rv += x;
        }
        if (cs(a, n, 0) != r0) bad++;
        if (cs(a, n, 5) != r5) bad++;
        if (cs(a, n, -100) != rn) bad++;
        if (cs(a, n, 30) != rv) bad++;
    }
    printf("%s\n", bad == 0 ? "OK" : "FAIL");
    return bad;
}
