/* Shared timing driver for scripts/bench_kernels.py.
 *
 * Compiled ONCE by the reference compiler and linked against every arm, so the
 * only thing that differs between arms is the kernel translation unit's
 * codegen. Prints "<seconds> <checksum>" for the harness to parse.
 *
 * The checksum exists so a faster arm that computes something different is
 * reported as a correctness failure instead of a speed win.
 */
#include <stdio.h>
#include <stdlib.h>
#include <time.h>

#include "bench.h"

/* Volatile sink: prevents any arm from deleting the kernel as dead code. */
volatile unsigned long long bench_sink;

static double now_seconds(void) {
    struct timespec ts;
    /* MONOTONIC_RAW is immune to NTP slew, which matters when samples are
     * tens of milliseconds and the harness takes the minimum. */
#ifdef CLOCK_MONOTONIC_RAW
    clock_gettime(CLOCK_MONOTONIC_RAW, &ts);
#else
    clock_gettime(CLOCK_MONOTONIC, &ts);
#endif
    return (double) ts.tv_sec + (double) ts.tv_nsec * 1e-9;
}

int main(int argc, char **argv) {
    long inner = (argc > 1) ? strtol(argv[1], NULL, 10) : 1000;
    if (inner <= 0) {
        inner = 1000;
    }

    bench_setup();

    /* Warm caches, branch predictors and any lazy PLT resolution so the timed
     * region measures steady-state code quality rather than first-touch cost. */
    unsigned long long warm = 0;
    for (long i = 0; i < 8; i++) {
        warm += bench_run();
    }
    bench_sink = warm;

    double t0 = now_seconds();
    unsigned long long acc = 0;
    for (long i = 0; i < inner; i++) {
        acc += bench_run();
    }
    double t1 = now_seconds();

    bench_sink = acc;
    printf("%.9f %llu\n", t1 - t0, acc);
    return 0;
}
