/* Per-call cost of the runtime __builtin_cpu_supports probe (AUD-CPU-1).
 *
 * The builtin is loop-invariant, so a naive loop lets the optimiser hoist it
 * out (correctly - it is a plain load).  Measuring the *per-site* cost
 * therefore needs one probe per call: `pick()` is noinline and the result
 * depends on the induction variable, so every iteration pays one real call,
 * one real probe and one select, and no compiler can collapse the loop.
 *
 * Three modes, all built from this file so the loop is character-identical:
 *
 *   (default)    pick() = __builtin_cpu_supports("avx2") ? i+1 : -i
 *   -DNO_PROBE   pick() = control_flag          ? i+1 : -i   (one volatile
 *                load, i.e. the cheapest possible stand-in for "read a flag";
 *                volatile so no compiler may hoist or fold it).  The delta
 *                `probe - control` is the marginal cost of the probe
 *                machinery: its initialisation guard plus the word load and
 *                AND, over and above one memory read.
 *   -DINIT_ONLY  calls the builtin exactly once, so callgrind sees the
 *                one-time initialisation cost (the CPUID sequence).
 *
 * A measured binary must print `1` (probe stable across the loop); run.sh
 * refuses to report a count for a binary that did not.
 */
#include <stdio.h>

#if defined(NO_PROBE)
static volatile int control_flag = 1;
#endif

#ifndef ITERS
#define ITERS 50000000
#endif

__attribute__((noinline)) int pick(long i) {
#if defined(INIT_ONLY)
    (void)i;
    return !!__builtin_cpu_supports("avx2");
#elif defined(NO_PROBE)
    return control_flag ? (int)i + 1 : -(int)i;
#else
    return __builtin_cpu_supports("avx2") ? (int)i + 1 : -(int)i;
#endif
}

int main(void) {
#ifdef INIT_ONLY
    printf("%d\n", pick(0));
    return 0;
#else
    long i;
    int s = 0;
    for (i = 0; i < ITERS; i++) {
        s += pick(i);
    }
    /* Both arms are positive for every i and the sign of the sum is stable,
     * so this is 1 iff the probe answer did not change under the loop. */
    printf("%d\n", s > 0 ? 1 : 0);
    return 0;
#endif
}
