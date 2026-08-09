/* Regression: GVN must not CSE/forward loads from parameter allocas.
 * If it does, the inliner's store-into-param-alloca argument passing is
 * bypassed: the forwarded value's definition (the ParamRef) is removed at
 * inline time, leaving an undefined value. Symptom: inlined descending
 * `for (i = n-1; i >= 0; i--)` loops produced i = -1 (n treated as 0),
 * returning 0 instead of the correct sum. Only reproducible when GVN runs
 * before inlining (pre-inline canonicalization).
 */
#include <stdint.h>
#include <stdio.h>

static int loop(int n) {
    int a = 0;
    for (int i = n - 1; i >= 0; i--) a = (a + i) & 0x7ffff;
    return a;
}

static int arr(const int *v, int n) {
    int a = 0;
    for (int i = n - 1; i >= 0; i--) a = (a + v[i]) & 0x7ffff;
    return a;
}

int main(void) {
    const int vals[8] = {1, -2, 3, -4, 5, -6, 7, -8};
    int x = 8; /* runtime value, must not be confused with the init 0 */
    if (loop(x) != 28) { printf("FAIL loop(%d)=%d want 28\n", x, loop(x)); return 1; }
    if (arr(vals, x) != 524284) { printf("FAIL arr=%d want 524284\n", arr(vals, x)); return 2; }
    if (loop(0) != 0) { printf("FAIL loop(0)=%d\n", loop(0)); return 3; }
    printf("GVN-PARAM-ALLOCA-OK\n");
    return 0;
}
