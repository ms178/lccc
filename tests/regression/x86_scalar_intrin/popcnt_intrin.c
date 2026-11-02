/* `_mm_popcnt_u32`/`_mm_popcnt_u64` (popcntintrin.h) over edge inputs;
 * stdout must match GCC -mpopcnt on its headers. lccc builds this on every
 * flag set: without POPCNT the portable form computes the same counts. */
#include <immintrin.h>
#include <stdio.h>

static const unsigned long long in64[] = {
    0, 1, 2, 3, 0x80, 0x8000, 0x80000000ULL, 0xFFFFFFFFULL, 0x100000000ULL,
    0x8000000000000000ULL, 0xFFFFFFFFFFFFFFFFULL, 0x00F0F0F0F0F0F0F0ULL,
    0x123456789ABCDEF0ULL, 0x5555555555555555ULL, 0x7FFFFFFFFFFFFFFFULL,
};

int main(void)
{
    for (unsigned i = 0; i < sizeof in64 / sizeof in64[0]; i++) {
        volatile unsigned long long vx = in64[i];
        unsigned long long x = vx;
        printf("%2u %d", i, _mm_popcnt_u32((unsigned)x));
#ifdef __x86_64__
        printf(" %lld", _mm_popcnt_u64(x));
#endif
        printf("\n");
    }
    return 0;
}
