/* Inline-lowered GCC 16 half-precision math builtins (zero libm *f16 calls):
 * ceilf16, floorf16, truncf16, roundf16, roundevenf16, rintf16, nearbyintf16,
 * sqrtf16, fmaxf16, fminf16, fmaf16, nextafterf16, plus compile-time folding. */
#include <stdint.h>
#include <string.h>

static uint16_t bits_f16(_Float16 x) {
    uint16_t u;
    memcpy(&u, &x, 2);
    return u;
}

static const _Float16 c_ceil = __builtin_ceilf16(1.25f16);
static const _Float16 c_floor = __builtin_floorf16(-1.25f16);
static const _Float16 c_trunc = __builtin_truncf16(-2.75f16);
static const _Float16 c_round = __builtin_roundf16(-2.5f16);
static const _Float16 c_roundeven = __builtin_roundevenf16(2.5f16);
static const _Float16 c_sqrt = __builtin_sqrtf16(2.25f16);
static const _Float16 c_fma = __builtin_fmaf16(1.5f16, 1.5f16, -2.25f16);
static const _Float16 c_next = __builtin_nextafterf16(0.0f16, 1.0f16);
_Static_assert(sizeof(__fp16) == 2 && _Alignof(__fp16) == 2, "__fp16 layout");

int main(void) {
    if (bits_f16(c_ceil) != bits_f16(2.0f16)) return 1;
    if (bits_f16(c_floor) != bits_f16(-2.0f16)) return 2;
    if (bits_f16(c_trunc) != bits_f16(-2.0f16)) return 3;
    if (bits_f16(c_round) != bits_f16(-3.0f16)) return 4;
    if (bits_f16(c_roundeven) != bits_f16(2.0f16)) return 5;
    if (bits_f16(c_sqrt) != bits_f16(1.5f16)) return 6;
    if (bits_f16(c_fma) != 0x0000) return 7;
    if (bits_f16(c_next) != 0x0001) return 8;

    volatile _Float16 vp = 2.5f16;
    volatile _Float16 vn = -2.5f16;
    volatile _Float16 v35 = 3.5f16;
    volatile _Float16 v9 = 9.0f16;
    volatile _Float16 vnan = __builtin_nanf16("");
    volatile _Float16 vmz = -0.0f16;

    if (bits_f16(__builtin_ceilf16(vp)) != bits_f16(3.0f16)) return 9;
    if (bits_f16(__builtin_floorf16(vn)) != bits_f16(-3.0f16)) return 10;
    if (bits_f16(__builtin_truncf16(vn)) != bits_f16(-2.0f16)) return 11;
    if (bits_f16(__builtin_roundf16(vp)) != bits_f16(3.0f16)) return 12;
    if (bits_f16(__builtin_roundf16(vn)) != bits_f16(-3.0f16)) return 13;
    if (bits_f16(__builtin_roundevenf16(vp)) != bits_f16(2.0f16)) return 14;
    if (bits_f16(__builtin_roundevenf16(v35)) != bits_f16(4.0f16)) return 15;
    if (bits_f16(__builtin_rintf16(vp)) != bits_f16(2.0f16)) return 16;
    if (bits_f16(__builtin_nearbyintf16(v35)) != bits_f16(4.0f16)) return 17;
    if (bits_f16(__builtin_sqrtf16(v9)) != bits_f16(3.0f16)) return 18;
    if (bits_f16(__builtin_fmaxf16(vnan, 7.0f16)) != bits_f16(7.0f16)) return 19;
    if (bits_f16(__builtin_fminf16(7.0f16, vnan)) != bits_f16(7.0f16)) return 20;
    if (bits_f16(__builtin_fmaf16(vp, vp, -6.25f16)) != 0x0000) return 21;
    if (bits_f16(__builtin_nextafterf16(vmz, 1.0f16)) != 0x0001) return 22;
    if (bits_f16(__builtin_nextafterf16(0.0f16, -1.0f16)) != 0x8001) return 23;

    return 0;
}
