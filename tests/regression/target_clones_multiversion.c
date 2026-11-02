#include <stdio.h>

#if defined(__x86_64__)
__attribute__((target_clones("default", "sse4.2", "avx2", "arch=x86-64-v3")))
#endif
int vec_add_reduce(int * restrict out, const int * restrict a, const int * restrict b, int n) {
    int acc = 0;
    for (int i = 0; i < n; i++) {
        out[i] = a[i] + b[i];
        acc += out[i];
    }
    return acc;
}

#if defined(__x86_64__)
[[gnu::target_clones("default", "avx2")]]
#endif
static int scale_sum(const int *a, int n) {
    int s = 0;
    for (int i = 0; i < n; i++)
        s += a[i] * 3;
    return s;
}

int main(void) {
    int a[64], b[64], out[64];
    for (int i = 0; i < 64; i++) {
        a[i] = i + 1;
        b[i] = 100 - i;
    }
    int r1 = vec_add_reduce(out, a, b, 64);
    int r2 = scale_sum(a, 64);
    printf("%d %d\n", r1, r2);
    return (r1 == 6464 && r2 == 6240) ? 0 : 1;
}
