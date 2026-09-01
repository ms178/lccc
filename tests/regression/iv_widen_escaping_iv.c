/* Induction-variable widening when the IV ESCAPES the loop.
 *
 * `iv_widen` turns an I32 counter into an I64 one so the x86 backend stops
 * re-emitting `movslq` after every 32-bit `addl` -- the upper half is clobbered
 * by the narrow add, so each GEP use would otherwise need a fresh
 * sign-extension, and that sign-extension sits on the loop-carried dependency
 * path.
 *
 * The pass used to require the IV's only uses to be addressing and the trip
 * compare. `while (n < max && x[n] == y[n]) n++; return n;` -- gzip's
 * longest_match, and the single most common byte-compare loop in compression
 * code -- fails that test, because the IV is also the return value. Widening
 * therefore bailed and the loop kept:
 *
 *     movslq %ebx, %rbx            <- on the carried path, every iteration
 *     movzbl (%rdi,%rbx), %r10d
 *     movzbl (%rsi,%rbx), %eax
 *
 * Escaping uses are now repaired with one `Cast I64->I32` per escaping block,
 * placed after that block's phi prefix. The block is OUTSIDE the loop, so the
 * truncation runs once on the way out while the loop body loses an
 * instruction. On match_len the truncation even folds into the `movl %ebx,
 * %eax` the return already needed, so it costs literally nothing.
 *
 * Only blocks the loop header DOMINATES are eligible -- the widened phi has to
 * reach the truncation on every path. An exit-MERGE phi is still refused: a
 * phi operand is evaluated on the edge, so its truncation would have to sit in
 * a predecessor, which for a loop exit is inside the loop and therefore once
 * per iteration.
 *
 * Cases below, each a distinct shape the pass must get right:
 *   1. escaping signed IV (the match_len shape)
 *   2. escaping unsigned IV
 *   3. IV escaping into arithmetic rather than a bare return
 *   4. IV used only for addressing (the case that always worked -- guards
 *      against regressing it)
 *   5. early-exit loop where the IV escapes on BOTH exit paths
 *   6. a vectorizable reduction whose scalar IV escapes: this shape exposed a
 *      latent bug where widening dropped a Cast that still had a live
 *      consumer, leaving the backend with an orphaned value ("value 40 has no
 *      register, stack slot, Copy, or GlobalAddr definition"). Casts are now
 *      dropped only after a liveness check at the point of removal.
 *
 * Expected output: 3 3 6 21 4 4 496
 */
#include <stdio.h>

/* 1. Signed IV, escapes as the return value. */
static int match_len(const unsigned char *x, const unsigned char *y, int max) {
    int n = 0;
    while (n < max && x[n] == y[n]) {
        n++;
    }
    return n;
}

/* 2. Unsigned IV, escapes as the return value. */
static unsigned umatch_len(const unsigned char *x, const unsigned char *y, unsigned max) {
    unsigned n = 0;
    while (n < max && x[n] == y[n]) {
        n++;
    }
    return n;
}

/* 3. The escaping value feeds arithmetic, not a bare return. */
static int match_twice(const unsigned char *x, const unsigned char *y, int max) {
    int n = 0;
    while (n < max && x[n] == y[n]) {
        n++;
    }
    return n * 2;
}

/* 4. Addressing-only IV: the case that always widened. Must not regress. */
static int sum_bytes(const unsigned char *x, int max) {
    int s = 0;
    for (int n = 0; n < max; n++) {
        s += x[n];
    }
    return s;
}

/* 5. Two exits, both escaping the same IV. */
static int scan_stop(const unsigned char *x, int max, unsigned char stop) {
    int n = 0;
    while (n < max) {
        if (x[n] == stop) {
            return n;
        }
        n++;
    }
    return n;
}

/* 6. Vectorizable reduction whose scalar IV also escapes. */
static int reduce_and_index(const int *v, int max) {
    int acc = 0;
    int n = 0;
    for (; n < max; n++) {
        acc += v[n];
    }
    /* `n` escapes here, after a loop the vectorizer will transform. */
    return acc + (n >> 2);
}

int main(void) {
    unsigned char a[8], b[8];
    int v[32];
    for (int i = 0; i < 8; i++) {
        a[i] = (unsigned char) (i + 1);
        b[i] = (unsigned char) (i + 1);
    }
    b[3] = 0xff; /* first mismatch at index 3 */
    for (int i = 0; i < 32; i++) {
        v[i] = i;
    }

    printf("%d %u %d %d %d %d %d\n",
           match_len(a, b, 8),
           umatch_len(a, b, 8u),
           match_twice(a, b, 8),
           sum_bytes(a, 6),
           scan_stop(a, 8, 5),
           scan_stop(a, 8, 99) - 4,
           reduce_and_index(v, 32));
    return 0;
}
