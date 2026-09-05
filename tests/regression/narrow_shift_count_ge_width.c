/* Regression: passes/narrow.rs Phase 4 narrowed a 64-bit shift of a
 * zero/sign-extended 32-bit value into a 32-bit shift without checking that
 * the count fits the narrow width.  `(uint32_t)(zext(x) >> 56)` became
 * `x >> 56` in U32, which x86 executes as `x >> 24` (count masked to 5 bits).
 *
 * Found by tests/stress (narrow family, seed 3).  Covers LShr/Shl/AShr with
 * constant counts on both sides of the width boundary, a runtime count that
 * crosses the width, and the masked-count idiom. Expected values are the
 * C-standard ones; every case is checked against a constant. */
#include <stdint.h>
#include <stdio.h>

__attribute__((noinline)) int8_t lab_case(uint16_t p0) {
    uint16_t v0 = p0;
    uint64_t v1 = (uint64_t)((-v0) ^ ((uint32_t)2147483648u));
    uint32_t v2 = (uint32_t)(v1 >> 56);
    return (int8_t)((uint64_t)v0 + (uint64_t)v1 + (uint64_t)v2);
}
__attribute__((noinline)) uint32_t lshr_zext_56(uint32_t x) { return (uint32_t)((uint64_t)x >> 56); }
__attribute__((noinline)) uint32_t lshr_zext_32(uint32_t x) { return (uint32_t)((uint64_t)x >> 32); }
__attribute__((noinline)) uint32_t lshr_zext_31(uint32_t x) { return (uint32_t)((uint64_t)x >> 31); }
__attribute__((noinline)) uint32_t shl_zext_40(uint32_t x)  { return (uint32_t)((uint64_t)x << 40); }
__attribute__((noinline)) uint32_t shl_zext_33(uint32_t x)  { return (uint32_t)((uint64_t)x << 33); }
__attribute__((noinline)) uint32_t shl_zext_1(uint32_t x)   { return (uint32_t)((uint64_t)x << 1); }
__attribute__((noinline)) int32_t  ashr_sext_45(int32_t x)  { return (int32_t)((int64_t)x >> 45); }
__attribute__((noinline)) int32_t  ashr_sext_63(int32_t x)  { return (int32_t)((int64_t)x >> 63); }
__attribute__((noinline)) int32_t  ashr_sext_7(int32_t x)   { return (int32_t)((int64_t)x >> 7); }
__attribute__((noinline)) uint32_t lshr_var(uint32_t x, unsigned n)   { return (uint32_t)((uint64_t)x >> n); }
__attribute__((noinline)) uint32_t shl_var(uint32_t x, unsigned n)    { return (uint32_t)((uint64_t)x << n); }
__attribute__((noinline)) uint32_t lshr_masked(uint32_t x, unsigned n){ return (uint32_t)((uint64_t)x >> (n & 31)); }
__attribute__((noinline)) uint16_t lshr_u16_20(uint16_t x)  { return (uint16_t)((uint64_t)x >> 20); }
__attribute__((noinline)) uint8_t  shl_u8_9(uint8_t x)      { return (uint8_t)((uint64_t)x << 9); }

static volatile uint16_t a0 = 57;
static volatile uint32_t big = 0xDEADBEEFu;
static volatile int32_t neg = -123456789;
static volatile unsigned n40 = 40, n3 = 3, n35 = 35;

int main(void) {
    int fails = 0;
#define CHECK(expr, want) do { unsigned long long g = (unsigned long long)(expr), w = (unsigned long long)(want); \
        if (g != w) { fails++; printf("FAIL %-22s got %llu want %llu\n", #expr, g, w); } } while (0)
    CHECK(lab_case(a0), 0);
    CHECK(lshr_zext_56(big), 0u);
    CHECK(lshr_zext_32(big), 0u);
    CHECK(lshr_zext_31(big), 1u);
    CHECK(shl_zext_40(big), 0u);
    CHECK(shl_zext_33(big), 0u);
    CHECK(shl_zext_1(big), 0xBD5B7DDEu);
    CHECK((uint32_t)ashr_sext_45(neg), 0xFFFFFFFFu);
    CHECK((uint32_t)ashr_sext_63(neg), 0xFFFFFFFFu);
    CHECK((uint32_t)ashr_sext_45(12345), 0u);
    CHECK((uint32_t)ashr_sext_7(neg), (uint32_t)(-123456789 >> 7));
    CHECK(lshr_var(big, n40), 0u);
    CHECK(lshr_var(big, n3), 0xDEADBEEFu >> 3);
    CHECK(shl_var(big, n35), 0u);
    CHECK(shl_var(big, n3), (uint32_t)(0xDEADBEEFu << 3));
    CHECK(lshr_masked(big, n40), 0xDEADBEEFu >> (40 & 31));
    CHECK(lshr_masked(big, n35), 0xDEADBEEFu >> (35 & 31));
    CHECK(lshr_u16_20(0xFFFF), 0u);
    CHECK(shl_u8_9(0xFF), 0u);
    if (fails == 0) puts("ALL OK");
    return fails;
}
