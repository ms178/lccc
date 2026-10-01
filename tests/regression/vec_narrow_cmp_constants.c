/* REGRESSION: out-of-domain compare constants in the byte/word map vectorizer.
 *
 * The lane-compare parser may rewrite a compare constant modulo the lane
 * width -- but ONLY when the compare itself is carried out at the lane width
 * (C's `(unsigned char)x == 0xAA` arrives as a U8 compare against the I8
 * spelling -86; both are the same lane value).  For a WIDE (`int`) compare the
 * constant is an exact wide value, and wrapping it is a miscompile:
 *
 *     uint8_t x:  x > -1      is always 1      (wrapped: x >u 255  -> 0)
 *     uint8_t x:  x < -86     is always 0      (wrapped: x <u 170)
 *     int8_t  x:  x == 170    is always 0      (wrapped: x == -86)
 *     uint8_t x:  x < 300     is always 1      (wrapped: x <u 44)
 *     uint8_t x:  x == 256    is always 0      (wrapped: x == 0)
 *
 * Fourteen such kernels diverged from GCC at the PR #705 baseline.  The
 * legitimate narrow spellings (`(uint8_t)x == 0xAA`, `(int8_t)x == -86`) are
 * kept in the matrix so the fix cannot "win" by refusing to vectorize them.
 * Each result is checked against an in-program scalar oracle (volatile loads)
 * and the suite diffs the output with GCC. */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#define N 96
#define PAD 8

/* K(T, SFX, name, expr): `expr` may use x[i] and the lane type T. */
#define KERNEL_LIST(K, T, SFX)                                                   \
    K(T, SFX, gt_m1, x[i] > -1)                                                  \
    K(T, SFX, ge_m1, x[i] >= -1)                                                 \
    K(T, SFX, lt_m86, x[i] < -86)                                                \
    K(T, SFX, eq_m86, x[i] == -86)                                               \
    K(T, SFX, ne_m86, x[i] != -86)                                               \
    K(T, SFX, ge_m200, x[i] >= -200)                                             \
    K(T, SFX, eq_170, x[i] == 170)                                               \
    K(T, SFX, gt_170, x[i] > 170)                                                \
    K(T, SFX, lt_170, x[i] < 170)                                                \
    K(T, SFX, eq_255, x[i] == 255)                                               \
    K(T, SFX, eq_256, x[i] == 256)                                               \
    K(T, SFX, lt_300, x[i] < 300)                                                \
    K(T, SFX, gt_300, x[i] > 300)                                                \
    K(T, SFX, eq_32768, x[i] == 32768)                                           \
    K(T, SFX, gt_65535, x[i] > 65535)                                            \
    K(T, SFX, lt_70000, x[i] < 70000)                                            \
    K(T, SFX, ge_m40000, x[i] >= -40000)                                         \
    K(T, SFX, sel_gt_300, x[i] > 300 ? 1 : 2)                                    \
    K(T, SFX, sel_lt_m86, x[i] < -86 ? x[i] : (T)5)                              \
    K(T, SFX, and_cmp, (x[i] > -1) & (x[i] < 300))                               \
    /* legitimate lane-typed spellings: must stay exact (and vectorized) */      \
    K(T, SFX, u8_eq_aa, (uint8_t)x[i] == 0xAA)                                   \
    K(T, SFX, u8_gt_7f, (uint8_t)x[i] > 0x7F)                                    \
    K(T, SFX, i8_eq_m86, (int8_t)x[i] == -86)                                    \
    K(T, SFX, i8_lt_0, (int8_t)x[i] < 0)                                         \
    K(T, SFX, u16_ge_8000, (uint16_t)x[i] >= 0x8000)                             \
    K(T, SFX, i16_eq_m2, (int16_t)x[i] == -2)

#define DEFINE_KERNEL(T, SFX, NAME, EXPR)                                        \
    __attribute__((noinline)) static void k_##NAME##_##SFX(T *o, const T *x,     \
                                                           long n) {            \
        for (long i = 0; i < n; i++) o[i] = (T)(EXPR);                           \
    }                                                                            \
    __attribute__((noinline)) static void r_##NAME##_##SFX(                      \
        T *o, const volatile T *x, long n) {                                     \
        for (volatile long i = 0; i < n; i++) o[i] = (T)(EXPR);                  \
    }

#define TABLE_ENTRY(T, SFX, NAME, EXPR) {#NAME, k_##NAME##_##SFX, r_##NAME##_##SFX},

#define DEFINE_TYPE(T, SFX)                                                      \
    KERNEL_LIST(DEFINE_KERNEL, T, SFX)                                           \
    static int run_##SFX(uint64_t *hash) {                                       \
        static const struct {                                                    \
            const char *name;                                                    \
            void (*k)(T *, const T *, long);                                     \
            void (*r)(T *, const volatile T *, long);                            \
        } table[] = {KERNEL_LIST(TABLE_ENTRY, T, SFX)};                          \
        static T X[N + 2 * PAD], O[N + 2 * PAD], R[N + 2 * PAD];                 \
        static const long edge[] = {0,   1,   2,   85,  86,  127, 128, 129, 169, \
                                    170, 171, 254, 255, 256, 257, 300, 301, -1,  \
                                    -2,  -86, -87, -127, -128, -129, 32767,      \
                                    32768, 32769, 65535, 40000, -32768};         \
        uint64_t r = 88172645463325252ULL;                                       \
        int bad = 0;                                                             \
        for (int k = 0; k < N + 2 * PAD; k++) {                                  \
            r = r * 6364136223846793005ULL + 1442695040888963407ULL;             \
            X[k] = (T)edge[(r >> 33) % (sizeof edge / sizeof *edge)];            \
        }                                                                        \
        for (size_t t = 0; t < sizeof table / sizeof *table; t++) {              \
            int kbad = 0;                                                        \
            for (long n = 0; n <= 80; n++) {                                     \
                for (int off = 0; off < 3; off++) {                              \
                    memset(O, 0x5a, sizeof O);                                   \
                    memset(R, 0x5a, sizeof R);                                   \
                    table[t].k(O + PAD + off, X + PAD + (off ^ 1), n);           \
                    table[t].r(R + PAD + off, X + PAD + (off ^ 1), n);           \
                    for (size_t b = 0; b < sizeof O; b++) {                      \
                        *hash ^= ((unsigned char *)O)[b];                        \
                        *hash *= 1099511628211ULL;                               \
                    }                                                            \
                    if (memcmp(O, R, sizeof O) != 0) {                           \
                        bad++;                                                   \
                        if (kbad++ == 0) {                                       \
                            size_t e = 0;                                        \
                            while (((unsigned char *)O)[e] ==                    \
                                   ((unsigned char *)R)[e])                      \
                                e++;                                             \
                            printf("MISMATCH %s %s n=%ld off=%d byte=%zu\n",     \
                                   #SFX, table[t].name, n, off, e);              \
                        }                                                        \
                    }                                                            \
                }                                                                \
            }                                                                    \
        }                                                                        \
        return bad;                                                              \
    }

DEFINE_TYPE(int8_t, i8)
DEFINE_TYPE(uint8_t, u8)
DEFINE_TYPE(int16_t, i16)
DEFINE_TYPE(uint16_t, u16)
DEFINE_TYPE(int32_t, i32)
DEFINE_TYPE(uint32_t, u32)

int main(void) {
    uint64_t h = 1469598103934665603ULL;
    int bad = 0;
    bad += run_i8(&h);
    bad += run_u8(&h);
    bad += run_i16(&h);
    bad += run_u16(&h);
    bad += run_i32(&h);
    bad += run_u32(&h);
    printf("%s %016llx\n", bad ? "FAIL" : "PASS", (unsigned long long)h);
    return bad != 0;
}
