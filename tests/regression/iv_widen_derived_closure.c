/* Induction-variable widening of the counter's DERIVED CLOSURE.
 *
 * `iv_widen` widens an I32 counter to I64 so the backend stops re-emitting
 * `movslq` after every 32-bit `addl`. The original pass only admitted a bare
 * `Cast(I32->I64)->GEP` use, so every counter with *derived* addressing
 * values -- `s[i-1]`, `s[(i&7)+1]`, `s[i*3]` -- kept a per-iteration
 * `movslq`, one of them on the loop-carried dependency path.
 *
 * This pass now widens the counter's whole derived closure: every value
 * reachable from the phi through a width-preserving op (`Add`/`Sub`/`And`/
 * `Or`/`Xor`/`Shl`/`Mul` with a constant, plus transparent `Copy`s) is
 * retyped to I64 with the phi.
 *
 * SOUNDNESS is not a probabilistic argument. `ext(a op c) == ext(a) op_ext c`
 * (ext = sext for I32 / zext for U32) must hold on every defined execution:
 *   * SIGNED counter: signed overflow is UB, so the narrow op cannot wrap on
 *     any defined execution — `Add`/`Sub`/`SubConst`/`Mul`/`Shl`/`AShr`
 *     admit unconditionally (no range proof needed, and a runtime trip bound
 *     is fine).
 *   * UNSIGNED counter: unsigned ops wrap DEFINED, so `Add`/`Sub`/`Mul`/`Shl`
 *     need a proven parent range whose image stays in [0, u32::MAX] (from the
 *     trip bound); `And`/`Or`/`Xor`/`LShr` with a constant commute with zext
 *     unconditionally and always admit.
 * Every member records which closure value it derives from (explicit
 * provenance) and that provenance is debug_assert-ed; the previous design
 * admitted ops whose operand was never in the closure and shipped a
 * sqlite_varint miscompile.
 *
 * Expected output (GCC oracle): 435130 68 5641461920691080488
 */
#include <stdio.h>

__attribute__((noinline)) static long stencil(int *s, long p, long *acc) {
    for (int i = 2; i <= 64; i++) {
        s[i] = s[i - 1] + (int)p;
        *acc ^= s[(i & 7) + 1];
    }
    return *acc;
}

__attribute__((noinline)) static long mask_off(const int *a, int n) {
    long s = 0;
    for (int i = 0; i < n; i++) {
        s += a[(i & 15) + 3];
    }
    return s;
}

__attribute__((noinline)) static long stride3(const int *a, int n) {
    long s = 0;
    for (int i = 0; i < n; i++) {
        s += a[i * 3];
    }
    return s;
}

__attribute__((noinline)) static long countdown(const int *a, int n) {
    long s = 0;
    for (int i = n - 1; i >= 1; i--) {
        s += a[i - 1];
    }
    return s;
}

__attribute__((noinline)) static unsigned umask(const unsigned *a, unsigned n) {
    unsigned s = 0;
    for (unsigned i = 0; i < n; i++) {
        s += a[(i & 7) + 1];
    }
    return s;
}

__attribute__((noinline)) static long shift2(const int *a, int n) {
    long s = 0;
    for (int i = 0; i < n; i++) {
        s += a[i << 1];
    }
    return s;
}

/* Soundness pin: a RUNTIME trip bound is fine for the SIGNED counter
 * (`i + 1` is admissible via the signed-overflow-is-UB argument), and the
 * bitwise `(i & 7) + 1` is admissible regardless. The function must simply
 * stay correct. */
__attribute__((noinline)) static long runtime_bound(const int *a, int n) {
    long s = 0;
    for (int i = 0; i < n; i++) {
        s += a[i + 1] + a[(i & 7) + 1];
    }
    return s;
}

int main(void) {
    enum { N = 300 };
    static int a[N];
    static unsigned u[N];
    long acc = 0x99;
    for (int i = 0; i < N; i++) {
        a[i] = (i * 17 + 3) % 1000;
        u[i] = (unsigned)(i * 13 + 7) % 1000;
    }
    long r = 0;
    r += stencil(a, 7, &acc);
    r += mask_off(a, N);
    r += stride3(a, 100);
    r += countdown(a, N);
    r += umask(u, N);
    r += shift2(a, 150);
    r += runtime_bound(a, N - 1);
    long ck = 0;
    for (int i = 0; i < N; i++) {
        ck = ck * 31 + a[i];
        ck ^= u[i];
    }
    printf("%ld %ld %ld\n", r, acc, ck);
    return 0;
}
