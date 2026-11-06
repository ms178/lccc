/* Self-checking parameter-home fixture: BOTH index build paths on one TU.
 *
 * `aligned()` has the layout lowering emits (strictly increasing home ids,
 * the first n entry-block instructions are the parameter allocas), so it
 * takes the hash-table-free positional path.  `shifted()` declares a volatile
 * local first, which inserts an Alloca before the parameter homes and forces
 * the general (resumed) path.  Both must return 0+1+...+15 == 120.  Distinct
 * nonzero exit codes say WHICH path broke, so a green run cannot be a
 * compiler that silently dropped one of the two shapes.
 *
 * Freestanding by construction: no includes, no libc, no global initializers.
 * `_start` (supplied by the harness) turns main's return value into the
 * process exit status via the raw kernel syscall.
 */

int aligned(int p0, int p1, int p2, int p3, int p4, int p5, int p6, int p7,
            int p8, int p9, int p10, int p11, int p12, int p13, int p14, int p15)
{
    return p0 + p1 + p2 + p3 + p4 + p5 + p6 + p7
         + p8 + p9 + p10 + p11 + p12 + p13 + p14 + p15;
}

int shifted(int p0, int p1, int p2, int p3, int p4, int p5, int p6, int p7,
            int p8, int p9, int p10, int p11, int p12, int p13, int p14, int p15)
{
    /* A volatile local is an Alloca that lowering cannot promote away.  It
     * shifts the parameter homes, so the aligned probe fails early and the
     * general pass must resolve all 16 ordinals from later instructions. */
    volatile int local = p0;
    int s = p0 + p1 + p2 + p3 + p4 + p5 + p6 + p7
          + p8 + p9 + p10 + p11 + p12 + p13 + p14 + p15;
    if (local != p0)
        return 1;
    return s;
}

int main(void)
{
    int a = aligned(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15);
    int b = shifted(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15);
    if (a != 120)
        return 1;   /* aligned (positional) path broke */
    if (b != 120)
        return 2;   /* shifted (general) path broke */
    return 0;
}
