/* `res = res * base + val` with an 8- or 16-bit `val` or `base`.
 *
 * On i686 a 64-bit `res * base + val` is one fused sequence, and the cast
 * that widens `val` emits nothing: the sequence reads `val` where it lives.
 * It read all 32 bits of that home. Narrow arithmetic is not cut back in
 * its home (the casts that read it do the cutting), so after
 *
 *     unsigned short n = 0;
 *     n--;
 *     sum = sum * 31 + n;
 *
 * the home of `n` holds 0xffffffff and the sum got that where the program
 * says 0xffff. A signed char that went from 127 to -128 holds 128 there,
 * and was added as 128. The same for a narrow `base`, which the multiply
 * read whole. (Found by tests/regression/latch_test_retarget_wrap.c built
 * for i686: 47 of its 1,205 lines differed from GCC's, in every function
 * with a 16-bit counter that comes round.)
 *
 * Each recurrence below runs twice: as written, which is the fused shape,
 * and with the operand sent through a volatile 64-bit object first, which
 * no fusion reaches. The two must agree, and the program says so itself
 * (exit status), so that it also tells on a target the corpus does not
 * compare against GCC. Every start is chosen so that the narrow value
 * wraps within the first trips, in both directions, for both signs; the
 * last function keeps enough values alive that some of them live in stack
 * slots, which the sequence reads through another path than registers.
 */
#include <stdio.h>

#define NOINLINE __attribute__((noinline))

typedef unsigned long long u64;

/* The addend is the narrow value: `(u64)n` of an `n` that just wrapped. */
#define ADDEND(TAG, T, STEP)                                                  \
    NOINLINE static u64 addend_fused_##TAG(T n, unsigned trips)               \
    {                                                                         \
        u64 sum = 1;                                                          \
        while (trips--) {                                                     \
            n = (T)(n STEP);                                                  \
            sum = sum * 31u + (u64)n;                                         \
        }                                                                     \
        return sum;                                                           \
    }                                                                         \
    NOINLINE static u64 addend_plain_##TAG(T n, unsigned trips)               \
    {                                                                         \
        u64 sum = 1;                                                          \
        volatile u64 wide;                                                    \
        while (trips--) {                                                     \
            n = (T)(n STEP);                                                  \
            wide = (u64)n;                                                    \
            sum = sum * 31u + wide;                                           \
        }                                                                     \
        return sum;                                                           \
    }

/* The base is the narrow value. */
#define BASE(TAG, T, STEP)                                                    \
    NOINLINE static u64 base_fused_##TAG(T b, unsigned trips)                 \
    {                                                                         \
        u64 sum = 1;                                                          \
        while (trips--) {                                                     \
            b = (T)(b STEP);                                                  \
            sum = sum * (u64)b + 12345u;                                      \
        }                                                                     \
        return sum;                                                           \
    }                                                                         \
    NOINLINE static u64 base_plain_##TAG(T b, unsigned trips)                 \
    {                                                                         \
        u64 sum = 1;                                                          \
        volatile u64 wide;                                                    \
        while (trips--) {                                                     \
            b = (T)(b STEP);                                                  \
            wide = (u64)b;                                                    \
            sum = sum * wide + 12345u;                                        \
        }                                                                     \
        return sum;                                                           \
    }

ADDEND(u8_up, unsigned char, +1)
ADDEND(u8_down, unsigned char, -1)
ADDEND(u16_up, unsigned short, +1)
ADDEND(u16_down, unsigned short, -1)
ADDEND(i8_up, signed char, +1)
ADDEND(i8_down, signed char, -1)
ADDEND(i16_up, short, +1)
ADDEND(i16_down, short, -1)
ADDEND(u8_by_100, unsigned char, +100)
ADDEND(i16_by_9000, short, -9000)

BASE(u8_up, unsigned char, +1)
BASE(u8_down, unsigned char, -1)
BASE(u16_up, unsigned short, +1)
BASE(u16_down, unsigned short, -1)

/* Four recurrences at once: more values than there are registers. */
NOINLINE static u64 crowd_fused(unsigned char a, unsigned short b, signed char c, short d,
                                unsigned trips)
{
    u64 s1 = 1, s2 = 2, s3 = 3, s4 = 4;
    while (trips--) {
        a = (unsigned char)(a + 3);
        b = (unsigned short)(b - 5);
        c = (signed char)(c + 7);
        d = (short)(d - 9);
        s1 = s1 * 31u + (u64)a;
        s2 = s2 * 33u + (u64)b;
        s3 = s3 * 35u + (u64)c;
        s4 = s4 * 37u + (u64)d;
    }
    return s1 ^ (s2 << 1) ^ (s3 << 2) ^ (s4 << 3);
}

NOINLINE static u64 crowd_plain(unsigned char a, unsigned short b, signed char c, short d,
                                unsigned trips)
{
    u64 s1 = 1, s2 = 2, s3 = 3, s4 = 4;
    volatile u64 wide;
    while (trips--) {
        a = (unsigned char)(a + 3);
        b = (unsigned short)(b - 5);
        c = (signed char)(c + 7);
        d = (short)(d - 9);
        wide = (u64)a;
        s1 = s1 * 31u + wide;
        wide = (u64)b;
        s2 = s2 * 33u + wide;
        wide = (u64)c;
        s3 = s3 * 35u + wide;
        wide = (u64)d;
        s4 = s4 * 37u + wide;
    }
    return s1 ^ (s2 << 1) ^ (s3 << 2) ^ (s4 << 3);
}

static int failed;

static void same(const char *name, long long start, u64 fused, u64 plain)
{
    printf("%-20s %6lld: %016llx%s\n", name, start, fused, fused == plain ? "" : "  DIFFERS");
    if (fused != plain) {
        printf("%-20s %6s  %016llx  as written through memory\n", "", "", plain);
        failed = 1;
    }
}

/* Starts and trip counts come out of volatile objects: known to the
 * compiler, the loops would be run by it and leave nothing to fuse. */
static volatile long long opaque_start;
static volatile unsigned opaque_trips;

static long long start_of(long long start)
{
    opaque_start = start;
    return opaque_start;
}

static unsigned trips_of(unsigned trips)
{
    opaque_trips = trips;
    return opaque_trips;
}

#define RUN(KIND, TAG, T, START)                                              \
    do {                                                                      \
        T from = (T)start_of(START);                                          \
        unsigned trips = trips_of(6);                                         \
        same(#KIND "_" #TAG, (long long)from, KIND##_fused_##TAG(from, trips), \
             KIND##_plain_##TAG(from, trips));                                \
    } while (0)
#define RUN_ADDEND(TAG, T, START) RUN(addend, TAG, T, START)
#define RUN_BASE(TAG, T, START) RUN(base, TAG, T, START)

int main(void)
{
    /* Each start is three steps short of the end of its range. */
    RUN_ADDEND(u8_up, unsigned char, 253);
    RUN_ADDEND(u8_down, unsigned char, 2);
    RUN_ADDEND(u16_up, unsigned short, 65533);
    RUN_ADDEND(u16_down, unsigned short, 2);
    RUN_ADDEND(i8_up, signed char, 125);
    RUN_ADDEND(i8_down, signed char, -126);
    RUN_ADDEND(i16_up, short, 32765);
    RUN_ADDEND(i16_down, short, -32766);
    RUN_ADDEND(u8_by_100, unsigned char, 90);
    RUN_ADDEND(i16_by_9000, short, -20000);
    /* And one start in the middle, where nothing wraps. */
    RUN_ADDEND(u8_up, unsigned char, 40);
    RUN_ADDEND(i16_down, short, 1000);

    RUN_BASE(u8_up, unsigned char, 253);
    RUN_BASE(u8_down, unsigned char, 2);
    RUN_BASE(u16_up, unsigned short, 65533);
    RUN_BASE(u16_down, unsigned short, 2);
    RUN_BASE(u16_up, unsigned short, 7);

    for (int round = 0; round < 2; round++) {
        unsigned char a = (unsigned char)start_of(round ? 1 : 250);
        unsigned short b = (unsigned short)start_of(round ? 60000 : 12);
        signed char c = (signed char)start_of(round ? -128 : 120);
        short d = (short)start_of(round ? 32767 : -32760);
        unsigned trips = trips_of(round ? 40 : 9);
        same("crowd", round, crowd_fused(a, b, c, d, trips), crowd_plain(a, b, c, d, trips));
    }

    puts(failed ? "FAILED" : "ok");
    return failed;
}
