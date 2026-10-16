#include <stdint.h>
#include <stdio.h>
__attribute__((noinline)) uint64_t bounded_u64(const unsigned char *p) {
    uint64_t s = 0;
    for (uint64_t i = 0; i + 8 <= 4096; i += 8) s += p[i] ^ i;
    return s;
}
__attribute__((noinline)) uint32_t bounded_u32(const unsigned char *p) {
    uint32_t s = 0;
    for (uint32_t i = 0; i + 8 <= 4096; i += 8) s += p[i] ^ i;
    return s;
}
__attribute__((noinline)) uint64_t wraps_u64(void) {
    uint64_t s = 0;
    for (uint64_t i = UINT64_MAX - 3; i + 8 <= 32; i += 4) s += i;
    return s;
}
__attribute__((noinline)) uint32_t wraps_u32(void) {
    uint32_t s = 0;
    for (uint32_t i = UINT32_MAX - 3; i + 8 <= 32; i += 4) s += i;
    return s;
}
int main(void) {
    unsigned char p[4096]; uint64_t total = 0;
    for (unsigned sample = 0; sample < 32; ++sample) {
        for (unsigned i = 0; i < 4096; ++i) p[i] = (unsigned char)(i * 37 + sample * 71);
        uint64_t ref = 0;
        for (unsigned i = 0; i < 4096; i += 8) ref += p[i] ^ i;
        if (bounded_u64(p) != ref || bounded_u32(p) != ref) return 1;
        total += ref;
    }
    if (wraps_u64() != 80 || wraps_u32() != 80) return 2;
    printf("unsigned-affine %llu\n", (unsigned long long)total);
    return 0;
}
