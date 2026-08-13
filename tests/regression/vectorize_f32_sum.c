/* F32 reduction vectorization (sum + dot product), AVX2 8-wide / SSE2 4-wide.
 * Values are exactly representable so the result is order-independent. */
#include <stdio.h>

float fa[1024], fb[1024];

static float fsum(float *a, int n) { float s = 0; for (int i = 0; i < n; i++) s += a[i]; return s; }
static float fdot(float *a, float *b, int n) { float s = 0; for (int i = 0; i < n; i++) s += a[i]*b[i]; return s; }

int main(void) {
    for (int i = 0; i < 1024; i++) { fa[i] = (float)(i + 1); fb[i] = (float)(i + 1); }
    static const int ns[] = {1, 2, 3, 4, 5, 7, 8, 9, 15, 16, 17, 63, 64, 65, 255, 256, 257};
    for (unsigned k = 0; k < sizeof(ns)/sizeof(ns[0]); k++) {
        long long n = ns[k];
        float sum_exp = (float)(n * (n + 1) / 2);
        float dot_exp = (float)(n * (n + 1) * (2 * n + 1) / 6); /* sum (i+1)^2 */
        if (fsum(fa, (int)n) != sum_exp) { printf("FAIL fsum n=%lld\n", n); return 1; }
        if (fdot(fa, fb, (int)n) != dot_exp) { printf("FAIL fdot n=%lld\n", n); return 1; }
    }
    printf("OK\n");
    return 0;
}
