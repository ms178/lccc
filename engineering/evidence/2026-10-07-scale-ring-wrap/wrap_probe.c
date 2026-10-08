/* Adversarial differential: a U32 loop bound >= 2^31 whose *product* wraps.
 *
 * `buf[i*2]` with `uint32_t i`: the product is unsigned int, so it wraps
 * modulo 2^32 (defined C), and the wrapped value is then zero-extended for
 * the address.  A pointer recurrence `p += 2` per iteration is only
 * equivalent while that product does not wrap.  Every index arithmetic here
 * is unsigned and every access is inside the mapping, so the program is
 * fully defined; a compiler that walks the pointer linearly instead reads a
 * different set of bytes and prints a different sum.
 */
#include <stdint.h>
#include <stdio.h>
#include <sys/mman.h>
#include <unistd.h>
#include <stdlib.h>

#define SPAN ((size_t)0x100000000ull + 16u)

__attribute__((noinline)) static unsigned long walk(const unsigned char *buf) {
    unsigned long s = 0;
    uint32_t i;
    for (i = 0x7FFFFFFFu; i < 0x80000003u; i++)
        s += (unsigned long)buf[i * 2];   /* product wraps in U32 at the top */
    return s;
}

int main(void) {
    size_t page = (size_t)sysconf(_SC_PAGESIZE);
    size_t len = SPAN + 2 * page;
    char *map = mmap(0, len, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1, 0);
    if (map == MAP_FAILED) {
        /* Shared executor VMs may refuse a 4 GiB sparse mapping; that is a
         * resource limit, not a compiler result. Say so loudly. */
        puts("wrap_probe: SKIP (no 4 GiB sparse mapping on this executor)");
        return 0;
    }
    unsigned char *buf = (unsigned char *)(map + page);
    if (mprotect(buf, page, PROT_READ | PROT_WRITE)) return 3;
    if (mprotect(buf + 0xFFFFF000u, page, PROT_READ | PROT_WRITE)) return 4;
    if (mprotect(buf + 0x100000000u, page, PROT_READ | PROT_WRITE)) return 5;

    /* Wrapped product offsets: 0xFFFFFFFE, 0x0, 0x2, 0x4 */
    buf[0xFFFFFFFEu] = 11; buf[0] = 22; buf[2] = 33; buf[4] = 44;
    /* Linear (non-wrapping) recurrence offsets: 0x100000000, +2, +4 */
    buf[0x100000000u] = 55; buf[0x100000002u] = 66; buf[0x100000004u] = 77;

    unsigned long s = walk(buf);
    printf("sum %lu  (wrapped=110, linear=264)\n", s);
    munmap(map, len);
    return 0;
}
