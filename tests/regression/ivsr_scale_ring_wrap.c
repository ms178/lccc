/* IVSR-WRAP-2, both halves, as one executable differential.
 *
 * `buf[i * 2]` with `uint32_t i` must compute the INDEX in `unsigned int`
 * (32-bit, wrapping by definition) and only then zero-extend it for the
 * address.  Two independent transformations used to linearise that product
 * into pointer-ring arithmetic, where nothing wraps:
 *
 *   (1) the back end's SIB peel, which folded the `add(i, i)` that the
 *       canonicalizer makes of `i * 2` into `scale = 2` of a zero-extended
 *       index;
 *   (2) IVSR, whose narrow-unsigned no-wrap proof decoded the loop limit with
 *       `IrConst::to_i64` -- a `uint32_t` bound above 2^31 is stored in its own
 *       ring as a NEGATIVE `I32`, so the proof was handed a negative maximum
 *       and certified exactly the wrap it exists to refuse.
 *
 * Every index arithmetic below is unsigned and every access is inside the
 * mapping, so the program is defined C and both the wrapped and the linear
 * result are reachable: the correct sum is the wrapped one.
 *
 * 16 GiB of VIRTUAL address space would not be needed here; 4 GiB + 16 bytes
 * of index space with three committed pages is, and guard pages turn a wrong
 * access into a fault rather than a wrong sum.  Needs LP64: on ILP32 the span
 * cannot be expressed by `size_t` and the mapping collapses.
 */
#include <stdint.h>
#include <stdio.h>
#include <sys/mman.h>
#include <unistd.h>

#if UINTPTR_MAX < 0xFFFFFFFFFFFFFFFFull
int main(void) {
    puts("ivsr_scale_ring_wrap: SKIP (needs a 64-bit address space)");
    return 0;
}
#else

#define SPAN ((size_t)0x100000000ull + 16u)
#define WRAPPED_SUM 110ul
#define LINEAR_SUM 264ul

/* Constant bound: the loop is unrolled at -O2, which is the path where the
 * back-end peel applies.  (0x80000003 - 1) * 2 crosses 2^32 on the last
 * iterations, so the product wraps. */
__attribute__((noinline)) static unsigned long walk_const(const unsigned char *buf) {
    unsigned long s = 0;
    uint32_t i;
    for (i = 0x7FFFFFFFu; i < 0x80000003u; i++)
        s += (unsigned long)buf[i * 2];
    return s;
}

/* Same shape with the bound arriving as a parameter: interprocedural constant
 * propagation specialises this clone back into the constant-bound case, which
 * is the path where the limit decode above bit. */
__attribute__((noinline)) static unsigned long walk_param(const unsigned char *buf, uint32_t lim) {
    unsigned long s = 0;
    for (uint32_t i = 0x7FFFFFFFu; i < lim; i++)
        s += (unsigned long)buf[i * 2];
    return s;
}

/* The bound stays dynamic: no exact no-wrap proof exists, so neither
 * transformation may fire and the wrap must survive on its own. */
__attribute__((noinline)) static unsigned long walk_dynamic(const unsigned char *buf, unsigned k) {
    unsigned long s = 0;
    uint32_t lim = 0x80000003u + (k & 0u); /* opaque to the analyzer, not folded away */
    for (uint32_t i = 0x7FFFFFFFu; i < lim; i++)
        s += (unsigned long)buf[i * 2];
    return s;
}

int main(void) {
    size_t page = (size_t)sysconf(_SC_PAGESIZE);
    size_t len = SPAN + 2 * page;
    char *map = mmap(0, len, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1, 0);
    if (map == MAP_FAILED) {
        perror("ivsr_scale_ring_wrap: mmap");
        return 1;
    }
    unsigned char *buf = (unsigned char *)(map + page);
    if (mprotect(buf, page, PROT_READ | PROT_WRITE)) return 2;
    if (mprotect(buf + 0xFFFFF000u, page, PROT_READ | PROT_WRITE)) return 3;
    if (mprotect(buf + 0x100000000u, page, PROT_READ | PROT_WRITE)) return 4;

    /* Wrapped offsets { 0xFFFFFFFE, 0x0, 0x2, 0x4 } sum to WRAPPED_SUM. */
    buf[0xFFFFFFFEu] = 11;
    buf[0] = 22;
    buf[2] = 33;
    buf[4] = 44;
    /* Linear offsets { 0xFFFFFFFE, 0x100000000, +2, +4 } sum to LINEAR_SUM. */
    buf[0x100000000u] = 55;
    buf[0x100000002u] = 66;
    buf[0x100000004u] = 77;

    int bad = 0;
    struct {
        const char *name;
        unsigned long got;
    } cases[3];
    cases[0].name = "const";
    cases[0].got = walk_const(buf);
    cases[1].name = "param";
    cases[1].got = walk_param(buf, 0x80000003u);
    cases[2].name = "dynamic";
    cases[2].got = walk_dynamic(buf, (unsigned)page);

    for (int i = 0; i < 3; i++) {
        if (cases[i].got != WRAPPED_SUM) {
            printf("ivsr_scale_ring_wrap: FAIL %s: got %lu, want %lu (linear=%lu)\n",
                   cases[i].name, cases[i].got, WRAPPED_SUM, LINEAR_SUM);
            bad = 1;
        }
    }
    munmap(map, len);
    if (bad) return 5;
    puts("ivsr_scale_ring_wrap: OK");
    return 0;
}
#endif
