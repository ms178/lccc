/* BMI2 BZHI edge cases: `_bzhi_u32`/`_bzhi_u64` read only bits 7:0 of the
 * index and return the source unchanged from the operand width on.  Every
 * result is printed so a mismatch names its case.  `vbzhi32`/`vbzhi64` keep
 * both operands opaque (one `bzhi` each with BMI2); the `const_*` functions
 * give the simplifier a constant index and the constant folder two constant
 * operands, so all three evaluation paths are compared with the oracle. */
#include <immintrin.h>
#include <stdio.h>

static const unsigned long long values[] = {
    0x0ULL, 0x1ULL, 0x80000000ULL, 0x80000001ULL, 0xffffffffULL,
    0x8000000000000000ULL, 0xffffffffffffffffULL, 0x0123456789abcdefULL,
    0xfedcba9876543210ULL,
};

static const unsigned long long indices[] = {
    0, 1, 7, 8, 15, 16, 31, 32, 33, 63, 64, 65, 127, 128, 200, 255, 256, 257,
    0x1ff, 0x120, 0x13f, 0x140, 0x80000000ULL, 0x100000020ULL,
    0x8000000000000005ULL, 0xffffffffffffffffULL, 0xffffffffffffff1fULL,
};

__attribute__((noinline)) unsigned long long vbzhi64(unsigned long long x, unsigned long long n)
{
    return _bzhi_u64(x, n);
}

__attribute__((noinline)) unsigned vbzhi32(unsigned x, unsigned n)
{
    return _bzhi_u32(x, n);
}

/* Index known, source opaque: Bzhi(x, k) simplifies to an `and`, a copy of
 * x (k >= W after `& 0xFF`) or zero (k == 0). */
__attribute__((noinline)) unsigned long long const_index64(unsigned long long x)
{
    return _bzhi_u64(x, 0) ^ (_bzhi_u64(x, 1) << 1) ^ (_bzhi_u64(x, 40) << 2)
           ^ (_bzhi_u64(x, 63) << 3) ^ (_bzhi_u64(x, 64) << 4) ^ (_bzhi_u64(x, 256 + 12) << 5)
           ^ (_bzhi_u64(x, 0xffffffffffffff3fULL) << 6);
}

__attribute__((noinline)) unsigned const_index32(unsigned x)
{
    return _bzhi_u32(x, 0) ^ (_bzhi_u32(x, 1) << 1) ^ (_bzhi_u32(x, 20) << 2)
           ^ (_bzhi_u32(x, 31) << 3) ^ (_bzhi_u32(x, 32) << 4) ^ (_bzhi_u32(x, 0x11f) << 5)
           ^ (_bzhi_u32(x, 0xffffff40U) << 6);
}

/* Both operands constant: folded at compile time. */
static unsigned long long const_both(void)
{
    return _bzhi_u64(0xfedcba9876543210ULL, 36) + _bzhi_u64(0x8000000000000001ULL, 64)
           + _bzhi_u64(0xffffffffffffffffULL, 0x100) + _bzhi_u32(0x80000001U, 31)
           + _bzhi_u32(0xffffffffU, 33) + _bzhi_u32(0xdeadbeefU, 0x210);
}

int main(void)
{
    unsigned nv = sizeof values / sizeof values[0];
    unsigned ni = sizeof indices / sizeof indices[0];
    for (unsigned i = 0; i < nv; i++) {
        for (unsigned j = 0; j < ni; j++) {
            unsigned long long x = values[i], n = indices[j];
            printf("64 %016llx %016llx %016llx\n", x, n, vbzhi64(x, n));
            printf("32 %08x %08x %08x\n", (unsigned)x, (unsigned)n, vbzhi32((unsigned)x, (unsigned)n));
        }
        printf("k64 %016llx %016llx\n", values[i], const_index64(values[i]));
        printf("k32 %08x %08x\n", (unsigned)values[i], const_index32((unsigned)values[i]));
    }
    printf("both %016llx\n", const_both());
    return 0;
}
