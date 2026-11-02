/* gcc.c-torture/execute/20031003-1.c
 *
 * Out-of-range float-to-int constant conversion saturates to the
 * destination min/max under GCC -fno-trapping-math (LCCC default).
 * 2147483648.0f is 2^31, one past INT_MAX. */
#include <limits.h>

void abort(void);

int f1(void) { return (int)2147483648.0f; }

int f2(void) { return (int)(float)(2147483647); }

int main(void) {
#if INT_MAX == 2147483647
    if (f1() != 2147483647)
        abort();
    if (f2() != 2147483647)
        abort();
#endif
    return 0;
}
