/* Compound assignment performs the usual arithmetic conversions BEFORE
   widening to an IR carrier (C11 6.3.1.3, 6.3.1.8, 6.5.16.2).
   Inspired by sametyilmaztemel/claudes-c-compiler a50a017f26.
   All quotient/remainder results below fit their signed destination: this
   reproducer does not depend on an out-of-range unsigned-to-signed result. */
#define OPS(name, lhs, rhs) \
    __attribute__((noinline)) static lhs name##_div(lhs x, rhs y) { \
        x /= y; return x; \
    } \
    __attribute__((noinline)) static lhs name##_mod(lhs x, rhs y) { \
        x %= y; return x; \
    }
OPS(c32, signed char, unsigned int)
OPS(s32, short, unsigned int)
OPS(i32, int, unsigned int)
OPS(c64, signed char, unsigned long long)
OPS(s64, short, unsigned long long)
OPS(i64, int, unsigned long long)
OPS(l64, long long, unsigned long long)
OPS(u64, unsigned int, unsigned long long)
OPS(signed64, int, long long)
#ifdef __SIZEOF_INT128__
OPS(i128, int, unsigned __int128)
OPS(l128, long long, unsigned __int128)
OPS(u128, unsigned int, unsigned __int128)
#endif
#undef OPS

static volatile short cell;
static int calls;
__attribute__((noinline)) static volatile short *cell_address(void) {
    ++calls;
    return &cell;
}

#define CHECK(expr) do { if (!(expr)) return __LINE__; } while (0)
int main(void) {
    CHECK(c32_div(-8, 1U << 25) == 127);
    CHECK(s32_div(-8, 1U << 17) == 32767);
    CHECK(i32_div(-8, 1U << 31) == 1);
    CHECK(c64_div(-8, 1ULL << 57) == 127);
    CHECK(s64_div(-8, 1ULL << 49) == 32767);
    CHECK(i64_div(-8, 1ULL << 63) == 1);
    CHECK(l64_div(-8, 1ULL << 63) == 1);

    /* Dynamic operands and boundary values; ordinary casts are an independent
       lowering path. Remainders are at most 96 and fit every signed LHS. */
    for (int x = -128; x <= 127; ++x) {
        unsigned int u32 = (unsigned int)x;
        unsigned long long u64 = (unsigned long long)x;
        CHECK(c32_mod((signed char)x, 97U) == u32 % 97U);
        CHECK(s32_mod((short)x, 97U) == u32 % 97U);
        CHECK(i32_mod(x, 97U) == u32 % 97U);
        CHECK(c64_mod((signed char)x, 97ULL) == u64 % 97ULL);
        CHECK(s64_mod((short)x, 97ULL) == u64 % 97ULL);
        CHECK(i64_mod(x, 97ULL) == u64 % 97ULL);
        CHECK(l64_mod(x, 97ULL) == u64 % 97ULL);
#ifdef __SIZEOF_INT128__
        unsigned __int128 u128_value = (unsigned __int128)x;
        CHECK(i128_mod(x, 97) == u128_value % 97);
        CHECK(l128_mod(x, 97) == u128_value % 97);
#endif
    }
    CHECK(s32_div(-32768, 1U << 17) == 32767);
    CHECK(i64_div(-2147483647 - 1, 1ULL << 63) == 1);
    CHECK(i64_div(2147483647, 1ULL << 63) == 0);
    CHECK(u64_div(0xffffffffU, 1ULL << 31) == 1);
    CHECK(u64_mod(0xffffffffU, 1ULL << 32) == 0xffffffffU);
    CHECK(signed64_div(-8, 4) == -2);
    CHECK(signed64_mod(-8, 3) == -2);
#ifdef __SIZEOF_INT128__
    CHECK(i128_div(-8, (unsigned __int128)1 << 127) == 1);
    CHECK(l128_div(-8, (unsigned __int128)1 << 127) == 1);
    CHECK(u128_div(0xffffffffU, (unsigned __int128)1 << 64) == 0);
    CHECK(u128_mod(0xffffffffU, (unsigned __int128)1 << 64) == 0xffffffffU);
#endif
    cell = -8;
    calls = 0;
    *cell_address() /= 1U << 17;
    CHECK(calls == 1 && cell == 32767);

    /* The RHS type must not turn a signed shift into a logical shift. */
    int signed_shift = -8;
    unsigned unsigned_shift = 0xfffffff0U;
    unsigned char byte_shift = 248;
    signed_shift >>= 1U;
    unsigned_shift >>= 1ULL;
    byte_shift >>= 1U;
    CHECK(signed_shift == -4 && unsigned_shift == 0x7ffffff8U && byte_shift == 124);
    return 0;
}
