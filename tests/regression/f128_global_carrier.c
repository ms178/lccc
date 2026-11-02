// _Float128 globals and negation end-to-end (aarch64 carrier regression):
// 1. a 16-byte global _Float128 must be loaded whole through the GOT address
//    (the x9-address peephole once deleted the defining `mov x9, xN` after
//    copy propagation rewrote only the FIRST use, leaving `ldr x1, [x9, #8]`
//    reading a stale x9 -> SIGSEGV);
// 2. unary minus must flip the sign bit of the full binary128 pattern
//    (the F128Neg operand staging once degraded to the f64-extend fallback
//    and U128-typed consumers truncated tracked results to 8 bytes).
// Returns nonzero on the first failure.
#include <stdio.h>

_Float128 g_pos = 42.0F128;
_Float128 g_neg = -1.5F128;

static int fails = 0;
#define CHECK(cond) do { if (!(cond)) { fails++; } } while (0)

_Float128 negate(_Float128 x) { return -x; }

int main(void) {
    CHECK(g_pos == 42.0F128);
    CHECK(g_neg == -1.5F128);
    CHECK(-g_pos == -42.0F128);
    _Float128 a = 1.5F128;
    CHECK(negate(a) == -1.5F128);
    CHECK(-a == -1.5F128);
    CHECK(negate(g_pos) == -42.0F128);
    _Float128 round = negate(negate(a));
    CHECK(round == 1.5F128);
    if (fails != 0) {
        printf("f128_global_carrier: %d check(s) failed\n", fails);
    }
    return fails != 0;
}
