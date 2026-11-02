/* Numerical oracle independent of libm: binary32 floor constructed with integer
 * masks, then widened exactly. Do not replace the routing/withdrawal fixture.
 * This checks values, signed zero and NaN classification, not NaN payloads or
 * floating-environment exception flags (which are a separate contract). */
#include <stdint.h>

extern double floor(double);

__attribute__((noinline)) static double plain(float x) { return floor(x); }
__attribute__((noinline)) static double explicit_builtin(float x) {
    return __builtin_floor(x);
}
__attribute__((noinline)) static double explicit_widen(float x) {
    return floor((double)x);
}

static float expected_floor(uint32_t bits) {
    union { uint32_t u; float f; } value = { .u = bits };
    unsigned exponent = (bits >> 23) & 255u;
    uint32_t magnitude = bits & 0x7fffffffu;
    if (exponent >= 150u || magnitude == 0)
        return value.f; /* already integral, zero, infinity or NaN */
    if (exponent < 127u)
        return (bits >> 31) ? -1.0f : 0.0f;
    uint32_t fraction = (1u << (150u - exponent)) - 1u;
    if (!(bits & fraction))
        return value.f;
    value.u = bits & ~fraction;
    if (bits >> 31)
        value.u += fraction + 1u;
    return value.f;
}

static int check(uint32_t bits) {
    union { uint32_t u; float f; } input = { .u = bits };
    double expected = (double)expected_floor(bits);
    double actual[3] = { plain(input.f), explicit_builtin(input.f), explicit_widen(input.f) };
    for (unsigned i = 0; i < 3; ++i) {
        if ((bits & 0x7fffffffu) > 0x7f800000u) {
            if (!__builtin_isnan(actual[i]))
                return 1;
        } else {
            if (actual[i] != expected)
                return 2;
            if (expected == 0.0) {
                union { double d; uint64_t u; } a = { .d = actual[i] }, e = { .d = expected };
                if (a.u != e.u)
                    return 3;
            }
        }
    }
    return 0;
}

int main(void) {
    static const uint32_t boundaries[] = {
        0, 1, 0x007fffff, 0x00800000, 0x3effffff, 0x3f000000,
        0x3f7fffff, 0x3f800000, 0x3f800001, 0x3fc00000, 0x3fffffff,
        0x4affffff, 0x4b000000, 0x4b000001, 0x4b7fffff, 0x4b800000,
        0x4b800001, 0x7f7fffff, 0x7f800000, 0x7fc00000, 0x7fc12345
    };
    for (unsigned i = 0; i < sizeof(boundaries) / sizeof(boundaries[0]); ++i) {
        if (check(boundaries[i]) || check(boundaries[i] | 0x80000000u))
            return 1;
    }
    /* Deterministic coverage of signs, exponents and mantissas. Quiet any NaN
     * so the test does not accidentally make an exception-behavior claim. */
    uint32_t state = 0x12345678u;
    for (unsigned i = 0; i < 65536; ++i) {
        state = state * 1664525u + 1013904223u;
        uint32_t bits = state;
        if ((bits & 0x7fffffffu) > 0x7f800000u)
            bits |= 0x00400000u;
        if (check(bits))
            return 2;
    }
    return 0;
}
