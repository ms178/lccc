/* Regression: phi-latch coalescing vs. a folded SIB index peeled through Shl.
 *
 * The loop stores twice with the same `i<<3` offset (`x[i]`, `xs[i]`), and the
 * second store's VALUE consumes `i+1` — the same expression as the IV
 * increment, which CSE merges into ONE value that phi elimination then copies
 * back into `i` at the latch.  The phi-latch coalescer may give `i+1` the IV's
 * register (a destructive in-place `addl $1`) only when nothing reads the OLD
 * `i` after that update.  The `xs[i]` store does: its address is formed at the
 * store as `(%base,%i,8)` because the backend folds `GEP(base, Shl(i,3))` into
 * the SIB operand and re-reads `i` there — a read the IR does not show, since
 * the GEP consumed `Shl(i,3)` and the syntactic closure only followed
 * Cast/GEP, not the Shl.
 *
 * Before the fix: xs[] was shifted by one element (xs[i] = 1/i, xs[0] = 0).
 * Fixed by (a) mirroring the backend's peel set (Shl/Mul/Add/Sub by a
 * constant) in the hidden-read closure and (b) the authoritative segment
 * veto: the phi may not be live at any point inside the update window
 * according to the hole-aware segments, which already carry the folded-index
 * extension.
 */
#include <stdio.h>

static double x[257], xs[257];

__attribute__((noinline)) static void fill(int n) {
    for (int i = 0; i < n; i++) {
        x[i] = (i % 13) * 0.25 - 1.5;
        xs[i] = 1.0 / (i + 1);
    }
}

/* Same shape with the increment displaced into the address chain
 * (`p[i+1]` → SIB displacement peel) and a 4-byte element. */
__attribute__((noinline)) static void fill_disp(int *a, int *b, int n) {
    for (int i = 0; i < n - 1; i++) {
        a[i] = i * 3;
        b[i + 1] = a[i] + (i + 1);
    }
}

/* Scaled-by-multiply index (`Mul` peel) with the IV re-read at the store. */
__attribute__((noinline)) static void fill_mul(short *a, long *b, int n) {
    for (int i = 0; i < n; i++) {
        a[i * 2] = (short)i;
        b[i] = a[i * 2] * (long)(i + 1);
    }
}

int main(void) {
    fill(257);
    unsigned long h = 0;
    for (int i = 0; i < 257; i++) {
        h = h * 1000003u ^ (unsigned long)(x[i] * 4.0 + 8.0);
        h = h * 1000003u ^ (unsigned long)(xs[i] * 1e6);
    }
    printf("fill %.4f %.4f %.4f %.6f %lu\n", x[0], xs[0], xs[1], xs[256], h);

    static int a[64], b[64];
    fill_disp(a, b, 64);
    long s = 0;
    for (int i = 0; i < 64; i++) s += a[i] * 7 + b[i];
    printf("fill_disp %d %d %d %ld\n", a[5], b[1], b[63], s);

    static short sa[128];
    static long lb[64];
    fill_mul(sa, lb, 64);
    long t = 0;
    for (int i = 0; i < 64; i++) t += lb[i] + sa[i * 2];
    printf("fill_mul %d %ld %ld %ld\n", sa[6], lb[3], lb[63], t);
    return 0;
}
