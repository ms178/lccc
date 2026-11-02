#include <stdio.h>
#include <stdint.h>

struct Pair16 {
    uint16_t lo;
    uint16_t hi;
};

static const int *get_static_complit_ptr(void) {
    return &(static const int){ 12345 };
}

static int *get_tls_complit_ptr(void) {
    return &(static thread_local int){ 77 };
}

int main(void) {
    /* 1. GCC 16 type-generic bit builtins */
    int b1 = __builtin_clzg((uint8_t)0x04u);
    int b2 = __builtin_clzg((uint16_t)0u, 99);
    int b3 = __builtin_ctzg((uint16_t)0x0200u);
    int b4 = __builtin_ctzg((uint32_t)0u, 55);
    int b5 = __builtin_popcountg((uint8_t)0xA5u);
    int b6 = __builtin_parityg((uint16_t)0x8003u);
    int b7 = __builtin_ffsg((int16_t)0x0010);
    int b8 = __builtin_clrsbg((int8_t)-4);
    if (b1 != 5 || b2 != 99 || b3 != 9 || b4 != 55 || b5 != 4 || b6 != 1 || b7 != 5 || b8 != 5)
        return 1;

    /* 2. Carry/borrow chain builtins */
    unsigned int cout = 0;
    unsigned int sum = __builtin_addc(0xFFFFFFFFu, 5u, 1u, &cout);
    if (sum != 5u || cout != 1u)
        return 2;
    unsigned long long bout = 0;
    unsigned long long diff = __builtin_subcll(3ULL, 5ULL, 0ULL, &bout);
    if (diff != ~1ULL || bout != 1ULL)
        return 3;

    /* 3. 128-bit bswap & __builtin_bit_cast */
#ifdef __SIZEOF_INT128__
    unsigned __int128 x128 = (((unsigned __int128)0x0102030405060708ULL) << 64) | 0x1112131415161718ULL;
    unsigned __int128 y128 = __builtin_bswap128(x128);
    if ((uint64_t)(y128 >> 64) != 0x1817161514131211ULL || (uint64_t)y128 != 0x0807060504030201ULL)
        return 4;
#endif
    struct Pair16 p = { 0xCAFEu, 0xBABEu };
    uint32_t raw = 0;
    struct Pair16 p2;
#if __has_builtin(__builtin_bit_cast)
    raw = __builtin_bit_cast(uint32_t, p);
    p2 = __builtin_bit_cast(struct Pair16, raw);
#else
    __builtin_memcpy(&raw, &p, sizeof(raw));
    __builtin_memcpy(&p2, &raw, sizeof(p2));
#endif
    if (p2.lo != 0xCAFEu || p2.hi != 0xBABEu)
        return 5;

    /* 4. IEEE-754 sNaN / speculation barrier */
    uint32_t snan_bits = 0x7F800001u;
    float snan_f;
#if __has_builtin(__builtin_bit_cast)
    snan_f = __builtin_bit_cast(float, snan_bits);
#else
    __builtin_memcpy(&snan_f, &snan_bits, sizeof(snan_f));
#endif
    if (!__builtin_issignaling(snan_f) || __builtin_issignaling(1.5f))
        return 6;
    if (__builtin_speculation_safe_value(42, 0) != 42)
        return 7;

    /* 5. Clang-compatible bitreverse & fixed-width rotate builtins when present */
#if __has_builtin(__builtin_bitreverse32) && __has_builtin(__builtin_rotateleft32)
    if (__builtin_bitreverse8(0xB2u) != 0x4Du)
        return 8;
    if (__builtin_rotateleft32(0x12345678u, 8) != 0x34567812u)
        return 9;
    if (__builtin_rotateright64(0x0123456789ABCDEFULL, 8) != 0xEF0123456789ABCDULL)
        return 10;
#endif

    /* 6. C23 storage-class compound literals */
    constexpr struct Pair16 cp = (constexpr struct Pair16){ 10u, 20u };
    static_assert(cp.lo + cp.hi == 30u);
    int *tp = get_tls_complit_ptr();
    *tp += 3;
    int reg_sum = (register int){ 11 } + 9;
    printf("%d %u %d %d\n", *get_static_complit_ptr(), (unsigned)(cp.lo + cp.hi), *get_tls_complit_ptr(), reg_sum);
    return 0;
}
