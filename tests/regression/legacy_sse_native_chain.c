/* Native public packed arithmetic, nested values and four-vector ABI pressure.
 * Also compiled as separate caller/provider objects by check_legacy_sse_native.py. */
#include <immintrin.h>
extern int printf(const char *, ...);

#ifdef USE_AVX
#define F __m256
#define D __m256d
#define ADD_F _mm256_add_ps
#define SUB_F _mm256_sub_ps
#define MUL_F _mm256_mul_ps
#define ADD_D _mm256_add_pd
#define SUB_D _mm256_sub_pd
#define MUL_D _mm256_mul_pd
#else
#define F __m128
#define D __m128d
#define ADD_F _mm_add_ps
#define SUB_F _mm_sub_ps
#define MUL_F _mm_mul_ps
#define ADD_D _mm_add_pd
#define SUB_D _mm_sub_pd
#define MUL_D _mm_mul_pd
#endif

unsigned packed_bits(float);
float packed_from_bits(unsigned);
F packed_f(F, F, F, F);
#ifndef SSE1_ONLY
D packed_d(D, D, D, D);
#endif

#ifndef CALLER_ONLY
__attribute__((noinline)) unsigned packed_bits(float f) {
    union { float f; unsigned u; } a = { .f = f };
    return a.u;
}
__attribute__((noinline)) float packed_from_bits(unsigned u) {
    union { float f; unsigned u; } a = { .u = u };
    return a.f;
}
__attribute__((noinline)) F packed_f(F a, F b, F c, F d) {
    return ADD_F(SUB_F(MUL_F(a, b), c), d);
}
#ifndef SSE1_ONLY
__attribute__((noinline)) D packed_d(D a, D b, D c, D d) {
    return ADD_D(SUB_D(MUL_D(a, b), c), d);
}
#endif
#endif

#ifndef PROVIDER_ONLY
int main(void) {
    unsigned long long hash = 0x123456789abcdef0ULL;
    F (* volatile indirect_f)(F, F, F, F) = packed_f;
#ifndef SSE1_ONLY
    D (* volatile indirect_d)(D, D, D, D) = packed_d;
#endif
    for (int i = 0; i < 67; ++i) {
        F a, b, c, d;
        for (unsigned j = 0; j < sizeof(F) / sizeof(float); ++j) {
            unsigned raw = (i & 1 ? 0x80000000u : 0u) | ((i % 4 == 0) ? j : 0x3f000000u + 65537u * j);
            if (packed_bits(packed_from_bits(raw)) != raw) return 3;
            __builtin_memcpy((unsigned char *)&a + 4 * j, &raw, 4);
            b[j] = (float)(i - 33) * 0.125f;
            c[j] = (float)j * 0.0625f;
            d[j] = (i & 1) ? -0.0f : 0.0f;
        }
        F direct = packed_f(a, b, c, d);
        F indirect = indirect_f(a, b, c, d);
        unsigned bits[sizeof(F) / 4], other[sizeof(F) / 4];
        __builtin_memcpy(bits, &direct, sizeof(F));
        __builtin_memcpy(other, &indirect, sizeof(F));
        for (unsigned j = 0; j < sizeof(F) / 4; ++j) {
            if (bits[j] != other[j]) return 1;
            hash = (hash ^ bits[j]) * 1099511628211ULL;
        }
#ifndef SSE1_ONLY
        D x, y, z, w;
        for (unsigned j = 0; j < sizeof(D) / sizeof(double); ++j) {
            x[j] = (double)(i - 33) * 0.0625;
            y[j] = (double)(j + 1) * 0.125;
            z[j] = (i & 1) ? -0.0 : 0.0;
            w[j] = (double)j * -0.03125;
        }
        D dr = packed_d(x, y, z, w);
        D ir = indirect_d(x, y, z, w);
        unsigned long long db[sizeof(D) / 8], ib[sizeof(D) / 8];
        __builtin_memcpy(db, &dr, sizeof(D));
        __builtin_memcpy(ib, &ir, sizeof(D));
        for (unsigned j = 0; j < sizeof(D) / 8; ++j) {
            if (db[j] != ib[j]) return 2;
            hash = (hash ^ db[j]) * 1099511628211ULL;
        }
#endif
    }
    printf("%016llx\n", hash);
    return 0;
}
#endif
