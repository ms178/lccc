/* loop_preheader_insertion_shapes.c — when LOOP-PREHEADER-1 may insert a block.
 *
 * NOT a standalone regression-corpus case: there is no `main`, and the
 * regression runner (tests/regression/run_regression.py) auto-discovers and
 * builds every `tests/regression/*.c` on its own.  A second file that only
 * makes sense next to this one would therefore be compiled without its
 * partner and fail the corpus with an undefined reference — the exact failure
 * that turned PR #681's Test Suite job red.  Hence the subdirectory, which the
 * runner's non-recursive glob skips.  Only check_loop_preheader.sh builds it.
 *
 * Each function below is a named claim about when a dedicated preheader may
 * be created, and check_loop_preheader.sh asserts the corresponding emitted
 * assembly.
 */

/* 1. THE POSITIVE CASE.  The guard is OUTSIDE the loop, so the loop header is
 *    the body and the derived-pointer load sits in a block that dominates
 *    every loop block -- LICM's must-execute rule is satisfied.  With a
 *    dedicated preheader the load is hoisted; with the pass disabled it stays
 *    in the loop.  `c` is a parameter, so this is a load that can fault. */
int dowhile_sum(const int *c, int n) {
    if (n <= 0) return 0;
    int i = 0, t = 0;
    do { t += c[0]; i++; } while (i < n);
    return t;
}

/* 2. THE REFUSAL.  A guard-at-top loop lowers with the guard as the header, so
 *    the load lives in a block that does NOT dominate every loop block.
 *    Creating a dedicated preheader here would let LICM hoist a possibly
 *    faulting load onto a path that never entered the loop, so the pass
 *    declines and the emitted code is identical with and without it. */
int while_sum(const int *c, int n) {
    int t = 0;
    while (n-- > 0) t += c[0];
    return t;
}

/* 3. THE SOUNDNESS CASE THE PASS'S OWN DOCSTRING CITES.  `if (p == 0) return
 *    0;` makes that block the loop's unique outside predecessor, so it *is*
 *    the preheader -- but it also branches to the early return, so it is not
 *    dedicated.  Hoisting `p[0]` into it would dereference NULL whenever the
 *    guard took the early exit.  This must stay in the loop at every setting. */
int guarded_sum(const char *p, int n) {
    if (p == 0) return 0;
    int t = 0;
    for (int i = 0; i < n; i++) t += p[0];
    return t;
}

/* 4. NOT PROFITABLE.  Every load in this loop reads an alloca.  An alloca
 *    never needs a dedicated preheader (LICM's alloca path is independent of
 *    it), so an inserted empty block would be pure cost -- the regression the
 *    ungated version of this pass measured corpus-wide. */
int alloca_sum(int n) {
    int a[64], t = 0;
    for (int i = 0; i < 64; i++) a[i] = i;
    for (int i = 0; i < n && i < 64; i++) t += a[i];
    return t;
}
