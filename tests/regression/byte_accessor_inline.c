/* Early byte-load canonicalization must preserve bounds, endian order,
 * volatile accesses, noinline, and stores between individual byte reads. */
#include <stdint.h>
#include <sys/mman.h>
#include <unistd.h>

#define LE_BODY(p) ((uint64_t)(p)[0] | ((uint64_t)(p)[1] << 8) | \
    ((uint64_t)(p)[2] << 16) | ((uint64_t)(p)[3] << 24) | \
    ((uint64_t)(p)[4] << 32) | ((uint64_t)(p)[5] << 40) | \
    ((uint64_t)(p)[6] << 48) | ((uint64_t)(p)[7] << 56))
static uint64_t read_le(const unsigned char *p) { return LE_BODY(p); }
static __attribute__((noinline)) uint64_t read_noinline(const unsigned char *p) {
    return LE_BODY(p);
}
static uint64_t read_volatile(const volatile unsigned char *p) { return LE_BODY(p); }
static uint64_t read_be(const unsigned char *p) {
    return ((uint64_t)p[0] << 56) | ((uint64_t)p[1] << 48) |
        ((uint64_t)p[2] << 40) | ((uint64_t)p[3] << 32) |
        ((uint64_t)p[4] << 24) | ((uint64_t)p[5] << 16) |
        ((uint64_t)p[6] << 8) | (uint64_t)p[7];
}
static uint64_t read_across_store(unsigned char *p) {
    uint64_t low = p[0];
    p[0] ^= 255;
    return low | ((uint64_t)p[1] << 8) | ((uint64_t)p[2] << 16) |
        ((uint64_t)p[3] << 24) | ((uint64_t)p[4] << 32) |
        ((uint64_t)p[5] << 40) | ((uint64_t)p[6] << 48) |
        ((uint64_t)p[7] << 56);
}
int main(void) {
    long page = sysconf(_SC_PAGESIZE);
    if (page < 64) return 1;
    unsigned char *p = mmap(0, (unsigned long)page * 2,
        PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (p == MAP_FAILED) return 2;
    if (mprotect(p + page, (unsigned long)page, PROT_NONE)) return 3;
    for (int sample = 0; sample < 32; ++sample) {
        for (int offset = 0; offset < 32; ++offset) {
            unsigned char *q = p + page - 8 - offset;
            uint64_t le = 0, be = 0;
            for (int i = 0; i < 8; ++i) {
                q[i] = (unsigned char)(sample * 71 + i * 37 + offset);
                le |= (uint64_t)q[i] << (8 * i);
                be = (be << 8) | q[i];
            }
            if (read_le(q) != le || read_noinline(q) != le) return 4;
            if (read_volatile(q) != le || read_be(q) != be) return 5;
            if (read_across_store(q) != le) return 6;
            if (read_le(q) != (le ^ 255)) return 7;
        }
    }
    return munmap(p, (unsigned long)page * 2) != 0;
}
