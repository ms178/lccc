/* expr_sink load-barrier regression: vector store-class intrinsics
 * (Intrinsic { dest_ptr: Some(..) }, e.g. VecStoreF64x4 from the
 * vectorizer) write memory, so a non-volatile load must NEVER be sunk
 * across them.
 *
 * Root cause (2026-09-07 red-team audit): passes/expr_sink.rs kept a
 * hand-copied memory-barrier list that predated the SIMD intrinsics.  The
 * vectorizer (phase 2b, EARLY in the pipeline) lowers the store side of
 * read-modify-write loops like `g[i] = g[i]*2 + 0.25` into
 * `Intrinsic { dest_ptr }` instructions; those were absent from the list.
 *
 * Reachability note (empirical, 2026-09-07): every currently constructible
 * vectorized-loop shape keeps a scalar Store in the region a sunk load
 * would traverse (the versioning/remainder twin), and a scalar Store was
 * already in the historical barrier list — so the hole was masked by
 * accident, not closed by design.  The fix delegates to the canonical
 * Instruction::may_write_memory() predicate (ir/instruction.rs), an
 * exhaustive wildcard-free match: the hole is now closed STRUCTURALLY —
 * a future vectorizer path that drops the scalar twin (exact-width
 * unversioned loops, SLP store packing) cannot reopen it silently, and
 * adding any new memory-writing opcode fails to compile until it is
 * classified there.  The same predicate closed the sibling gaps in LICM
 * (AtomicInc, VaArgStruct, InitTrampoline, NonlocalGotoSave) and
 * de-duplicated the drift class at its root.
 *
 * Shape notes (each detail keeps the test meaningful):
 * - The load's pointer `p` is used again after the load, so the sink's
 *   profitability guard ("no operand's live range may be extended")
 *   admits the move — a single-use pointer would reject it.
 * - The use of `t` sits behind a conditional (less frequent than the
 *   entry block), the pass's favorite direction.
 * - The loops are read-modify-write on `g` (no constant-foldable store
 *   side), so they vectorize into VecStore intrinsics.
 * - GCC is the oracle (stdout + exit code). */
#include <stdio.h>

double g[8] = {42.5, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0};

static void reset_g(void) {
    g[0] = 42.5; g[1] = 1.0; g[2] = 2.0; g[3] = 3.0;
    g[4] = 4.0; g[5] = 5.0; g[6] = 6.0; g[7] = 7.0;
}

__attribute__((noinline)) int read_before_overwrite(int cond) {
    double *p = &g[0];
    double t = *p;                                   /* pre-write load */
    for (int i = 0; i < 8; i++) g[i] = g[i] * 2.0 + 0.25; /* vectorized */
    if (cond) return (int)t + (int)(p == &g[0]);     /* p keeps the move profitable */
    return -1;
}

/* Two vectorized loops between load and use: the sink must refuse to
 * cross BOTH VecStore regions. */
__attribute__((noinline)) int read_before_two_overwrites(int cond) {
    double *p = &g[2];
    double t = *p;
    for (int i = 0; i < 8; i++) g[i] = g[i] * 2.0 + 0.25;
    for (int i = 0; i < 8; i++) g[i] = g[i] * 3.0 - 0.5;
    if (cond) return (int)(t + (double)(p != NULL));
    return -1;
}

int main(void) {
    int a = read_before_overwrite(1);        /* 42 + 1 = 43 (pre-write g[0]) */
    double post = g[0];
    reset_g();
    int b = read_before_overwrite(0);        /* -1 */
    reset_g();
    int c = read_before_two_overwrites(1);   /* 2 + 1 = 3 (pre-write g[2]) */
    double post2 = g[2];
    reset_g();
    int d = read_before_two_overwrites(0);   /* -1 */
    printf("%d %g %d %g %d\n", a, post, c, post2, d);
    return !(a == 43 && b == -1 && c == 3 && d == -1);
}
