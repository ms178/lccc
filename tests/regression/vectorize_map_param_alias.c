/* Ordinary pointer parameters are not disjoint merely because they have
 * different SSA value numbers.  dst=src+1 creates a real loop-carried
 * dependence; an 8-wide map used to read stale lanes and return mostly zero. */
#include <stdio.h>

__attribute__((noinline))
static void map(float *dst, const float *src, int n) {
    for (int i = 0; i < n; i++) dst[i] = src[i] * 2.0f + 1.0f;
}

int main(void) {
    float a[25];
    for (int i = 0; i < 25; i++) a[i] = (float)i;
    map(a + 1, a, 16);
    for (int i = 0; i <= 16; i++) {
        float expect = (float)((1U << i) - 1U);
        if (a[i] != expect) {
            printf("FAIL i=%d got=%.0f expect=%.0f\n", i, (double)a[i], (double)expect);
            return 1;
        }
    }
    puts("OK");
    return 0;
}
