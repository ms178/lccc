/* IV-widening of UNSIGNED counters with RUNTIME trip bounds and
 * signedness-changing index chains.
 *
 * The C frontend promotes `unsigned i` arithmetic to U32 and zero-extends the
 * index to the 64-bit GEP width (`Cast(U32->I64)`). `iv_widen` may only
 * widen such an IV when the loop provably cannot wrap in U32 — which holds
 * BY CONSTRUCTION for a unit-step `i < n` (Ult) loop with any init and any
 * invariant bound `n`: the body visits `p..n-1` and the latch reaches `n`
 * at most, all < 2^32, so the wide recurrence matches the narrow one on
 * every executed iteration. `prove_counted_bound` admits exactly this
 * shape, and the cross-signed `U32->I64` cast is retained as a same-size
 * bitcast of the widened U64 member so the index chain (`>>`, `&`, `+`)
 * widens to 64-bit arithmetic (no per-iteration `movslq`/re-extension).
 *
 * Every function below is differential-tested against GCC by the harness.
 *
 * Ported from the Agent B g4 battery; the CrossCast member kind (the
 * signedness-changing U32->I64 cast retained as a same-size reinterpretation
 * of the widened member, with the chain below it enqueued) is what unlocks
 * these shapes — see MemberKind::CrossCast in src/passes/iv_widen.rs.
 */
#include <stdio.h>

/* `unsigned i; for (; i < n; i++) s += a[i >> 1]` — the shape whose
 * U32->I64 zext chain used to stop widening dead (g4 battery). */
__attribute__((noinline)) static long shift_half(const int *a, unsigned n) {
    long s = 0;
    for (unsigned i = 0; i < n; i++) {
        s += a[i >> 1];
    }
    return s;
}

/* Masked index: `a[i & 7]`, plus an offset `a[(i >> 2) & 3] + 1`. */
__attribute__((noinline)) static long masked(const int *a, unsigned n) {
    long s = 0;
    for (unsigned i = 0; i < n; i++) {
        s += a[(i & 7)] + a[((i >> 2) & 3) + 1];
    }
    return s;
}

/* Plain unsigned counter, offset index `a[i + 1]` (needs i+1 in bounds:
 * caller guarantees n < N-1). */
__attribute__((noinline)) static long offset(const int *a, unsigned n) {
    long s = 0;
    for (unsigned i = 0; i < n; i++) {
        s += a[i + 1];
    }
    return s;
}

/* Non-zero start: `for (i = 3; i < n; i++)`. */
__attribute__((noinline)) static long nonzero_start(const int *a, unsigned n) {
    long s = 0;
    for (unsigned i = 3; i < n; i++) {
        s += a[i >> 2];
    }
    return s;
}

/* Decrementing unsigned counter `for (i = n; i > 0; i--) s += a[i - 1]`. */
__attribute__((noinline)) static long down(const int *a, unsigned n) {
    long s = 0;
    for (unsigned i = n; i > 0; i--) {
        s += a[i - 1];
    }
    return s;
}

int main(void) {
    enum { N = 1200 };
    static int a[N];
    for (int i = 0; i < N; i++) {
        a[i] = (i * 2654435761u) % 100000 - 50000;
    }
    long h = 0;
    for (unsigned n = 0; n <= 600; n++) {
        h = h * 31 + shift_half(a, n);
        h = h * 31 + masked(a, n);
        h = h * 31 + offset(a, n);
        h = h * 31 + nonzero_start(a, n);
        h = h * 31 + down(a, n);
    }
    printf("%ld\n", h);
    return 0;
}
