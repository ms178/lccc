/* Narrowing must preserve high-half uses of a wide product across a same-width
 * signedness cast.  This is the core of signed constant-division sequences:
 * narrowing the multiply to 32 bits before `>> 32` discards the quotient. */
#include <limits.h>
#include <stdlib.h>

__attribute__((noinline))
static int magic_div3(int x)
{
    return (int)((unsigned long long)(x * 0x55555556LL) >> 32) - (x >> 31);
}

int main(void)
{
    static const int values[] = {
        INT_MIN, INT_MIN + 1, -123456789, -4, -3, -2, -1,
        0, 1, 2, 3, 4, 123456789, INT_MAX
    };
    for (unsigned i = 0; i < sizeof(values) / sizeof(values[0]); ++i)
        if (magic_div3(values[i]) != values[i] / 3)
            abort();
    return 0;
}
