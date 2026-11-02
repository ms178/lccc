/* Ordered packed min/max, division and square root: native helper IR.
 * Raw exceptional inputs and MXCSR flags are compared to GCC by the driver. */
#include <immintrin.h>
extern int printf(const char *, ...);
#ifndef REPEATS
#define REPEATS 1
#endif
#ifdef USE_AVX
#define F __m256
#define D __m256d
#define MINF _mm256_min_ps
#define MAXF _mm256_max_ps
#define DIVF _mm256_div_ps
#define SQRTF _mm256_sqrt_ps
#define MIND _mm256_min_pd
#define MAXD _mm256_max_pd
#define DIVD _mm256_div_pd
#define SQRTD _mm256_sqrt_pd
#else
#define F __m128
#define D __m128d
#define MINF _mm_min_ps
#define MAXF _mm_max_ps
#define DIVF _mm_div_ps
#define SQRTF _mm_sqrt_ps
#define MIND _mm_min_pd
#define MAXD _mm_max_pd
#define DIVD _mm_div_pd
#define SQRTD _mm_sqrt_pd
#endif
F helpers_f(F, F, int);
#ifndef SSE1_ONLY
D helpers_d(D, D, int);
#endif
#ifndef CALLER_ONLY
__attribute__((noinline)) F helpers_f(F a, F b, int op) {
    if (op == 0) return MINF(MAXF(a,b), MINF(b,a));
    if (op == 1) return MAXF(MINF(a,b), MAXF(b,a));
    if (op == 2) return DIVF(a,b);
    return SQRTF(a);
}
#ifndef SSE1_ONLY
__attribute__((noinline)) D helpers_d(D a, D b, int op) {
    if (op == 0) return MIND(MAXD(a,b), MIND(b,a));
    if (op == 1) return MAXD(MIND(a,b), MAXD(b,a));
    if (op == 2) return DIVD(a,b);
    return SQRTD(a);
}
#endif
#endif
#ifndef PROVIDER_ONLY
static const unsigned bits_f[] = {0, 0x80000000u, 0x3f800000u, 0xbf800000u,
    0x7f800000u, 0xff800000u, 0x7fc12345u, 0xffc54321u, 0x7f812345u,
    1, 0x80000001u, 0x00800000u, 0x3eaaaaabu, 0x40800000u, 0x7f7fffffu};
#ifndef SSE1_ONLY
static const unsigned long long bits_d[] = {0, 0x8000000000000000ULL,
    0x3ff0000000000000ULL, 0xbff0000000000000ULL, 0x7ff0000000000000ULL,
    0xfff0000000000000ULL, 0x7ff8123456789abcULL, 0xfff8abcdef012345ULL,
    0x7ff0123456789abcULL, 1, 0x8000000000000001ULL, 0x0010000000000000ULL,
    0x3fd5555555555555ULL, 0x4010000000000000ULL, 0x7fefffffffffffffULL};
#endif
int main(void) {
    unsigned original;
    __asm__ volatile("stmxcsr %0" : "=m"(original));
    unsigned long long hash = 0xcbf29ce484222325ULL;
    unsigned exceptions = 0;
    F (*volatile indirect_f)(F,F,int) = helpers_f;
#ifndef SSE1_ONLY
    D (*volatile indirect_d)(D,D,int) = helpers_d;
#endif
    for (int repeat = 0; repeat < REPEATS; ++repeat)
    for (int rounding = 0; rounding < 4; ++rounding)
    for (int i = 0; i < 15; ++i)
    for (int k = 0; k < 15; ++k)
    for (int op = 0; op < 4; ++op) {
        unsigned control = 0x1f80u | ((unsigned)rounding << 13);
        __asm__ volatile("ldmxcsr %0" : : "m"(control));
        F a, b;
        for (unsigned j = 0; j < sizeof(F)/4; ++j) {
            unsigned u = bits_f[(i+j)%15], v = bits_f[(k+j)%15];
            __builtin_memcpy((char *)&a+4*j, &u, 4);
            __builtin_memcpy((char *)&b+4*j, &v, 4);
        }
        F r = helpers_f(a,b,op), indirect = indirect_f(a,b,op);
        unsigned direct_bits[sizeof(F)/4], indirect_bits[sizeof(F)/4];
        __builtin_memcpy(direct_bits, &r, sizeof(F));
        __builtin_memcpy(indirect_bits, &indirect, sizeof(F));
        for (unsigned j = 0; j < sizeof(F)/4; ++j) {
            if (direct_bits[j] != indirect_bits[j]) return 1;
            hash = (hash ^ direct_bits[j]) * 1099511628211ULL;
        }
#ifndef SSE1_ONLY
        D x, y;
        for (unsigned j = 0; j < sizeof(D)/8; ++j) {
            unsigned long long u = bits_d[(i+j)%15], v = bits_d[(k+j)%15];
            __builtin_memcpy((char *)&x+8*j, &u, 8);
            __builtin_memcpy((char *)&y+8*j, &v, 8);
        }
        D z = helpers_d(x,y,op), iz = indirect_d(x,y,op);
        unsigned long long db[sizeof(D)/8], ib[sizeof(D)/8];
        __builtin_memcpy(db, &z, sizeof(D));
        __builtin_memcpy(ib, &iz, sizeof(D));
        for (unsigned j = 0; j < sizeof(D)/8; ++j) {
            if (db[j] != ib[j]) return 2;
            hash = (hash ^ db[j]) * 1099511628211ULL;
        }
#endif
        unsigned status;
        __asm__ volatile("stmxcsr %0" : "=m"(status));
        exceptions |= status & 63u;
    }
    __asm__ volatile("ldmxcsr %0" : : "m"(original));
    printf("%llx %x\n", hash, exceptions);
    return 0;
}
#endif
