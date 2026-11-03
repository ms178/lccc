/* Caller half of the register-pair interop fixture; see psabi_pair_interop.h.
 * Self-checking: exit status 0 is success; abort() otherwise. */
#include "psabi_pair_interop.h"

extern void abort(void);

static volatile int zero;

int main(void)
{
    const struct T t = {T_A + zero, T_B};
    const struct N n = {{T_A, T_B + zero}};
    const __int128 x = ((__int128)I128_HI << 64) | I128_LO;
    const long long xt = MIX(I128_HI, (long long)I128_LO, 0);

    if (named_t(7, t) != MIX(T_A, T_B, 7))
        abort();
    if (named_t_late(1, 2, 4, t) != MIX(T_A, T_B, 7))
        abort();
    if (named_n(7, n) != MIX(T_A, T_B, 7))
        abort();
    if (named_i128(7, x) != xt + 7)
        abort();
    if (named_i128_late(1, 1, 1, 1, 3, x) != xt + 7)
        abort();
    /* Zero to eight leading ints walk each pair from x1/a1 (odd) through
     * the even registers to the stack. */
    if (variadic_t(0, t, 9) != MIX(T_A, T_B, 9) || variadic_t(1, 1, t, 9) != MIX(T_A, T_B, 10) ||
        variadic_t(2, 1, 2, t, 9) != MIX(T_A, T_B, 12) ||
        variadic_t(6, 1, 1, 1, 1, 1, 1, t, 9) != MIX(T_A, T_B, 15) ||
        variadic_t(7, 1, 1, 1, 1, 1, 1, 1, t, 9) != MIX(T_A, T_B, 16) ||
        variadic_t(8, 1, 1, 1, 1, 1, 1, 1, 1, t, 9) != MIX(T_A, T_B, 17))
        abort();
    if (variadic_n(0, n, 9) != MIX(T_A, T_B, 9) || variadic_n(1, 1, n, 9) != MIX(T_A, T_B, 10) ||
        variadic_n(6, 1, 1, 1, 1, 1, 1, n, 9) != MIX(T_A, T_B, 15) ||
        variadic_n(7, 1, 1, 1, 1, 1, 1, 1, n, 9) != MIX(T_A, T_B, 16))
        abort();
    if (variadic_i128(0, x, 9) != xt + 9 || variadic_i128(1, 1, x, 9) != xt + 10 ||
        variadic_i128(6, 1, 1, 1, 1, 1, 1, x, 9) != xt + 15 ||
        variadic_i128(7, 1, 1, 1, 1, 1, 1, 1, x, 9) != xt + 16)
        abort();
    if (named_double_then_variadic(1.5, 2.25, 7ll) != 1.5 * 4.0 + 2.25 * 2.0 + 7.0)
        abort();
    if (named_int_double_then_variadic(3, 0.5, 2, 1.5) != 4 + 3 + 300 + 20000)
        abort();
    return 0;
}
