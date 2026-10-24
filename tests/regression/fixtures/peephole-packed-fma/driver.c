/* Exact before/after comparison: no reassociation and no NaN normalization.
 * Exercise payloads, signaling NaNs, signed zero, subnormals and all four
 * rounding modes. The assembler functions differ only by the copy fold. */
#include <fenv.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

typedef float f4 __attribute__((vector_size(16)));
typedef float f8 __attribute__((vector_size(32)));
typedef double d2 __attribute__((vector_size(16)));
typedef double d4 __attribute__((vector_size(32)));
#define DECL(T) extern T original_##T(T,T,T,T); extern T optimized_##T(T,T,T,T);
DECL(f4) DECL(f8) DECL(d2) DECL(d4)

static uint64_t rng = UINT64_C(0x39471a2bf50d6c89);
static uint64_t next(void) {
    rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17;
    return rng;
}
static const uint32_t edge32[] = {
    0, 0x80000000u, 1, 0x80000001u, 0x007fffffu, 0x00800000u,
    0x3f800000u, 0xbf800000u, 0x7f7fffffu, 0xff7fffffu,
    0x7f800000u, 0xff800000u, 0x7fc12345u, 0xffc54321u,
    0x7f812345u, 0xff854321u
};
static const uint64_t edge64[] = {
    0, UINT64_C(0x8000000000000000), 1, UINT64_C(0x8000000000000001),
    UINT64_C(0x000fffffffffffff), UINT64_C(0x0010000000000000),
    UINT64_C(0x3ff0000000000000), UINT64_C(0xbff0000000000000),
    UINT64_C(0x7fefffffffffffff), UINT64_C(0xffefffffffffffff),
    UINT64_C(0x7ff0000000000000), UINT64_C(0xfff0000000000000),
    UINT64_C(0x7ff8123456789abc), UINT64_C(0xfff854321abcdef0),
    UINT64_C(0x7ff0123456789abc), UINT64_C(0xfff054321abcdef0)
};

#define CHECK(T, U, N, EDGES) do { \
    union { T v; U bits[N]; } a[4]; \
    for (unsigned trial = 0; trial < 4096; ++trial) { \
        for (unsigned arg = 0; arg < 4; ++arg) \
            for (unsigned lane = 0; lane < N; ++lane) \
                a[arg].bits[lane] = trial < 256 \
                    ? EDGES[(trial + arg * 3 + lane * 7) % 16] : (U)next(); \
        feclearexcept(FE_ALL_EXCEPT); \
        T before = original_##T(a[0].v, a[1].v, a[2].v, a[3].v); \
        int before_flags = fetestexcept(FE_ALL_EXCEPT); \
        feclearexcept(FE_ALL_EXCEPT); \
        T after = optimized_##T(a[0].v, a[1].v, a[2].v, a[3].v); \
        int after_flags = fetestexcept(FE_ALL_EXCEPT); \
        if (memcmp(&before, &after, sizeof before) || before_flags != after_flags) { \
            fprintf(stderr, #T " trial=%u rounding=%d flags=%x/%x\n", \
                    trial, fegetround(), before_flags, after_flags); \
            return 1; \
        } \
    } \
} while (0)

int main(void) {
    const int modes[] = {FE_TONEAREST, FE_DOWNWARD, FE_UPWARD, FE_TOWARDZERO};
    for (unsigned mode = 0; mode < 4; ++mode) {
        if (fesetround(modes[mode])) return 2;
        CHECK(f4, uint32_t, 4, edge32);
        CHECK(f8, uint32_t, 8, edge32);
        CHECK(d2, uint64_t, 2, edge64);
        CHECK(d4, uint64_t, 4, edge64);
    }
    puts("PASS: 65536 exact packed-FMA comparisons, including exception flags");
    return 0;
}
