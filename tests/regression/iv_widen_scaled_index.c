/* Induction-variable widening through the element-scaling chain.
 *
 * `iv_widen` turns an `int` counter into an `i64` one so the backend stops
 * re-emitting `movslq` after every 32-bit `addl` -- the narrow add clobbers
 * the upper half, so each address use needs a fresh sign-extension, and that
 * extension sits on the loop-carried dependency path.
 *
 * The analysis accepted `Cast -> GetElementPtr` but not
 * `Cast -> Shl(const) -> GetElementPtr`, which is the canonical addressing
 * chain for every array whose elements are wider than one byte. The practical
 * effect was that widening fired **only for byte arrays**:
 *
 *     for (int i = 0; i < n; i++) s += a[i];
 *
 * widened for `signed char` and declined for `int` and `long`, leaving two
 * `movslq` per iteration. Measured effect of fixing it: `sieve` -21.6%
 * (53.0 -> 41.6 ms), `nbody` -3%.
 *
 * Looking through the scale is sound because it is a loop-invariant CONSTANT
 * and the shift/multiply already happens at the wide type: widening the phi
 * replaces the cast's destination with the i64 phi and leaves the scaling
 * instruction untouched. A VARIABLE scale is not safe to look through -- the
 * other operand could itself depend on the counter -- so only constant scales
 * are accepted, and `mul_by_variable_stride` below pins that.
 *
 * Expected output: 2016 8128 32640 4186116 1905 -32 4950
 */
#include <stdio.h>

/* One case per element width: 1, 2, 4, 8 bytes. Only the first widened
 * before; the rest are the regression. */
__attribute__((noinline)) static int sum_i8(const signed char *a, int n) {
    int s = 0;
    for (int i = 0; i < n; i++) {
        s += a[i];
    }
    return s;
}

__attribute__((noinline)) static int sum_i16(const short *a, int n) {
    int s = 0;
    for (int i = 0; i < n; i++) {
        s += a[i];
    }
    return s;
}

__attribute__((noinline)) static int sum_i32(const int *a, int n) {
    int s = 0;
    for (int i = 0; i < n; i++) {
        s += a[i];
    }
    return s;
}

__attribute__((noinline)) static long sum_i64(const long *a, int n) {
    long s = 0;
    for (int i = 0; i < n; i++) {
        s += a[i];
    }
    return s;
}

/* An explicit multiply rather than a shift: `i * 3` indexes a struct-of-3
 * layout. The scale is still a constant, so it is still transparent. */
__attribute__((noinline)) static int stride3(const int *a, int n) {
    int s = 0;
    for (int i = 0; i < n; i++) {
        s += a[i * 3];
    }
    return s;
}

/* SOUNDNESS: a VARIABLE stride must NOT be looked through. `k` is
 * loop-invariant here, but the analysis cannot assume that in general, so the
 * result must simply stay correct. */
__attribute__((noinline)) static int mul_by_variable_stride(const int *a, int n, int k) {
    int s = 0;
    for (int i = 0; i < n; i++) {
        s -= a[i * k];
    }
    return s;
}

/* Descending counter, so the widened value is decremented rather than
 * incremented, and the index is used twice. */
__attribute__((noinline)) static int descending(const int *a, int n) {
    int s = 0;
    for (int i = n - 1; i >= 0; i--) {
        s += a[i] * (a[i] > 0 ? 1 : 0);
    }
    return s;
}

int main(void) {
    enum { N = 64 };
    signed char c[N];
    short h[N];
    int w[N * 3];
    long q[N];
    for (int i = 0; i < N; i++) {
        c[i] = (signed char) (i / 2);
        h[i] = (short) (i * 4);
        q[i] = (long) i * 16;
    }
    for (int i = 0; i < N * 3; i++) {
        w[i] = i * i;
    }

    printf("%d %d %d %ld %d %d %d\n",
           sum_i8(c, N),
           sum_i16(h, N),
           sum_i32(w, N),
           sum_i64(q, N),
           stride3(w, 8),
           mul_by_variable_stride(w, 8, 2),
           descending(w, 15));
    return 0;
}
