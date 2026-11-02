/* The last comparison in a flattened &&/|| chain is an IR byte boolean but C
 * gives it int semantics. It must be widened before entering the machine-word
 * short-circuit result slot/phi; a byte copy leaves upper register bits stale. */
#include <stdlib.h>

__attribute__((noinline))
static int all_signs(int a, int b, long long c)
{
    return a >= 0 && b < 0 && c < 0;
}

__attribute__((noinline))
static int any_sign(int a, int b, long long c)
{
    return a < 0 || b >= 0 || c >= 0;
}

int main(void)
{
    if (all_signs(1, -2, -3) != 1)
        abort();
    if (all_signs(1, -2, 3) != 0)
        abort();
    if (any_sign(1, -2, -3) != 0)
        abort();
    if (any_sign(1, -2, 3) != 1)
        abort();
    return 0;
}
