/* ============================================================================
 * matmul_chain_exit_phi.c — chained j-loops after k-unroll: exit-phi and
 * ZERO-REM-2 coverage for the FMA (quad FmaF64x4HoistedSIB) vectorizer.
 *
 * Shape. `DEFINE_MM` builds the canonical matmul kernel
 * `C[i][j] += A[i][k] * B[k][j]` with the k-loop small and constant, so
 * lccc's unroller turns it into K SEQUENTIAL j-loops over the same C row.
 * The vectorizer rewrites each j-loop into a quad-FMA packed loop plus a
 * scalar remainder, which re-routes the loop's exit edge through the new
 * remainder header — but the exit block of j-loop N is the HEADER of
 * j-loop N+1 (a chained loop, not a plain post-loop block).  Its IV phi
 * carries the `j = 0` copy for the next loop with an incoming label that
 * names j-loop N's header; after vectorization that edge comes from the
 * remainder header instead.  A missing retarget leaves the phi with a
 * predecessor that no longer reaches it: phi elimination loses the
 * `j = 0` copy and the next j-loop starts from an uninitialised register.
 *
 * Two mechanisms are therefore covered by ONE comparison, because both are
 * exercised by the same chain:
 *
 *   1. ZERO-REM-2.  For N an exact multiple of the quad width (16 doubles)
 *      the scalar remainder provably cannot iterate, and the transform now
 *      omits it entirely — the vectorized header keeps its ORIGINAL exit
 *      edge, so the chained exit phis stay valid without relabelling.
 *      N in {256, 48, 32, 16, 64} take this path.
 *
 *   2. THE RETARGET.  For every other N the remainder loop exists, and the
 *      chained exit phis' loop-label incoming must be relabelled to the
 *      remainder header.  N in {255, 250, 17, 33, 300} take this path.
 *
 * Sentinels.  Each C row is STRIDE wide (MAXN + PAD); the kernel only ever
 * touches columns < N, so the PAD columns and the columns in [N, MAXN) are
 * never written and are compared too.  The reference uses a volatile
 * accumulator row so it can never be vectorized into the same shape it is
 * meant to validate, and the comparison is a bit compare (memcmp) of the
 * whole row: sums of small integer products are exact in binary64, so a
 * correct transform must reproduce the reference BITWISE, and any
 * out-of-bounds packed store shows up as a changed sentinel.
 *
 * Success line: "matmul chain exit-phi: all shapes exact"
 * ==========================================================================*/
#include <stdio.h>
#include <string.h>

#define ROWS 4
#define MAXN 300
#define PAD 16
#define STRIDE (MAXN + PAD)
#define MAXK 8

static double C[ROWS][STRIDE];
static double REF[ROWS][STRIDE];
static double A[ROWS][MAXK];
static double B[MAXK][STRIDE];

#define DEFINE_MM(name, N, K)                                     \
    static void name(void) {                                      \
        for (int i = 0; i < ROWS; i++)                            \
            for (int k = 0; k < (K); k++)                         \
                for (int j = 0; j < (N); j++)                     \
                    C[i][j] += A[i][k] * B[k][j];                 \
    }

/* Exact-multiple shapes: ZERO-REM-2 omits the remainder outright. */
DEFINE_MM(mm_256_4, 256, 4)
DEFINE_MM(mm_48_5, 48, 5)
DEFINE_MM(mm_32_2, 32, 2)
DEFINE_MM(mm_16_4, 16, 4)
DEFINE_MM(mm_64_1, 64, 1)
/* Non-multiple shapes: the remainder exists and the exit phis must be
 * retargeted from the vectorized loop's labels to the remainder header. */
DEFINE_MM(mm_255_4, 255, 4)
DEFINE_MM(mm_250_3, 250, 3)
DEFINE_MM(mm_17_4, 17, 4)
DEFINE_MM(mm_33_4, 33, 4)
DEFINE_MM(mm_300_2, 300, 2)

static void ref_mm(int n, int k) {
    for (int i = 0; i < ROWS; i++) {
        volatile double *row = &REF[i][0];
        for (int kk = 0; kk < k; kk++)
            for (int j = 0; j < n; j++)
                row[j] = row[j] + A[i][kk] * B[kk][j];
    }
}

static int fails = 0;

static void run_shape(const char *label, void (*fn)(void), int n, int k) {
    /* Distinct sentinels per row and column, including the padding: an
     * out-of-bounds packed write cannot land on a matching value. */
    for (int i = 0; i < ROWS; i++)
        for (int j = 0; j < STRIDE; j++) {
            C[i][j] = (double)(1000 + 37 * i + j);
            REF[i][j] = C[i][j];
        }
    fn();
    ref_mm(n, k);
    for (int i = 0; i < ROWS; i++)
        for (int j = 0; j < STRIDE; j++) {
            if (memcmp(&C[i][j], &REF[i][j], sizeof(double)) != 0) {
                printf("FAIL %s: C[%d][%d] = %.17g, expected %.17g\n", label, i, j,
                       C[i][j], REF[i][j]);
                fails++;
                return;
            }
        }
}

int main(void) {
    for (int i = 0; i < ROWS; i++)
        for (int k = 0; k < MAXK; k++)
            A[i][k] = (double)(((i * 31 + k * 7) % 13) - 6);
    for (int k = 0; k < MAXK; k++)
        for (int j = 0; j < STRIDE; j++)
            B[k][j] = (double)(((k * 5 + j * 11) % 11) - 5);

    run_shape("mm_256_4", mm_256_4, 256, 4);
    run_shape("mm_48_5", mm_48_5, 48, 5);
    run_shape("mm_32_2", mm_32_2, 32, 2);
    run_shape("mm_16_4", mm_16_4, 16, 4);
    run_shape("mm_64_1", mm_64_1, 64, 1);
    run_shape("mm_255_4", mm_255_4, 255, 4);
    run_shape("mm_250_3", mm_250_3, 250, 3);
    run_shape("mm_17_4", mm_17_4, 17, 4);
    run_shape("mm_33_4", mm_33_4, 33, 4);
    run_shape("mm_300_2", mm_300_2, 300, 2);

    if (fails == 0) {
        printf("matmul chain exit-phi: all shapes exact\n");
    }
    return fails != 0;
}
