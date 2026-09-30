/* Shapes for tests/regression/check_loop_preheader.sh.
 *
 * This file is compiled TWO ways by the corpus runner and by the gate:
 *   (a) standalone, as a runnable program with `main` (exit 0 == all shapes
 *       agree with their scalar reference), and
 *   (b) with `-S`, one function at a time, so the gate can assert on the
 *       EMITTED ASSEMBLY rather than on runtime output.
 *
 * (b) is the point. A preheader insertion that is structurally valid but
 * hoists nothing is invisible to every Rust-side unit test and to every
 * runtime comparison, because "inserts nothing and is correct" and "inserts
 * a block and is correct" produce identical stdout. Only the assembly
 * distinguishes them, so only the assembly can gate this pass.
 *
 * Self-contained by construction: no external symbols, no headers. The
 * corpus runner globs tests/regression/*.c and compiles each one alone, so
 * anything that needs a second translation unit belongs in a subdirectory.
 */

/* ---------------------------------------------------------------------- */
/* 1. unguarded invariant load -- the shape the backlog specifies.         */
/*    `c[0]` is loop-invariant and provably safe to read (c is never       */
/*    null-checked here, so LICM's own dominance test decides).            */
/* ---------------------------------------------------------------------- */
int invariant_ptr(const int *c, int n) {
    int t = 0;
    for (int i = 0; i < n; i++)
        t += c[0];
    return t;
}

/* ---------------------------------------------------------------------- */
/* 2. the SQLite shape: a NULL guard, then a loop over p->nUsed.           */
/*    This is the shape the pass exists for. The guard makes the load      */
/*    conditionally safe, which is exactly what a dedicated preheader      */
/*    unlocks: the hoisted load can sit after the early return.            */
/* ---------------------------------------------------------------------- */
struct box {
    int nUsed;
    int a[64];
};

int guarded_sum(struct box *p) {
    if (p == 0)
        return 0;
    int s = 0;
    for (int i = 0; i < p->nUsed; i++)
        s += p->a[i];
    return s;
}

/* ---------------------------------------------------------------------- */
/* 3. switch-entered loop -- two edges enter the header from outside.      */
/*    A preheader must NOT be claimed here: with two outside predecessors  */
/*    there is no single block that dominates the header, so hoisting into */
/*    either one would leave the bound undefined on the other path.        */
/* ---------------------------------------------------------------------- */
int switch_entered(int k, int n) {
    int i = 0, s = 0;
    switch (k) {
    case 0:
        goto loop;
    case 1:
        i = 1;
        goto loop;
    default:
        return -1;
    }
loop:
    for (; i < n; i++)
        s += i;
    return s;
}

/* ---------------------------------------------------------------------- */
/* 4. computed goto into the header -- cannot be rerouted at all.          */
/*    `goto *p` produces an IndirectBranch; there is no edge to retarget.  */
/* ---------------------------------------------------------------------- */
int computed_goto(int n, int which) {
    static void *const table[2] = {&&l0, &&l1};
    int s = 0, i = 0;
    goto *table[which & 1];
l0:
    for (; i < n; i++)
        s += i;
    goto done;
l1:
    for (; i < n; i++)
        s += 2 * i;
done:
    return s;
}

/* ---------------------------------------------------------------------- */
/* 5. already-dedicated loop -- idempotence.                               */
/*    The `while` form gives the header exactly one outside predecessor    */
/*    whose terminator is dedicated to it, so the pass must recognise it,  */
/*    decline to insert a second block, and leave the code alone.          */
/* ---------------------------------------------------------------------- */
int already_dedicated(const int *c, int n) {
    int t = 0;
    int i = 0;
    while (i < n) {
        t += c[0];
        i++;
    }
    return t;
}

/* ---------------------------------------------------------------------- */
/* Runtime cross-check: every shape against a scalar reference.            */
/* Exits non-zero on the first disagreement so the corpus runner (which    */
/* only compares stdout/exit status) also catches a semantic regression,   */
/* not just the codegen one the gate asserts on.                           */
/* ---------------------------------------------------------------------- */
int main(void) {
    struct box b;
    int arr[4] = {3, 1, 4, 1};
    int i;

    /* invariant_ptr: c[0] == 7 summed n times */
    int c0[1];
    c0[0] = 7;
    if (invariant_ptr(c0, 5) != 35)
        return 1;

    /* guarded_sum */
    b.nUsed = 4;
    for (i = 0; i < 4; i++)
        b.a[i] = arr[i];
    if (guarded_sum(&b) != 9)
        return 2;
    if (guarded_sum(0) != 0)
        return 3;

    /* switch_entered: k=0 -> i starts 0; k=1 -> i starts 1 */
    if (switch_entered(0, 5) != 0 + 1 + 2 + 3 + 4)
        return 4;
    if (switch_entered(1, 5) != 1 + 2 + 3 + 4)
        return 5;
    if (switch_entered(9, 5) != -1)
        return 6;

    /* computed_goto: which=0 -> sum i; which=1 -> sum 2i */
    if (computed_goto(5, 0) != 10)
        return 7;
    if (computed_goto(5, 1) != 20)
        return 8;

    /* already_dedicated */
    if (already_dedicated(c0, 3) != 21)
        return 9;

    return 0;
}
