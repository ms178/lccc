/* C23/C2y <stdbit.h> + __builtin_stdc_* + algebraic bit/rotate identities. */
#if __has_include(<stdbit.h>)
#include <stdbit.h>
#else
#include "../../include/stdbit.h"
#endif
#include <stdint.h>
#include <stdio.h>

_Static_assert(__STDC_VERSION_STDBIT_H__ == 202311L, "stdbit version");
_Static_assert(__STDC_ENDIAN_NATIVE__ == __STDC_ENDIAN_LITTLE__ ||
                   __STDC_ENDIAN_NATIVE__ == __STDC_ENDIAN_BIG__,
               "stdbit endian");

_Static_assert(stdc_leading_zeros((unsigned char)0) == 8, "lz8(0)");
_Static_assert(stdc_leading_zeros((unsigned char)0x10) == 3, "lz8(0x10)");
_Static_assert(stdc_leading_ones((unsigned char)0xF0) == 4, "lo8");
_Static_assert(stdc_trailing_zeros((unsigned short)0) == 16, "tz16(0)");
_Static_assert(stdc_trailing_zeros((unsigned short)0x0100) == 8, "tz16");
_Static_assert(stdc_trailing_ones((unsigned int)0x0Fu) == 4, "to32");
_Static_assert(stdc_first_leading_one((unsigned char)0x10) == 4, "flo8");
_Static_assert(stdc_first_leading_one((unsigned char)0) == 0, "flo8(0)");
_Static_assert(stdc_first_leading_zero((unsigned char)0xF0) == 5, "flz8");
_Static_assert(stdc_first_leading_zero((unsigned char)0xFF) == 0, "flz8(FF)");
_Static_assert(stdc_first_trailing_one((unsigned short)0x0010) == 5, "fto16");
_Static_assert(stdc_first_trailing_zero((unsigned short)0x000F) == 5, "ftz16");
_Static_assert(stdc_count_ones((unsigned int)0xF0F00F0Fu) == 16, "co32");
_Static_assert(stdc_count_zeros((unsigned char)0x0F) == 4, "cz8");
_Static_assert(stdc_has_single_bit((unsigned long)0) == 0, "hsb(0)");
_Static_assert(stdc_has_single_bit((unsigned long)64) == 1, "hsb(64)");
_Static_assert(stdc_bit_width((unsigned int)0u) == 0, "bw(0)");
_Static_assert(stdc_bit_width((unsigned int)19u) == 5, "bw(19)");
_Static_assert(stdc_bit_floor((unsigned int)0u) == 0u, "bf(0)");
_Static_assert(stdc_bit_floor((unsigned int)19u) == 16u, "bf(19)");
_Static_assert(stdc_bit_ceil((unsigned int)0u) == 1u, "bc(0)");
_Static_assert(stdc_bit_ceil((unsigned int)1u) == 1u, "bc(1)");
_Static_assert(stdc_bit_ceil((unsigned int)17u) == 32u, "bc(17)");
_Static_assert(__builtin_stdc_rotate_left((unsigned char)0x81, 1) == 0x03, "rotl8");
_Static_assert(__builtin_stdc_rotate_right((unsigned char)0x81, 1) == 0xC0, "rotr8");
_Static_assert(__builtin_stdc_rotate_left((unsigned short)0x8001, 1) == 0x0003, "rotl16");
_Static_assert(stdc_leading_zeros(0x00010000u) == 15, "lz32");
_Static_assert(stdc_bit_ceil(19u) == 32u, "bc32");

__attribute__((noinline)) unsigned ring_ident1(unsigned x) {
    return stdc_count_ones(x) + stdc_count_zeros(x);
}
__attribute__((noinline)) unsigned ring_ident2(unsigned x) {
    return stdc_leading_zeros(x) + stdc_bit_width(x);
}
__attribute__((noinline)) unsigned ring_ident3(unsigned x) {
    return (unsigned)(__builtin_popcount(x) + __builtin_popcount(~x));
}
__attribute__((noinline)) unsigned rot_chain(unsigned x, int a, int b) {
    return __builtin_stdc_rotate_left(__builtin_stdc_rotate_left(x, a), b);
}
__attribute__((noinline)) unsigned rot_cancel(unsigned x, int a) {
    return __builtin_stdc_rotate_left(__builtin_stdc_rotate_right(x, a), a);
}

static uint64_t mix(uint64_t h, uint64_t v) {
    return (h ^ v) * 0x9e3779b97f4a7c15ULL + 0x100000001b3ULL;
}

int main(void) {
    uint64_t h = 0xcbf29ce484222325ULL;
    const uint64_t samples[] = {
        0ULL, 1ULL, 2ULL, 3ULL, 7ULL, 8ULL, 15ULL, 16ULL,
        31ULL, 32ULL, 127ULL, 128ULL, 254ULL, 255ULL,
        0x7fffULL, 0x8000ULL, 0xffffULL,
        0x7fffffffULL, 0x80000000ULL, 0xffffffffULL,
        0x0123456789abcdefULL, 0x7fffffffffffffffULL, 0xffffffffffffffffULL,
    };

    for (unsigned i = 0; i < sizeof(samples) / sizeof(samples[0]); ++i) {
        uint64_t s = samples[i];
        unsigned char v8 = (unsigned char)s;
        unsigned short v16 = (unsigned short)s;
        unsigned int v32 = (unsigned int)s;
        unsigned long long v64 = (unsigned long long)s;

        h = mix(h, stdc_leading_zeros(v8));
        h = mix(h, stdc_leading_ones(v8));
        h = mix(h, stdc_trailing_zeros(v8));
        h = mix(h, stdc_trailing_ones(v8));
        h = mix(h, stdc_first_leading_one(v8));
        h = mix(h, stdc_first_leading_zero(v8));
        h = mix(h, stdc_first_trailing_one(v8));
        h = mix(h, stdc_first_trailing_zero(v8));
        h = mix(h, stdc_count_ones(v8));
        h = mix(h, stdc_count_zeros(v8));
        h = mix(h, stdc_has_single_bit(v8));
        h = mix(h, stdc_bit_width(v8));
        h = mix(h, stdc_bit_floor(v8));
        if (v8 <= 0x80u) h = mix(h, stdc_bit_ceil(v8));
        h = mix(h, __builtin_stdc_rotate_left(v8, (int)(i * 3 - 5)));
        h = mix(h, __builtin_stdc_rotate_right(v8, (int)(i * 5 - 7)));

        h = mix(h, stdc_leading_zeros_us(v16));
        h = mix(h, stdc_trailing_zeros_us(v16));
        h = mix(h, stdc_bit_floor_us(v16));
        if (v16 <= 0x8000u) h = mix(h, stdc_bit_ceil_us(v16));
        h = mix(h, __builtin_stdc_rotate_left(v16, (int)i));

        h = mix(h, stdc_leading_zeros_ui(v32));
        h = mix(h, stdc_trailing_zeros_ui(v32));
        h = mix(h, stdc_first_leading_one_ui(v32));
        h = mix(h, stdc_first_trailing_one_ui(v32));
        h = mix(h, stdc_bit_floor_ui(v32));
        if (v32 <= 0x80000000u) h = mix(h, stdc_bit_ceil_ui(v32));
        h = mix(h, ring_ident1(v32));
        h = mix(h, ring_ident2(v32));
        h = mix(h, ring_ident3(v32));
        h = mix(h, rot_chain(v32, (int)i, (int)(i * 7 + 3)));
        h = mix(h, rot_cancel(v32, (int)(i * 11 - 13)));

        h = mix(h, stdc_leading_zeros_ull(v64));
        h = mix(h, stdc_trailing_zeros_ull(v64));
        h = mix(h, stdc_first_leading_one_ull(v64));
        h = mix(h, stdc_first_leading_zero_ull(v64));
        h = mix(h, stdc_first_trailing_one_ull(v64));
        h = mix(h, stdc_first_trailing_zero_ull(v64));
        h = mix(h, stdc_count_ones_ull(v64));
        h = mix(h, stdc_count_zeros_ull(v64));
        h = mix(h, stdc_has_single_bit_ull(v64));
        h = mix(h, stdc_bit_width_ull(v64));
        h = mix(h, stdc_bit_floor_ull(v64));
        if (v64 <= 0x8000000000000000ULL) h = mix(h, stdc_bit_ceil_ull(v64));
        h = mix(h, __builtin_stdc_rotate_left(v64, (int)(i * 9 - 4)));
        h = mix(h, __builtin_stdc_rotate_right(v64, (int)(i * 9 - 4)));

        unsigned long vl = (unsigned long)s;
        h = mix(h, stdc_leading_zeros_ul(vl));
        h = mix(h, stdc_trailing_zeros_ul(vl));
        h = mix(h, stdc_count_ones_ul(vl));
        h = mix(h, stdc_count_zeros_ul(vl));
        h = mix(h, stdc_bit_width_ul(vl));
        h = mix(h, (uint64_t)stdc_bit_floor_ul(vl));
        h = mix(h, (uint64_t)__builtin_stdc_rotate_left(vl, (int)i));
        h = mix(h, (uint64_t)__builtin_stdc_rotate_right(vl, (int)i));

#ifdef __SIZEOF_INT128__
        unsigned __int128 v128 = ((unsigned __int128)v64 << 64) | (v64 ^ 0x5a5a5a5a5a5a5a5aULL);
        if (i == 0) v128 = 0;
        h = mix(h, stdc_leading_zeros(v128));
        h = mix(h, stdc_trailing_zeros(v128));
        h = mix(h, stdc_count_ones(v128));
        h = mix(h, stdc_bit_width(v128));
        unsigned __int128 bf128 = stdc_bit_floor(v128);
        h = mix(h, (uint64_t)bf128 ^ (uint64_t)(bf128 >> 64));
        unsigned __int128 rl128 = __builtin_stdc_rotate_left(v128, (int)(i * 13));
        h = mix(h, (uint64_t)rl128 ^ (uint64_t)(rl128 >> 64));
#endif
    }

    printf("stdbit_hash=%016llx\n", (unsigned long long)h);
    return 0;
}
