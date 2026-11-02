/* Loop shapes: vectorisable stride, an FP reduction with four accumulators
 * (to pin down the reassociation freedom the standard grants), and a serial
 * dependency chain.  FP is rounded exactly once, at the end, so the answer
 * is bit-identical whatever order the additions happen in. */
#include <stdio.h>
#include <stdint.h>

static uint64_t stride_sum(const int32_t *p, int n) {   /* vectorises */
    uint64_t s = 0;
    for (int i = 0; i < n; i++) s += (uint64_t)(p[i] & 0xFF);
    return s;
}
static double fp_reduce(const double *p, int n) {
    double a0 = 0, a1 = 0, a2 = 0, a3 = 0;
    int i = 0;
    for (; i + 3 < n; i += 4) { a0 += p[i]; a1 += p[i+1]; a2 += p[i+2]; a3 += p[i+3]; }
    for (; i < n; i++) a0 += p[i];
    return (a0 + a1) + (a2 + a3);
}
static uint64_t serial(uint64_t n) {                   /* NOT vectorisable */
    uint64_t x = 1;
    for (uint64_t i = 0; i < n; i++)
        x = x * 6364136223846793005ull + 1442695040888963407ull;
    return x;
}
static int64_t nested(int n) {
    /* Asymmetric on purpose: a symmetric body cancels to 0 and the whole
     * case then passes whatever the compiler does. */
    int64_t c = 0;
    for (int i = 0; i < n; i++)
        for (int j = 0; j < n; j += 2)
            if (((i ^ j) & 3) == 0) c += (int64_t)i * 7 - j;
    return c;
}

int main(void) {
    int32_t v[256];
    double d[64];
    for (int i = 0; i < 256; i++) v[i] = (int32_t)((uint32_t)i * 2654435761u >> 13);
    for (int i = 0; i < 64; i++) d[i] = (double)(i % 7) * 0.125 - 0.5;
    printf("loops stride %llu\n", (unsigned long long)stride_sum(v, 256));
    printf("loops fp %016llx\n", (unsigned long long)(int64_t)(fp_reduce(d, 64) * 1024.0));
    printf("loops serial %016llx\n", (unsigned long long)serial(1000));
    printf("loops nested %lld\n", (long long)nested(64));
    return 0;
}
