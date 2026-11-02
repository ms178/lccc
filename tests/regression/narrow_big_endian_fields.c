/* Promoted big-endian fields must read exactly the original bytes and retain
   the correct zero/sign extension, including at an inaccessible page edge. */
#include <stdint.h>
#include <stdio.h>
#include <sys/mman.h>
#include <unistd.h>

__attribute__((noinline))
static unsigned read_u16(const unsigned char *p) {
    return ((unsigned)p[0] << 8) | (unsigned)p[1];
}

__attribute__((noinline))
static int64_t read_s16(const unsigned char *p) {
    return (int16_t)(((uint16_t)p[0] << 8) | (uint16_t)p[1]);
}

__attribute__((noinline))
static uint64_t read_u16_wide(const unsigned char *p) {
    return ((uint64_t)p[0] << 8) | (uint64_t)p[1];
}

__attribute__((noinline))
static uint64_t read_u32_wide(const unsigned char *p) {
    return ((uint64_t)p[0] << 24) | ((uint64_t)p[1] << 16)
         | ((uint64_t)p[2] << 8) | (uint64_t)p[3];
}

__attribute__((noinline))
static unsigned read_volatile(const volatile unsigned char *p) {
    return ((unsigned)p[0] << 8) | (unsigned)p[1];
}

int main(void) {
    unsigned char bytes[12] = {0};
    for (unsigned n = 0; n <= 65535u; ++n) {
        unsigned off = n & 7u;
        uint32_t value = n * 2654435761u;
        bytes[off] = (unsigned char)(n >> 8);
        bytes[off + 1] = (unsigned char)n;
        if (read_u16(bytes + off) != n || read_u16_wide(bytes + off) != n)
            return 1;
        if (read_s16(bytes + off) != (int64_t)((int)(n ^ 32768u) - 32768))
            return 2;
        if (read_volatile(bytes + off) != n)
            return 3;
        bytes[off] = (unsigned char)(value >> 24);
        bytes[off + 1] = (unsigned char)(value >> 16);
        bytes[off + 2] = (unsigned char)(value >> 8);
        bytes[off + 3] = (unsigned char)value;
        if (read_u32_wide(bytes + off) != (uint64_t)value)
            return 4;
    }

    long page = sysconf(_SC_PAGESIZE);
    if (page <= 0) return 5;
    unsigned char *mapping = mmap(0, (size_t)page * 2u, PROT_READ | PROT_WRITE,
                                  MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (mapping == MAP_FAILED) return 6;
    if (mprotect(mapping + page, (size_t)page, PROT_NONE) != 0) return 7;
    unsigned char *p = mapping + page - 4;
    p[0] = 0x80; p[1] = 0x01; p[2] = 0xfe; p[3] = 0xdc;
    if (read_u32_wide(p) != UINT64_C(0x8001fedc)) return 8;
    if (read_u16(p + 2) != 0xfedcu || read_s16(p + 2) != -292) return 9;
    if (read_u16_wide(p + 2) != 0xfedcu || read_volatile(p + 2) != 0xfedcu)
        return 10;
    if (munmap(mapping, (size_t)page * 2u) != 0) return 11;
    puts("narrow_big_endian_fields: PASS");
    return 0;
}
