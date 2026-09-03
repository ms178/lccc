/* Regression: `try_complete_unroll_two_block` must repair the EXIT block's
 * phi predecessor labels after it rewires the CFG.
 *
 * ── The defect ───────────────────────────────────────────────────────────
 * The two-block complete unroller replaces
 *
 *      preheader -> header -{exit}-> exit_target
 *                     |  ^              ^
 *                     v  |              |
 *                    latch -------------+   (latch -> header back-edge)
 *
 * with a straight clone chain
 *
 *      preheader -> header -> clone_0 -> clone_1 -> ... -> clone_{n-1}
 *                                                              |
 *                                                              v
 *                                                         exit_target
 *
 * Two control-flow facts change, and the pass used to honour neither:
 *
 *   1. `header -> exit_target` NO LONGER EXISTS.  The header's terminator is
 *      now an unconditional branch into `clone_0`.  Any phi in the exit block
 *      that still carries an incoming labelled with the header names a block
 *      that is not one of its predecessors.
 *
 *   2. The real predecessor of the exit block is the LAST CLONE, and no phi
 *      had an incoming for it.
 *
 * On top of that the pass set `latch.terminator = Branch(exit_target)`,
 * inventing a `latch -> exit` edge the program never had (the latch's only
 * successor was the header, which the pass itself verifies before it runs).
 * That edge gave the exit block a third predecessor, also with no incoming.
 * The latch is genuinely unreachable after the rewrite and is now marked
 * `Unreachable`.
 *
 * ── How it manifested ────────────────────────────────────────────────────
 * Found while building linux-cachymod 6.18.47 with lccc.  In
 * `drivers/gpu/drm/i915/display/intel_sprite.c`, `vlv_sprite_update_gamma`
 * (a `for (i = 1; i < 8 - 1; i++)` MMIO loop) is inlined into
 * `vlv_sprite_update_arm`; `CCC_VERIFY_IR=1` reported
 *
 *   after `loop_unroll_post_vec` in `vlv_sprite_update_arm`:
 *     phi v904 has an incoming from BlockId(94080), which is not a
 *       predecessor (real predecessors: [93725, 94081, 94090])
 *     phi v904 has no incoming for predecessor BlockId(94090)
 *
 * The malformed phi produced a value with no reaching definition on the live
 * edge.  It survived every later pass and reached x86 instruction selection
 * with no register home and no stack slot, where `operand_to_rax`'s hard gate
 * aborted the build:
 *
 *   ccc: internal error: x86 codegen: operand_to_rax: value 904 in function
 *   'vlv_sprite_update_arm' has no register home, no stack slot and no
 *   acc-cache entry — refusing to fabricate a value
 *
 * That gate is the ONLY reason this was a loud failure rather than a silent
 * one: before it existed the same path emitted `xorl %eax,%eax`, fabricating
 * a zero.  Four functions in that one translation unit were affected
 * (`vlv_sprite_update_gamma`, `vlv_sprite_update_arm`,
 * `g4x_sprite_update_gamma`, `g4x_sprite_update_arm`).
 *
 * ── Shape requirements (all necessary to reach the bug) ──────────────────
 *   - a counted loop with a compile-time-constant trip count in 2..=16 and a
 *     2-or-3 block body, so `try_complete_unroll_two_block` fires;
 *   - the loop must be reached CONDITIONALLY, and the exit target must be a
 *     join of that condition, so the exit block genuinely carries phis
 *     (an unconditional loop lets the exit block have a single predecessor
 *     and no phi at all — the bug is then invisible);
 *   - values defined before the loop must be live ACROSS it and used after
 *     the join, so those phis have header-labelled incomings to relabel;
 *   - the loop must write through a pointer, so it is not simply deleted.
 *
 * Run under CCC_VERIFY_IR=1 this must print nothing.
 *
 * Expected output: 3 381 406 14 7
 */
#include <stdio.h>

static unsigned mmio[64];

/* `is_yuv` guards the loop exactly as in the i915 original, so the block
 * after the loop is a control-flow join carrying phis. */
static unsigned gamma_write(int pipe, int plane_id, int is_yuv, unsigned seed)
{
    unsigned short gamma[8];
    unsigned acc = seed;
    int i;

    for (i = 0; i < 8; i++)
        gamma[i] = (unsigned short)(i * 17 + pipe);

    if (!is_yuv)
        return acc;

    /* trip count 6 (1..7): the two-block complete-unroll path. */
    for (i = 1; i < 8 - 1; i++) {
        unsigned reg = (unsigned)(pipe * 8 + plane_id * 4 + (i - 1));
        unsigned v = (unsigned)gamma[i] << 16 | (unsigned)gamma[i] << 8
                   | (unsigned)gamma[i];
        mmio[reg & 63] = v;
        acc += (unsigned)gamma[i];
    }

    /* `pipe`, `plane_id` and `seed` are all live across the loop and used
     * after the join — these are the phis whose labels must be repaired. */
    return acc + (unsigned)(pipe + plane_id) + seed;
}

/* A second caller with extra pre-loop control flow, mirroring the i915
 * `vlv_sprite_update_arm` wrapper that made the header phi's predecessor set
 * large enough for the verifier message quoted above. */
static unsigned update_arm(int pipe, int plane_id, int is_yuv, int extra)
{
    unsigned r = 0;

    if (extra > 3)
        mmio[0] = (unsigned)extra;
    if (extra & 1)
        mmio[1] = 7u;

    r = gamma_write(pipe, plane_id, is_yuv, (unsigned)extra);

    if (extra > 3)
        r += mmio[0];
    return r;
}

int main(void)
{
    unsigned a = update_arm(1, 2, 0, 3);   /* loop skipped */
    unsigned b = update_arm(1, 2, 1, 5);   /* loop taken   */
    unsigned c = update_arm(3, 1, 1, 9);   /* loop taken   */
    unsigned nonzero = 0;
    unsigned i;

    for (i = 0; i < 64; i++)
        if (mmio[i])
            nonzero++;

    printf("%u %u %u %u %u\n", a, b, c, nonzero, mmio[1]);
    return 0;
}
