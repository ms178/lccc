/* Integer reduction register homes.
 *
 * The reduction vectorizer keeps loop-carried integer accumulators in YMM/XMM
 * registers (classes 5/6 in collect_x86_reduction_vector_values).  This test
 * pins the three soundness corners of that optimization:
 *
 *   1. NESTED loops: the accumulator must be re-zeroed on every outer
 *      iteration (the preheader copy), not once at function entry.  A
 *      register-homed accumulator that skips the per-pass zero carries the
 *      previous pass's sum (miscompile found for `for(p){ int sa=0,sb=0;
 *      for(i){ sa+=a[i]; sb+=b[i]; } tot+=sa+sb; }`).
 *   2. ODD trip counts: the scalar remainder loop must still contribute.
 *   3. I64x2 reductions stay on the stack-slot path (its multiply has no
 *      pre-AVX-512 SIMD form and is GPR-emulated) and must stay correct.
 */
#include <stdio.h>
#include <stdlib.h>

enum { N = 1 << 15, PASSES = 16 };

static int a[N], b[N], c[N];
static long long la[N], lb[N], lc[N];

int main(void) {
    unsigned seed = 1;
    for (int i = 0; i < N; i++) {
        seed = seed * 1664525u + 1013904223u;
        a[i] = (int)((seed >> 16) & 31u) - 15;
        seed = seed * 1664525u + 1013904223u;
        b[i] = (int)((seed >> 16) & 31u) - 15;
        seed = seed * 1664525u + 1013904223u;
        c[i] = (int)((seed >> 16) & 31u) - 15;
        la[i] = a[i] * 1000LL;
        lb[i] = b[i] * 1000LL;
        lc[i] = c[i] * 1000LL;
    }

    long long total = 0;

    /* 1. Nested double sum (odd and even trip counts). */
    for (int p = 0; p < PASSES; p++) {
        int sa = 0, sb = 0;
        for (int i = 0; i < N - 1; i++) {
            sa += a[i];
            sb += b[i];
        }
        total += sa + sb;
    }

    /* 2. Nested double dot (odd trip count). */
    for (int p = 0; p < PASSES; p++) {
        int s_ab = 0, s_ac = 0;
        for (int i = 0; i < N - 3; i++) {
            s_ab += a[i] * b[i];
            s_ac += a[i] * c[i];
        }
        total += s_ab + s_ac;
    }

    /* 3. I64x2 dot (stack-slot path). */
    for (int p = 0; p < PASSES; p++) {
        long long s = 0;
        for (int i = 0; i < N - 1; i++)
            s += la[i] * lb[i];
        total += s;
    }

    /* 4. Single I32 sum, dynamic-ish bound, odd remainder. */
    for (int p = 0; p < PASSES; p++) {
        int s = 0;
        for (int i = 0; i < N - 5; i++)
            s += c[i];
        total += s;
    }

    /* 5. Three accumulators (multi-reduction beyond two). */
    for (int p = 0; p < PASSES; p++) {
        int s_ab = 0, s_bc = 0, s_ca = 0;
        for (int i = 0; i < N - 1; i++) {
            s_ab += a[i] * b[i];
            s_bc += b[i] * c[i];
            s_ca += c[i] * a[i];
        }
        total += s_ab + s_bc + s_ca;
    }

    /* 6. Shared GLOBAL array across two accumulators: the dedup pass must
     *    emit one load of the shared array per iteration, but correctness is
     *    what this pins (distinct GlobalAddr values naming one symbol). */
    for (int p = 0; p < PASSES; p++) {
        int s_ab = 0, s_ac = 0;
        for (int i = 0; i < N - 1; i++) {
            s_ab += a[i] * b[i];
            s_ac += a[i] * c[i];
        }
        total += s_ab + s_ac;
    }

    printf("%lld\n", total);
    return 0;
}
