/* Scalar read-only unrolling must not widen loads across this guard page. */
#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <sys/mman.h>
#include <unistd.h>

#define KERNEL(N) \
__attribute__((noinline)) static uint32_t sum_##N(const unsigned char *p, uint32_t sum) { \
    for (unsigned long i = 0; i < (N); ++i) { \
        const unsigned char *q = p + i * 8; \
        uint32_t v = (uint32_t)q[0] | ((uint32_t)q[1] << 8) | \
                     ((uint32_t)q[2] << 16) | ((uint32_t)q[3] << 24); \
        sum += v ^ (uint32_t)i; \
    } \
    return sum; \
}
KERNEL(0) KERNEL(1) KERNEL(16) KERNEL(31) KERNEL(32) KERNEL(33)
KERNEL(63) KERNEL(64) KERNEL(65) KERNEL(128) KERNEL(129)

typedef uint32_t (*kernel)(const unsigned char *, uint32_t);
int main(void) {
    const unsigned sizes[] = {0,1,16,31,32,33,63,64,65,128,129};
    const kernel functions[] = {sum_0,sum_1,sum_16,sum_31,sum_32,sum_33,
                               sum_63,sum_64,sum_65,sum_128,sum_129};
    const uint32_t seeds[] = {0, UINT32_C(0xfffffff0), UINT32_C(0x80000000)};
    long page = sysconf(_SC_PAGESIZE);
    if (page < 2048) return 90;
    unsigned char *map = mmap(0, (size_t)page * 2, PROT_READ | PROT_WRITE,
                              MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (map == MAP_FAILED || mprotect(map + page, (size_t)page, PROT_NONE)) return 91;
    uint32_t checksum = 0;
    for (unsigned c = 0; c < sizeof sizes / sizeof sizes[0]; ++c) {
        unsigned n = sizes[c];
        size_t bytes = n ? (size_t)(n - 1) * 8 + 4 : 0;
        unsigned char *p = map + page - bytes;
        for (size_t i = 0; i < bytes; ++i) p[i] = (unsigned char)(i * 37 + c * 11);
        for (unsigned s = 0; s < 3; ++s) {
            uint32_t expected = seeds[s];
            const volatile unsigned char *q = p;
            for (unsigned i = 0; i < n; ++i) {
                uint32_t v = (uint32_t)q[8*i] | ((uint32_t)q[8*i+1] << 8) |
                             ((uint32_t)q[8*i+2] << 16) | ((uint32_t)q[8*i+3] << 24);
                expected += v ^ i;
            }
            uint32_t actual = functions[c](p, seeds[s]);
            if (actual != expected) return 1;
            checksum += actual;
        }
    }
    if (munmap(map, (size_t)page * 2)) return 92;
    printf("%08x\n", checksum);
    return 0;
}
