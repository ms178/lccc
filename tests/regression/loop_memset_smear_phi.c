// loop_memset after loop_carried_forward: the forward-overlap smear
// `G[i+1] = G[i]` is rewritten by loop_carried_forward into a store of a
// degenerate header phi `v = φ(G[0] entry, v backedge)`.  loop_memset used
// to accept that phi as the "loop-invariant" fill byte (it is not defined
// in the BODY block) and emitted `memset(G+1, v, n)` after deleting the
// loop — and the phi with it: a use of an undefined value the IR verifier
// reported (`loop_idiom_copy_norewrite` under CCC_VERIFY_IR=1).  The pass
// now sees through the degenerate phi to its entry value (`memset(G+1,
// G[0], n)`, which IS the smear) and refuses every other loop-defined
// fill value.  Differential against GCC; run_regression_suite.sh verifies
// the IR between passes.
#include <stdio.h>

unsigned char G1[512];
unsigned char G2[64];

static void overlap(unsigned n) {
    for (unsigned i = 0; i < n; i++) G1[i + 1] = G1[i];
}

// Same shape through a pointer parameter and a non-zero start.
static void overlap_from(unsigned char *p, unsigned k, unsigned n) {
    for (unsigned i = k; i < n; i++) p[i + 1] = p[i];
}

int main(void) {
    for (int i = 0; i < 512; i++) G1[i] = (unsigned char)(i * 7 + 3);
    overlap(0);
    overlap(1);
    overlap(37);
    overlap(200);
    unsigned long s = 0;
    for (int i = 0; i < 512; i++) s = s * 31 + G1[i];
    printf("smear %lu %u %u %u\n", s, G1[0], G1[100], G1[300]);

    for (int i = 0; i < 64; i++) G2[i] = (unsigned char)(200 - i);
    overlap_from(G2, 5, 40);
    s = 0;
    for (int i = 0; i < 64; i++) s = s * 31 + G2[i];
    printf("smear_from %lu %u %u %u\n", s, G2[4], G2[5], G2[41]);
    return 0;
}
