// S10 pin: a vectorized map loop whose induction variable escapes (read
// after the loop). The IV is the loop's only carried value, so the map
// vectorizes; the vectorizer must then rewire the escaping post-loop IV
// use to the widened loop's final value. An unwired escape reads a stale
// scalar IV and `i` below diverges from GCC (which prints N).
#include <stdio.h>

unsigned IN[512], OUT[512];
unsigned N;

int main(void) {
    N = 512;
    for (unsigned k = 0; k < N; k++) IN[k] = (k * 3u + 1u) & 255u;
    unsigned i;
    for (i = 0; i < N; i++) OUT[i] = IN[i] + 1u;
    unsigned wsum = 0;
    for (unsigned k = 0; k < N; k++) wsum += OUT[k] * (k & 7u);
    printf("i=%u wsum=%u\n", i, wsum);
    return 0;
}
