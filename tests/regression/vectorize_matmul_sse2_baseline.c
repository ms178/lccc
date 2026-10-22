/*
 * Regression: the two-wide matmul arm must stay VECTORIZED on the plain
 * x86-64 baseline (SSE2, no FMA3), and its separately rounded lowering must be
 * bit-identical to the scalar loop.
 *
 * Root cause (x86-64 v1 default, 2026-10): the arm's only two-wide lowering
 * was `FmaF64x2Hoisted` (`vfmadd231pd`/`fmla`), so the contraction
 * precondition `fp_contract == fast && -mfma` doubled as a *vectorization*
 * precondition.  Making plain x86-64 the default baseline therefore silently
 * un-vectorized every matmul stencil on a default `-O2` build: N=1024 went
 * 278 ms -> 601 ms while GCC 16.2 ran it in 361 ms, and the only diagnostic
 * was a `[VEC] Skip matmul arm` line behind `LCCC_DEBUG_VECTORIZE`.
 *
 * What is pinned here, for tail lengths 0..3 and for both constant and
 * run-time trip counts (the group step is four doubles, so 16/17/20/23 cover
 * the remainder path and 7 covers the refusal path):
 *
 *   * every cell equals the value the scalar multiply-then-add definition
 *     produces.  The reference goes through a `volatile` accumulator, so it
 *     cannot itself be vectorized, contracted or reassociated;
 *   * on a build with no FMA3 (`__x86_64__ && !__FMA__`, i.e. exactly the
 *     default baseline) the answer must be the two-ROUNDING one.  A fused
 *     lowering is observable in this data: (1+2^-27)*(1-2^-27) rounds to 1
 *     before the add, so the cell stays +0.0, while an FMA keeps the exact
 *     product and the cell becomes -2^-54;
 *   * on an FMA-capable build either rounding is accepted, but it must be one
 *     of the two -- a wrong lane map, stride, group step or remainder start
 *     lands on a value that is neither.
 *
 * The emitted-instruction half of the contract (default baseline: `mulpd` +
 * `addpd`, no `vfmadd`, no `ymm`; `-march=x86-64-v3`: fused `vfmadd231pd`)
 * lives in `check_matmul_sse2_fallback.sh`; that is a codegen property, not a
 * numerical one.
 */
#include <stdint.h>
#include <stdio.h>

#define MAXN 40

static double A[MAXN][MAXN], B[MAXN][MAXN], C[MAXN][MAXN];

/* Is the single-rounding answer reachable on this target?  On the x86-64
 * baseline it is not, and a fused result would then be a defect. */
#if defined(__x86_64__) && !defined(__FMA__)
#define HAVE_FUSED_REFERENCE 0
#else
#define HAVE_FUSED_REFERENCE 1
#endif

/* Every cell starts at -1.0 (see `reset`).  The reference takes that seed
 * explicitly instead of reading `C`, which the kernel under test has already
 * overwritten. */
#define CELL_SEED (-1.0)

/* Scalar, two-rounding reference for one cell.  `volatile` keeps the compiler
 * from contracting, vectorizing or reassociating this loop. */
static double ref_cell(int i, int k, int n)
{
    volatile double acc = CELL_SEED;
    for (int j = 0; j < n; j++) {
        volatile double prod = A[i][j] * B[j][k];
        acc = acc + prod;
    }
    return acc;
}

#if HAVE_FUSED_REFERENCE
/* Single-rounding reference for the same cell. */
static double fused_cell(int i, int k, int n)
{
    double acc = CELL_SEED;
    for (int j = 0; j < n; j++)
        acc = __builtin_fma(A[i][j], B[j][k], acc);
    return acc;
}
#endif

static uint64_t bits(double d)
{
    union { double d; uint64_t u; } v;
    v.d = d;
    return v.u;
}

/* The shape the arm recognizes: three nested loops, `C[i][k] += ...` with the
 * innermost stream contiguous in the last index. */
#define MATMUL_KERNEL(name, bound)                                            \
    __attribute__((noinline)) static void name(void)                          \
    {                                                                         \
        for (int i = 0; i < (bound); i++)                                     \
            for (int j = 0; j < (bound); j++)                                 \
                for (int k = 0; k < (bound); k++)                             \
                    C[i][k] += A[i][j] * B[j][k];                             \
    }

/* 32 is an exact multiple of the four-double group step (no remainder), 17
 * leaves a one-element tail, and 16 is short enough that the j-loop is
 * completely unrolled before the vectorizer sees it -- the shape that stays
 * scalar on every target and must therefore still be correct. */
MATMUL_KERNEL(mm_const32, 32)
MATMUL_KERNEL(mm_const17, 17)
MATMUL_KERNEL(mm_const16, 16)

/* Same shape with a run-time bound: the guarded (dynamic trip count) path. */
__attribute__((noinline)) static void mm_dyn(int n)
{
    for (int i = 0; i < n; i++)
        for (int j = 0; j < n; j++)
            for (int k = 0; k < n; k++)
                C[i][k] += A[i][j] * B[j][k];
}

static void reset(void)
{
    /* Every product is (1+2^-27)*(1-2^-27), so a two-rounding cell stays
     * +0.0 and a fused one becomes -2^-54.  The whole array is filled, so no
     * kernel bound can read a stale zero and the accumulator seeds outside
     * the active bound stay exactly -1.0 as write canaries. */
    for (int i = 0; i < MAXN; i++)
        for (int j = 0; j < MAXN; j++) {
            A[i][j] = 1.0 + 0x1p-27;
            B[i][j] = 1.0 - 0x1p-27;
            C[i][j] = CELL_SEED;
        }
}

/* `bound` is the kernel's own trip count: the cells inside it must hold the
 * scalar answer, and every cell outside it must still hold the seed (a vector
 * body that runs past the bound, or a remainder that restarts at the wrong
 * element, moves them). */
static int verify(const char *what, int bound, uint64_t *hash)
{
    uint64_t h = UINT64_C(1469598103934665603);
    int bad = 0;
    for (int i = 0; i < MAXN; i++)
        for (int k = 0; k < MAXN; k++) {
            uint64_t got = bits(C[i][k]);
            uint64_t want = bits(i < bound && k < bound ? ref_cell(i, k, bound)
                                                       : CELL_SEED);
            int ok = got == want;
#if HAVE_FUSED_REFERENCE
            ok = ok || (i < bound && k < bound && got == bits(fused_cell(i, k, bound)));
#endif
            h = (h ^ got) * UINT64_C(1099511628211);
            if (!ok && bad == 0)
                printf("FAIL %s bound=%d C[%d][%d] = %016llx, want %016llx\n",
                       what, bound, i, k, (unsigned long long)got,
                       (unsigned long long)want);
            bad += !ok;
        }
    *hash = h;
    return bad;
}

int main(void)
{
    static const int ns[] = {7, 8, 12, 15, 16, 17, 20, 23, 31, 32, 36, 39};
    uint64_t total = UINT64_C(1469598103934665603);
    uint64_t h;
    int bad = 0;

    for (unsigned t = 0; t < sizeof(ns) / sizeof(ns[0]); t++) {
        int n = ns[t];

        reset();
        mm_dyn(n);
        bad += verify("dyn", n, &h);
        total = (total ^ h) * UINT64_C(1099511628211);

        reset();
        mm_const32();
        bad += verify("const32", 32, &h);
        total = (total ^ h) * UINT64_C(1099511628211);

        reset();
        mm_const17();
        bad += verify("const17", 17, &h);
        total = (total ^ h) * UINT64_C(1099511628211);

        reset();
        mm_const16();
        bad += verify("const16", 16, &h);
        total = (total ^ h) * UINT64_C(1099511628211);
    }

    if (bad) {
        printf("%d mismatch(es)\n", bad);
        return 1;
    }
    printf("%016llx\n", (unsigned long long)total);
    printf("OK\n");
    return 0;
}
