// FLAGS: -mavx
/* The 32 predicates of the floating-point compares, at 128 and 256 bits.
 *
 * `cmpps`/`cmppd` encode eight of them; the other 24 exist in the VEX
 * encoding only. A 128-bit compare with one of the 24 was written as the
 * legacy instruction, which ignores the upper bits of the immediate:
 * `_mm_cmp_pd(a, b, _CMP_GE_OQ)` (13) computed NLT (5), the opposite answer
 * for a NaN. Whether the legacy spelling survived depended on a late
 * rewrite that leaves an instruction alone in front of a call, so the
 * 128-bit rows print from the function that compares, with no 256-bit code
 * beside them. Each pair of operands is one of the four relations a compare
 * distinguishes (less, equal, greater, unordered), so a predicate taken for
 * another shows in at least one lane. */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

#define ROW_PD(WHAT, P, VALUE)                                                    \
    do {                                                                          \
        __m128d r_ = (VALUE);                                                     \
        unsigned long long u_[2];                                                 \
        memcpy(u_, &r_, sizeof u_);                                               \
        printf("%s %2d: %016llx %016llx\n", WHAT, P, u_[0], u_[1]);               \
    } while (0)

#define ROW_PS(WHAT, P, VALUE)                                                    \
    do {                                                                          \
        __m128 r_ = (VALUE);                                                      \
        unsigned u_[4];                                                           \
        memcpy(u_, &r_, sizeof u_);                                               \
        printf("%s %2d: %08x %08x %08x %08x\n", WHAT, P, u_[0], u_[1], u_[2], u_[3]); \
    } while (0)

#define EVERY_PREDICATE(ROWS)                                                     \
    ROWS(0) ROWS(1) ROWS(2) ROWS(3) ROWS(4) ROWS(5) ROWS(6) ROWS(7)               \
    ROWS(8) ROWS(9) ROWS(10) ROWS(11) ROWS(12) ROWS(13) ROWS(14) ROWS(15)         \
    ROWS(16) ROWS(17) ROWS(18) ROWS(19) ROWS(20) ROWS(21) ROWS(22) ROWS(23)       \
    ROWS(24) ROWS(25) ROWS(26) ROWS(27) ROWS(28) ROWS(29) ROWS(30) ROWS(31)

__attribute__((noinline)) static void doubles(void)
{
    volatile double nan = __builtin_nan(""), one = 1.0, two = 2.0;
    /* lanes: less | unordered, and equal | greater */
    __m128d lt_un_a = _mm_set_pd(nan, one), lt_un_b = _mm_set_pd(one, two);
    __m128d eq_gt_a = _mm_set_pd(two, one), eq_gt_b = _mm_set_pd(one, one);
    __m128d un_lt_a = _mm_set_pd(one, nan);

#define ROWS(P)                                                                   \
    ROW_PD("pd lt|un", P, _mm_cmp_pd(lt_un_a, lt_un_b, P));                       \
    ROW_PD("pd eq|gt", P, _mm_cmp_pd(eq_gt_a, eq_gt_b, P));                       \
    ROW_PD("sd lt   ", P, _mm_cmp_sd(lt_un_a, lt_un_b, P));                       \
    ROW_PD("sd un   ", P, _mm_cmp_sd(un_lt_a, lt_un_b, P));
    EVERY_PREDICATE(ROWS)
#undef ROWS
}

__attribute__((noinline)) static void floats(void)
{
    volatile float nan = __builtin_nanf(""), one = 1.0f, two = 2.0f;
    /* lanes: less, equal, greater, unordered */
    __m128 a = _mm_set_ps(nan, two, one, one), b = _mm_set_ps(one, one, one, two);
    __m128 un = _mm_set_ps(one, one, one, nan);

#define ROWS(P)                                                                   \
    ROW_PS("ps      ", P, _mm_cmp_ps(a, b, P));                                   \
    ROW_PS("ss lt   ", P, _mm_cmp_ss(a, b, P));                                   \
    ROW_PS("ss un   ", P, _mm_cmp_ss(un, b, P));
    EVERY_PREDICATE(ROWS)
#undef ROWS
}

__attribute__((noinline)) static void wide(void)
{
    volatile float nan = __builtin_nanf(""), one = 1.0f, two = 2.0f;
    volatile double dnan = __builtin_nan(""), done = 1.0, dtwo = 2.0;
    __m256 a = _mm256_set_ps(one, nan, two, one, nan, two, one, one);
    __m256 b = _mm256_set_ps(nan, nan, one, one, one, one, one, two);
    __m256d c = _mm256_set_pd(dnan, dtwo, done, done), d = _mm256_set_pd(done, done, done, dtwo);

#define ROWS(P)                                                                   \
    do {                                                                          \
        __m256 r_ = _mm256_cmp_ps(a, b, P);                                       \
        __m256d q_ = _mm256_cmp_pd(c, d, P);                                      \
        unsigned u_[8];                                                           \
        unsigned long long v_[4];                                                 \
        memcpy(u_, &r_, sizeof u_);                                               \
        memcpy(v_, &q_, sizeof v_);                                               \
        printf("ps256 %2d: %08x %08x %08x %08x %08x %08x %08x %08x\n", P, u_[0], u_[1], \
               u_[2], u_[3], u_[4], u_[5], u_[6], u_[7]);                         \
        printf("pd256 %2d: %016llx %016llx %016llx %016llx\n", P, v_[0], v_[1], v_[2], v_[3]); \
    } while (0);
    EVERY_PREDICATE(ROWS)
#undef ROWS
}

int main(void)
{
    doubles();
    floats();
    wide();
    return 0;
}
