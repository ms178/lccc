/* A/B driver for engineering/kernels/isel_roi.c.
 * Compile the kernels + this file with LCCC and with gcc -O2; stdout must match.
 */
#include <stdio.h>

unsigned popcount32(unsigned x);
unsigned long popcount64(unsigned long x);
int clz32(unsigned x);
int ctz32(unsigned x);
unsigned andn32(unsigned a, unsigned b);
unsigned blsr32(unsigned x);
unsigned blsi32(unsigned x);
unsigned bzhi32(unsigned x, unsigned n);
unsigned bit_test32(unsigned x, unsigned k);
unsigned min_u32(unsigned a, unsigned b);
int max_i32(int a, int b);
int abs_i32(int x);
unsigned mul3(unsigned x);
unsigned mul5(unsigned x);
unsigned mul9(unsigned x);
unsigned add_imm(unsigned x);
unsigned zero(void);
unsigned rotl32(unsigned x, unsigned n);
unsigned hash_mul(unsigned x);
unsigned select_inc(unsigned x, unsigned y);
int cmp0(int x);
unsigned zext_load(const unsigned char *p);
unsigned lea_index(unsigned *a, unsigned i);

int main(void) {
    unsigned xs[] = {1u, 2u, 3u, 31u, 32u, 42u, 0x80000000u, 0xffffffffu,
                     0x9e3779b1u, 0x00ff00ffu, 0x7fffffffu, 0x12345678u};
    unsigned nxs = (unsigned)(sizeof(xs) / sizeof(xs[0]));
    unsigned char bytes[] = {0, 1, 127, 128, 255};
    unsigned arr[8] = {10, 20, 30, 40, 50, 60, 70, 80};

    printf("zero=%u\n", zero());
    for (unsigned i = 0; i < nxs; i++) {
        unsigned x = xs[i];
        unsigned y = xs[(i + 3) % nxs];
        printf("x=%u y=%u\n", x, y);
        printf("  pop32=%u pop64=%lu\n", popcount32(x), popcount64((unsigned long)x << 1));
        printf("  clz=%d ctz=%d\n", clz32(x), ctz32(x));
        printf("  andn=%u blsr=%u blsi=%u\n", andn32(x, y), blsr32(x), blsi32(x));
        printf("  bzhi0=%u bzhi5=%u bzhi31=%u\n", bzhi32(x, 0), bzhi32(x, 5), bzhi32(x, 31));
        printf("  bt0=%u bt7=%u bt31=%u\n", bit_test32(x, 0), bit_test32(x, 7), bit_test32(x, 31));
        printf("  minu=%u maxi=%d abs=%d\n", min_u32(x, y), max_i32((int)x, (int)y),
               abs_i32((int)x));
        printf("  mul3=%u mul5=%u mul9=%u add=%u hash=%u\n", mul3(x), mul5(x), mul9(x),
               add_imm(x), hash_mul(x));
        printf("  rot0=%u rot1=%u rot16=%u rot31=%u\n", rotl32(x, 0), rotl32(x, 1),
               rotl32(x, 16), rotl32(x, 31));
        printf("  sel=%u cmp0=%d\n", select_inc(x, y), cmp0((int)x));
    }
    for (unsigned i = 0; i < sizeof(bytes); i++)
        printf("zext[%u]=%u\n", i, zext_load(&bytes[i]));
    for (unsigned i = 0; i < 8; i++)
        printf("lea[%u]=%u\n", i, lea_index(arr, i));
    /* signed abs / max edges */
    printf("abs-1=%d absmin=%d max(-3,5)=%d max(5,-3)=%d\n", abs_i32(-1),
           abs_i32((int)0x80000000), max_i32(-3, 5), max_i32(5, -3));
    printf("cmp0_0=%d cmp0_neg=%d\n", cmp0(0), cmp0(-7));
    printf("minu_eq=%u sel0=%u sel1=%u\n", min_u32(9, 9), select_inc(0, 41),
           select_inc(1, 41));
    return 0;
}
