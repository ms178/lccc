/*
 * LOOP-PREHEADER-1 shape fixture: which loops get a dedicated preheader, and
 * what that unlocks.
 *
 * A dedicated preheader is a block whose only successor is the loop header.
 * LICM refuses to hoist a possibly-faulting derived-pointer load unless the
 * preheader is dedicated, because hoisting into a block that also leaves the
 * loop would execute the load on a path that never entered it.  LCCC used to
 * never *create* one, so that gate refused almost everything.
 *
 * Three shapes, and each is pinned by `check_loop_preheader.sh`:
 *
 *   dw_hoist      A do-while.  The frontend lowers this with the load in the
 *                 loop HEADER, which is the only shape LICM's must-execute
 *                 rule can hoist at all — so it is the shape where a dedicated
 *                 preheader actually unlocks something.  `c[0]` is loop
 *                 invariant; after the pass it is read once, outside the loop.
 *
 *   sqlite_shape  The documented miscompile this gate exists to prevent.  The
 *                 `if (p == 0) return 0;` block IS the loop's only outside
 *                 predecessor, so it is the preheader — and it also branches
 *                 to the early return.  Splicing a dedicated preheader below
 *                 the guard is what makes hoisting `p->v` legal: the new block
 *                 runs only when the loop is entered, i.e. only when the guard
 *                 already passed.  `p->v` must stay hoisted and `p->nUsed`
 *                 must stay behind the NULL check.
 *
 *   counted_no_unlock  The shape the pass's own module documentation used to
 *                 claim as its motivation.  Measured: it does NOT get a
 *                 preheader, because the load sits in the loop BODY and LICM's
 *                 must-execute rule (the load's block must dominate every loop
 *                 block) refuses it whatever the CFG looks like.  Recorded
 *                 here so the next reader does not re-derive that surprise,
 *                 and so a future relaxation of the must-execute rule has a
 *                 fixture waiting for it.
 *
 * This copy has NO `main` and lives in a subdirectory on purpose: the assembly
 * assertions in `check_loop_preheader.sh` need functions the optimiser has not
 * specialised, and a `main` that calls `dw_hoist(c, 5)` turns the kernel into a
 * constant-trip-count clone (measured: the loop gets peeled and the shape the
 * gate asserts disappears).  A root-level `.c` would also be picked up by
 * `run_regression.py`'s non-recursive `*.c` glob, which requires every file it
 * finds to be a complete, linkable TU.  The runtime half of this fixture —
 * the same three kernels plus a self-checking `main` — is
 * `tests/regression/loop_preheader_shapes.c`.
 */

int dw_hoist(const int *c, int n) {
    int t = 0, i = 0;
    do {
        t += c[0];        /* invariant: read once after the pass */
        i++;
    } while (i < n);
    return t;
}

struct Json {
    int nUsed;
    int v;
};

int sqlite_shape(struct Json *p) {
    if (p == 0) return 0;             /* the guard is the original preheader */
    int t = 0;
    for (int i = 0; i < p->nUsed; i++)
        t += p->v;                    /* invariant, but only after the guard */
    return t;
}

int counted_no_unlock(const int *c, int n) {
    int t = 0;
    for (int i = 0; i < n; i++)
        t += c[0];                    /* body-block load: must-execute refuses */
    return t;
}
