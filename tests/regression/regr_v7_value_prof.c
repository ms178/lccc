// regr_v7_value_prof.c
//
// v7 indirect-call value profiling: the instrumented build must run
// correctly (counters + per-site recorders + addr2name registry + exit dump
// must not perturb the program), and the self-checking reference must agree
// with the executed code. The profile-use side is exercised by
// run_pgo_roundtrip.sh (devirtualization must produce identical results).
//
// The switch with duplicate case targets (`case 0: case 1:`) also exercises
// the v7 CFG-edge deduplication: the two edges to one block must produce ONE
// counter slot, not a shared/double-counted pair.
#include <stdio.h>

__attribute__((noinline)) static int add1(int x) { return x + 1; }
__attribute__((noinline)) static int add2(int x) { return x + 2; }
__attribute__((noinline)) static int sub1(int x) { return x - 1; }

__attribute__((noinline)) static int work(int n, int (*f)(int), int mode) {
    int s = 0;
    for (int i = 0; i < n; i++) {
        s += f(i & 127);
        switch (mode & 3) {
        case 0: case 1: s += i & 1; break;   /* duplicate-case switch */
        case 2: s += 2; break;
        default: s -= 1; break;
        }
    }
    return s;
}

int main(int argc, char** argv) {
    int (*f)(int) = (argc > 1) ? add1 : add2;
    int r1 = work(10000, f, 0);
    int r2 = work(10000, f, 2);
    long e = 0;
    for (int i = 0; i < 10000; i++) { e += (f)(i & 127); e += i & 1; }
    for (int i = 0; i < 10000; i++) { e += (f)(i & 127); e += 2; }
    if ((long)r1 + r2 != e) {
        printf("MISMATCH r1=%d r2=%d e=%ld\n", r1, r2, e);
        return 1;
    }
    /* exercise a second target so the recorder sees a mixed site */
    int r3 = work(5000, sub1, 3);
    long e3 = 0;
    for (int i = 0; i < 5000; i++) { e3 += (sub1)(i & 127); e3 -= 1; }
    if ((long)r3 != e3) {
        printf("MISMATCH r3=%d e3=%ld\n", r3, e3);
        return 1;
    }
    printf("ok %d %d %d\n", r1, r2, r3);
    return 0;
}
