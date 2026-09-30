/* REGRESSION: a compare used as a VALUE inside a vectorized elementwise map.
 *
 * A packed compare yields all-ones (-1) per true lane, while C's `a > b` is
 * the 0/1 boolean.  The map vectorizer lowers one expression tree twice --
 * packed body and scalar remainder -- and the byte-lane parser (and the
 * dword parser) accepted a mask as an ARITHMETIC operand, so
 * `a[i] += (b[i] > k)` added -1 per true lane in the vector body but +1 in
 * the remainder loop.  Found by scripts/vectorize_stress.py (cond_inc_i8).
 *
 * Every shape below consumes a compare as a value (add/sub/and/or/xor
 * operand, stored root, select arm, negation, `!`), at byte, word and dword
 * lanes, for trip counts that straddle every vector/remainder boundary and
 * for misaligned pointers.  Output is compared against GCC by the suite. */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#define N 200
#define PAD 8

#define DEFINE_KERNELS(T, SFX)                                                              \
    __attribute__((noinline)) static void cond_inc_##SFX(T *a, const T *b, long n) {        \
        for (long i = 0; i < n; i++) a[i] += (b[i] > (T)5);                                 \
    }                                                                                       \
    __attribute__((noinline)) static void add2_##SFX(T *a, const T *b, const T *c, long n) {\
        for (long i = 0; i < n; i++) a[i] = (b[i] > (T)5) + (c[i] < (T)7);                  \
    }                                                                                       \
    __attribute__((noinline)) static void sub_##SFX(T *a, const T *b, const T *c, long n) { \
        for (long i = 0; i < n; i++) a[i] = b[i] - (c[i] > (T)5);                           \
    }                                                                                       \
    __attribute__((noinline)) static void and_val_##SFX(T *a, const T *b, const T *c,       \
                                                        long n) {                           \
        for (long i = 0; i < n; i++) a[i] = b[i] & (c[i] > (T)5);                           \
    }                                                                                       \
    __attribute__((noinline)) static void or_val_##SFX(T *a, const T *b, const T *c,        \
                                                       long n) {                            \
        for (long i = 0; i < n; i++) a[i] = b[i] | (c[i] > (T)5);                           \
    }                                                                                       \
    __attribute__((noinline)) static void xor_eq_##SFX(T *a, const T *b, const T *c,        \
                                                       long n) {                            \
        for (long i = 0; i < n; i++) a[i] ^= (b[i] == c[i]);                                \
    }                                                                                       \
    __attribute__((noinline)) static void and2_##SFX(T *a, const T *b, const T *c, long n) {\
        for (long i = 0; i < n; i++) a[i] = (b[i] > (T)5) & (c[i] < (T)7);                  \
    }                                                                                       \
    __attribute__((noinline)) static void or2_##SFX(T *a, const T *b, const T *c, long n) { \
        for (long i = 0; i < n; i++) a[i] = (b[i] > (T)5) | (c[i] < (T)7);                  \
    }                                                                                       \
    __attribute__((noinline)) static void xor2_##SFX(T *a, const T *b, const T *c, long n) {\
        for (long i = 0; i < n; i++) a[i] = (b[i] > (T)5) ^ (c[i] < (T)7);                  \
    }                                                                                       \
    __attribute__((noinline)) static void store_##SFX(T *a, const T *b, long n) {           \
        for (long i = 0; i < n; i++) a[i] = b[i] > (T)5;                                    \
    }                                                                                       \
    __attribute__((noinline)) static void neg_##SFX(T *a, const T *b, long n) {             \
        for (long i = 0; i < n; i++) a[i] = -(b[i] > (T)5);                                 \
    }                                                                                       \
    __attribute__((noinline)) static void not_##SFX(T *a, const T *b, long n) {             \
        for (long i = 0; i < n; i++) a[i] = !(b[i] == (T)5);                                \
    }                                                                                       \
    __attribute__((noinline)) static void arm_##SFX(T *a, const T *b, const T *c, long n) { \
        for (long i = 0; i < n; i++) a[i] = c[i] > (T)5 ? (b[i] > (T)5) : 2;                \
    }                                                                                       \
    __attribute__((noinline)) static void arm2_##SFX(T *a, const T *b, const T *c, long n) {\
        for (long i = 0; i < n; i++) a[i] = c[i] > (T)5 ? 2 : (b[i] == c[i]);               \
    }                                                                                       \
    __attribute__((noinline)) static void acc_##SFX(T *a, const T *b, long n) {             \
        for (long i = 0; i < n; i++) a[i] = a[i] + (b[i] > (T)5) + (b[i] < (T)-5);          \
    }                                                                                       \
    static uint64_t run_##SFX(void) {                                                       \
        static T A[N + 2 * PAD], A0[N + 2 * PAD], B[N + 2 * PAD], C[N + 2 * PAD];           \
        uint64_t h = 1469598103934665603ULL, r = 88172645463325252ULL;                      \
        for (int k = 0; k < N + 2 * PAD; k++) {                                             \
            r = r * 6364136223846793005ULL + 1442695040888963407ULL;                        \
            B[k] = (T)((r >> 24) % 13 - 3);                                                 \
            r = r * 6364136223846793005ULL + 1442695040888963407ULL;                        \
            C[k] = (T)((r >> 24) % 13 - 3);                                                 \
            A0[k] = (T)(k * 7);                                                             \
        }                                                                                   \
        B[PAD + 5] = (T)-128; /* extremes: the biased-compare constants wrap */             \
        C[PAD + 6] = (T)127;                                                                \
        for (long n = 0; n <= 70; n++) {                                                    \
            for (int off = 0; off < 3; off++) {                                             \
                T *a = A + PAD + off, *b = B + PAD + (off ^ 1), *c = C + PAD + off * 2;     \
                void (*k2[])(T *, const T *, const T *, long) = {                           \
                    add2_##SFX, sub_##SFX, and_val_##SFX, or_val_##SFX, xor_eq_##SFX,       \
                    and2_##SFX, or2_##SFX, xor2_##SFX, arm_##SFX, arm2_##SFX};              \
                void (*k1[])(T *, const T *, long) = {cond_inc_##SFX, store_##SFX,          \
                                                      neg_##SFX,      not_##SFX,            \
                                                      acc_##SFX};                           \
                for (size_t q = 0; q < sizeof k2 / sizeof *k2; q++) {                       \
                    memcpy(A, A0, sizeof A);                                                \
                    k2[q](a, b, c, n);                                                      \
                    for (size_t x = 0; x < sizeof A; x++) {                                 \
                        h ^= ((unsigned char *)A)[x];                                       \
                        h *= 1099511628211ULL;                                              \
                    }                                                                       \
                }                                                                           \
                for (size_t q = 0; q < sizeof k1 / sizeof *k1; q++) {                       \
                    memcpy(A, A0, sizeof A);                                                \
                    k1[q](a, b, n);                                                         \
                    for (size_t x = 0; x < sizeof A; x++) {                                 \
                        h ^= ((unsigned char *)A)[x];                                       \
                        h *= 1099511628211ULL;                                              \
                    }                                                                       \
                }                                                                           \
            }                                                                               \
        }                                                                                   \
        return h;                                                                           \
    }

DEFINE_KERNELS(int8_t, i8)
DEFINE_KERNELS(uint8_t, u8)
DEFINE_KERNELS(int16_t, i16)
DEFINE_KERNELS(uint16_t, u16)
DEFINE_KERNELS(int32_t, i32)
DEFINE_KERNELS(uint32_t, u32)

int main(void) {
    printf("i8  %016llx\n", (unsigned long long)run_i8());
    printf("u8  %016llx\n", (unsigned long long)run_u8());
    printf("i16 %016llx\n", (unsigned long long)run_i16());
    printf("u16 %016llx\n", (unsigned long long)run_u16());
    printf("i32 %016llx\n", (unsigned long long)run_i32());
    printf("u32 %016llx\n", (unsigned long long)run_u32());
    return 0;
}
