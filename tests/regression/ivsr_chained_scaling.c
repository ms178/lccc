/* IVSR composed-chain collection: `Shl/Mul(Shl/Mul(iv, c1), c2)`.
 *
 * The vectorizer emits a chunked loop's byte offset as a CHAIN — matmul's
 * packed j-loop carries `(iv << 3) << 2` (element index scaled to bytes,
 * then to the 4-lane chunk) — and IVSR can run before the canonicalizer
 * folds it to a single `iv << 5`. The direct collection arms then see only
 * the INNER scaling (stride 8): the group looks SIB-friendly and the x86
 * cost rule skips it, but the COMPOSED stride (32) is not SIB-encodable, so
 * the loop re-derived the offset with a `mov`+`shl` pair every iteration —
 * a 12% matmul loss. IVSR now composes one chain level so the group is
 * judged at its real stride.
 *
 * Soundness pins (these all miscompiled or nearly did during development):
 *   1. Composition must NOT scale the affine pre-offset: for the group
 *      `(iv + k) * s1`, chaining `* f` yields `(iv + k) * (s1 * f)` — the
 *      first cut multiplied `k` by `f` too, double-counting the lane offset
 *      of unrolled affine chains (`stride3` below walked 48 bytes/lane
 *      instead of 12 and silently read the wrong cells).
 *   2. Only same-type chains compose (a cast between the scalings changes
 *      the wrapping ring).
 *   3. Chains nobody dereferences stay plain integer arithmetic.
 *
 * `kernel` reproduces the matmul shape (packed f64, stride-32 chain after
 * vectorization); the scalar loops reproduce the unrolled affine lanes that
 * caught pin 1. Output is compared against the GCC oracle.
 */
#include <stdio.h>

#define N 64

static double A[N][N], B[N][N], C[N][N];

__attribute__((noinline)) static void kernel(void) {
    for (int i = 0; i < N; i++)
        for (int k = 0; k < N; k++) {
            double a = A[i][k];
            for (int j = 0; j < N; j++)
                C[i][j] += a * B[k][j];
        }
}

/* Unrolled affine lanes: the derived groups carry add_offset k != 0 and the
 * byte-scaling Shl composes on top (pin 1). */
__attribute__((noinline)) static long stride3(const int *a, int n) {
    long s = 0;
    for (int i = 0; i < n; i++)
        s += a[i * 3];
    return s;
}

__attribute__((noinline)) static long stride12(const int *a, int n) {
    long s = 0;
    for (int i = 0; i < n; i++)
        s += a[(i * 3) << 2]; /* explicit chain in the source */
    return s;
}

/* Chain whose result is ALSO consumed as plain arithmetic (pin 3: the
 * integer use must keep its exact value when the address use is reduced). */
__attribute__((noinline)) static long mixed_use(const int *a, int n) {
    long s = 0;
    for (int i = 0; i < n; i++) {
        int byte_idx = (i << 1) << 1; /* i * 4 */
        s += a[byte_idx] + byte_idx;
    }
    return s;
}

int main(void) {
    for (int i = 0; i < N; i++)
        for (int j = 0; j < N; j++) {
            A[i][j] = (double)((i * 31 + j * 7) % 13) * 0.5;
            B[i][j] = (double)((i * 17 + j * 3) % 11) * 0.25;
        }
    kernel();
    double chk = 0.0;
    for (int i = 0; i < N; i++)
        for (int j = 0; j < N; j++)
            chk += C[i][j] * (double)((i + 2 * j) % 5);
    static int arr[4 * 300];
    for (int i = 0; i < 4 * 300; i++)
        arr[i] = (i * 17 + 3) % 1000;
    printf("%.4f %ld %ld %ld\n", chk, stride3(arr, 300), stride12(arr, 97),
           mixed_use(arr, 250));
    return 0;
}
