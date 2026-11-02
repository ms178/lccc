/* IVSR-DOMAIN-2: a SIGNED counter whose increment is performed in UNSIGNED
 * arithmetic.  C17 6.3.1.3p3 makes INT_MAX -> INT_MIN here implementation-
 * defined, NOT undefined, so the "signed overflow is UB" theorem that licenses
 * linear pointer recurrences does not apply.  The mathematical index sequence
 * 2147483646, 2147483647, -2147483648, -2147483647 is not linear, while the
 * bit patterns are consecutive: recognising the backedge through the
 * same-width signedness cast and bumping a pointer instead walks off the end
 * of the object.  16 GiB of VIRTUAL address space, four committed pages;
 * guard pages turn a wrong recurrence into a fault rather than a wrong sum. */
#include <stdint.h>
#include <stdio.h>
#include <sys/mman.h>
#include <unistd.h>

__attribute__((noinline)) static unsigned walk(const signed char *p, int i, int n) {
    unsigned s = 0;
    for (int k = 0; k < n; ++k) {
        s += (unsigned)(unsigned char)p[i];
        i = (int)((unsigned)i + 1u);
    }
    return s;
}

int main(void) {
#if UINTPTR_MAX < 0xFFFFFFFFFFFFFFFFull
    /* This case needs the whole 2^32-byte signed-int index range addressable at
     * once. An ILP32 process cannot express it: `(size_t)UINT32_MAX + 1` wraps
     * to 0, the span computation collapses, and the writes below go through a
     * wild pointer. Skipping LOUDLY is the correct behaviour; silently
     * computing a bogus span is what made this segfault on -m32 instead of
     * reporting a result. The LP64 run is the one that carries the proof. */
    puts("ivsr_signed_wrap_impldef: SKIP (needs a 64-bit address space)");
    return 0;
#else
    long ps = sysconf(_SC_PAGESIZE);
    size_t page = (size_t)(ps > 0 ? ps : 4096);
    size_t span = (size_t)UINT32_MAX + 1; /* 2^32 bytes, the int index range */
    size_t len = span + 2 * page;
    char *map = mmap(0, len, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1, 0);
    if (map == MAP_FAILED) {
        perror("sparse mmap");
        return 1;
    }
    signed char *p = (signed char *)(map + page + span / 2);
    if (mprotect(map + page, page, PROT_READ | PROT_WRITE) != 0) return 2;
    if (mprotect(map + page + span - page, page, PROT_READ | PROT_WRITE) != 0) return 3;
    p[INT32_MAX - 1] = (signed char)0x11;
    p[INT32_MAX] = (signed char)0x22;
    p[INT32_MIN] = (signed char)0x33;
    p[INT32_MIN + 1] = (signed char)0x44;
    unsigned r = walk(p, INT32_MAX - 1, 4);
    munmap(map, len);
    if (r != 0x11u + 0x22u + 0x33u + 0x44u) {
        printf("ivsr_signed_wrap_impldef: WRONG SUM %u\n", r);
        return 4;
    }
    puts("ivsr_signed_wrap_impldef: OK");
    return 0;
#endif
}
