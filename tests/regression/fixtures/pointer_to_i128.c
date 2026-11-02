/*
 * Pointer-to-wider-integer conversions are implementation-defined in C. The
 * x86-64 compiler contract is tested per implementation: current GCC widens
 * high-bit pointers by sign extension, while LCCC's Ptr-as-U64 model and Clang
 * zero-extend. No pointer is dereferenced and the expected values are built by
 * bit operations, not by repeating the conversion under test.
 *
 * The oracle harness must select the expected GCC-vs-LLVM family explicitly.
 */
#include <stdint.h>
#include <stdio.h>
#if !defined(__SIZEOF_INT128__) || __SIZEOF_POINTER__ != 8
#error "this implementation-defined oracle is specifically an LP64 __int128 test"
#endif
#ifndef POINTER_I128_SIGN_EXTENDS_HIGH_BIT
#error "the harness must specify the compiler's expected pointer extension"
#endif
#define NI __attribute__((noinline))
typedef unsigned __int128 uint128_t;
typedef __int128 int128_t;

static NI uint128_t pointer_to_u128(void *p) { return (uint128_t)p; }
static NI int128_t pointer_to_i128(void *p) { return (int128_t)p; }

static uint128_t expected_pointer_integer(uint64_t bits) {
    uint128_t expected = (uint128_t)bits;
#if POINTER_I128_SIGN_EXTENDS_HIGH_BIT
    if (bits >> 63)
        expected |= (uint128_t)UINT64_MAX << 64;
#endif
    return expected;
}

int main(void) {
    static const uint64_t patterns[] = {
        0x0000000000000000ULL,
        0x0000000000000001ULL,
        0x7fffffffffffffffULL,
        0x8000000000000001ULL,
        0xffffffff80000000ULL,
        UINT64_MAX,
    };
    for (unsigned i = 0; i < sizeof(patterns) / sizeof(patterns[0]); ++i) {
        volatile uint64_t bits = patterns[i];
        void *p = (void *)(uintptr_t)bits;
        uint128_t expected = expected_pointer_integer(bits);
        uint128_t as_unsigned = pointer_to_u128(p);
        uint128_t as_signed_bits = (uint128_t)pointer_to_i128(p);
        if (as_unsigned != expected || as_signed_bits != expected) {
            printf("pointer-to-int128 case %u mismatch\n", i);
            return 1;
        }
    }
    puts("pointer-to-int128 implementation-defined extension: PASS");
    return 0;
}
