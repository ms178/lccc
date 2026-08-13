/* Regression: reduction cast-sum type check.
 * `float s += (float)int_arr[i]` was vectorized as packed I32 adds on a F32
 * accumulator (only widening was rejected), returning 0.0. Only redundant
 * (same-type) casts are vectorizable; cross-kind casts must be rejected. */
#include <stdio.h>

int arr[256];

static float fsum(int *x, int n) {
    float s = 0.0f;
    for (int i = 0; i < n; i++)
        s += (float)x[i];
    return s;
}

int main(void) {
    for (int i = 0; i < 256; i++) arr[i] = 1;
    if (fsum(arr, 256) != 256.0f) {
        printf("FAIL got=%f\n", (double)fsum(arr, 256));
        return 1;
    }
    printf("OK\n");
    return 0;
}
