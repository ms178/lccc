/* Loop rotation correctness on every loop shape.
 *
 * History: v16 rotated every loop by default and introduced 16
 * miscompiles; v17 made the pass opt-in. The pass has since been rewritten
 * and is on by default again, but only for the loops out of which
 * something then moves (RotatePolicy::Profit). The companion .env file
 * sets CCC_LOOP_ROTATE=1 (RotatePolicy::Always) so that every shape below
 * that can be rotated is: without it the default leaves most of them
 * alone and the test would pass without exercising the transformation.
 *
 * Every shape below rotates under RotatePolicy::Always (the pass before
 * the rewrite refused 2's outer loop and 3 to 5), and the output has to be
 * bit-identical to GCC's:
 *  1. Canonical counted loop with an accumulator. The merge behind the
 *     loop must read the accumulator of the last trip, and the test at
 *     the bottom must terminate.
 *  2. Nested counted loops: both rotate, and the inner rotation must not
 *     corrupt the outer induction variable.
 *  3. A loop with a `break`: the body has an exit of its own, and every
 *     reader behind the loop gets the value of the edge it was reached
 *     over.
 *  4. A loop with a function call in the body.
 *  5. A loop whose accumulator is read by a branch behind the loop and by
 *     both blocks that branch reaches.
 *
 * Differential vs GCC -O2. Every printed line must match.
 */
#include <stdio.h>

#define N 1000

static int arr[N];
static int arr2[N];

/* Shape 1: canonical counted loop with accumulator. */
static int sum_canonical(int n) {
    int s = 0;
    for (int i = 0; i < n; i++)
        s += arr[i];
    return s;
}

/* Shape 2: nested counted loops. */
static int sum_nested(int n) {
    int s = 0;
    for (int i = 0; i < n; i++) {
        int row_sum = 0;
        for (int j = 0; j < n; j++)
            row_sum += arr[j] * (i + 1);
        s += row_sum;
    }
    return s;
}

/* Shape 3: multi-exit loop with break. */
static int sum_until_negative(int n) {
    int s = 0;
    for (int i = 0; i < n; i++) {
        if (arr[i] < 0)
            break;
        s += arr[i];
    }
    return s;
}

/* Shape 4: loop with a function call in the body. */
static int add_one(int x) { return x + 1; }
static int sum_with_call(int n) {
    int s = 0;
    for (int i = 0; i < n; i++)
        s += add_one(arr[i]);
    return s;
}

/* Shape 5: the accumulator is read behind the loop by a branch and by the
 * blocks on both of its edges. */
static int sum_conditional_use(int n, int threshold) {
    int s = 0;
    for (int i = 0; i < n; i++)
        s += arr[i];
    /* Each of the three readers of `s` sits behind the merge of the
     * guard's edge (zero trips: s == 0) with the loop's (the last sum). */
    if (s > threshold)
        return s * 2;
    return s;
}

int main(void) {
    /* Initialize arrays with a deterministic pattern. */
    for (int i = 0; i < N; i++) {
        arr[i] = (i * 7) % 100;
        arr2[i] = (i * 13) % 50;
    }
    /* Plant a negative to exercise the break path. */
    arr[N / 2] = -1;

    int r1 = sum_canonical(N);
    int r2 = sum_nested(20);
    int r3 = sum_until_negative(N);
    int r4 = sum_with_call(50);
    int r5a = sum_conditional_use(N, 50000);
    int r5b = sum_conditional_use(N, 500000);

    printf("%d %d %d %d %d %d\n", r1, r2, r3, r4, r5a, r5b);
    return 0;
}
