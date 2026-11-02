// Persist-gate probe P1: a `goto` decides the inner loop's INIT (`start` is
// 0 on fall-through, 2 via the goto). If-conversion linearizes the goto
// into a Select, so the inner init is genuinely runtime: the inner would
// persist in every clone and the gate must VETO the outer unroll
// (arm=dynamic-bound). Pins: veto-trace (see check_unroll_gate_verdicts.sh)
// + runtime bit-exactness vs GCC (this file, via run_regression.py).
#include <stdio.h>
int goto_bound(int c) {
    int s = 0;
    for (int i = 0; i < 4; i++) {
        int start = 0;
        if (c) { start = 2; goto hdr; }
        s += i;
    hdr:
        for (int j = start; j < 4; j++) s += i + j;
    }
    return s;
}
int main(void) { printf("%d %d\n", goto_bound(0), goto_bound(1)); return 0; }
