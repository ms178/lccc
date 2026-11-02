/* Scalar sqrt may not depend on an unrelated old destination. Compare the
   emitted instruction with independent, indirect libm calls, including flags
   and every rounding mode. Keep x live after sqrt to need a distinct home. */
#include <math.h>
#include <fenv.h>
#include <stdint.h>
#include <stdio.h>

static double (*volatile reference64)(double) = sqrt;
static float (*volatile reference32)(float) = sqrtf;

__attribute__((noinline)) double root64(double x, double scale) {
    return __builtin_sqrt(x) * scale + x;
}
__attribute__((noinline)) float root32(float x, float scale) {
    return __builtin_sqrtf(x) * scale + x;
}
__attribute__((noinline)) double oracle64(double x, double scale) {
    return reference64(x) * scale + x;
}
__attribute__((noinline)) float oracle32(float x, float scale) {
    return reference32(x) * scale + x;
}
static uint64_t canonical64(double x) {
    union { double f; uint64_t u; } b = {x};
    if ((b.u & UINT64_C(0x7ff0000000000000)) == UINT64_C(0x7ff0000000000000)
        && (b.u & UINT64_C(0x000fffffffffffff))) return UINT64_C(0x7ff8000000000000);
    return b.u;
}
static uint32_t canonical32(float x) {
    union { float f; uint32_t u; } b = {x};
    if ((b.u & 0x7f800000u) == 0x7f800000u && (b.u & 0x007fffffu)) return 0x7fc00000u;
    return b.u;
}
int main(void) {
    const int modes[] = {FE_TONEAREST, FE_DOWNWARD, FE_UPWARD, FE_TOWARDZERO};
    const uint64_t edge64[] = {0, UINT64_C(0x8000000000000000), 1,
        UINT64_C(0x0010000000000000), UINT64_C(0x3ff0000000000000),
        UINT64_C(0x4000000000000000), UINT64_C(0x7fefffffffffffff),
        UINT64_C(0x7ff0000000000000), UINT64_C(0xfff0000000000000),
        UINT64_C(0x7ff8000000001234), UINT64_C(0x7ff0000000001234),
        UINT64_C(0xbff0000000000000)};
    const uint32_t edge32[] = {0, 0x80000000u, 1, 0x00800000u, 0x3f800000u,
        0x40000000u, 0x7f7fffffu, 0x7f800000u, 0xff800000u,
        0x7fc01234u, 0x7f801234u, 0xbf800000u};
    for (int mode = 0; mode < 4; ++mode) {
        if (fesetround(modes[mode])) return 2;
        uint64_t seed = UINT64_C(0x123456789abcdef);
        for (int i = 0; i < 2060; ++i) {
            seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17;
            union { double f; uint64_t u; } d;
            union { float f; uint32_t u; } f;
            d.u = i < 12 ? edge64[i] : seed & UINT64_C(0x7fffffffffffffff);
            f.u = i < 12 ? edge32[i] : (uint32_t)seed & 0x7fffffffu;
            feclearexcept(FE_ALL_EXCEPT);
            uint64_t a = canonical64(root64(d.f, 0.75));
            int af = fetestexcept(FE_ALL_EXCEPT);
            feclearexcept(FE_ALL_EXCEPT);
            uint64_t b = canonical64(oracle64(d.f, 0.75));
            int bf = fetestexcept(FE_ALL_EXCEPT);
            if (a != b || af != bf) { printf("double %d %d %d %d\n", mode, i, af, bf); return 1; }
            feclearexcept(FE_ALL_EXCEPT);
            uint32_t c = canonical32(root32(f.f, 0.75f));
            int cf = fetestexcept(FE_ALL_EXCEPT);
            feclearexcept(FE_ALL_EXCEPT);
            uint32_t e = canonical32(oracle32(f.f, 0.75f));
            int ef = fetestexcept(FE_ALL_EXCEPT);
            if (c != e || cf != ef) { printf("float %d %d %d %d\n", mode, i, cf, ef); return 1; }
        }
    }
    return fesetround(FE_TONEAREST) != 0;
}
