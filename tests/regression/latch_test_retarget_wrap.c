/* A loop's test may be moved from the counter to the stepped counter.
 *
 * `do { … } while (--n);` reaches phi elimination as `n1 = n - 1; n != 1`:
 * the mid-level compares the operands of a difference that is tested for
 * zero. Left like that, `n` and `n1` are alive at once, the copy between
 * them cannot go, and the back edge gets a block of its own. Phi elimination
 * therefore turns `n ==/!= K` into `n1 ==/!= K ± k` wherever the block hands
 * `n1 = n ± k` to the phi of `n` (`retarget_loop_tests`).
 *
 * That is an identity only modulo the width of the counter, and only for
 * equality, so this file walks the cases where getting it wrong shows: every
 * integer width, both signs, steps of 1, 3 and 7 in both directions, bounds
 * at both ends of the range (the stepped bound wraps: `n != 0` with `n - 1`
 * is `n1 != 255` in eight bits), the test on either side of the step, the
 * step in the block of the test and at the top of a body of several blocks,
 * `==` with the branch the other way round, and the two 64-bit bounds whose
 * stepped value no longer fits a 32-bit immediate.
 *
 * Every loop folds the counter values it sees into a checksum, so a loop
 * that leaves on another trip, or walks other values, prints another line.
 * A loop that misses its bound in a narrow type comes round and still ends;
 * in 64 bits it does not, and the test times out.
 *
 * Signed counters stay inside their range throughout: nothing here relies on
 * signed overflow.
 */
#include <stdio.h>

#define NOINLINE __attribute__((noinline))

typedef unsigned long long u64;

/* A store on one arm only keeps the body of the two-block shapes in two
 * blocks: it cannot be turned into a select. */
static unsigned char odd_seen[8];

#define MIX(sum, n) ((sum) * 31u + (u64)(n))

/* --- the shapes, one function per type and bound ------------------------ */

/* The zero test of a difference: `--n` is `n1 = n - 1; n1 != 0`. */
#define PRE_DEC(TAG, T)                                                       \
    NOINLINE static u64 pre_dec_##TAG(T n)                                    \
    {                                                                         \
        u64 sum = 0;                                                          \
        do {                                                                  \
            sum = MIX(sum, n);                                                \
        } while (--n);                                                        \
        return sum;                                                           \
    }                                                                         \
    static u64 call_pre_dec_##TAG(long long start) { return pre_dec_##TAG((T)start); }

/* The same with the step at the top of a body of two blocks. */
#define PRE_DEC_TOP(TAG, T)                                                   \
    NOINLINE static u64 pre_dec_top_##TAG(T n)                                \
    {                                                                         \
        u64 sum = 0;                                                          \
        do {                                                                  \
            n--;                                                              \
            if (n & 1)                                                        \
                odd_seen[n & 7]++;                                            \
            else                                                              \
                sum ^= (u64)n;                                                \
            sum = MIX(sum, n);                                                \
        } while (n);                                                          \
        return sum;                                                           \
    }                                                                         \
    static u64 call_pre_dec_top_##TAG(long long start) { return pre_dec_top_##TAG((T)start); }

/* The test written on the old value, counting down and up. */
#define POST_STEP(TAG, T, KTAG, K)                                            \
    NOINLINE static u64 post_dec_##TAG##_##KTAG(T n)                          \
    {                                                                         \
        u64 sum = 0;                                                          \
        do {                                                                  \
            sum = MIX(sum, n);                                                \
        } while (n-- != (T)(K));                                              \
        return sum + (u64)n;                                                  \
    }                                                                         \
    static u64 call_post_dec_##TAG##_##KTAG(long long start)                  \
    {                                                                         \
        return post_dec_##TAG##_##KTAG((T)start);                             \
    }                                                                         \
    NOINLINE static u64 post_inc_##TAG##_##KTAG(T n)                          \
    {                                                                         \
        u64 sum = 0;                                                          \
        do {                                                                  \
            sum = MIX(sum, n);                                                \
        } while ((T)(K) != n++);                                              \
        return sum + (u64)n;                                                  \
    }                                                                         \
    static u64 call_post_inc_##TAG##_##KTAG(long long start)                  \
    {                                                                         \
        return post_inc_##TAG##_##KTAG((T)start);                             \
    }

/* `==`, and the branch that leaves the loop is the one taken. */
#define BREAK_EQ(TAG, T, KTAG, K)                                             \
    NOINLINE static u64 break_eq_##TAG##_##KTAG(T n)                          \
    {                                                                         \
        u64 sum = 0;                                                          \
        for (;;) {                                                            \
            sum = MIX(sum, n);                                                \
            if (n-- == (T)(K))                                                \
                break;                                                        \
        }                                                                     \
        return sum + (u64)n;                                                  \
    }                                                                         \
    static u64 call_break_eq_##TAG##_##KTAG(long long start)                  \
    {                                                                         \
        return break_eq_##TAG##_##KTAG((T)start);                             \
    }

/* A step other than one, the test on the value from before the step; in one
 * block, and with the step at the top of a body of two. */
#define WIDE_STEP(TAG, T, KTAG, K, STEP)                                      \
    NOINLINE static u64 down_##STEP##_##TAG##_##KTAG(T n)                     \
    {                                                                         \
        u64 sum = 0;                                                          \
        T old;                                                                \
        do {                                                                  \
            old = n;                                                          \
            sum = MIX(sum, n);                                                \
            n = (T)(n - STEP);                                                \
        } while (old != (T)(K));                                              \
        return sum + (u64)n;                                                  \
    }                                                                         \
    static u64 call_down_##STEP##_##TAG##_##KTAG(long long start)             \
    {                                                                         \
        return down_##STEP##_##TAG##_##KTAG((T)start);                        \
    }                                                                         \
    NOINLINE static u64 up_top_##STEP##_##TAG##_##KTAG(T n)                   \
    {                                                                         \
        u64 sum = 0;                                                          \
        T old;                                                                \
        do {                                                                  \
            old = n;                                                          \
            n = (T)(n + STEP);                                                \
            if (old & 1)                                                      \
                odd_seen[old & 7]++;                                          \
            else                                                              \
                sum ^= (u64)old;                                              \
            sum = MIX(sum, old);                                              \
        } while (old != (T)(K));                                              \
        return sum + (u64)n;                                                  \
    }                                                                         \
    static u64 call_up_top_##STEP##_##TAG##_##KTAG(long long start)           \
    {                                                                         \
        return up_top_##STEP##_##TAG##_##KTAG((T)start);                      \
    }

/* The bounds, the same expressions for every type: 0, 1 and 5, and the two
 * values below zero, which are the top of the range of an unsigned type. */
#define FOR_EACH_BOUND(X, TAG, T)                                             \
    X(TAG, T, k0, 0)                                                          \
    X(TAG, T, k1, 1)                                                          \
    X(TAG, T, k5, 5)                                                          \
    X(TAG, T, m1, -1)                                                         \
    X(TAG, T, m2, -2)

#define WIDE_STEP_3(TAG, T, KTAG, K) WIDE_STEP(TAG, T, KTAG, K, 3)
#define WIDE_STEP_7(TAG, T, KTAG, K) WIDE_STEP(TAG, T, KTAG, K, 7)

#define ALL_SHAPES(TAG, T)                                                    \
    PRE_DEC(TAG, T)                                                           \
    PRE_DEC_TOP(TAG, T)                                                       \
    FOR_EACH_BOUND(POST_STEP, TAG, T)                                         \
    FOR_EACH_BOUND(BREAK_EQ, TAG, T)                                          \
    FOR_EACH_BOUND(WIDE_STEP_3, TAG, T)                                       \
    FOR_EACH_BOUND(WIDE_STEP_7, TAG, T)

#define FOR_EACH_TYPE(X)                                                      \
    X(u8, unsigned char)                                                      \
    X(u16, unsigned short)                                                    \
    X(u32, unsigned int)                                                      \
    X(u64, unsigned long long)                                                \
    X(i8, signed char)                                                        \
    X(i16, short)                                                             \
    X(i32, int)                                                               \
    X(i64, long long)

FOR_EACH_TYPE(ALL_SHAPES)

/* 64 bits, where the bound is an immediate of 32: stepping 0x7fffffff up
 * or -0x80000000 down leaves that range, and the test stays as written. */
POST_STEP(i64, long long, top32, 0x7fffffffLL)
POST_STEP(i64, long long, bottom32, -0x7fffffffLL - 1)
POST_STEP(u64, unsigned long long, top32, 0x7fffffffULL)
POST_STEP(u64, unsigned long long, utop32, 0xffffffffULL)

/* --- the table ---------------------------------------------------------- */

enum walk {
    TO_ZERO, /* starts at the distance itself */
    DOWN,    /* starts above the bound */
    UP,      /* starts below it */
};

struct loop_case {
    const char *name;
    u64 (*run)(long long start);
    enum walk walk;
    long long bound;
    int step;
    /* A counter of at most 16 bits may start on its bound's far side and
     * come round. */
    int narrow_unsigned;
};

#define CASE_PRE_DEC(TAG, T)                                                  \
    { "pre_dec_" #TAG, call_pre_dec_##TAG, TO_ZERO, 0, 1,                     \
      (T)-1 > 0 && sizeof(T) <= 2 },                                          \
    { "pre_dec_top_" #TAG, call_pre_dec_top_##TAG, TO_ZERO, 0, 1,             \
      (T)-1 > 0 && sizeof(T) <= 2 },
#define CASE_POST_STEP(TAG, T, KTAG, K)                                       \
    { "post_dec_" #TAG "_" #KTAG, call_post_dec_##TAG##_##KTAG, DOWN,         \
      (long long)(T)(K), 1, 0 },                                              \
    { "post_inc_" #TAG "_" #KTAG, call_post_inc_##TAG##_##KTAG, UP,           \
      (long long)(T)(K), 1, 0 },
#define CASE_BREAK_EQ(TAG, T, KTAG, K)                                        \
    { "break_eq_" #TAG "_" #KTAG, call_break_eq_##TAG##_##KTAG, DOWN,         \
      (long long)(T)(K), 1, 0 },
#define CASE_WIDE_STEP(TAG, T, KTAG, K, STEP)                                 \
    { "down_" #STEP "_" #TAG "_" #KTAG, call_down_##STEP##_##TAG##_##KTAG,    \
      DOWN, (long long)(T)(K), STEP, 0 },                                     \
    { "up_top_" #STEP "_" #TAG "_" #KTAG,                                     \
      call_up_top_##STEP##_##TAG##_##KTAG, UP, (long long)(T)(K), STEP, 0 },
#define CASE_WIDE_STEP_3(TAG, T, KTAG, K) CASE_WIDE_STEP(TAG, T, KTAG, K, 3)
#define CASE_WIDE_STEP_7(TAG, T, KTAG, K) CASE_WIDE_STEP(TAG, T, KTAG, K, 7)

#define ALL_CASES(TAG, T)                                                     \
    CASE_PRE_DEC(TAG, T)                                                      \
    FOR_EACH_BOUND(CASE_POST_STEP, TAG, T)                                    \
    FOR_EACH_BOUND(CASE_BREAK_EQ, TAG, T)                                     \
    FOR_EACH_BOUND(CASE_WIDE_STEP_3, TAG, T)                                  \
    FOR_EACH_BOUND(CASE_WIDE_STEP_7, TAG, T)

static const struct loop_case cases[] = {
    FOR_EACH_TYPE(ALL_CASES)
    CASE_POST_STEP(i64, long long, top32, 0x7fffffffLL)
    CASE_POST_STEP(i64, long long, bottom32, -0x7fffffffLL - 1)
    CASE_POST_STEP(u64, unsigned long long, top32, 0x7fffffffULL)
    CASE_POST_STEP(u64, unsigned long long, utop32, 0xffffffffULL)
};

int main(void)
{
    /* How many steps the start is away from the bound. */
    static const int distances[] = { 0, 1, 2, 9 };
    u64 all = 0;

    for (unsigned c = 0; c < sizeof cases / sizeof cases[0]; c++) {
        const struct loop_case *lc = &cases[c];
        for (unsigned d = 0; d < sizeof distances / sizeof distances[0]; d++) {
            long long away = (long long)lc->step * distances[d];
            long long start;
            switch (lc->walk) {
            case TO_ZERO:
                /* From zero the counter goes all the way round, which only
                 * a narrow unsigned one does in a time worth waiting for. */
                if (distances[d] == 0 && !lc->narrow_unsigned)
                    continue;
                start = away;
                break;
            case DOWN:
                start = lc->bound + away;
                break;
            default:
                start = lc->bound - away;
                break;
            }
            u64 sum = lc->run(start);
            all = all * 1099511628211ULL + sum;
            printf("%-24s %2d away: %016llx\n", lc->name, distances[d], sum);
        }
    }
    unsigned odd = 0;
    for (unsigned i = 0; i < 8; i++)
        odd = odd * 257u + odd_seen[i];
    printf("all %016llx odd %08x\n", all, odd);
    return 0;
}
