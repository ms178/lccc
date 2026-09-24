// Persist-gate probe P3: the inner LIMIT is decided per-outer-iteration by
// the outer IV (`i == 0 ? 8 : 4`). Per-clone the bound IS a constant, but
// no pass folds selects between unroll rounds, so the substituted bound
// never resolves, the inner persists, and the gate must VETO the outer
// unroll (arm=dynamic-bound). Allowing here would clone a surviving loop.
// Pins: veto-trace + rolled nest (see check_unroll_gate_verdicts.sh) +
// runtime bit-exactness vs GCC (this file, via run_regression.py).
#include <stdio.h>
int iv_decided_limit(void) {
    int s = 0;
    for (int i = 0; i < 2; i++) {
        int lim = (i == 0) ? 8 : 4;
        for (int j = 0; j < lim; j++) s += j;
    }
    return s;
}
int main(void) { printf("%d\n", iv_decided_limit()); return 0; }
