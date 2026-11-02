/*
 * Loop-rotation regression: a header side effect that does NOT feed the loop
 * condition must execute for every guard evaluation.
 *
 * The original rotation cloned only the transitive closure of `i < n` into
 * the latch.  In this loop `trace[i] = ...` is deliberately sequenced before
 * the comparison but is outside that closure, so the transformed code wrote
 * trace[0] at the one-shot header guard and skipped trace[1..n] on later
 * iterations.  It returned a result different from GCC (for n == 4, the old
 * output had the final trace slot's sentinel rather than 59).
 *
 * The companion .env opt-in is essential: rotation is intentionally not
 * default-on.  The conservative Guard F must reject this loop, leaving its
 * sequenced header store on every trip.  A future transform may instead clone
 * all observably ordered header effects, but it must keep this result exact.
 */
#include <stdio.h>

static int data[8] = { 3, -7, 11, 5, -13, 17, 19, -23 };
static int trace[8];

__attribute__((noinline)) static int header_store_each_trip(int n) {
    int sum = 0;
    for (int i = 0; (trace[i] = 7 + i * 13), i < n; ++i)
        sum += data[i] ^ trace[i];
    /* The final failing guard evaluation writes trace[n] too. */
    return sum * 17 + trace[n];
}

int main(int argc, char **argv) {
    (void)argv;
    /* Runtime-derived and bounded: do not let a constant-trip transform hide
     * the header/latch shape that this regression exercises. */
    int n = (argc & 3) + 3;
    for (int i = 0; i < 8; ++i)
        trace[i] = -999;

    int got = header_store_each_trip(n);
    int sum = 0;
    for (int i = 0; i < n; ++i)
        sum += data[i] ^ (7 + i * 13);
    int expected = sum * 17 + (7 + n * 13);

    int trace_ok = 1;
    for (int i = 0; i <= n; ++i)
        trace_ok &= trace[i] == 7 + i * 13;
    printf("%d %d %d %d\n", n, got, expected, trace_ok);
    return got == expected && trace_ok ? 0 : 1;
}
