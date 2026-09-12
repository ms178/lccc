/* Regression (strcmp-1/strncmp-1): two stacked loop_memset defects.
 *
 * 1. Stale label counter: loop_unroll minted blocks without writing back
 *    `next_label`, so loop_memset re-minted a live label, duplicating a
 *    block id and detaching its guard chain (uses orphaned in unreachable
 *    blocks, defs later deleted -> NOHOME ICE).
 * 2. Incomplete substitution: the while-form exit-iv remap walked only
 *    `for_each_operand_mut`, which skips bare-Value positions (Store ptr,
 *    GEP base, ...), leaving uses of the deleted iv phis dangling.
 *
 * This TU needs both an unrollable loop and a fill loop whose pointer iv
 * escapes into bare-Value positions after the loop.
 */
extern void abort(void);

static unsigned char buf[64];
static unsigned sum;

/* Small constant-trip loop for the unroller (stale-counter trigger). */
static void spin(void) {
    for (unsigned i = 0; i < 10; i++)
        buf[i] += (unsigned char)i;
}

/* Fill loop whose pointer iv escapes into later stores: loop-memset must
   reconstruct it and remap every use, including Store-ptr/GEP-base ones. */
static void fill(unsigned n) {
    unsigned char *p = buf;
    for (unsigned i = 0; i < n; i++)
        *p++ = 0x5a;
    p[-1] = 0x5b;
    unsigned char *q = p;
    *q = 0x5c;
    sum = (unsigned)(p - buf);
}

int main(void) {
    spin();
    fill(32);
    if (sum != 32)
        abort();
    if (buf[0] != 0x5a || buf[30] != 0x5a)
        abort();
    if (buf[31] != 0x5b || buf[32] != 0x5c)
        abort();
    if (buf[33] != 0)
        abort();
    return 0;
}
