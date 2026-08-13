// regr_v11_costaware_devirt.c
//
// v11 red-team regression: devirtualization must be COST-AWARE. Promoting an
// effectively SINGLE-TARGET indirect call (top target share >= LCCC_PGO_PROMOTE_STABLE,
// default 95%) is a net REGRESSION: the hardware indirect-branch predictor (BTB)
// already predicts a stable single target perfectly, so the guarded
// `cmp fp,target; jne cold; call target; cold: call *fp` transform only adds a
// compare + branch to every call with no accuracy benefit (measured: op_dispatch
// 38.9ms -> 49.9ms, +28%).
//
// This kernel uses a SINGLE, loop-invariant function pointer, so devirtualization
// must NOT fire (otherwise we'd inject overhead into the hot loop). The
// self-checking reference verifies correctness; run_pgo_roundtrip.sh verifies
// that a MULTI-VALUED site (regr_v7) still promotes.
#include <stdio.h>

__attribute__((noinline)) static int op_xor(int x) { return x ^ 7; }
__attribute__((noinline)) static int op_add(int x) { return x + 7; }

__attribute__((noinline)) static int run(int n, int (*f)(int)) {
    int s = 0;
    for (int i = 0; i < n; i++) s += f(i & 255);   /* single, stable target */
    return s;
}

int main(int argc, char** argv) {
    (void)argv;
    int (*f)(int) = op_xor;   /* loop-invariant single target */
    long s = 0;
    for (int k = 0; k < 3000; k++) s += run(2000, f);
    long e = 0;
    for (int k = 0; k < 3000; k++)
        for (int i = 0; i < 2000; i++) e += ((i & 255) ^ 7);
    if (s != e) { printf("MISMATCH s=%ld e=%ld\n", s, e); return 1; }
    printf("ok %ld\n", s);
    return 0;
}
