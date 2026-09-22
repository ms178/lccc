/* Wide-lane truncated SELECT: constants that must NOT be re-typed narrow.
 *
 * Pins `narrow_const` in the BB-SLP vectorizer. The sub-word SELECT
 * demotion rewrites `trunc(promoted select)` as a select at the LANE
 * width, and both its arm constants and its compare constants are
 * re-materialized at that width by `narrow_const(cv, ty)`. If that helper
 * returned a constant whose `IrConst` variant is narrower than `ty`, the
 * value would be truncated on the way in: `IrConst::I16(100000i16)` is
 * `-31072`, so a 32-bit lane whose arm constant exceeds the 16-bit range
 * would silently store a corrupted value.
 *
 * Every kernel below uses a lane type WIDER than 16 bits with arm and
 * compare constants outside the i16 range, in both the straight-line
 * 4-lane form the BB-SLP pass seeds from and the loop form. The expected
 * values are computed at runtime in scalar `int64_t` arithmetic, so the
 * test asserts the demotion is either correct or absent -- never wrong.
 *
 * Values are chosen so that a mis-typed narrow constant changes the
 * result: 100000 mod 2^16 == -31072 (signed) and 2000000000 does not fit
 * in 16 bits at all.
 */
#include <stdint.h>
#include <stdio.h>

#define LANES 4

/* Straight-line 4-lane: the shape BB-SLP seeds a dword pack from. */
__attribute__((noinline))
static void gt_big(const int64_t *restrict a, int32_t *restrict d) {
    d[0] = (int32_t)(a[0] > 5 ? 100000 : a[0]);
    d[1] = (int32_t)(a[1] > 5 ? 100000 : a[1]);
    d[2] = (int32_t)(a[2] > 5 ? 100000 : a[2]);
    d[3] = (int32_t)(a[3] > 5 ? 100000 : a[3]);
}

__attribute__((noinline))
static void lt_negbig(const int64_t *restrict a, int32_t *restrict d) {
    d[0] = (int32_t)(a[0] < -5 ? -100000 : a[0]);
    d[1] = (int32_t)(a[1] < -5 ? -100000 : a[1]);
    d[2] = (int32_t)(a[2] < -5 ? -100000 : a[2]);
    d[3] = (int32_t)(a[3] < -5 ? -100000 : a[3]);
}

/* Constant far outside i16 in both directions; the compare constant is
 * also outside i16, so the promoted-compare side must reject rather than
 * truncate the predicate operand too. */
__attribute__((noinline))
static void gt_huge(const int64_t *restrict a, int32_t *restrict d) {
    d[0] = (int32_t)(a[0] > 70000 ? 2000000000 : a[0]);
    d[1] = (int32_t)(a[1] > 70000 ? 2000000000 : a[1]);
    d[2] = (int32_t)(a[2] > 70000 ? 2000000000 : a[2]);
    d[3] = (int32_t)(a[3] > 70000 ? 2000000000 : a[3]);
}

/* Unsigned dword lanes: the unsigned predicate path shares the helper. */
__attribute__((noinline))
static void ugt_big(const uint64_t *restrict a, uint32_t *restrict d) {
    d[0] = (uint32_t)(a[0] > 5u ? 4000000000u : a[0]);
    d[1] = (uint32_t)(a[1] > 5u ? 4000000000u : a[1]);
    d[2] = (uint32_t)(a[2] > 5u ? 4000000000u : a[2]);
    d[3] = (uint32_t)(a[3] > 5u ? 4000000000u : a[3]);
}

/* Loop forms of the same three shapes. */
__attribute__((noinline))
static void gt_big_loop(const int64_t *a, int32_t *d, int n) {
    for (int i = 0; i < n; i++)
        d[i] = (int32_t)(a[i] > 5 ? 100000 : a[i]);
}

__attribute__((noinline))
static void lt_negbig_loop(const int64_t *a, int32_t *d, int n) {
    for (int i = 0; i < n; i++)
        d[i] = (int32_t)(a[i] < -5 ? -100000 : a[i]);
}

int main(void) {
    /* Spans both arms of every predicate, including the boundary values
     * 5, -5 and 70000, and values whose low 16 bits differ from the
     * constant's low 16 bits. */
    const int64_t sa[8] = {1, 2, 6, 100, -9, 70001, 3, 70000};
    const uint64_t ua[8] = {1u, 2u, 6u, 100u, 0u, 4000000001u, 3u, 5u};
    int32_t d[8], want[8];
    uint32_t ud[8], uwant[8];
    int fails = 0;

    for (int i = 0; i < LANES; i++) {
        want[i] = (int32_t)(sa[i] > 5 ? 100000 : sa[i]);
        d[i] = 0;
    }
    gt_big(sa, d);
    for (int i = 0; i < LANES; i++)
        if (d[i] != want[i]) {
            printf("gt_big[%d]: got %d want %d\n", i, d[i], want[i]);
            fails++;
        }

    for (int i = 0; i < LANES; i++) {
        want[i] = (int32_t)(sa[i] < -5 ? -100000 : sa[i]);
        d[i] = 0;
    }
    lt_negbig(sa, d);
    for (int i = 0; i < LANES; i++)
        if (d[i] != want[i]) {
            printf("lt_negbig[%d]: got %d want %d\n", i, d[i], want[i]);
            fails++;
        }

    for (int i = 0; i < LANES; i++) {
        want[i] = (int32_t)(sa[i] > 70000 ? 2000000000 : sa[i]);
        d[i] = 0;
    }
    gt_huge(sa, d);
    for (int i = 0; i < LANES; i++)
        if (d[i] != want[i]) {
            printf("gt_huge[%d]: got %d want %d\n", i, d[i], want[i]);
            fails++;
        }

    for (int i = 0; i < LANES; i++) {
        uwant[i] = (uint32_t)(ua[i] > 5u ? 4000000000u : ua[i]);
        ud[i] = 0;
    }
    ugt_big(ua, ud);
    for (int i = 0; i < LANES; i++)
        if (ud[i] != uwant[i]) {
            printf("ugt_big[%d]: got %u want %u\n", i, ud[i], uwant[i]);
            fails++;
        }

    for (int i = 0; i < 8; i++) {
        want[i] = (int32_t)(sa[i] > 5 ? 100000 : sa[i]);
        d[i] = 0;
    }
    gt_big_loop(sa, d, 8);
    for (int i = 0; i < 8; i++)
        if (d[i] != want[i]) {
            printf("gt_big_loop[%d]: got %d want %d\n", i, d[i], want[i]);
            fails++;
        }

    for (int i = 0; i < 8; i++) {
        want[i] = (int32_t)(sa[i] < -5 ? -100000 : sa[i]);
        d[i] = 0;
    }
    lt_negbig_loop(sa, d, 8);
    for (int i = 0; i < 8; i++)
        if (d[i] != want[i]) {
            printf("lt_negbig_loop[%d]: got %d want %d\n", i, d[i], want[i]);
            fails++;
        }

    if (fails == 0)
        printf("bb_slp_i64_to_i32_select: all wide-lane select constants exact\n");
    else
        printf("bb_slp_i64_to_i32_select: %d lane(s) corrupted\n", fails);
    return fails != 0;
}
