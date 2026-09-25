/* Deterministic differential battery for the merged FP select/copysign work.
 * Compare bit patterns, including signed zero, infinity and NaN payloads;
 * no floating comparisons or type-punned pointer aliasing. */
#include <stdint.h>
#include <stdio.h>

typedef union { uint64_t u; double d; } D;
typedef union { uint32_t u; float f; } F;
static uint64_t state = UINT64_C(0xb871239ddb73412f);
static uint64_t next(void) {
    state ^= state << 13;
    state ^= state >> 7;
    state ^= state << 17;
    return state;
}
static double dfrom(uint64_t bits) { D d; d.u = bits; return d.d; }
static uint64_t dto(double value) { D d; d.d = value; return d.u; }
static float ffrom(uint32_t bits) { F f; f.u = bits; return f.f; }
static uint32_t fto(float value) { F f; f.f = value; return f.u; }

__attribute__((noinline)) static double punned_d(double magnitude, double sign) {
    D a, b;
    a.d = magnitude; b.d = sign;
    a.u = (a.u & UINT64_C(0x7fffffffffffffff)) |
          (b.u & UINT64_C(0x8000000000000000));
    return a.d;
}
__attribute__((noinline)) static float punned_f(float magnitude, float sign) {
    F a, b;
    a.f = magnitude; b.f = sign;
    a.u = (a.u & UINT32_C(0x7fffffff)) | (b.u & UINT32_C(0x80000000));
    return a.f;
}

int main(void) {
    static const uint64_t edges64[] = {
        0, UINT64_C(0x8000000000000000), UINT64_C(0x7ff0000000000000),
        UINT64_C(0xfff0000000000000), UINT64_C(0x7ff8234512345678),
        UINT64_C(0xfffa123456789abc), UINT64_C(0x0000000000000001),
        UINT64_C(0x8000000000000001)
    };
    static const uint32_t edges32[] = {
        0, UINT32_C(0x80000000), UINT32_C(0x7f800000),
        UINT32_C(0xff800000), UINT32_C(0x7fc34567),
        UINT32_C(0xffda1234), UINT32_C(0x00000001),
        UINT32_C(0x80000001)
    };
    uint64_t check = UINT64_C(0x103214561070e21e);
    for (unsigned i = 0; i < 8192; ++i) {
        uint64_t x = i < 64 ? edges64[i & 7] : next();
        uint64_t y = i < 64 ? edges64[(i >> 3) & 7] : next();
        uint32_t a = i < 64 ? edges32[i & 7] : (uint32_t)next();
        uint32_t b = i < 64 ? edges32[(i >> 3) & 7] : (uint32_t)next();
        double dx = dfrom(x), dy = dfrom(y);
        float fa = ffrom(a), fb = ffrom(b);
        uint64_t wanted_d = (x & UINT64_C(0x7fffffffffffffff)) |
                            (y & UINT64_C(0x8000000000000000));
        uint32_t wanted_f = (a & UINT32_C(0x7fffffff)) | (b & UINT32_C(0x80000000));
        uint64_t got_d = dto(punned_d(dx, dy));
        uint32_t got_f = fto(punned_f(fa, fb));
        if (got_d != wanted_d || got_f != wanted_f) {
            printf("copysign mismatch at %u: %016llx/%016llx %08x/%08x\n",
                   i, (unsigned long long)got_d, (unsigned long long)wanted_d,
                   got_f, wanted_f);
            return 1;
        }
        /* Force a genuine memory round-trip to check exact FP select slot
         * size, signed zeros, and unmodified NaN payloads under optimization. */
        int cond = (int)((i ^ (unsigned)(x >> 60)) & 1);
        volatile double selected_d = cond ? dx : dy;
        volatile float selected_f = cond ? fa : fb;
        uint64_t sd = dto(selected_d);
        uint32_t sf = fto(selected_f);
        if (sd != (cond ? x : y) || sf != (cond ? a : b)) {
            printf("select mismatch at %u: %016llx/%016llx %08x/%08x\n",
                   i, (unsigned long long)sd,
                   (unsigned long long)(cond ? x : y),
                   sf, cond ? a : b);
            return 2;
        }
        check = (check ^ got_d ^ sd ^ got_f ^ sf) * UINT64_C(0x9e3779b97f4a7c15);
    }
    printf("%016llx\n", (unsigned long long)check);
    return 0;
}
