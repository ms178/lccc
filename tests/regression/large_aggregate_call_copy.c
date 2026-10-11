/* Exact-size, bounded by-value argument copies, including two arguments,
   surviving scalar/register arguments, over-aligned local sources, and a
   source ending immediately before an inaccessible page. The latter catches
   reads of rounded-up ABI padding outside the actual C object.
   CALLER_ONLY / CALLEE_ONLY permit two-way GCC ABI comparisons. */
#include <stdint.h>
#ifndef BLOB_BYTES
#define BLOB_BYTES 2049
#endif
struct blob { unsigned char data[BLOB_BYTES]; };
#define RP __attribute__((noinline, regparm(3)))
#define FC __attribute__((noinline, fastcall))
#define NI __attribute__((noinline))

NI uint64_t mixed(unsigned, struct blob, uint64_t, double, unsigned, struct blob, unsigned);
RP uint64_t registers(unsigned, struct blob, unsigned, uint64_t);
FC uint64_t fast(unsigned, struct blob, unsigned);

#ifndef CALLER_ONLY
static __attribute__((noinline)) uint64_t checksum(const struct blob *p)
{
    unsigned i;
    uint64_t sum = 0;
    for (i = 0; i < BLOB_BYTES; ++i) sum += p->data[i];
    return sum;
}
NI uint64_t mixed(unsigned a, struct blob b, uint64_t c, double d, unsigned e, struct blob f, unsigned g)
{
    return a * 3U + checksum(&b) + c + (uint64_t)d + e * 7U + checksum(&f) * 11U + g;
}
RP uint64_t registers(unsigned a, struct blob b, unsigned c, uint64_t d)
{
    return a * 5U + checksum(&b) + c * 13U + d;
}
FC uint64_t fast(unsigned a, struct blob b, unsigned c)
{
    return a * 17U + checksum(&b) + c * 19U;
}
#endif

#ifndef CALLEE_ONLY
#include <sys/mman.h>
#include <unistd.h>
static volatile unsigned input = 7;
static uint64_t (*volatile indirect)(unsigned, struct blob, uint64_t, double, unsigned, struct blob, unsigned) = mixed;
int main(void)
{
    long page = sysconf(_SC_PAGESIZE);
    size_t span;
    unsigned char *mapping;
    struct blob *guarded;
    struct blob local __attribute__((aligned(64)));
    unsigned a = input, i;
    uint64_t sum = 0, other = 0, wide = UINT64_C(0x123456780000009b);
    if (page <= 0) return 90;
    span = ((BLOB_BYTES + (size_t)page - 1) / (size_t)page) * (size_t)page;
    mapping = mmap(0, span + (size_t)page, PROT_READ | PROT_WRITE,
                   MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (mapping == MAP_FAILED) return 91;
    if (mprotect(mapping + span, (size_t)page, PROT_NONE)) return 92;
    guarded = (struct blob *)(mapping + span - BLOB_BYTES);
    for (i = 0; i < BLOB_BYTES; ++i) {
        unsigned char x = (unsigned char)(i * 13U + a);
        unsigned char y = (unsigned char)(i * 29U + a + 1U);
        guarded->data[i] = x;
        local.data[i] = y;
        sum += x;
        other += y;
    }
    if (mixed(a, *guarded, wide, 8.5, 13, local, 17) !=
        a * 3U + sum + wide + 8U + 13U * 7U + other * 11U + 17U) return 1;
    if (registers(a, *guarded, 13, wide) != a * 5U + sum + 13U * 13U + wide) return 2;
    if (fast(a, *guarded, 13) != a * 17U + sum + 13U * 19U) return 3;
    if (indirect(a, *guarded, wide, 8.5, 13, local, 17) !=
        a * 3U + sum + wide + 8U + 13U * 7U + other * 11U + 17U) return 4;
    if (munmap(mapping, span + (size_t)page)) return 93;
    return 0;
}
#endif
