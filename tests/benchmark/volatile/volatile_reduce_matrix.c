/* Parameterized diagnostic kernel; no timing thresholds belong in CI.
 * KIND: add, mul, and, or, xor. ADDRESS: pointer, TLS. MANUAL_LANES: 0/2/4. */
#include <stdint.h>
#ifndef WIDTH
#define WIDTH 64
#endif
#ifndef KIND
#define KIND 0
#endif
#ifndef ADDRESS
#define ADDRESS 0
#endif
#ifndef PRESSURE
#define PRESSURE 0
#endif
#ifndef MANUAL_LANES
#define MANUAL_LANES 0
#endif
#if (WIDTH != 32 && WIDTH != 64) || KIND < 0 || KIND > 4 || \
    ADDRESS < 0 || ADDRESS > 1 || PRESSURE < 0 || PRESSURE > 1 || \
    (MANUAL_LANES != 0 && MANUAL_LANES != 2 && MANUAL_LANES != 4)
#error Invalid volatile reduction diagnostic configuration
#endif
#if WIDTH == 32
typedef uint32_t T;
#else
typedef uint64_t T;
#endif
#if KIND == 0
#define UPDATE +=
#define COMBINE +
#define IDENTITY 0
#elif KIND == 1
#define UPDATE *=
#define COMBINE *
#define IDENTITY 1
#elif KIND == 2
#define UPDATE &=
#define COMBINE &
#define IDENTITY ((T)-1)
#elif KIND == 3
#define UPDATE |=
#define COMBINE |
#define IDENTITY 0
#else
#define UPDATE ^=
#define COMBINE ^
#define IDENTITY 0
#endif
__thread volatile T volatile_matrix_value = (T)UINT64_C(0x9e3779b97f4a7c15);
#if ADDRESS
#define READ volatile_matrix_value
#else
#define READ (*p)
#endif
__attribute__((noinline)) uint64_t reduce(volatile void *pointer, uint64_t n, uint64_t seed,
    uint64_t a0, uint64_t a1, uint64_t a2, uint64_t a3,
    uint64_t a4, uint64_t a5, uint64_t a6, uint64_t a7) {
    volatile T *p = pointer;
    (void)p;
    (void)a0; (void)a1; (void)a2; (void)a3;
    (void)a4; (void)a5; (void)a6; (void)a7;
    T result = (T)seed;
    uint64_t i = 0;
#if MANUAL_LANES
    T b = IDENTITY;
#if MANUAL_LANES == 4
    T c = IDENTITY, d = IDENTITY;
#endif
    for (; i < (n & ~(uint64_t)(MANUAL_LANES-1)); i += MANUAL_LANES) {
        result UPDATE READ;
        b UPDATE READ;
#if MANUAL_LANES == 4
        c UPDATE READ;
        d UPDATE READ;
#endif
    }
    result = result COMBINE b;
#if MANUAL_LANES == 4
    result = result COMBINE (c COMBINE d);
#endif
#endif
    for (; i < n; ++i) result UPDATE READ;
#if PRESSURE
    return (uint64_t)result ^ a0 ^ a1 ^ a2 ^ a3 ^ a4 ^ a5 ^ a6 ^ a7;
#else
    return result;
#endif
}
