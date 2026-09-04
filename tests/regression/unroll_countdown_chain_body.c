#include <stdio.h>
/* Red-team cfg 3169 (unroll_stress seed=1): runtime-trip UNSIGNED COUNTDOWN
 * loop (`lim != i; i -= 1`) whose body is the straight-line chain of a
 * fully-unrolled inner loop.  The merged `analyze_loop` (through the extended
 * `find_iv_in_loop_ext`, which accepts `Sub(phi, k)` and sign-reinterprets
 * unsigned constants) used to hand this countdown IV to `do_unroll`, whose
 * guard arithmetic only knows Add-form steps: the partial unroll produced a
 * 4x clone chain whose early exits fell through to the exit block with the
 * PREVIOUS iteration's accumulator — 252 instead of 126.  `analyze_loop` now
 * uses a strict Add-only detector (the extended one stays confined to the
 * complete unrollers, which re-verify the step in closed form), so the
 * countdown loop simply stays rolled. */
__attribute__((noinline)) unsigned long long f0000(unsigned long lim) {
    unsigned long long acc = 0;
    unsigned long i;
    for (i = ((unsigned long)6ul); (lim != i); i -= 1) {
        { int j; for (j = 0; j < 3; j++) acc += (unsigned long long)i * (j + 1); }
    }
    return acc;
}
int main(void) {
    volatile int zero = 0; (void)zero;
    printf("f0000 %llu\n", f0000((unsigned long)(((unsigned long)0ul) + zero)));
    return 0;
}
