/* Regression: a 128-bit argument after six GP arguments is a 16-byte stack arg. */
#include <stdio.h>

typedef unsigned __int128 u128;

static u128 combine(long a, long b, long c, long d, long e, long f, u128 x) {
    return x + (u128)(a + b + c + d + e + f);
}

int main(void) {
    u128 input = ((u128)1 << 100) + 17;
    u128 got = combine(1, 2, 3, 4, 5, 6, input);
    unsigned long high = (unsigned long)(got >> 64);
    unsigned long low = (unsigned long)got;
    printf("%lu %lu\n", high, low);
    return high != ((unsigned long)((u128)1 << 36)) || low != 38;
}
