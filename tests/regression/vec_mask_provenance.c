/* REGRESSION: lane-mask PROVENANCE and value-context range analysis in the
 * byte/word map vectorizer (follow-up to vec_mask_value_demask.c).
 *
 * A packed compare produces an all-ones / all-zeros lane mask while C's
 * `a > b` is the 0/1 boolean.  The byte-lane parser therefore tracks a
 * compare's *packed* range [-1, 0], and the vectorizer must never confuse
 * that with a plain numeric value that merely happens to lie in [-1, 0]:
 *
 *   1. `x + (p > 0 ? -1 : 0)`   a numeric select of -1/0 is NOT a mask;
 *                               demasking it (`& 1`) turned -1 into +1.
 *   2. `(a ? -1 : 0) & (b ? -1 : 0)` was recovered as a mask conjunction and
 *                               then demasked the same way.
 *   3. `(x > 5) > 255`          a compare consumed by an outer compare must
 *                               be range-analysed as C's 0/1, not the packed
 *                               [-1, 0]: the constant-wrapping side condition
 *                               wrapped 255 to -1 and the result became
 *                               `mask > -1`, i.e. inverted.
 *   4. selects of compare results (`c ? (a > k) : (b < k)`) consumed by an
 *                               outer compare / used as a condition / value.
 *   5. narrowing casts of compare results.
 *
 * Every kernel runs at byte, word and dword lanes (signed and unsigned), for
 * every trip count 0..70 (all vector/remainder boundaries) and three pointer
 * misalignments.  Each result is checked against an in-program SCALAR ORACLE
 * -- the same C expression evaluated through volatile loads, which no
 * vectorizer may touch -- and the suite additionally diffs stdout with GCC.
 * The expected output is therefore "PASS" plus a hash equal to GCC's.
 *
 * All arithmetic is defined C: 32-bit data is kept far from INT_MIN/INT_MAX.
 */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#define N 80
#define PAD 8

/* K(T, SFX, name, expr): `expr` may use x[i], p[i], q[i] and the lane type T. */
#define KERNEL_LIST(K, T, SFX)                                                      \
    /* 1. numeric -1/0 selects (never masks) */                                    \
    K(T, SFX, sel_neg1_add, x[i] + (p[i] > 0 ? -1 : 0))                            \
    K(T, SFX, sel_neg1_store, p[i] > 0 ? -1 : 0)                                   \
    K(T, SFX, sel_neg1_and_x, x[i] & (p[i] > 0 ? -1 : 0))                          \
    K(T, SFX, sel_neg1_or_x, x[i] | (p[i] > 0 ? -1 : 0))                           \
    K(T, SFX, sel_neg1_sub, x[i] - (p[i] > 0 ? -1 : 0))                            \
    K(T, SFX, sel_one_add, x[i] + (p[i] > 0 ? 1 : 0))                              \
    K(T, SFX, sel_neg1_cond, (p[i] > 0 ? -1 : 0) ? x[i] : (T)7)                    \
    /* 2. and/or/xor of two numeric -1/0 values */                                 \
    K(T, SFX, sel_and_sel, (p[i] > 0 ? -1 : 0) & (q[i] > 0 ? -1 : 0))              \
    K(T, SFX, sel_or_sel, (p[i] > 0 ? -1 : 0) | (q[i] < 3 ? -1 : 0))               \
    K(T, SFX, sel_xor_sel, (p[i] > 0 ? -1 : 0) ^ (q[i] < 3 ? -1 : 0))              \
    K(T, SFX, sel_and_sel_add,                                                      \
      x[i] + ((p[i] > 0 ? -1 : 0) & (q[i] > 0 ? -1 : 0)))                          \
    /* 3. a compare consumed by an outer compare */                                \
    K(T, SFX, cmp_gt255, (p[i] > 5) > 255)                                         \
    K(T, SFX, cmp_gt65535, (p[i] > 5) > 65535)                                     \
    K(T, SFX, cmp_ne255, (p[i] > 5) != 255)                                        \
    K(T, SFX, cmp_lt256, (p[i] > 5) < 256)                                         \
    K(T, SFX, cmp_eq_m1, (p[i] > 5) == -1)                                         \
    K(T, SFX, cmp_gt_m1, (p[i] > 5) > -1)                                          \
    K(T, SFX, cmp_ge_1, (p[i] > 5) >= 1)                                           \
    K(T, SFX, cmp_eq_1, (p[i] > 5) == 1)                                           \
    K(T, SFX, cmp_lt_1, (p[i] > 5) < 1)                                            \
    K(T, SFX, cmp_eq_cmp, (p[i] > 3) == (q[i] > 3))                                \
    K(T, SFX, cmp_lt_cmp, (p[i] > 3) < (q[i] > 3))                                 \
    K(T, SFX, cmp_gt255_cond, (p[i] > 5) > 255 ? x[i] : (T)9)                      \
    K(T, SFX, cmp_gt_m1_cond, (p[i] > 5) > -1 ? x[i] : (T)9)                       \
    /* 4. selects of compare results */                                            \
    K(T, SFX, selcmp_gt0, (p[i] > 3 ? (q[i] > 2) : (x[i] < 4)) > 0)                \
    K(T, SFX, selcmp_eq1, (p[i] > 3 ? (q[i] > 2) : 0) == 1)                        \
    K(T, SFX, selcmp_eq255, (p[i] > 3 ? (q[i] > 2) : (x[i] < 4)) == 255)           \
    K(T, SFX, selcmp_cond, (p[i] > 3 ? (q[i] > 2) : (x[i] < 4)) ? x[i] : (T)9)     \
    K(T, SFX, selcmp_value, (p[i] > 3 ? (q[i] > 2) : (x[i] < 4)))                  \
    K(T, SFX, selcmp_add, x[i] + (p[i] > 3 ? (q[i] > 2) : (x[i] < 4)))             \
    K(T, SFX, selcmp_mixed_arm, x[i] + (p[i] > 3 ? (q[i] > 2) : 2))                \
    K(T, SFX, land_cond, (p[i] > 3 && q[i] < 9) ? x[i] : (T)9)                     \
    K(T, SFX, land_add, x[i] + (p[i] > 3 && q[i] < 9))                             \
    K(T, SFX, lor_add, x[i] + (p[i] > 3 || q[i] < 2))                              \
    K(T, SFX, land_gt255, (p[i] > 3 && q[i] < 9) > 255)                            \
    /* 5. narrowing casts of compare results */                                    \
    K(T, SFX, cast_u8_add, (uint8_t)(p[i] > 5) + x[i])                             \
    K(T, SFX, cast_i8_add, (int8_t)(p[i] > 5) + x[i])                              \
    K(T, SFX, cast_t_xor, (T)(p[i] > 5) ^ x[i])

#define DEFINE_KERNEL(T, SFX, NAME, EXPR)                                           \
    __attribute__((noinline)) static void k_##NAME##_##SFX(                         \
        T *o, const T *x, const T *p, const T *q, long n) {                         \
        for (long i = 0; i < n; i++) o[i] = (T)(EXPR);                              \
    }                                                                               \
    __attribute__((noinline)) static void r_##NAME##_##SFX(                         \
        T *o, const volatile T *x, const volatile T *p, const volatile T *q,        \
        long n) {                                                                   \
        for (volatile long i = 0; i < n; i++) o[i] = (T)(EXPR);                     \
    }

#define TABLE_ENTRY(T, SFX, NAME, EXPR) {#NAME, k_##NAME##_##SFX, r_##NAME##_##SFX},

#define DEFINE_TYPE(T, SFX)                                                         \
    KERNEL_LIST(DEFINE_KERNEL, T, SFX)                                              \
    static int run_##SFX(uint64_t *hash) {                                          \
        static const struct {                                                       \
            const char *name;                                                       \
            void (*k)(T *, const T *, const T *, const T *, long);                  \
            void (*r)(T *, const volatile T *, const volatile T *,                  \
                      const volatile T *, long);                                    \
        } table[] = {KERNEL_LIST(TABLE_ENTRY, T, SFX)};                             \
        static T X[N + 2 * PAD], P[N + 2 * PAD], Q[N + 2 * PAD];                    \
        static T O[N + 2 * PAD], R[N + 2 * PAD];                                    \
        /* Compare operands cover both signs and the lane extremes; the data  */   \
        /* operand is zero at every 7th lane (255 vs 1 wrap) and stays far    */   \
        /* from the int range edges so the oracle expressions are defined C.  */   \
        static const long edge[] = {-128, -127, -6, -5, -1, 0, 1, 2, 3,             \
                                    4,    5,    6,  7,  8,  9, 10, 126, 127};       \
        uint64_t r = 88172645463325252ULL;                                          \
        int bad = 0;                                                                \
        for (int k = 0; k < N + 2 * PAD; k++) {                                     \
            r = r * 6364136223846793005ULL + 1442695040888963407ULL;                \
            P[k] = (T)edge[(r >> 33) % (sizeof edge / sizeof *edge)];               \
            r = r * 6364136223846793005ULL + 1442695040888963407ULL;                \
            Q[k] = (T)edge[(r >> 33) % (sizeof edge / sizeof *edge)];               \
            r = r * 6364136223846793005ULL + 1442695040888963407ULL;                \
            X[k] = sizeof(T) < 4                                                    \
                       ? (T)(r >> 24)                                               \
                       : (T)((int64_t)((r >> 24) % 2000001) - 1000000);             \
            if (k % 7 == 0) X[k] = 0;                                               \
        }                                                                           \
        for (size_t t = 0; t < sizeof table / sizeof *table; t++) {                 \
            int kbad = 0;                                                           \
            for (long n = 0; n <= 70; n++) {                                        \
                for (int off = 0; off < 3; off++) {                                 \
                    memset(O, 0x5a, sizeof O);                                      \
                    memset(R, 0x5a, sizeof R);                                      \
                    table[t].k(O + PAD + off, X + PAD + (off ^ 1), P + PAD + off,   \
                               Q + PAD + off * 2, n);                               \
                    table[t].r(R + PAD + off, X + PAD + (off ^ 1), P + PAD + off,   \
                               Q + PAD + off * 2, n);                               \
                    for (size_t b = 0; b < sizeof O; b++) {                         \
                        *hash ^= ((unsigned char *)O)[b];                           \
                        *hash *= 1099511628211ULL;                                  \
                    }                                                               \
                    if (memcmp(O, R, sizeof O) != 0) {                              \
                        bad++;                                                      \
                        if (kbad++ == 0) {                                          \
                            size_t e = 0;                                           \
                            while (((unsigned char *)O)[e] ==                       \
                                   ((unsigned char *)R)[e])                         \
                                e++;                                                \
                            printf("MISMATCH %s %s n=%ld off=%d byte=%zu got=%02x " \
                                   "want=%02x\n",                                   \
                                   #SFX, table[t].name, n, off, e,                  \
                                   ((unsigned char *)O)[e], ((unsigned char *)R)[e]); \
                        }                                                           \
                    }                                                               \
                }                                                                   \
            }                                                                       \
        }                                                                           \
        return bad;                                                                 \
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
