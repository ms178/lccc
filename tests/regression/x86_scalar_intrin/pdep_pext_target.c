/* PDEP/PEXT where the caller's own target forbids BMI2.
 *
 * bmi2intrin.h is compiled under the translation unit's ISA and inlined
 * into every caller. Under -mbmi2, a caller declared
 * target("arch=x86-64") must still not execute `pdep`/`pext` (#UD on CPUs
 * before Haswell and Excavator): lccc computes the same value with a
 * portable sequence there. GCC rejects that call outright, so its oracle
 * is this file built with -DTARGET_BASELINE= (BMI2 everywhere).
 */
#include <immintrin.h>
#include <stdio.h>

#ifndef TARGET_BASELINE
#define TARGET_BASELINE __attribute__((target("arch=x86-64")))
#endif

TARGET_BASELINE __attribute__((noinline)) unsigned base_dep32(unsigned s, unsigned m)
{
    return _pdep_u32(s, m);
}
TARGET_BASELINE __attribute__((noinline)) unsigned base_ext32(unsigned s, unsigned m)
{
    return _pext_u32(s, m);
}
__attribute__((noinline)) unsigned dep32(unsigned s, unsigned m) { return _pdep_u32(s, m); }
#ifndef __x86_64__ /* GCC has _mulx_u32 on i386 only (lccc and Clang: both) */
__attribute__((noinline)) unsigned mulx32(unsigned a, unsigned b, unsigned *hi)
{
    return _mulx_u32(a, b, hi);
}
#endif
__attribute__((noinline)) unsigned ext32(unsigned s, unsigned m) { return _pext_u32(s, m); }

#ifdef __x86_64__
TARGET_BASELINE __attribute__((noinline)) unsigned long long
base_dep64(unsigned long long s, unsigned long long m)
{
    return _pdep_u64(s, m);
}
TARGET_BASELINE __attribute__((noinline)) unsigned long long
base_ext64(unsigned long long s, unsigned long long m)
{
    return _pext_u64(s, m);
}
__attribute__((noinline)) unsigned long long dep64(unsigned long long s, unsigned long long m)
{
    return _pdep_u64(s, m);
}
__attribute__((noinline)) unsigned long long ext64(unsigned long long s, unsigned long long m)
{
    return _pext_u64(s, m);
}
__attribute__((noinline)) unsigned long long
mulx64(unsigned long long a, unsigned long long b, unsigned long long *hi)
{
    return _mulx_u64(a, b, hi);
}
#endif

static unsigned long long state = 0x9E3779B97F4A7C15ULL;
static unsigned long long next(void)
{
    state = state * 6364136223846793005ULL + 1442695040888963407ULL;
    return state ^ (state >> 29);
}

int main(void)
{
    static const unsigned long long edge[] = {
        0, 1, 2, 3, 0x80000000ULL, 0xFFFFFFFFULL, 0x55555555ULL, 0xAAAAAAAAULL,
        0x0F0F0F0FULL, 0x80000001ULL, 0x8000000000000000ULL, 0xFFFFFFFFFFFFFFFFULL,
        0x5555555555555555ULL, 0xAAAAAAAAAAAAAAAAULL, 0x00000001FFFFFFFFULL,
        0xFFFFFFFF00000000ULL, 0x8000000000000001ULL, 0x0123456789ABCDEFULL,
    };
    const unsigned n = sizeof edge / sizeof edge[0];
    unsigned long long sum = 0;
    for (unsigned i = 0; i < n + 64; i++) {
        for (unsigned j = 0; j < n + 64; j++) {
            unsigned long long s = i < n ? edge[i] : next();
            unsigned long long m = j < n ? edge[j] : next();
            unsigned r[4] = {base_dep32(s, m), base_ext32(s, m), dep32(s, m), ext32(s, m)};
            sum = sum * 31 + r[0] + 7 * r[1];
#ifndef __x86_64__
            unsigned hi32, lo32 = mulx32(s, m, &hi32);
            sum = sum * 31 + lo32 + 3 * (unsigned long long)hi32;
#endif
            if (r[0] != r[2] || r[1] != r[3]) {
                printf("32-bit mismatch s=%#llx m=%#llx\n", s, m);
                return 1;
            }
#ifdef __x86_64__
            unsigned long long q[4] = {base_dep64(s, m), base_ext64(s, m), dep64(s, m), ext64(s, m)};
            sum = sum * 31 + q[0] + 7 * q[1];
            unsigned long long hi64, lo64 = mulx64(s, m, &hi64);
            sum = sum * 31 + lo64 + 3 * hi64;
            if (q[0] != q[2] || q[1] != q[3]) {
                printf("64-bit mismatch s=%#llx m=%#llx\n", s, m);
                return 1;
            }
#endif
        }
    }
    printf("%016llx\n", sum);
    return 0;
}
