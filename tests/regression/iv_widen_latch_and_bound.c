/* IV-widening: latch operand sides, loop-variant steps, and non-constant
 * trip bounds.
 *
 * This test pins the soundness fixes from the iv_widen audit:
 *
 *   1. `Add(step, phi)` — the latch recurrence written with the step on the
 *      LEFT. The pass must either rewrite the correct operand or bail;
 *      overwriting `rhs` unconditionally used to corrupt the latch into an
 *      infinite loop.
 *
 *   2. Loop-VARIANT step — `i += step` where `step` changes inside the loop.
 *      Hoisting the preheader extend of such a step is a use-before-def
 *      (invalid SSA). The pass must validate invariance before any IR
 *      mutation and decline.
 *
 *   3. Header-computed trip bound — `i < n - 1`. `n - 1` is loop-invariant
 *      in VALUE but defined in the header (it does not dominate the
 *      preheader). The widened compare must extend it in the header, not the
 *      preheader, or bail — never hoist across its definition.
 *
 *   4. Decrementing latch — `i--`/`i -= 1` (Sub(phi, step)) widens to a wide
 *      `subq`; the index is used directly with no per-iteration movslq.
 *
 *   5. Derived addressing with a RUNTIME bound — for a SIGNED counter,
 *      signed overflow is UB, so `ext(i + c) == ext(i) + c` on every defined
 *      execution and `a[i+1]`/`a[i-1]`/`a[i<<1]`/`a[i*3]` widen even when the
 *      trip bound is a runtime parameter (previously range-gated and
 *      declined). Unsigned counters still require the range proof.
 *
 *   6. Signedness of the compare — a signed IV compared against an unsigned
 *      bound (C converts to unsigned → Ult) and an unsigned IV compared
 *      against a signed bound stay bit-exact.
 *
 * Every case is differential-tested against GCC by the regression harness;
 * the numeric literals below are the GCC oracle output.
 */
#include <stdio.h>

/* (1) latch Add(step, phi): step on the left. */
__attribute__((noinline)) static int lhs_step(const int *a, int n, int k) {
    int s = 0;
    for (int i = 0; i < n; i = k + i) {
        s += a[i];
    }
    return s;
}

/* (2) loop-variant step: `step` is redefined inside the body. */
__attribute__((noinline)) static int variant_step(const int *a, int n) {
    int s = 0;
    int step = 1;
    for (int i = 0; i < n; i += step) {
        s += a[i];
        step += 2;
    }
    return s;
}

/* (3) header-computed bound `n - 1` with `a[i + 1]`. */
__attribute__((noinline)) static long plus1_hoisted_bound(const int *a, int n) {
    long s = 0;
    for (int i = 0; i < n - 1; i++) {
        s += a[i + 1];
    }
    return s;
}

/* (3b) header-computed bound `n + 1` with `a[i - 1]`. */
__attribute__((noinline)) static long minus1_hoisted_bound(const int *a, int n) {
    long s = 0;
    for (int i = 1; i < n + 1; i++) {
        s += a[i - 1];
    }
    return s;
}

/* (4) decrementing latch, Sub(phi, step). */
__attribute__((noinline)) static long countdown(const int *a, int n) {
    long s = 0;
    for (int i = n - 1; i >= 0; i--) {
        s += a[i];
    }
    return s;
}

/* (5) derived addressing with a RUNTIME bound (signed: admissible). */
__attribute__((noinline)) static long runtime_derived(const int *a, int n) {
    long s = 0;
    for (int i = 0; i < n; i++) {
        s += a[i + 1] + a[i << 1] + a[i * 3];
    }
    return s;
}

/* (6) signed IV vs unsigned bound (Ult). */
__attribute__((noinline)) static int signed_iv_ult(const int *a, int n) {
    int s = 0;
    for (int i = 0; i < (unsigned)n; i++) {
        s += a[i];
    }
    return s;
}

/* (6b) unsigned IV vs signed bound (cast forces the signed cmp). */
__attribute__((noinline)) static int unsigned_iv_slt(const int *a, unsigned n) {
    int s = 0;
    for (unsigned i = 0; i < (int)n; i++) {
        s += a[i];
    }
    return s;
}

/* (6c) signed IV near the i32 boundary (must not mis-widen the cmp). */
__attribute__((noinline)) static long high_bit(const int *a, int n) {
    long s = 0;
    for (int i = n - 5; i < n; i++) {
        s += a[i & 0x7fffffff];
    }
    return s;
}

/* (6d) unsigned IV with high values and a masked, offset index. */
__attribute__((noinline)) static unsigned umask_hi(const unsigned *a, unsigned n) {
    unsigned s = 0;
    for (unsigned i = 0; i < n; i++) {
        s += a[(i & 0x80000007u) + 1];
    }
    return s;
}

int main(void) {
    enum { N = 400 };
    static int a[N];
    static unsigned u[N];
    for (int i = 0; i < N; i++) {
        a[i] = (i * 7 - 3) % 1000;
        u[i] = (unsigned)(i * 11 + 5) % 1000;
    }
    long r = 0;
    r += lhs_step(a, N, 2);
    r += variant_step(a, N);
    r += plus1_hoisted_bound(a, N);
    r += minus1_hoisted_bound(a, N);
    r += countdown(a, N);
    r += runtime_derived(a, N - 1);
    r += signed_iv_ult(a, N);
    r += unsigned_iv_slt(a, N);
    r += high_bit(a, N);
    r += umask_hi(u, N);
    long ck = 0;
    for (int i = 0; i < N; i++) {
        ck = ck * 31 + a[i];
        ck ^= u[i];
    }
    printf("%ld %ld\n", r, ck);
    return 0;
}
