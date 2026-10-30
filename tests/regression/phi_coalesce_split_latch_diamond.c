/* Destructive phi coalescing around a conditional latch update.
 *
 * The incoming value is loaded before the if and copied to the phi at the
 * latch. Three shapes are exercised: the plain update, an old-phi read on the
 * store arm (which must refuse coalescing), and a call on that arm (which
 * constrains the register home).  Zero-trip and both if arms are covered.
 *
 * SCOPE, measured rather than assumed: on this pipeline the latch copy is
 * scheduled into the same block as its source definition, so these functions
 * take the SAME-BLOCK coalescing path.  The `source -> {middle, copy}` diamond
 * that `phi_update_region` also admits is not produced by the current C front
 * end; it is covered by the `phi_coalesce_*split_latch_diamond*` unit tests,
 * which construct the CFG directly.  This fixture is therefore an execution
 * and differential-correctness guard for the same-block path, NOT a witness
 * for the diamond -- do not read a diamond-specific claim out of it.
 *
 * Differential vs GCC.
 */
#include <stdlib.h>

static const unsigned input[9] = { 0, 3, 8, 5, 2, 7, 1, 4, 9 };
static unsigned observed;

__attribute__((noinline)) static void observe(unsigned x)
{
    observed = observed * 33U + x;
}

__attribute__((noinline)) static unsigned plain_diamond(const unsigned *src,
                                                        unsigned *dst,
                                                        unsigned n)
{
    unsigned last = 0;
    for (unsigned i = 0; i < n; ++i) {
        unsigned h = src[i];
        if (h & 1U)
            *dst += h;
        last = h;
    }
    return last;
}

__attribute__((noinline)) static unsigned old_phi_read(const unsigned *src,
                                                       unsigned *dst,
                                                       unsigned n)
{
    unsigned last = 0;
    for (unsigned i = 0; i < n; ++i) {
        unsigned h = src[i];
        if (h & 1U)
            *dst += last;
        last = h;
    }
    return last;
}

__attribute__((noinline)) static unsigned call_in_middle(const unsigned *src,
                                                         unsigned *dst,
                                                         unsigned n)
{
    unsigned last = 0;
    for (unsigned i = 0; i < n; ++i) {
        unsigned h = src[i];
        if (h & 1U) {
            *dst += h;
            observe(h);
        }
        last = h;
    }
    return last;
}

int main(void)
{
    for (unsigned n = 0; n <= 9; ++n) {
        unsigned expected_last = n == 0 ? 0 : input[n - 1];
        unsigned expected_plain = 0, expected_old = 0, expected_observed = 1;
        unsigned previous = 0;
        for (unsigned i = 0; i < n; ++i) {
            if (input[i] & 1U) {
                expected_plain += input[i];
                expected_old += previous;
                expected_observed = expected_observed * 33U + input[i];
            }
            previous = input[i];
        }

        unsigned dst = 0;
        if (plain_diamond(input, &dst, n) != expected_last || dst != expected_plain)
            abort();
        dst = 0;
        if (old_phi_read(input, &dst, n) != expected_last || dst != expected_old)
            abort();
        dst = 0;
        observed = 1;
        if (call_in_middle(input, &dst, n) != expected_last || dst != expected_plain
            || observed != expected_observed)
            abort();
    }
    return 0;
}
