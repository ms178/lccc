/*
 * MINMAX-1 shape census fixture (assembly only: compiled to -S, never linked).
 *
 * Two groups, and BOTH are pinned:
 *
 *   VECTORIZED  min/max over int[] whose accumulator update is a Select after
 *               if-conversion.  Includes the loop starting at 1 (iv_init != 0,
 *               admitted through the marching-pointer form) and a constant
 *               non-zero seed.
 *
 *   REFUSED     every shape the transform does not model: unsigned compares,
 *               16-bit elements, float elements, a `continue` guard, and any
 *               second accumulator in the same loop.  These must stay scalar —
 *               the alternative is the sum == 0 miscompile this gate exists to
 *               prevent (a single-accumulator pattern silently dropping the
 *               loop's other accumulators).
 *
 * `scripts/check_minmax_shapes.py` measures each function's steady-state loop
 * and asserts the group, so a future capability increase has to edit an
 * expectation on purpose instead of drifting past the gate.
 */

/* ---- group 1: must lower to 256-bit packed min / max ------------------- */

int max_i32(const int *a, int n) {
    int m = a[0];
    for (int i = 0; i < n; i++)
        if (a[i] > m) m = a[i];
    return m;
}

int min_i32(const int *a, int n) {
    int m = a[0];
    for (int i = 0; i < n; i++)
        if (a[i] < m) m = a[i];
    return m;
}

/* Operand order swapped: `m < a[i]` instead of `a[i] > m`. */
int max_swapped(const int *a, int n) {
    int m = a[0];
    for (int i = 0; i < n; i++)
        if (m < a[i]) m = a[i];
    return m;
}

/* `<=` / `>=` spellings (equal arms: either value is the same). */
int min_le(const int *a, int n) {
    int m = a[0];
    for (int i = 0; i < n; i++)
        if (m >= a[i]) m = a[i];
    return m;
}

int max_ge(const int *a, int n) {
    int m = a[0];
    for (int i = 0; i < n; i++)
        if (a[i] >= m) m = a[i];
    return m;
}

/* iv_init != 0 (element c = 1): admitted through the marching-pointer form. */
int max_start1(const int *a, int n) {
    int m = a[0];
    for (int i = 1; i < n; i++)
        if (a[i] > m) m = a[i];
    return m;
}

/* Constant non-zero seed (not an element of the array). */
int max_const_init(const int *a, int n) {
    int m = -2147483647;
    for (int i = 0; i < n; i++)
        if (a[i] > m) m = a[i];
    return m;
}

/* ---- group 2: must stay scalar ----------------------------------------- */

/* Unsigned compare: vpmaxsd/vpminsd are signed-only. */
unsigned max_u32(const unsigned *a, int n) {
    unsigned m = a[0];
    for (int i = 0; i < n; i++)
        if (a[i] > m) m = a[i];
    return m;
}

/* 16-bit elements: no packed min/max at that lane width yet. */
short max_i16(const short *a, int n) {
    short m = a[0];
    for (int i = 0; i < n; i++)
        if (a[i] > m) m = a[i];
    return m;
}

short min_i16(const short *a, int n) {
    short m = a[0];
    for (int i = 0; i < n; i++)
        if (a[i] < m) m = a[i];
    return m;
}

/* Float: min/max is not bit-identical under NaN, so FP needs fast-math. */
float min_f32(const float *a, int n) {
    float m = a[0];
    for (int i = 0; i < n; i++)
        if (a[i] < m) m = a[i];
    return m;
}

/* Guarded update: `continue` means not every iteration contributes. */
int max_guarded(const int *a, int n) {
    int m = a[0];
    for (int i = 0; i < n; i++) {
        if (a[i] > 1000) continue;
        if (a[i] > m) m = a[i];
    }
    return m;
}

/* Second accumulator in the same loop (an independent sum). */
int max_with_sum(const int *a, int n, int *sum) {
    int m = a[0];
    int s = 0;
    for (int i = 0; i < n; i++) {
        if (a[i] > m) m = a[i];
        s += a[i];
    }
    *sum = s;
    return m;
}

/* Two extremes in the same loop (min AND max): also a second accumulator. */
int minmax_pair(const int *a, int n, int *mn) {
    int lo = a[0], hi = a[0];
    for (int i = 0; i < n; i++) {
        if (a[i] < lo) lo = a[i];
        if (a[i] > hi) hi = a[i];
    }
    *mn = lo;
    return hi;
}
