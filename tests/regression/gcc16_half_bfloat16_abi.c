/* Differential oracle regression for ISO C23 _Float16, __fp16, and __bf16:
 * layout, predefined macros, 9-parameter register + stack calling convention,
 * aggregate pass/return, variadic forwarding, Boldo-Melquiond RTO single-step
 * conversions (double/_Float128/u64/u128 -> _Float16/__bf16), and inline
 * sign/Inf/NaN/classification builtins. */
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

static uint16_t bits_f16(_Float16 x) {
    uint16_t u;
    memcpy(&u, &x, 2);
    return u;
}

static uint16_t bits_bf16(__bf16 x) {
    uint16_t u;
    memcpy(&u, &x, 2);
    return u;
}

_Static_assert(sizeof(_Float16) == 2 && _Alignof(_Float16) == 2, "_Float16 layout");
#if defined(__aarch64__) || defined(__arm__) || defined(__clang__)
_Static_assert(sizeof(__fp16) == 2 && _Alignof(__fp16) == 2, "__fp16 layout");
#endif
_Static_assert(sizeof(__bf16) == 2 && _Alignof(__bf16) == 2, "__bf16 layout");
_Static_assert(__FLT16_MANT_DIG__ == 11, "__FLT16_MANT_DIG__");
_Static_assert(__BFLT16_MANT_DIG__ == 8, "__BFLT16_MANT_DIG__");

struct HalfPair {
    _Float16 a;
    _Float16 b;
};

struct MixedHalf {
    __bf16 bf;
    _Float16 f16;
    int tag;
};

static const _Float16 g_f16_tbl[3] = { 1.5f16, -2.25f16, 0.125f16 };
static const __bf16 g_bf16_tbl[2] = { 3.5bf16, -0.5bf16 };

__attribute__((noinline))
static _Float16 sum9_f16(
    _Float16 a0, _Float16 a1, _Float16 a2, _Float16 a3,
    _Float16 a4, _Float16 a5, _Float16 a6, _Float16 a7,
    _Float16 a8
) {
    return a0 + a1 + a2 + a3 + a4 + a5 + a6 + a7 - a8;
}

__attribute__((noinline))
static __bf16 mix9_bf16(
    __bf16 b0, int k0, __bf16 b1, _Float16 h0,
    __bf16 b2, __bf16 b3, __bf16 b4, __bf16 b5,
    __bf16 b6, __bf16 b7, __bf16 b8
) {
    float acc = (float)b0 + (float)k0 + (float)b1 + (float)h0
              + (float)b2 + (float)b3 + (float)b4 + (float)b5
              + (float)b6 + (float)b7 - (float)b8;
    return (__bf16)acc;
}

__attribute__((noinline))
static struct HalfPair step_pair(struct HalfPair p) {
    struct HalfPair out;
    out.a = p.b + 1.0f16;
    out.b = p.a - 0.5f16;
    return out;
}

__attribute__((noinline))
static struct MixedHalf step_mixed(struct MixedHalf m, _Float16 scale) {
    struct MixedHalf r;
    r.bf = (__bf16)((float)m.bf + (float)scale);
    r.f16 = m.f16 * scale;
    r.tag = m.tag + 7;
    return r;
}

__attribute__((noinline))
static _Float16 sum_var_f16(int n, ...) {
    va_list ap;
    va_start(ap, n);
    _Float16 acc = 0.0f16;
    for (int i = 0; i < n; ++i) {
        acc += va_arg(ap, _Float16);
    }
    va_end(ap);
    return acc;
}

int main(void) {
    if (bits_f16(g_f16_tbl[0] + g_f16_tbl[1] + g_f16_tbl[2]) != bits_f16(-0.625f16)) return 1;
    if (bits_bf16((__bf16)((float)g_bf16_tbl[0] + (float)g_bf16_tbl[1])) != bits_bf16(3.0bf16)) return 2;

    _Float16 s9 = sum9_f16(1.0f16, 2.0f16, 3.0f16, 4.0f16, 5.0f16, 6.0f16, 7.0f16, 8.0f16, 6.0f16);
    if (bits_f16(s9) != bits_f16(30.0f16)) return 3;

    __bf16 m9 = mix9_bf16(1.0bf16, 2, 3.0bf16, 0.5f16, 1.5bf16, 2.0bf16, 2.5bf16, 3.5bf16, 4.0bf16, 5.0bf16, 5.0bf16);
    if (bits_bf16(m9) != bits_bf16(20.0bf16)) return 4;

    struct HalfPair hp = step_pair((struct HalfPair){ 2.5f16, 4.0f16 });
    if (bits_f16(hp.a) != bits_f16(5.0f16) || bits_f16(hp.b) != bits_f16(2.0f16)) return 5;

    struct MixedHalf mh = step_mixed((struct MixedHalf){ 1.5bf16, 3.0f16, 10 }, 2.0f16);
    if (bits_bf16(mh.bf) != bits_bf16(3.5bf16) || bits_f16(mh.f16) != bits_f16(6.0f16) || mh.tag != 17) return 6;

    _Float16 vs = sum_var_f16(4, (_Float16)1.25f16, (_Float16)2.5f16, (_Float16)3.75f16, (_Float16)0.5f16);
    if (bits_f16(vs) != bits_f16(8.0f16)) return 7;

    /* Single-step rounding boundary witnesses (naive f64/f128/int -> f32 -> half double-rounds): */
    volatile double d_tie = 1.0 + 0x1p-11 + 0x1p-52;
    if (bits_f16((_Float16)d_tie) != 0x3c01) return 8;

    volatile _Float128 q_tie = 1.0Q + 0x1p-11Q + 0x1p-100Q;
    if (bits_f16((_Float16)q_tie) != 0x3c01) return 9;

    volatile unsigned long long u64_tie = (1ULL << 50) + (1ULL << 42) + 1ULL;
    if (bits_bf16((__bf16)u64_tie) != bits_bf16(0x1.02p50bf16)) return 10;

    volatile unsigned __int128 u128_tie = (((unsigned __int128)1) << 100) + (((unsigned __int128)1) << 92) + 1u;
    if (bits_bf16((__bf16)u128_tie) != bits_bf16(0x1.02p100bf16)) return 11;

    /* Inline sign / NaN / Inf / classification builtins: */
    volatile _Float16 neg_h = -3.5f16;
    volatile __bf16 neg_b = -3.5bf16;
    if (bits_f16(__builtin_fabsf16(neg_h)) != bits_f16(3.5f16)) return 12;
    if (!__builtin_signbit(neg_b) || __builtin_signbit(-neg_b)) return 13;
    if (bits_f16(__builtin_copysignf16(2.0f16, neg_h)) != bits_f16(-2.0f16)) return 14;
    if (!__builtin_isfinite(neg_b) || __builtin_isnan(neg_b) || __builtin_isinf(neg_b)) return 15;
    if (bits_f16(__builtin_inff16()) != 0x7c00) return 16;
    if (bits_f16(__builtin_huge_valf16()) != 0x7c00) return 17;
    if (!__builtin_isnan(__builtin_nanf16(""))) return 18;
    if ((bits_f16(__builtin_nansf16("")) & 0x7e00) != 0x7c00) return 19;
    if ((bits_bf16(__builtin_nansf16b("")) & 0x7fc0) != 0x7f80) return 20;

    printf("half_abi_ok %04x %04x\n", (unsigned)bits_f16(s9), (unsigned)bits_bf16(m9));
    return 0;
}
