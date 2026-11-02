/*
 * OP-05a: generalized (stencil) non-reduction FP loop vectorization.
 *
 * Exercises the stencil matcher end to end:
 *   - five taps at constant element offsets (-2..+2) from one base
 *   - non-zero IV start (2) and a derived exit compare (i + 2 < n)
 *   - weighted taps (w != 1), a subtraction (exact via * -1 + add),
 *     and a loop-invariant constant term
 *   - dynamic (runtime) trip count -> runtime guard + scalar remainder
 *
 * The checksum is compared against a scalar oracle computed here; the
 * vector body must be bit-exact (same op order per lane, no
 * reassociation): the printed value must match on every run.
 */
#include <stdio.h>
#include <stdlib.h>

#define N 257

static float src[N + 8];
static float dst[N];
static float ref[N];

__attribute__((noinline)) static void stencil5(float *restrict d,
                                               const float *restrict s,
                                               int n) {
    const float w0 = 0.5f, w4 = 2.0f, c = 1.0f;
    for (int i = 2; i + 2 < n; ++i)
        d[i] = w0 * s[i - 2] + s[i - 1] + s[i] + s[i + 1]
               - w4 * s[i + 2] + c;
}

static void scalar_ref(float *restrict d, const float *restrict s, int n) {
    const float w0 = 0.5f, w4 = 2.0f, c = 1.0f;
    for (int i = 2; i + 2 < n; ++i)
        d[i] = w0 * s[i - 2] + s[i - 1] + s[i] + s[i + 1]
               - w4 * s[i + 2] + c;
}

int main(void) {
    for (int i = 0; i < N + 8; ++i)
        src[i] = (float)((i * 37) % 19) * 0.25f - 2.0f;
    for (int i = 0; i < N; ++i) {
        dst[i] = -1000.0f;
        ref[i] = -1000.0f;
    }
    stencil5(dst, src, N);
    scalar_ref(ref, src, N);
    float sum = 0.0f;
    int bad = 0;
    for (int i = 0; i < N; ++i) {
        if (dst[i] != ref[i] && bad < 4)
            bad++;
        sum += dst[i];
    }
    printf("%.3f %d\n", sum, bad);
    /* bit-exact against the scalar oracle, and the untouched boundary
     * lanes must survive */
    return (bad == 0 && dst[0] == -1000.0f && dst[1] == -1000.0f
            && dst[N - 1] == -1000.0f) ? 0 : 1;
}
