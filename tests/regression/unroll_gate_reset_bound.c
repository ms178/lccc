// Persist-gate probe P5a: the inner LIMIT is a reset idiom (`c` is 0 before
// the loop and reassigned 3 each iteration BEFORE the inner loop, so the
// bound use is dominated by the constant assignment). Cleanup folds the
// use to a constant before unrolling (verified: the pre-unroll IR shows
// `Cmp Slt(j, Const(3))`), so the gate allows and the nest cascades fully.
// Pins: gate silence + full unroll (see check_unroll_gate_verdicts.sh) +
// runtime bit-exactness vs GCC (this file, via run_regression.py).
#include <stdio.h>
int reset_bound(void) {
    int s = 0, c = 0;
    for (int i = 0; i < 4; i++) {
        c = 3;
        for (int j = 0; j < c; j++) s += j;
    }
    return s;
}
int main(void) { printf("%d\n", reset_bound()); return 0; }
