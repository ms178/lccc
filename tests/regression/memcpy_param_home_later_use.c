/*
 * Regression: parameters parked in their incoming ABI registers (the
 * x86_param_caller_homes pre-store model) must survive an IR-level Memcpy
 * even when they are READ AGAIN after the copy — no call in between, so
 * only the memcpy's call-point liveness eviction can protect them.
 *
 * Root-cause class (red-team audit of PR #434, 2026-09-07): a Memcpy
 * expands into instruction sequences that clobber the staging registers
 * (%rdi/%rsi/%rcx and friends). The soundness mechanism is
 * `call_spanning` eviction in the register allocator: Memcpy is a call
 * point, so any value live across it is evicted from its register home.
 * The paired test `memcpy_param_home_swap.c` covers params DEAD after the
 * copy; this one pins the complementary shape — the param value is live
 * past the memcpy and must be re-read from its post-eviction home.
 *
 * GCC is the oracle (stdout + exit code).
 */
#include <stdio.h>
#include <string.h>

struct B { int b[4]; };

/* src∈rdi, dst∈rsi, both read again AFTER the copy. */
__attribute__((noinline)) long read_after_copy(struct B *src, struct B *dst) {
    *dst = *src;                          /* IR Memcpy; both params live across */
    return (long)src->b[0] + (long)dst->b[1] + (long)src->b[3] * 2 + (long)dst->b[2] - 3;
}

/* Same shape with the pointer VALUES used after the copy (address
 * arithmetic inside one object, plus a load through the surviving
 * param): the param home must still be intact. */
__attribute__((noinline)) long ptr_after_copy(struct B *src, struct B *dst) {
    *dst = *src;
    struct B *p = src + 1;           /* address arithmetic on the src param */
    struct B *q = dst;               /* dst param must survive the eviction */
    return (long)(p - src) + (long)q->b[0] + (long)(sizeof(*src) / 4);
}

/* Copy, then a second copy, then reads: both evictions must stack. */
__attribute__((noinline)) long two_copies_then_read(struct B *src, struct B *dst, struct B *tmp) {
    *tmp = *src;
    *dst = *tmp;
    return (long)tmp->b[0] + (long)dst->b[0] + (long)src->b[1];
}

/* Nested: the memcpy feeds a loop that reads both params every
 * iteration — maximum liveness pressure across the call point. */
__attribute__((noinline)) long copy_then_loop(struct B *src, struct B *dst, long n) {
    *dst = *src;
    long acc = 0;
    for (long i = 0; i < n; i++)
        acc += (long)src->b[i & 3] * (i + 1) - (long)dst->b[(i + 1) & 3];
    return acc;
}

int main(void) {
    struct B a = {{10, 20, 30, 40}}, b = {{0}}, t = {{0}};
    long r1 = read_after_copy(&a, &b);
    printf("%ld %d %d %d %d\n", r1, b.b[0], b.b[1], b.b[2], b.b[3]);
    long r2 = ptr_after_copy(&a, &b);
    printf("%ld\n", r2);
    long r3 = two_copies_then_read(&a, &b, &t);
    printf("%ld %d %d\n", r3, t.b[1], b.b[2]);
    long r4 = copy_then_loop(&a, &b, 9);
    printf("%ld %d %d %d %d\n", r4, b.b[0], b.b[1], b.b[2], b.b[3]);
    return !(r1 == 137 && b.b[0] == 10 && r2 == 15 && r3 == 40 && r4 == 870);
}
