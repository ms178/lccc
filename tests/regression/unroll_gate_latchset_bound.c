// Persist-gate probe P5b: the inner LIMIT reads a value set in the outer
// LATCH (`c` is 0 on entry, 3 from the second iteration on). The bound
// use is NOT dominated by the assignment, so the outer-header phi
// `[0, 3]` survives to the gate (verified: the pre-unroll IR shows the
// inner exit comparing against the B1 phi). Only the header-phi rule
// (S3) allows this nest — blanket phi refusal would veto it — and the
// cascade completes, including the trip-0 first clone. This is the
// end-to-end exercise of the S3 header recursion (unit-pinned by G15).
// Pins: gate silence + full unroll (see check_unroll_gate_verdicts.sh) +
// runtime bit-exactness vs GCC (this file, via run_regression.py).
#include <stdio.h>
int latchset_bound(void) {
    int s = 0, c = 0;
    for (int i = 0; i < 4; i++) {
        for (int j = 0; j < c; j++) s += j;
        c = 3;
    }
    return s;
}
int main(void) { printf("%d\n", latchset_bound()); return 0; }
