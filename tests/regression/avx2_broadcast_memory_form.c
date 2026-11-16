/* Regression: under -mavx2 a scalar broadcast from memory must be the single
 * memory-source instruction (vbroadcastss / vbroadcastsd), not a load, a stack
 * store, and a reload. The check gate asserts the instruction is present in
 * the object; the values are checked against the GCC oracle as well. */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

__attribute__((noinline)) __m256 bcast_ss_mem(const float *p) {
    return _mm256_broadcast_ss(p);
}

__attribute__((noinline)) __m256d bcast_sd_mem(const double *p) {
    return _mm256_broadcast_sd(p);
}

static unsigned fold(unsigned acc, unsigned w) { return acc * 1000003u ^ w; }

int main(void) {
    float f = -0.0f;
    double d = 1.0 / 3.0;
    unsigned fbits = 0x7fa5a5a5u;   /* quiet NaN with a payload */
    memcpy(&f, &fbits, sizeof f);
    __m256 vf = bcast_ss_mem(&f);
    __m256d vd = bcast_sd_mem(&d);
    unsigned wf[8];
    unsigned long long wd[4];
    memcpy(wf, &vf, sizeof wf);
    memcpy(wd, &vd, sizeof wd);
    unsigned acc = 0;
    for (int i = 0; i < 8; ++i) acc = fold(acc, wf[i]);
    for (int i = 0; i < 4; ++i) {
        acc = fold(acc, (unsigned)wd[i]);
        acc = fold(acc, (unsigned)(wd[i] >> 32));
    }
    printf("avx2_broadcast_memory_form %08x\n", acc);
    return 0;
}
