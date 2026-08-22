// Deterministic two-accumulator reduction benchmark (statistical-moments /
// complex-correlation shape).
//
// Each hot loop carries TWO independent accumulators over the same pass — the
// multi-reduction vectorizer shape.  Element values are bounded so the 32-bit
// accumulators cannot overflow (signed overflow is UB).  The sequence is fixed
// so every compiler must produce identical output; a checksum mismatch is a
// miscompile.
#include <stdint.h>
#include <stdio.h>

enum { N = 1 << 20, PASSES = 128 };

static int a[N], b[N], c[N];

int main(void) {
    uint32_t seed = 1;
    for (int i = 0; i < N; i++) {
        seed = seed * 1664525u + 1013904223u;
        a[i] = (int)((seed >> 16) & 15u) - 7; /* [-7, 7] */
        seed = seed * 1664525u + 1013904223u;
        b[i] = (int)((seed >> 16) & 15u) - 7;
        seed = seed * 1664525u + 1013904223u;
        c[i] = (int)((seed >> 16) & 15u) - 7;
    }

    int64_t total = 0;
    for (int p = 0; p < PASSES; p++) {
        int s_ab = 0, s_ac = 0;
        for (int i = 0; i < N; i++) {
            s_ab += a[i] * b[i]; /* correlation 1 */
            s_ac += a[i] * c[i]; /* correlation 2 */
        }
        total += s_ab + s_ac;

        int t_a = 0, t_b = 0;
        for (int i = 0; i < N; i++) {
            t_a += a[i]; /* running total 1 */
            t_b += b[i]; /* running total 2 */
        }
        total += t_a + t_b;
    }

    printf("%lld\n", total);
    return 0;
}
