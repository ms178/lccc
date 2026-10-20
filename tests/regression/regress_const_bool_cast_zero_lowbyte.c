/* _Bool constant conversion must be by VALUE (C11 6.3.1.2: any nonzero
 * scalar converts to 1), never by bit-truncation to _Bool's 8-bit width.
 * The sema constant folder truncated the source bits to the target width
 * BEFORE the normalization in bits_to_irconst ran, so every nonzero
 * constant whose low byte is zero folded to _Bool 0:
 * (_Bool)256, (_Bool)65536u, (_Bool)67106816ULL.
 *
 * This is the dual of regress_const_bool_cast_normalization ((_Bool)61
 * staying 61): that fix normalized the truncated value, this one stops
 * the truncation from discarding the bits that carry the truth.
 *
 * Found by the resurrected YARPGen differential engine
 * (scripts/fuzz_diff.py --engine yarpgen, intel/yarpgen seed 20261023):
 * a single array diverged because ((_Bool)67106816ULL) ? a : b took the
 * b branch.  Covers the ternary-condition, value, loop-count, static
 * initializer and __int128-source forms across all four folding sites
 * (sema Cast eval, sema cast_i128_to_ctype, both eval_const_expr_as_bits). */

int printf(const char *, ...);

static _Bool static_bool = (_Bool)16777216; /* 2^24: low THREE bytes zero */

int main(void) {
    int acc = 0;
    acc += ((_Bool)256) ? 1 : 100;
    acc += ((_Bool)65536u) ? 2 : 200;
    acc += ((_Bool)67106816ULL) ? 4 : 400;        /* the yarpgen value */
    acc += ((_Bool)-256) ? 8 : 800;               /* negative, low byte 0 */
    acc += (int)(_Bool)16777216u * 16;
    acc += (int)static_bool * 32;
    acc += ((_Bool)((unsigned __int128)1 << 100)) ? 64 : 6400;
    acc += ((_Bool)0) ? 12800 : 128;              /* zero stays zero */
    acc += ((_Bool)5347240037557788348ULL) ? 256 : 25600; /* low byte != 0 */
    /* Loop trip count through a folded _Bool step (zero-low-byte form
     * of the normalization corpus case): must step by 1, not by 0. */
    unsigned i = 0;
    for (; i < 24; i += (_Bool)512 + 0) {
    }
    printf("%d %u\n", acc, i);
    return 0;
}
