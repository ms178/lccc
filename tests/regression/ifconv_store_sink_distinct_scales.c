/*
 * R2 red-team regression: the canonical-address key used by
 * rewrite_covered_arm_loads and sink_conditional_stores must be INJECTIVE
 * over addresses.  The original offset canonicalization traced `Shl(v, k)`
 * down to its root WITHOUT recording k, so `d[i]` (byte offset i<<2) and
 * `d[2*i]` (byte offset i<<3) collapsed to the same key:
 *
 *   - sink_conditional_stores merged the two arms' stores to DIFFERENT
 *     addresses into one store at the first arm's address (both cells
 *     corrupt), and
 *   - rewrite_covered_arm_loads replaced the arm's `d[2*i]` load with the
 *     pred's `d[i]` value (pre-existing latent bug, same root).
 *
 * Both kernels below are self-checking and GCC-differential; run at -O2 so
 * the Phase-7 if-conversion fixpoint (sink + diamond conversion) executes.
 */
#include <stdio.h>

#define N 32

/* Store-sink variant: the two arms store to d[i] and d[2*i]. */
__attribute__((noinline))
void sink_scales(float *d, float x, float y, int n) {
    for (int i = 0; i < n; i++) {
        if (x > 0.0f)
            d[i] = x + (float)i;
        else
            d[2 * i] = y - (float)i;
    }
}

/* Load-rewrite variant: the pred loads d[i], the arm loads d[2*i]. */
__attribute__((noinline))
float load_scales(const float *d, int i, int c) {
    float t = d[i];
    float r = 1.5f;
    if (c)
        r = d[2 * i];
    return t + r;
}

int main(void) {
    float d[2 * N];
    for (int i = 0; i < 2 * N; i++)
        d[i] = -7.0f;

    /* x > 0: true arm stores d[i]; d[2*i] must stay -7.0f. */
    sink_scales(d, 3.0f, 9.0f, N);
    for (int i = 0; i < N; i++)
        printf("sink %d %.1f %.1f\n", i, d[i], d[2 * i]);

    /* x < 0: false arm stores d[2*i]; d[i] must stay -7.0f. */
    for (int i = 0; i < 2 * N; i++)
        d[i] = -7.0f;
    sink_scales(d, -3.0f, 9.0f, N);
    for (int i = 0; i < N; i++)
        printf("sink2 %d %.1f %.1f\n", i, d[i], d[2 * i]);

    float e[N];
    for (int i = 0; i < N; i++)
        e[i] = (float)i * 0.5f + 1.0f;
    for (int i = 1; i < N / 2; i++)
        printf("load %d %.2f %.2f\n", i,
               load_scales(e, i, 0), load_scales(e, i, 1));
    return 0;
}
