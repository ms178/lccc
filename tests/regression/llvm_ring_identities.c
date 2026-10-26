/* Ring identities: remainder window compares, sign-mask shifts, saturated
   min/max selects and complement-of-negation, all with observable effects.
   The arithmetic-shift form of the sign-mask idiom is covered exhaustively by
   the unit tests; here `x >> 31` on an unsigned operand is a 0/1 mask, which
   the fold must and does refuse. */
#include <stdint.h>
#include <stdio.h>

static unsigned long long digest = 1469598103934665603ULL;
static void mix(unsigned long long value) {
    digest ^= value;
    digest *= 1099511628211ULL;
}

int ring_srem_eq_250(long x) { return (x % 250) == x; }
int ring_srem_ne_250(long x) { return (x % 250) != x; }
int ring_srem_eq_min(long x) { return (x % INT64_MIN) == x; }
int ring_srem_ne_min(long x) { return (x % INT64_MIN) != x; }
int ring_srem_eq_neg7(long x) { return (x % -7) == x; }
int ring_srem_eq32(int32_t x) { return (x % 1000000) == x; }
int ring_srem_ne32(int32_t x) { return (x % 1000000) != x; }
int ring_srem_eq16(int16_t x) { return (x % 250) == x; }
int ring_srem_ne16(int16_t x) { return (x % -250) != x; }
int ring_srem_eq8(int8_t x) { return (x % 100) == x; }

uint32_t ring_sign_shift(uint32_t x, int n) {
    uint32_t mask = -(x >> 31);
    return ((x ^ mask) >> n) ^ mask;
}
uint64_t ring_sign_shift64(uint64_t x, int n) {
    uint64_t mask = -(x >> 63);
    return ((x ^ mask) >> n) ^ mask;
}

uint32_t ring_not_neg(uint32_t x) { return ~(-x); }
int32_t ring_neg_not(int32_t x) { return -~x; }
int16_t ring_not_neg16(int16_t x) { return ~(-x); }
uint64_t ring_neg_not64(uint64_t x) { return -(~x); }

int32_t ring_min_imax(int32_t x) { return x < INT32_MAX ? x : INT32_MAX; }
int32_t ring_max_imin(int32_t x) { return x > INT32_MIN ? x : INT32_MIN; }
uint32_t ring_min_umax(uint32_t x) { return x < UINT32_MAX ? x : UINT32_MAX; }
uint32_t ring_max_u0(uint32_t x) { return x > 0u ? x : 0u; }
int64_t ring_min_imax64(int64_t x) { return x <= INT64_MAX ? x : INT64_MAX; }
int64_t ring_max_imin64(int64_t x) { return x >= INT64_MIN ? x : INT64_MIN; }
int32_t ring_clamp_imax(int32_t x) { return x < INT32_MAX - 1 ? x : INT32_MAX - 1; }
int32_t ring_zero_select(int32_t x) { return x != 0 ? x : 0; }
int32_t ring_zero_constant_eq(int32_t x) { return x == 0 ? 0 : x; }
int32_t ring_or_chain_value(int32_t a, int32_t b, int32_t c) { return (a | b | c) ? a : 0; }
int32_t ring_or_chain_zero(int32_t a, int32_t b, int32_t c) { return (a | b | c) ? 0 : a; }
int32_t ring_srem_pow2_32(int32_t x) { return x % 16; }
int64_t ring_srem_pow2_64(int64_t x) { return x % 1024; }
int32_t ring_srem_pow2_neg(int32_t x) { return x % -32; }
int32_t ring_sdiv_pow2_32(int32_t x) { return x / 32; }
uint32_t ring_urem_pow2(uint32_t x) { return x % 256; }
int32_t ring_nested_select(int32_t a, int32_t b) { return (a != 0 || b) ? a : 0; }
int32_t ring_bool_product_self(int32_t x, int32_t y) { return x * (x || y); }
int32_t ring_bool_product_test(int32_t x, uint32_t y) { return x * (y != 0); }
uint32_t ring_bool_product_cmp(uint32_t x, uint32_t y) { return (x < y) * x; }

enum { WIDE = 23, NARROW = 14 };

int main(void) {
    static const long wide[] = {
        0, 1, 2, 249, 250, 251, 499, 500, -1, -2, -6, -7, -8, -249, -250,
        -251, INT64_MAX, INT64_MIN, INT64_MIN + 1, 1000000, -1000000,
        1234567890123LL, -1234567890123LL,
    };
    static const int32_t narrow[] = {
        0, 1, -1, 249, 250, -250, 499999, 1000000, 1000001, -1000000,
        INT32_MAX, INT32_MIN, INT32_MIN + 1, 777,
    };
    static const int16_t small[] = {0, 1, -1, 249, 250, -250, 32767, -32768, -32767, 100, -100};
    _Static_assert(sizeof wide / sizeof *wide == WIDE, "wide probe count");
    _Static_assert(sizeof narrow / sizeof *narrow == NARROW, "narrow probe count");

    for (unsigned i = 0; i < WIDE; i++) {
        mix((unsigned)ring_srem_eq_250(wide[i]));
        mix((unsigned)ring_srem_ne_250(wide[i]));
        mix((unsigned)ring_srem_eq_min(wide[i]));
        mix((unsigned)ring_srem_ne_min(wide[i]));
        mix((unsigned)ring_srem_eq_neg7(wide[i]));
        mix((unsigned)ring_or_chain_value((int32_t)wide[i], (int32_t)(wide[i] >> 3),
                                          (int32_t)(wide[i] >> 7)));
        mix((unsigned)ring_or_chain_zero((int32_t)wide[i], (int32_t)(wide[i] >> 3),
                                         (int32_t)(wide[i] >> 7)));
    }
    for (unsigned i = 0; i < NARROW; i++) {
        mix((unsigned)ring_srem_eq32(narrow[i]));
        mix((unsigned)ring_srem_ne32(narrow[i]));
        mix((unsigned)ring_min_imax(narrow[i]));
        mix((unsigned)ring_max_imin(narrow[i]));
        mix((unsigned)ring_clamp_imax(narrow[i]));
        mix((unsigned)ring_zero_select(narrow[i]));
        mix((unsigned)ring_zero_constant_eq(narrow[i]));
        mix((unsigned)ring_nested_select(narrow[i], narrow[(i + 1) % NARROW]));
        mix((unsigned)ring_bool_product_self(narrow[i], narrow[(i + 2) % NARROW]));
        mix((unsigned)ring_bool_product_test(narrow[i], (uint32_t)narrow[(i + 3) % NARROW]));
        mix((unsigned)ring_srem_pow2_32(narrow[i]));
        mix((unsigned)ring_srem_pow2_neg(narrow[i]));
        mix((unsigned)ring_sdiv_pow2_32(narrow[i]));
        mix((unsigned)ring_urem_pow2((uint32_t)narrow[i]));
        mix(ring_bool_product_cmp((uint32_t)narrow[i], (uint32_t)narrow[(i + 5) % NARROW]));
        mix((unsigned)ring_srem_pow2_64((int64_t)narrow[i] * 1000003));
        mix((unsigned)ring_neg_not(narrow[i]));
        mix((unsigned)ring_min_umax((uint32_t)narrow[i]));
        mix((unsigned)ring_max_u0((uint32_t)narrow[i]));
        mix((unsigned)ring_not_neg((uint32_t)narrow[i]));
        mix((unsigned)ring_neg_not64((uint64_t)narrow[i]));
        mix((unsigned)ring_min_imax64(narrow[i]));
        mix((unsigned)ring_max_imin64(narrow[i]));
    }
    for (unsigned i = 0; i < sizeof small / sizeof *small; i++) {
        mix((unsigned)ring_srem_eq16(small[i]));
        mix((unsigned)ring_srem_ne16(small[i]));
        mix((unsigned)ring_srem_eq8((int8_t)small[i]));
        mix((unsigned)ring_not_neg16(small[i]));
    }
    static const uint32_t lanes[] = {0u, 1u, 2u, 0x7fffffffu, 0x80000000u, 0x80000001u,
                                     0xffffffffu, 0xdeadbeefu, 0x40000000u, 0x00ff00ffu};
    for (unsigned i = 0; i < sizeof lanes / sizeof *lanes; i++) {
        for (int n = 0; n <= 40; n++) {
            mix(ring_sign_shift(lanes[i], n));
            mix(ring_sign_shift64(((uint64_t)lanes[i] << 32) | lanes[(i + 3) % 10], n));
        }
    }
    printf("%016llx\n", digest);
    return 0;
}
