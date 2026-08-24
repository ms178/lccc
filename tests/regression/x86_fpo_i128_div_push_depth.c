/* The i128 div/rem libcall stages rhs with two pushes. Under omitted frame
 * pointers, lhs slot reloads must account for that temporary 16-byte depth. */
#include <stdlib.h>

volatile unsigned long long seed = 0x123456789abcdef0ULL;

__attribute__((noinline))
static unsigned __int128 quotient(unsigned __int128 a, unsigned __int128 b)
{
    return a / b;
}

int main(void)
{
    unsigned __int128 a = ((unsigned __int128)seed << 64) | 0xfedcba9876543210ULL;
    unsigned __int128 b = ((unsigned __int128)3 << 64) | 17;
    unsigned __int128 q = quotient(a, b);
    if (q != a / b)
        abort();
    if (a != q * b + a % b)
        abort();
    return 0;
}
