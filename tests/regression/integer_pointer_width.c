/* x86 integer/pointer conversions with cross-compiler-agreed results:
 * preserve source signedness and truncate at pointer width. Pointer-to-integer
 * inputs are representable in the destination type. Converted arbitrary bit
 * patterns are NEVER dereferenced; arithmetic is nonoverflowing under either
 * overflow policy. The implementation-defined high-bit pointer-to-__int128
 * choice is tested separately against each compiler's documented behavior. */
#ifdef LCCC_POINTER_WIDTH_FREESTANDING
typedef signed char int8_t;
typedef unsigned char uint8_t;
typedef short int16_t;
typedef unsigned short uint16_t;
typedef int int32_t;
typedef unsigned int uint32_t;
typedef long long int64_t;
typedef unsigned long long uint64_t;
typedef __INTPTR_TYPE__ intptr_t;
typedef __UINTPTR_TYPE__ uintptr_t;
#define UINT32_MAX 0xffffffffu
#define UINT64_MAX 0xffffffffffffffffULL
#define printf(...) ((void)0)
#define puts(...) ((void)0)
#else
#include <stdint.h>
#include <stdio.h>
#endif
#define NI __attribute__((noinline))
#define FN(n,e) static NI void *n(int x, int y) { (void)y; return (void *)(e); }
FN(param, x)
FN(add, x + y)
FN(sub, x - y)
FN(mul, x * y)
FN(neg, -x)
FN(inv, ~x)
FN(quot, x / y)
FN(rem, x % y)
FN(band, x & y)
FN(bor, x | y)
FN(bxor, x ^ y)
static NI void *selectp(int x, int y) { int z = y ? x : -x; return (void *)z; }
static NI void *loadp(int x, int y) { volatile int z = x; (void)y; return (void *)z; }
static NI int identity(int x) { return x; }
FN(callp, identity(x))
static NI void *asmp(int x, int y) {
    int z; (void)y;
#if defined(__i386__) || defined(__x86_64__)
    __asm__("movl %1, %0" : "=r"(z) : "r"(x));
#else
    z = x;
#endif
    return (void *)z;
}
static NI void *atomicp(int x, int y) {
    int z = x;
    return (void *)__atomic_fetch_add(&z, y, __ATOMIC_SEQ_CST);
}
static NI void *floatp(int x, int y) {
    volatile double z = (double)x; (void)y;
    return (void *)(int)z;
}
#define NARROW(n,t) static NI void *n(uint32_t x) { return (void *)(t)x; }
NARROW(s8, int8_t) NARROW(u8, uint8_t)
NARROW(s16, int16_t) NARROW(u16, uint16_t)
NARROW(s32, int32_t) NARROW(u32, uint32_t)
static NI void *s64(int64_t x) { return (void *)x; }
static NI void *u64(uint64_t x) { return (void *)x; }
static NI int64_t from8(void *p) { return (int8_t)p; }
static NI int64_t from16(void *p) { return (int16_t)p; }
static NI int64_t from32(void *p) { return (int32_t)p; }
static NI int64_t from64s(void *p) { return (int64_t)p; }
static NI uint64_t from64u(void *p) { return (uint64_t)p; }
static NI void *nullp(void) { return 0; }
#if defined(__SIZEOF_INT128__) && __SIZEOF_POINTER__ == 8
static NI void *u128(unsigned __int128 x) { return (void *)x; }
static NI void *s128(__int128 x) { return (void *)x; }
static NI unsigned __int128 from_u128(void *p) { return (unsigned __int128)p; }
static NI __int128 from_s128(void *p) { return (__int128)p; }
#endif
static unsigned checks, failures;
static void check(const char *name, uintptr_t actual, uintptr_t expected) {
    (void)name; /* diagnostics are compiled out in the freestanding matrix */
    ++checks;
    if (actual != expected) {
        printf("%s: got %llx expected %llx\n", name,
               (unsigned long long)actual, (unsigned long long)expected);
        ++failures;
    }
}
#if defined(__SIZEOF_INT128__) && __SIZEOF_POINTER__ == 8
static void check128(const char *name, unsigned __int128 actual,
                     unsigned __int128 expected) {
    (void)name; /* diagnostics are compiled out in the freestanding matrix */
    ++checks;
    if (actual != expected) {
        printf("%s: got %016llx:%016llx expected %016llx:%016llx\n", name,
               (unsigned long long)(actual >> 64), (unsigned long long)actual,
               (unsigned long long)(expected >> 64), (unsigned long long)expected);
        ++failures;
    }
}
#endif
/* Independent integer-only reference: all intermediate signed values fit. */
static int64_t signed_low(uint64_t bits, unsigned width) {
    uint64_t sign = (uint64_t)1 << (width - 1);
    return (int64_t)(bits & (sign - 1)) - (int64_t)(bits & sign);
}
int main(void) {
    static const struct { void *(*fn)(int,int); int x, y; int64_t expected; } rows[] = {
        {param,-2,1,-2}, {add,-2,1,-1}, {sub,-2,1,-3}, {mul,-2,3,-6},
        {neg,2,0,-2}, {inv,1,0,-2}, {quot,-7,2,-3}, {rem,-7,2,-1},
        {band,-3,-2,-4}, {bor,-4,1,-3}, {bxor,-4,1,-3},
        {selectp,-2,1,-2}, {selectp,2,0,-2}, {loadp,-2,0,-2},
        {callp,-2,0,-2}, {asmp,-2,0,-2}, {atomicp,-2,1,-2}, {floatp,-2,0,-2}
    };
    for (unsigned k = 0; k < sizeof(rows)/sizeof(rows[0]); ++k) {
        volatile int x = rows[k].x, y = rows[k].y;
        uintptr_t actual = (uintptr_t)rows[k].fn(x,y);
        if (actual != (uintptr_t)rows[k].expected) printf("producer index %u\n", k);
        check("producer", actual, (uintptr_t)rows[k].expected);
    }
    static const uint32_t inputs[] = {0,1,0x7f,0x80,0xff,0x100,0x7fff,
        0x8000,0xffff,0x10000,0x7fffffff,0x80000000u,0xffffffffu};
    for (unsigned k = 0; k < sizeof(inputs)/sizeof(inputs[0]); ++k) {
        volatile uint32_t x = inputs[k];
        check("s8", (uintptr_t)s8(x), (uintptr_t)signed_low(inputs[k],8));
        check("u8", (uintptr_t)u8(x), (uintptr_t)(inputs[k] & 255u));
        check("s16", (uintptr_t)s16(x), (uintptr_t)signed_low(inputs[k],16));
        check("u16", (uintptr_t)u16(x), (uintptr_t)(inputs[k] & 65535u));
        check("s32", (uintptr_t)s32(x), (uintptr_t)signed_low(inputs[k],32));
        check("u32", (uintptr_t)u32(x), (uintptr_t)inputs[k]);
    }
    static const uint64_t wide[] = {0xffffffffULL, 0x180000000ULL,
        0xffffffff80000000ULL, 0x8000000000000001ULL, UINT64_MAX};
    for (unsigned k = 0; k < sizeof(wide)/sizeof(wide[0]); ++k) {
        volatile uint64_t x = wide[k];
        uintptr_t expected = (uintptr_t)wide[k];
        check("u64", (uintptr_t)u64(x), expected);
        check("s64", (uintptr_t)s64((int64_t)x), expected);
        /* Pointer-to-integer results must be representable: do not turn
         * undefined out-of-range conversions into a differential oracle. */
        uintptr_t small8 = (uintptr_t)(wide[k] & 127u);
        uintptr_t small16 = (uintptr_t)(wide[k] & 32767u);
        uintptr_t small32 = (uintptr_t)(wide[k] & 2147483647u);
        check("from8", (uintptr_t)from8((void *)small8), small8);
        check("from16", (uintptr_t)from16((void *)small16), small16);
        check("from32", (uintptr_t)from32((void *)small32), small32);
    }
    /* These pointer values fit all pointer-to-I64 destinations, including
     * ILP32, and avoid implementation-defined out-of-range signed results. */
    static const uintptr_t pointer_inputs[] = {0, 1, 0x7f, 0x12345678, 0x7fffffff};
    for (unsigned k = 0; k < sizeof(pointer_inputs)/sizeof(pointer_inputs[0]); ++k) {
        volatile uintptr_t bits = pointer_inputs[k];
        void *p = (void *)bits;
        check("from64s", (uintptr_t)from64s(p), bits);
        check("from64u", (uintptr_t)from64u(p), bits);
    }
    check("literal unsigned", (uintptr_t)(void *)0xffffffffu, (uintptr_t)UINT32_MAX);
    check("literal signed", (uintptr_t)(void *)-1, (uintptr_t)(intptr_t)-1);
    check("implicit null", (uintptr_t)nullp(), 0);
    unsigned expected_checks = 134;
#if defined(__SIZEOF_INT128__) && __SIZEOF_POINTER__ == 8
    volatile uint64_t hi = 0x123456789abcdef0ULL, lo = 0xfedcba9876543210ULL;
    unsigned __int128 wide128 = ((unsigned __int128)hi << 64) | lo;
    check("u128", (uintptr_t)u128(wide128), (uintptr_t)lo);
    check("s128", (uintptr_t)s128(-((__int128)1 << 96) + lo), (uintptr_t)lo);
    /* Low pointer values keep all three compiler oracles on the same
     * implementation-defined conversion result. High-bit cases are tested
     * separately because GCC sign-extends there while Clang zero-extends. */
    static const uint64_t pointer128_inputs[] = {
        0, 1, 0x7f, 0x123456789abcdef0ULL, 0x7fffffffffffffffULL
    };
    for (unsigned k = 0; k < sizeof(pointer128_inputs)/sizeof(pointer128_inputs[0]); ++k) {
        volatile uint64_t bits = pointer128_inputs[k];
        void *p = (void *)(uintptr_t)bits;
        unsigned __int128 expected = (unsigned __int128)bits;
        check128("from_u128", from_u128(p), expected);
        check128("from_s128", (unsigned __int128)from_s128(p), expected);
    }
    expected_checks += 12;
#endif
    if (checks != expected_checks || failures) {
        printf("pointer checks %u/%u failures %u\n", checks, expected_checks, failures);
        return 1;
    }
    puts("integer pointer widths: PASS");
    return 0;
}
