/*  Byte-exact by-value aggregates whose *last eightbyte is partial*, laid out so
 *  that any read beyond the end of the object lands on a PROT_NONE page.
 *
 *  Why this file exists: a register-classified aggregate is copied one eightbyte
 *  at a time, and the psABI leaves the bytes past the object's end unspecified --
 *  which is a licence to omit them, not a licence to read them.  The INTEGER
 *  family was already pinned for byte arrays by large_aggregate_call_copy.c, but
 *  the SSE and mixed ([INTEGER,SSE] / [SSE,INTEGER]) classes were not: their
 *  eightbytes are staged with `movq` into XMM registers too, so a 12-byte
 *  `float[3]` argument reads 4 bytes it does not own and a 10-byte
 *  `struct { double; short; }` reads 6 past the end.  Sizes here are chosen so
 *  each class has a partial tail eightbyte:
 *
 *    struct fl3   12 B  -> [SSE, SSE]            second eightbyte 4 B of 8
 *    struct ds     9 B  -> [SSE, INTEGER]        second eightbyte 1 B of 8
 *    struct sbi    7 B  -> [INTEGER]             single  7 B of 8
 *    struct u13   13 B  -> [INTEGER, INTEGER]     second eightbyte 5 B of 8
 *
 *  Neighbouring scalars ride in the registers between the aggregates: if a class
 *  is skipped or shifted, the digest below changes even when the padding bytes
 *  themselves happen to look right, and a wrong argument *width* (not just extra
 *  bytes) shows up as a mismatch instead of a fault.
 *
 *  Compile with -DCALLER_ONLY (main + the checks) or -DCALLEE_ONLY (the callee
 *  side) and link one of each, in any combination, so a caller and a callee built
 *  by different compilers have to agree.  Returns 0 on success.
 */
#include <stdint.h>
#include <stddef.h>

struct fl3 {
    float v[3];
};
/* `packed` is load-bearing, not decoration: an unpadded-free `struct { double d;
 * short s; }` is 16 bytes with 6 bytes of trailing padding, which (a) makes the
 * tail eightbyte whole, so it probes nothing, and (b) would put *unspecified*
 * bytes into the digest below and let the two compilers disagree by design.  With
 * `packed` the object is 9 bytes: [SSE, INTEGER] with one defined byte in the
 * second eightbyte -- exactly the shape that makes an 8-byte load out of bounds. */
struct ds {
    double d;
    short s;
} __attribute__((packed));
struct sbi {
    unsigned char b[7];
};
struct u13 {
    unsigned char b[13];
};

#define TAG_A 0x9e3779b97f4a7c15ull
#define TAG_B 0x0123456789abcdefull

#ifndef CALLER_ONLY
static uint64_t fold(const volatile unsigned char *p, size_t n, uint64_t acc)
{
    for (size_t i = 0; i < n; ++i)
        acc = (acc + p[i]) * 1099511628211ull;
    return acc;
}

uint64_t probe(unsigned a, struct fl3 f, double x, struct ds d, uint64_t y,
               struct sbi s, unsigned z, struct u13 u, uint64_t w)
{
    uint64_t acc = TAG_A ^ (uint64_t)a ^ y ^ (uint64_t)z ^ w;
    /* Only the object's own bytes are examined -- never a rounded-up width. */
    acc = fold((const volatile unsigned char *)&f, sizeof f, acc);
    acc = fold((const volatile unsigned char *)&d, sizeof d, acc);
    acc = fold((const volatile unsigned char *)&s, sizeof s, acc);
    acc = fold((const volatile unsigned char *)&u, sizeof u, acc);
    /* Scalars must arrive unharmed; a partial-eightbyte fix that shifted a
     * register would be caught here rather than in the padding. */
    uint64_t xb;
    double xd = x;
    for (xb = 0; xb < 8; ++xb)
        acc = acc * 31u + ((const unsigned char *)&xd)[xb];
    return acc;
}
#endif

#ifndef CALLEE_ONLY
#include <sys/mman.h>
#include <unistd.h>

uint64_t probe(unsigned, struct fl3, double, struct ds, uint64_t, struct sbi,
               unsigned, struct u13, uint64_t);

/* A `T` living at the very end of a readable page, with the next page
 * PROT_NONE: an over-read of even one byte past `sizeof T` faults. */
#define PLACE(dst, T, init)                                                                    \
    do {                                                                                       \
        size_t pg = (size_t)page;                                                              \
        size_t span = ((sizeof(T) + pg - 1) / pg) * pg;                                       \
        unsigned char *m = mmap(0, span + pg, PROT_READ | PROT_WRITE,                          \
                                MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);                           \
        if (m == MAP_FAILED)                                                                    \
            return 91;                                                                          \
        if (mprotect(m + span, pg, PROT_NONE))                                                  \
            return 92;                                                                          \
        T *p = (T *)(m + span - sizeof(T));                                                    \
        *p = init;                                                                              \
        dst = p;                                                                                \
    } while (0)

int main(void)
{
    long page = sysconf(_SC_PAGESIZE);
    if (page <= 0)
        return 90;
    struct fl3 f0 = { { 1.5f, -2.25f, 8.0f } };
    struct ds d0 = { 3.75, 511 };
    struct sbi s0 = { { 1, 2, 3, 4, 5, 6, 7 } };
    struct u13 u0 = { { 9, 8, 7, 6, 5, 4, 3, 2, 1, 11, 12, 13, 14 } };
    struct fl3 *f = 0;
    struct ds *d = 0;
    struct sbi *s = 0;
    struct u13 *u = 0;
    PLACE(f, struct fl3, f0);
    PLACE(d, struct ds, d0);
    PLACE(s, struct sbi, s0);
    PLACE(u, struct u13, u0);

    volatile double xd = 1.25;
    uint64_t expect = TAG_A ^ 7u ^ 0xfeedfacecafebeefull ^ 13u ^ 0x1234567890abcdefull;
    const unsigned char *pb;
    uint64_t acc = expect;
    pb = (const unsigned char *)f;
    for (size_t i = 0; i < sizeof *f; ++i)
        acc = (acc + pb[i]) * 1099511628211ull;
    pb = (const unsigned char *)d;
    for (size_t i = 0; i < sizeof *d; ++i)
        acc = (acc + pb[i]) * 1099511628211ull;
    pb = (const unsigned char *)s;
    for (size_t i = 0; i < sizeof *s; ++i)
        acc = (acc + pb[i]) * 1099511628211ull;
    pb = (const unsigned char *)u;
    for (size_t i = 0; i < sizeof *u; ++i)
        acc = (acc + pb[i]) * 1099511628211ull;
    for (size_t i = 0; i < 8; ++i)
        acc = acc * 31u + ((const unsigned char *)&xd)[i];

    uint64_t got = probe(7, *f, xd, *d, 0xfeedfacecafebeefull, *s, 13, *u,
                         0x1234567890abcdefull);
    if (got != acc)
        return 1;
    /* Same objects through an indirect call: the classifier must not depend on
     * the call being direct. */
    uint64_t (*volatile indirect)(unsigned, struct fl3, double, struct ds, uint64_t,
                                 struct sbi, unsigned, struct u13, uint64_t) = probe;
    if (indirect(7, *f, xd, *d, 0xfeedfacecafebeefull, *s, 13, *u,
                 0x1234567890abcdefull) != acc)
        return 2;
    return 0;
}
#endif
