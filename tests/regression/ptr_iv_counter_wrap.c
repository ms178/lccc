/* A uint32_t loop counter can wrap while pointer IVs continue to advance.
 * Reserve sparse address space, touching only a few pages: this exercises
 * the real 32-bit wrap after four iterations without allocating 4 GiB of RAM.
 * The last byte is inside the mapping and is reached without pointer overflow.
 */
#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <sys/mman.h>

#define STRIDE (UINT32_C(1) << 30)

__attribute__((noinline))
static unsigned unreachable_bound(const unsigned char *a, const unsigned char *b) {
    uint32_t i = 0;
    do {
        if (*a != *b)
            return 42;
        a += STRIDE;
        b += STRIDE;
        i += STRIDE;
    } while (i != STRIDE + 1);
    return 17;
}

__attribute__((noinline))
static unsigned reachable_bound(const unsigned char *a, const unsigned char *b) {
    uint32_t i = 0;
    do {
        if (*a != *b)
            return 42;
        a += STRIDE;
        b += STRIDE;
        i += STRIDE;
    } while (i != STRIDE * 2);
    return 17;
}

int main(void) {
    const size_t size = ((size_t)1 << 32) + 4096;
    unsigned char *p = mmap(NULL, size, PROT_READ | PROT_WRITE,
                            MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1, 0);
    if (p == MAP_FAILED) {
        perror("sparse mmap for pointer-IV wrap regression");
        return 1;
    }
    p[(size_t)1 << 32] = 1;
    unsigned unreachable = unreachable_bound(p, p + 8);
    unsigned reachable = reachable_bound(p, p + 8);
    if (munmap(p, size) != 0) {
        perror("munmap");
        return 1;
    }
    if (unreachable != 42 || reachable != 17) {
        printf("incorrect IV wrap results: %u %u\n", unreachable, reachable);
        return 1;
    }
    puts("pointer IV counter-wrap semantics: PASS");
    return 0;
}
