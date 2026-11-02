/* Differential (vs GCC) coverage for the rotate / highest-set-bit / guarded
 * count lowerings:
 *  - the UB-free masked rotate `(x << (n & (W-1))) | (x >> (-n & (W-1)))`
 *    (and its mirror), now one rol/ror, at every count incl. 0 and W;
 *  - `(W-1) - __builtin_clz(x)` (highbit) under an `if (x)` guard, which
 *    proves the count's operand nonzero: a bare bsr, including a use whose
 *    flags feed a branch right after the cancelled XOR pair;
 *  - `x ? __builtin_ctz(x) : W` guards with zero inputs (defined), which
 *    fold to the zero-defined count.
 * Zero is never passed to an unguarded builtin (UB in C). */
#include <stdio.h>

typedef unsigned u32;
typedef unsigned long long u64;

static u32 rotl32(u32 x, int n) { return (x << (n & 31)) | (x >> (-n & 31)); }
static u32 rotr32(u32 x, int n) { return (x >> (n & 31)) | (x << (-n & 31)); }
static u64 rotl64(u64 x, int n) { return (x << (n & 63)) | (x >> (-n & 63)); }
static u64 rotr64(u64 x, unsigned n) { return (x >> (n & 63)) | (x << ((64 - n) & 63)); }
static u32 rotl32_unmasked(u32 x, u32 n) { return (x << n) | (x >> (-n & 31)); }

static u32 highbit32(u32 x) { return 31 - __builtin_clz(x); }
static u32 highbit64(u64 x) { return 63 - __builtin_clzll(x); }
static u32 tz32(u32 x) { return x ? __builtin_ctz(x) : 32; }
static u32 lz32(u32 x) { return x ? __builtin_clz(x) : 32; }
static u64 tz64(u64 x) { return x ? __builtin_ctzll(x) : 64; }
static u64 lz64(u64 x) { return x ? __builtin_clzll(x) : 64; }

static u64 h = 0x9E3779B97F4A7C15ull;
static void mix(u64 v) { h = (h ^ v) * 0x100000001B3ull; h ^= h >> 29; }

__attribute__((noinline)) static u32 bucket(u32 x)
{
    /* The highbit result feeds a compare-and-branch: the flags after the
     * cancelled `xor $31` pair must come from a fresh compare. */
    u32 hb = highbit32(x | 1);
    if (hb > 20)
        return hb * 3;
    if (hb == 0)
        return 7;
    return hb + ((x << 8) >> hb);
}

int main(void)
{
    static const u64 in[] = {
        0, 1, 2, 3, 0x80, 0xFF, 0x8000, 0x10000, 0x7FFFFFFF, 0x80000000,
        0xFFFFFFFF, 0x100000000ull, 0x8000000000000000ull, 0xFFFFFFFFFFFFFFFFull,
        0x0123456789ABCDEFull, 0xFEDCBA9876543210ull, 0x00000000DEADBEEFull,
    };
    static const int counts[] = {0, 1, 5, 31, 32, 33, 63, 64, 65, 127, -1, -31, -32, -33};
    volatile int vsel = 0;
    for (unsigned i = 0; i < sizeof in / sizeof in[0]; i++) {
        u64 x = in[i] + (u64)vsel;
        u32 x32 = (u32)x;
        for (unsigned c = 0; c < sizeof counts / sizeof counts[0]; c++) {
            int n = counts[c] + vsel;
            mix(rotl32(x32, n));
            mix(rotr32(x32, n));
            mix(rotl64(x, n));
            mix(rotr64(x, (unsigned)n));
            if ((u32)n < 32)
                mix(rotl32_unmasked(x32, (u32)n));
        }
        mix(tz32(x32));
        mix(lz32(x32));
        mix(tz64(x));
        mix(lz64(x));
        mix(bucket(x32));
        if (x32)
            mix(highbit32(x32) * 1000 + (__builtin_ctz(x32) >> 1));
        if (x)
            mix(highbit64(x) + ((x >> (__builtin_ctzll(x) & 63)) & 0xFF));
    }
    printf("%016llx\n", h);
    return 0;
}
