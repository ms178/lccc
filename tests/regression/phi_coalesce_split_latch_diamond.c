/* Destructive phi coalescing across a split-latch if diamond.
 *
 * The incoming value is loaded before the if; both the store and skip arms
 * then reach the latch copy `last = h`.  Cover zero-trip, both arms, an old
 * phi read on the middle arm (which must refuse coalescing), and a call in
 * that arm (which constrains the register home).  Differential vs GCC.
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
