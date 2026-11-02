/* mul-accumulate with SIGNED addend feeders and negative constants
 * (i686 fused head): AddendHi::Sext (movl/sarl high replication) and
 * Imm(hi<0) paths must be exact for negative values; bases at the
 * hi-zero boundary (0xFFFFFFFF) fuse while 2^32 bases must reject the
 * chain and take the generic 64-bit path — both must agree with GCC. */
#include <stdio.h>
#include <stdint.h>

int main(void) {
    volatile int32_t neg = -7;
    volatile int32_t small = 3;
    volatile uint32_t u = 4000000000u;

    /* sext feeder addend (i32 -> i64), negative */
    int64_t a = 1;
    for (int i = 0; i < 6; i++) a = a * 5 + (int64_t)neg;
    printf("A=%lld\n", (long long)a);

    /* sext feeder addend, positive, alternating sign */
    int64_t b = 2;
    for (int i = 0; i < 6; i++) b = b * 3 + (int64_t)(i % 2 ? neg : small);
    printf("B=%lld\n", (long long)b);

    /* negative constant addend: hi = -1 path */
    int64_t c = 10;
    for (int i = 0; i < 5; i++) c = c * 7 + (-5);
    printf("C=%lld\n", (long long)c);

    /* base = 0xFFFFFFFF: hi-zero-provable constant, must fuse correctly */
    uint64_t d = 1;
    for (int i = 0; i < 4; i++) d = d * 0xFFFFFFFFull + (uint64_t)u;
    printf("D=%016llx\n", (unsigned long long)d);

    /* base >= 2^32 through a value the analyzer cannot prove: generic path */
    volatile uint64_t big = 0x100000000ull;
    uint64_t e = 1;
    for (int i = 0; i < 3; i++) e = e * big + (uint64_t)u;
    printf("E=%016llx\n", (unsigned long long)e);

    /* signed overflow wraparound in the fused shape (UB-free unsigned math) */
    uint64_t f = 0;
    for (int i = 0; i < 20; i++) f = f * 10 + (uint64_t)(neg);
    printf("F=%016llx\n", (unsigned long long)f);

    /* u32 zext feeder with values above 2^31 (sign-bit set) */
    uint64_t g = 1;
    for (int i = 0; i < 5; i++) g = g * 10 + (uint64_t)(u + (uint32_t)i);
    printf("G=%016llx\n", (unsigned long long)g);
    return 0;
}
