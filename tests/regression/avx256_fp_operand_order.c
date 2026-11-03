/* Non-kernel AVX builtin fixture: MIN/MAX are not commutative for NaNs or
 * equal signed zeros. Check every lane's bits, not a floating checksum that
 * could hide zeros/NaNs. No target kernel code or headers are used here. */
#include <immintrin.h>

typedef float vf8 __attribute__((vector_size(32)));
typedef double vd4 __attribute__((vector_size(32)));
union fbits { vf8 v; unsigned u[8]; };
union dbits { vd4 v; unsigned long long u[4]; };

__attribute__((noinline)) static vf8 minf(vf8 a, vf8 b) {
    return __builtin_ia32_minps256(a, b);
}
__attribute__((noinline)) static vf8 maxf(vf8 a, vf8 b) {
    return __builtin_ia32_maxps256(a, b);
}
__attribute__((noinline)) static vd4 mind(vd4 a, vd4 b) {
    return _mm256_min_pd(a, b);
}
__attribute__((noinline)) static vd4 maxd(vd4 a, vd4 b) {
    return _mm256_max_pd(a, b);
}

int main(void) {
    /* Equal zeros and a NaN in each source position must return source two.
     * Include distinct NaN payloads and both infinities. */
    union fbits a = {.u = {0, 0x80000000u, 0x7fc00123u, 0x3f800000u,
                           0x7fc00456u, 0x7f800000u, 0xff800000u, 0x40000000u}};
    union fbits b = {.u = {0x80000000u, 0, 0x3f800000u, 0x7fc00789u,
                           0x7fc00abcu, 0xff800000u, 0x7f800000u, 0xbf800000u}};
    union fbits lo, hi;
    lo.v = minf(a.v, b.v); hi.v = maxf(a.v, b.v);
    for (int i = 0; i < 5; ++i)
        if (lo.u[i] != b.u[i] || hi.u[i] != b.u[i]) return 1 + i;
    if (lo.u[5] != b.u[5] || hi.u[5] != a.u[5] ||
        lo.u[6] != a.u[6] || hi.u[6] != b.u[6] ||
        lo.u[7] != b.u[7] || hi.u[7] != a.u[7]) return 9;

    for (int start = 0; start < 2; ++start) {
        union dbits x, y, l, h;
        x.u[0] = start ? 0x8000000000000000ull : 0;
        y.u[0] = start ? 0 : 0x8000000000000000ull;
        x.u[1] = 0x7ff8000000000123ull; y.u[1] = 0x3ff0000000000000ull;
        x.u[2] = 0x3ff0000000000000ull; y.u[2] = 0x7ff8000000000456ull;
        x.u[3] = 0x7ff8000000000789ull; y.u[3] = 0x7ff8000000000abcull;
        l.v = mind(x.v, y.v); h.v = maxd(x.v, y.v);
        for (int i = 0; i < 4; ++i)
            if (l.u[i] != y.u[i] || h.u[i] != y.u[i]) return 10 + i;
    }
    return 0;
}
