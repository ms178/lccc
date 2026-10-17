/* Common GCC -O0 driver. Compare optimized kernels to real volatile division,
 * not a reference compiler's constant-folded reciprocal. Check exact result
 * bits and all six SSE exception flags under four rounding modes and every
 * FTZ/DAZ combination. NaN input payloads include signaling NaNs.
 */
#pragma STDC FENV_ACCESS ON
#include <fenv.h>
#include <stdint.h>
#include <stdio.h>
#include <xmmintrin.h>
#define DECL64(n) double n(double);
#define DECL32(n) float n(float);
DECL64(div64_p256) DECL64(div64_n256) DECL64(div64_half) DECL64(div64_nhalf)
DECL64(div64_one) DECL64(div64_none) DECL64(div64_large) DECL64(div64_small)
DECL64(reject64_huge) DECL64(reject64_tiny) DECL64(reject64_three)
DECL32(div32_p256) DECL32(div32_n256) DECL32(div32_half) DECL32(div32_nhalf)
DECL32(div32_one) DECL32(div32_none) DECL32(div32_large) DECL32(div32_small)
DECL32(reject32_huge) DECL32(reject32_tiny) DECL32(reject32_three)

struct case64 { double (*fn)(double); double divisor; };
struct case32 { float (*fn)(float); float divisor; };
static const struct case64 c64[] = {
    {div64_p256,256.0}, {div64_n256,-256.0}, {div64_half,0.5}, {div64_nhalf,-0.5},
    {div64_one,1.0}, {div64_none,-1.0}, {div64_large,0x1p1022}, {div64_small,0x1p-1022},
    {reject64_huge,0x1p1023}, {reject64_tiny,0x1p-1074}, {reject64_three,3.0}
};
static const struct case32 c32[] = {
    {div32_p256,256.0f}, {div32_n256,-256.0f}, {div32_half,0.5f}, {div32_nhalf,-0.5f},
    {div32_one,1.0f}, {div32_none,-1.0f}, {div32_large,0x1p126f}, {div32_small,0x1p-126f},
    {reject32_huge,0x1p127f}, {reject32_tiny,0x1p-149f}, {reject32_three,3.0f}
};
static const uint64_t bits64[] = {
    0, UINT64_C(0x8000000000000000), 1, UINT64_C(0x8000000000000001),
    UINT64_C(0x000fffffffffffff), UINT64_C(0x0010000000000000),
    UINT64_C(0x3ff0000000000001), UINT64_C(0xbff0000000000001),
    UINT64_C(0x7fefffffffffffff), UINT64_C(0xffefffffffffffff),
    UINT64_C(0x7ff0000000000000), UINT64_C(0xfff0000000000000),
    UINT64_C(0x7ff8000012345678), UINT64_C(0x7ff0000012345678)
};
static const uint32_t bits32[] = {
    0, 0x80000000u, 1, 0x80000001u, 0x007fffffu, 0x00800000u,
    0x3f800001u, 0xbf800001u, 0x7f7fffffu, 0xff7fffffu,
    0x7f800000u, 0xff800000u, 0x7fc01234u, 0x7f801234u
};
#define COUNT(a) (sizeof(a)/sizeof((a)[0]))
int main(void)
{
    unsigned saved = _mm_getcsr(), checks = 0;
    int saved_round = fegetround();
    const int modes[] = {FE_TONEAREST, FE_DOWNWARD, FE_UPWARD, FE_TOWARDZERO};
    for (unsigned r = 0; r < COUNT(modes); ++r) {
        if (fesetround(modes[r])) return 2;
        unsigned base = _mm_getcsr() & ~((1u<<15) | (1u<<6) | 63u);
        for (unsigned flush = 0; flush < 4; ++flush) {
            unsigned control = base | ((flush & 1u) << 15) | ((flush >> 1) << 6);
            for (unsigned k = 0; k < COUNT(c64); ++k) {
                volatile double divisor = c64[k].divisor;
                for (unsigned i = 0; i < COUNT(bits64); ++i) {
                    union { double f; uint64_t u; } x, got, ref;
                    x.u = bits64[i];
                    _mm_setcsr(control);
                    got.f = c64[k].fn(x.f);
                    unsigned gf = _mm_getcsr() & 63u;
                    _mm_setcsr(control);
                    ref.f = x.f / divisor;
                    unsigned rf = _mm_getcsr() & 63u;
                    if (got.u != ref.u || gf != rf) {
                        printf("FAIL64 r=%u flush=%u k=%u i=%u got=%llx/%u ref=%llx/%u\n",
                               r, flush, k, i, (unsigned long long)got.u, gf, (unsigned long long)ref.u, rf);
                        return 1;
                    }
                    ++checks;
                }
            }
            for (unsigned k = 0; k < COUNT(c32); ++k) {
                volatile float divisor = c32[k].divisor;
                for (unsigned i = 0; i < COUNT(bits32); ++i) {
                    union { float f; uint32_t u; } x, got, ref;
                    x.u = bits32[i];
                    _mm_setcsr(control);
                    got.f = c32[k].fn(x.f);
                    unsigned gf = _mm_getcsr() & 63u;
                    _mm_setcsr(control);
                    ref.f = x.f / divisor;
                    unsigned rf = _mm_getcsr() & 63u;
                    if (got.u != ref.u || gf != rf) {
                        printf("FAIL32 r=%u flush=%u k=%u i=%u got=%x/%u ref=%x/%u\n", r, flush, k, i, got.u, gf, ref.u, rf);
                        return 1;
                    }
                    ++checks;
                }
            }
        }
    }
    fesetround(saved_round);
    _mm_setcsr(saved);
    printf("POW2_PASS %u\n", checks);
    return 0;
}
