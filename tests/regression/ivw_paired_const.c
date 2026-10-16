/* Constant-limit paired counter: the iv_widen sibling-mirror proof fires here.
 *
 * `j` counts down from a runtime seed to a strict constant threshold, and the
 * testless counter `i` indexes memory.  With -fwrapv, the proof widens `i` to
 * 64 bits and the counter is not read after the loop (that would be the
 * refused escape case). The function body must differ from the kill-switch
 * build, which tests/regression/check_ivw_paired_counter.sh asserts.  Output is compared with
 * the gcc -O0 reference.
 */
#include <stdio.h>

static int a[4096];

__attribute__((noinline)) static long paired_const(int hi, const int *a) {
    long acc = 0;
    int i = 0;
    int j = hi;
    while (j > 0) {
        acc += a[i];
        i++;
        j--;
    }
    return acc;
}

int main(void) {
    unsigned seed = 7u;
    for (int k = 0; k < 4096; ++k) {
        seed = seed * 1664525u + 1013904223u;
        a[k] = (int)(seed >> 8) - (1 << 22);
    }
    long sum = 0;
    for (int hi = -3; hi <= 1500; hi += 37) {
        sum = sum * 1000003L + paired_const(hi, a);
    }
    sum = sum * 1000003L + paired_const(4000, a);
    printf("ivw_paired_const %ld\n", sum);
    return 0;
}
