/* Full low-byte index semantics, width edges and high-index wraparound.
 * The scalar reference never performs an out-of-range C shift. */
#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>

__attribute__((noinline)) static uint64_t mask64(uint64_t x, unsigned n) {
    return _bzhi_u64(x, n);
}
__attribute__((noinline)) static uint32_t mask32(uint32_t x, unsigned n) {
    return _bzhi_u32(x, n);
}
static uint64_t ref(uint64_t x, unsigned n, unsigned width) {
    n &= 255u;
    if (width == 32) x = (uint32_t)x;
    return n >= width ? x : x & ((UINT64_C(1) << n) - 1);
}
int main(void) {
    uint64_t state = UINT64_C(0x1234567812345678), checksum = 0;
    const uint64_t edges[] = {0, 1, 3, UINT64_MAX, UINT64_C(0x8000000000000000),
        UINT64_C(0x80000000), UINT64_C(0xffffffff), UINT64_C(0xaaaaaaaaaaaaaaaa)};
    for (unsigned i = 0; i < 65536; ++i) {
        state = state * UINT64_C(6364136223846793005) + 1;
        unsigned n = i | ((unsigned)(state >> 32) & 0xffff0000u);
        uint64_t x = (i & 15) < 8 ? edges[i & 7] : state;
        uint64_t a = mask64(x, n), b = mask32((uint32_t)x, n);
        if (a != ref(x, n, 64) || b != ref(x, n, 32)) return 1;
        checksum += a ^ b;
    }
    printf("bmi2 mask: %llu\n", (unsigned long long)checksum);
    return 0;
}
