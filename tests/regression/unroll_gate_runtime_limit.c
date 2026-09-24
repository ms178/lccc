// Persist-gate probe P2: the inner LIMIT is runtime (`lim` is 8 or 4
// depending on the parameter). The inner trip is not knowable at compile
// time, so it would persist in every clone and the gate must VETO the
// outer unroll (arm=dynamic-bound). Pins: veto-trace + rolled inner (see
// check_unroll_gate_verdicts.sh) + runtime bit-exactness vs GCC (this
// file, via run_regression.py).
#include <stdio.h>
int runtime_limit(int c) {
    int s = 0;
    int lim = c ? 8 : 4;
    for (int i = 0; i < 2; i++)
        for (int j = 0; j < lim; j++) s += j;
    return s;
}
int main(void) { printf("%d %d\n", runtime_limit(0), runtime_limit(1)); return 0; }
