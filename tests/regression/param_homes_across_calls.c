/* x86-64: parameters keep their incoming ABI registers in functions that
 * call, when they are dead before the call (only a parameter live across a
 * call takes a callee-saved home).  The hazard this pins is argument
 * staging while parameters still sit in %rdi/%rsi/%rdx/%rcx/%r8/%r9: a
 * permutation of the six register parameters into a call must read every
 * parameter before overwriting its register, through direct and indirect
 * (%r10-staged) calls, with parameters read before, as, and after the call.
 * Every expectation is checked in-program; any mismatch exits nonzero. */
#include <stdio.h>

static long tally;

__attribute__((noinline)) long mix6(long a, long b, long c, long d, long e, long f) {
    tally += a;
    return a * 1 + b * 10 + c * 100 + d * 1000 + e * 10000 + f * 100000;
}

long (*volatile mix6p)(long, long, long, long, long, long) = mix6;

/* Reversed permutation: every argument register is both a source and a
 * destination of the staging. */
__attribute__((noinline)) long rev(long a, long b, long c, long d, long e, long f) {
    return mix6(f, e, d, c, b, a);
}

/* Rotation through an indirect call (the target address occupies %r10). */
__attribute__((noinline)) long rot(long a, long b, long c, long d, long e, long f) {
    return mix6p(b, c, d, e, f, a);
}

/* Parameters consumed before the call, one of them also live after it. */
__attribute__((noinline)) long before_after(long *p, long k, long m) {
    long x = p[0] + k;
    long r = mix6(x, m, k, x, m, k);
    return r + m;
}

/* A parameter read only after an earlier call that clobbers its register. */
__attribute__((noinline)) long after_only(long a, long b) {
    long r = mix6(1, 2, 3, 4, 5, 6);
    return r + a * 7 + b;
}

/* Swap of the first two arguments, the minimal two-register cycle. */
__attribute__((noinline)) long swap2(long a, long b) {
    return mix6(b, a, 0, 0, 0, 0);
}

/* Two calls: the second permutes parameters that the first call's staging
 * already overwrote, so they must have been saved. */
__attribute__((noinline)) long two_calls(long a, long b, long c) {
    long r = mix6(c, b, a, 0, 0, 0);
    return r + mix6(a, c, b, 0, 0, 0);
}

int main(void) {
    int bad = 0;
    long v;
    long arr[1] = {40};
    v = rev(1, 2, 3, 4, 5, 6);
    bad |= v != 6 * 1 + 5 * 10 + 4 * 100 + 3 * 1000 + 2 * 10000 + 1 * 100000;
    v = rot(1, 2, 3, 4, 5, 6);
    bad |= (v != 2 + 30 + 400 + 5000 + 60000 + 100000) << 1;
    v = before_after(arr, 2, 3);
    bad |= (v != 42 + 30 + 200 + 42000 + 30000 + 200000 + 3) << 2;
    v = after_only(5, 9);
    bad |= (v != 654321 + 35 + 9) << 3;
    v = swap2(7, 8);
    bad |= (v != 8 + 70) << 4;
    v = two_calls(1, 2, 3);
    bad |= (v != (3 + 20 + 100) + (1 + 30 + 200)) << 5;
    bad |= (tally != 6 + 2 + 42 + 1 + 8 + 3 + 1) << 6;
    printf("%d %ld\n", bad, tally);
    return bad != 0;
}
