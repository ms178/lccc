// S10 pin: a vectorized 3-point FP stencil whose induction variable
// escapes (read after the loop). The IV is the loop's only carried value,
// so the stencil vectorizes; the vectorizer must then rewire the escaping
// post-loop IV use to the widened loop's final value. An unwired escape
// reads a stale scalar IV and `i` below diverges from GCC (which prints N).
#include <stdio.h>

double IN[516], OUT[512];
unsigned N;

int main(void) {
    N = 512;
    for (unsigned k = 0; k < N + 4; k++) IN[k] = (double)((k * 5u - 3u) & 127) / 16.0 - 4.0;
    unsigned i;
    for (i = 0; i < N; i++) OUT[i] = IN[i] + IN[i + 1] + IN[i + 2];
    double sum = 0.0;
    for (unsigned k = 0; k < N; k++) sum += OUT[k];
    printf("i=%u sum=%.1f\n", i, sum);
    return 0;
}
