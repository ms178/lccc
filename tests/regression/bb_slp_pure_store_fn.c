/* A void/noinline callee that writes only through packed stores is not pure.
 * Pair stores write 8 bytes but consume a 16-byte SIMD value. Check both lane
 * values and neighbouring sentinels, then repeat after changing the source.
 * The gate also runs with SLP / interprocedural propagation / address folding disabled separately.
 */
#include <stdint.h>
#include <stdio.h>

__attribute__((noinline)) void copy_pair(uint32_t *restrict dst,
                                        const uint32_t *restrict src) {
    dst[1] = src[1];
    dst[2] = src[2];
}

__attribute__((noinline)) void copy_quad(uint64_t *restrict dst,
                                        const uint64_t *restrict src) {
    dst[1] = src[1];
    dst[2] = src[2];
    dst[3] = src[3];
    dst[4] = src[4];
}

static volatile unsigned seed = 37;
int main(void) {
    uint32_t src[4], dst[4];
    uint64_t wide_src[6], wide_dst[6];
    unsigned value = seed;
    uint64_t checksum = 0;
    for (unsigned round = 0; round != 17; ++round) {
        for (unsigned i = 0; i != 4; ++i) {
            src[i] = value + 101 * i + round;
            dst[i] = 0xdeadbeefU;
        }
        for (unsigned i = 0; i != 6; ++i) {
            wide_src[i] = ((uint64_t)(value + round) << 40) + 12345 * i;
            wide_dst[i] = UINT64_C(0x123456789abcdef0);
        }
        copy_pair(dst, src);
        copy_quad(wide_dst, wide_src);
        if (dst[0] != 0xdeadbeefU || dst[3] != 0xdeadbeefU ||
            dst[1] != src[1] || dst[2] != src[2]) return 1;
        if (wide_dst[0] != UINT64_C(0x123456789abcdef0) ||
            wide_dst[5] != UINT64_C(0x123456789abcdef0)) return 2;
        for (unsigned i = 1; i != 5; ++i) {
            if (wide_dst[i] != wide_src[i]) return 3;
            checksum += wide_dst[i];
        }
        checksum += dst[1] + dst[2];
    }
    printf("packed-store %llx\n", (unsigned long long)checksum);
    return 0;
}
