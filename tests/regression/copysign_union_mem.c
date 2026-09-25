/* copysign_union_mem.c — musl-style union-punned copysign through memory.

   Covers the bit_idioms transform that rewrites
     r.u64 = (a.u64 & ABS) | (b.u64 & SIGN)
   into `CopysignF64/F32` intrinsics. Must be correct for ±0.0, NaN with
   payload, and ±inf, and must preserve bit patterns (not just values).

   Prints result bits with %016llx / %08x for deterministic comparison
   against GCC.
*/
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <math.h>

static double copysign_mem_f64(double mag, double sign) {
    union { double f; uint64_t u; } a, b, r;
    a.f = mag;
    b.f = sign;
    r.u = (a.u & 0x7fffffffffffffffULL) | (b.u & 0x8000000000000000ULL);
    return r.f;
}

static float copysign_mem_f32(float mag, float sign) {
    union { float f; uint32_t u; } a, b, r;
    a.f = mag;
    b.f = sign;
    r.u = (a.u & 0x7fffffffU) | (b.u & 0x80000000U);
    return r.f;
}

static uint64_t d2u(double d) { uint64_t u; memcpy(&u, &d, sizeof u); return u; }
static uint32_t f2u(float f)  { uint32_t u; memcpy(&u, &f, sizeof u); return u; }

int main(void) {
    /* f64 cases */
    struct { double mag; double sign; uint64_t want_bits; } f64_cases[] = {
        {  1.0,  1.0, 0x3ff0000000000000ULL }, /* +1, +1 -> +1 */
        {  1.0, -1.0, 0xbff0000000000000ULL }, /* +1, -1 -> -1 */
        { -1.0,  1.0, 0x3ff0000000000000ULL }, /* -1, +1 -> +1 */
        { -1.0, -1.0, 0xbff0000000000000ULL }, /* -1, -1 -> -1 */
        {  0.0, -0.0, 0x8000000000000000ULL }, /* +0, -0 -> -0 */
        { -0.0,  0.0, 0x0000000000000000ULL }, /* -0, +0 -> +0 */
        {  INFINITY, -1.0, 0xfff0000000000000ULL }, /* inf, - -> -inf */
        { -INFINITY,  1.0, 0x7ff0000000000000ULL }, /* -inf, + -> +inf */
    };
    for (unsigned i = 0; i < sizeof(f64_cases)/sizeof(f64_cases[0]); i++) {
        double got = copysign_mem_f64(f64_cases[i].mag, f64_cases[i].sign);
        uint64_t got_bits = d2u(got);
        if (got_bits != f64_cases[i].want_bits) {
            printf("FAIL f64 case %u got %016llx want %016llx\n",
                   i, (unsigned long long)got_bits,
                   (unsigned long long)f64_cases[i].want_bits);
            return 1;
        }
    }
    /* NaN payload preservation: mag NaN with payload, sign - */
    {
        union { double f; uint64_t u; } mag, sign, want;
        mag.u = 0x7ff8000000000001ULL; /* qNaN with payload 1 */
        sign.u = 0xbff0000000000000ULL; /* -1 */
        want.u = (mag.u & 0x7fffffffffffffffULL) | (sign.u & 0x8000000000000000ULL);
        double got = copysign_mem_f64(mag.f, sign.f);
        uint64_t got_bits = d2u(got);
        if (got_bits != want.u) {
            printf("FAIL f64 nan payload got %016llx want %016llx\n",
                   (unsigned long long)got_bits, (unsigned long long)want.u);
            return 1;
        }
    }

    /* f32 cases */
    struct { float mag; float sign; uint32_t want_bits; } f32_cases[] = {
        {  1.0f,  1.0f, 0x3f800000U },
        {  1.0f, -1.0f, 0xbf800000U },
        { -1.0f,  1.0f, 0x3f800000U },
        {  0.0f, -0.0f, 0x80000000U },
        { -0.0f,  0.0f, 0x00000000U },
        {  INFINITY, -1.0f, 0xff800000U },
    };
    for (unsigned i = 0; i < sizeof(f32_cases)/sizeof(f32_cases[0]); i++) {
        float got = copysign_mem_f32(f32_cases[i].mag, f32_cases[i].sign);
        uint32_t got_bits = f2u(got);
        if (got_bits != f32_cases[i].want_bits) {
            printf("FAIL f32 case %u got %08x want %08x\n",
                   i, got_bits, f32_cases[i].want_bits);
            return 1;
        }
    }

    printf("PASS copysign_union_mem f64=%016llx f32=%08x\n",
           (unsigned long long)d2u(copysign_mem_f64(1.0, -1.0)),
           f2u(copysign_mem_f32(1.0f, -1.0f)));
    return 0;
}
