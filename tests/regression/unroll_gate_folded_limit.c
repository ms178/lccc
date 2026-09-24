// Persist-gate probe P4: the inner LIMIT is a single-valued then-set value
// (`t` is 8 before the loop and reassigned 8 every iteration). Cleanup
// folds it to a constant before unrolling, so the gate sees a const bound,
// ALLOWS the outer unroll, and the nest cascades fully (no loop-back
// jumps survive in the emitted asm). Pins: gate silence + full unroll
// (see check_unroll_gate_verdicts.sh) + runtime bit-exactness vs GCC
// (this file, via run_regression.py).
#include <stdio.h>
int folded_limit(void) {
    int s = 0;
    int t = 8;
    for (int i = 0; i < 2; i++) {
        t = 8;
        for (int j = 0; j < t; j++) s += j;
    }
    return s;
}
int main(void) { printf("%d\n", folded_limit()); return 0; }
