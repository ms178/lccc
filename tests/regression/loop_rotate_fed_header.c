/* Loops whose header computes a value the body reads.
 *
 * `while (n--) use(n);` tests the counter and steps it at the top, and the
 * body reads the stepped counter. Such a loop used to stay top-tested: the
 * machine-level inversion that bottom-tests every other loop copies the
 * header into the latch after phi elimination, and by then it cannot give
 * the body the latch's copy of what the header computed. Loop rotation can,
 * while the function is still in single-assignment form, and does so late
 * (`loop_rotate::rotate_fed_loops`), after the loop's test was moved onto
 * the stepped value where that is an identity (`retarget_loop_tests`).
 *
 * What can go wrong is what a rotation changes: which definition of a header
 * value a reader sees (the guard's on the first trip, the latch's on every
 * later one, either behind the loop), and how often the header's
 * instructions run (once more than the body, as before). So every loop here
 * folds each value its body sees into a checksum, returns what is left in
 * the counter behind the loop, and is run for no trip at all, one, two and
 * many. The shapes:
 *
 *   - the post-decrement test in every integer width and sign, including
 *     the counters that come round (`unsigned char n = 0; while (n--)` runs
 *     no trip and leaves 255; from -1 a `signed char` passes its lower end
 *     and runs 255);
 *   - the same against a bound other than zero, and counting up;
 *   - tests on the stepped value (`while (--n >= 0)`, `while (++i < n)`),
 *     which need no moving;
 *   - ordered tests on the old value (`while (n-- > 0)`), which cannot be
 *     moved and are left top-tested: they still have to be right;
 *   - a header of several instructions, two values leaving it;
 *   - a body of several blocks, with `continue` (two back edges) and
 *     `break` (an exit that does not pass the header);
 *   - loops inside such loops, and such loops inside others;
 *   - a pointer stepped in the test.
 *
 * Compared against GCC by the regression corpus.
 */
#include <stdio.h>

#define NOINLINE __attribute__((noinline))

typedef unsigned long long u64;

static u64 mix(u64 sum, u64 value) { return sum * 1099511628211ULL + value + 1; }

/* What a loop saw and what it left behind. */
struct seen {
    u64 sum;
    u64 left;
    u64 trips;
};

static void show(const char *name, long long arg, struct seen s)
{
    printf("%-22s %6lld: sum %016llx left %016llx trips %llu\n", name, arg, s.sum, s.left, s.trips);
}

/* --- the post-decrement test, every width ------------------------------- */

#define WHILE_POST_DEC(TAG, T)                                                \
    NOINLINE static struct seen while_post_dec_##TAG(T n)                     \
    {                                                                         \
        struct seen s = { 0, 0, 0 };                                          \
        while (n--) {                                                         \
            s.sum = mix(s.sum, (u64)n);                                       \
            s.trips++;                                                        \
        }                                                                     \
        s.left = (u64)n;                                                      \
        return s;                                                             \
    }

WHILE_POST_DEC(u8, unsigned char)
WHILE_POST_DEC(u16, unsigned short)
WHILE_POST_DEC(u32, unsigned int)
WHILE_POST_DEC(u64, unsigned long long)
WHILE_POST_DEC(i8, signed char)
WHILE_POST_DEC(i16, short)
WHILE_POST_DEC(i32, int)
WHILE_POST_DEC(i64, long long)

/* --- other bounds, the other direction, the stepped value --------------- */

NOINLINE static struct seen until_five(unsigned long n)
{
    struct seen s = { 0, 0, 0 };
    while (n-- != 5) {
        s.sum = mix(s.sum, n);
        s.trips++;
    }
    s.left = n;
    return s;
}

NOINLINE static struct seen up_to(unsigned i, unsigned stop)
{
    struct seen s = { 0, 0, 0 };
    while (i++ != stop) {
        s.sum = mix(s.sum, i);
        s.trips++;
    }
    s.left = i;
    return s;
}

NOINLINE static struct seen up_by_three_to_99(long i)
{
    struct seen s = { 0, 0, 0 };
    long before;
    while (before = i, i += 3, before != 99) {
        s.sum = mix(s.sum, (u64)i);
        s.trips++;
    }
    s.left = (u64)i;
    return s;
}

NOINLINE static struct seen pre_dec_ge(long n)
{
    struct seen s = { 0, 0, 0 };
    while (--n >= 0) {
        s.sum = mix(s.sum, (u64)n);
        s.trips++;
    }
    s.left = (u64)n;
    return s;
}

NOINLINE static struct seen pre_inc_lt(int i, int n)
{
    struct seen s = { 0, 0, 0 };
    while (++i < n) {
        s.sum = mix(s.sum, (u64)i);
        s.trips++;
    }
    s.left = (u64)i;
    return s;
}

/* An ordered test on the old value: not an identity to move, left as it is. */
NOINLINE static struct seen post_dec_gt(long n)
{
    struct seen s = { 0, 0, 0 };
    while (n-- > 0) {
        s.sum = mix(s.sum, (u64)n);
        s.trips++;
    }
    s.left = (u64)n;
    return s;
}

NOINLINE static struct seen post_inc_lt(unsigned i, unsigned n)
{
    struct seen s = { 0, 0, 0 };
    while (i++ < n) {
        s.sum = mix(s.sum, i);
        s.trips++;
    }
    s.left = i;
    return s;
}

/* --- headers of several instructions ------------------------------------ */

/* A step that is not a constant one; the stepped value is tested and read. */
NOINLINE static struct seen halve(u64 x)
{
    struct seen s = { 0, 0, 0 };
    while ((x >>= 1) != 0) {
        s.sum = mix(s.sum, x);
        s.trips++;
    }
    s.left = x;
    return s;
}

/* Two values leave the header: both are read in the body and behind it. */
NOINLINE static struct seen two_at_once(unsigned long a, unsigned long b)
{
    struct seen s = { 0, 0, 0 };
    while (a += 3, b--) {
        s.sum = mix(mix(s.sum, a), b);
        s.trips++;
    }
    s.left = a ^ (b << 32);
    return s;
}

/* The header value feeds a second one that the body reads. */
NOINLINE static struct seen derived(unsigned n, unsigned scale)
{
    struct seen s = { 0, 0, 0 };
    unsigned scaled;
    while (scaled = n * scale + 7u, n--) {
        s.sum = mix(mix(s.sum, n), scaled);
        s.trips++;
    }
    s.left = n ^ ((u64)scaled << 32);
    return s;
}

/* --- bodies of several blocks ------------------------------------------- */

static unsigned char odd_seen[8];

NOINLINE static struct seen with_continue(unsigned long n)
{
    struct seen s = { 0, 0, 0 };
    while (n--) {
        s.trips++;
        if (n & 1) {
            odd_seen[n & 7]++;
            continue;
        }
        s.sum = mix(s.sum, n);
    }
    s.left = n;
    return s;
}

NOINLINE static struct seen with_break(const unsigned char *bytes, unsigned long n, unsigned key)
{
    struct seen s = { 0, 0, 0 };
    while (n--) {
        s.trips++;
        if (bytes[n] == key)
            break;
        s.sum = mix(s.sum, bytes[n]);
    }
    /* `n` is the index found, or all ones. */
    s.left = n;
    return s;
}

NOINLINE static struct seen with_arms(unsigned n)
{
    struct seen s = { 0, 0, 0 };
    while (n--) {
        if (n % 3 == 0)
            s.sum = mix(s.sum, n);
        else if (n % 3 == 1)
            s.sum ^= n;
        else
            odd_seen[n & 7] += 3;
        s.trips++;
    }
    s.left = n;
    return s;
}

/* --- nests --------------------------------------------------------------- */

NOINLINE static struct seen grid(unsigned rows, unsigned cols)
{
    struct seen s = { 0, 0, 0 };
    while (rows--) {
        unsigned c = cols;
        while (c--) {
            s.sum = mix(s.sum, ((u64)rows << 16) | c);
            s.trips++;
        }
        s.left += c;
    }
    s.left ^= (u64)rows << 40;
    return s;
}

NOINLINE static struct seen inside_for(unsigned outer, unsigned inner)
{
    struct seen s = { 0, 0, 0 };
    for (unsigned i = 0; i < outer; i++) {
        unsigned n = inner + i;
        while (n--) {
            s.sum = mix(s.sum, n + i);
            s.trips++;
        }
        s.left += n;
    }
    return s;
}

NOINLINE static struct seen for_inside(unsigned n, const unsigned char *bytes, unsigned len)
{
    struct seen s = { 0, 0, 0 };
    while (n--) {
        for (unsigned i = 0; i < len; i++)
            s.sum = mix(s.sum, bytes[i] + n);
        s.trips++;
    }
    s.left = n;
    return s;
}

/* --- pointers and the everyday copies ------------------------------------ */

NOINLINE static struct seen copy_bytes(unsigned char *d, const unsigned char *s_, unsigned long n)
{
    struct seen s = { 0, 0, 0 };
    const unsigned char *from = s_;
    while (n--) {
        *d++ = *s_++;
        s.trips++;
    }
    s.sum = (u64)(s_ - from);
    s.left = n;
    return s;
}

NOINLINE static struct seen walk_to(const unsigned char *p, const unsigned char *end)
{
    struct seen s = { 0, 0, 0 };
    const unsigned char *start = p;
    while (p++ != end) {
        s.sum = mix(s.sum, p[-1]);
        s.trips++;
    }
    s.left = (u64)(p - start);
    return s;
}

NOINLINE static struct seen fill_back(unsigned *a, unsigned n, unsigned v)
{
    struct seen s = { 0, 0, 0 };
    while (n--) {
        a[n] = v + n;
        s.trips++;
    }
    s.left = n;
    return s;
}

int main(void)
{
    static const long long counts[] = { 0, 1, 2, 3, 9, 100 };
    unsigned char bytes[64], copy[64];
    unsigned words[40];

    for (unsigned i = 0; i < sizeof bytes; i++)
        bytes[i] = (unsigned char)(i * 37u + 11u);

    for (unsigned c = 0; c < sizeof counts / sizeof counts[0]; c++) {
        long long n = counts[c];
        show("while_post_dec_u8", n, while_post_dec_u8((unsigned char)n));
        show("while_post_dec_u16", n, while_post_dec_u16((unsigned short)n));
        show("while_post_dec_u32", n, while_post_dec_u32((unsigned)n));
        show("while_post_dec_u64", n, while_post_dec_u64((u64)n));
        show("while_post_dec_i8", n, while_post_dec_i8((signed char)n));
        show("while_post_dec_i16", n, while_post_dec_i16((short)n));
        show("while_post_dec_i32", n, while_post_dec_i32((int)n));
        show("while_post_dec_i64", n, while_post_dec_i64(n));
        show("until_five", n, until_five((unsigned long)n + 5));
        show("up_to", n, up_to(17u, 17u + (unsigned)n));
        show("up_by_three_to_99", n, up_by_three_to_99(99 - 3 * (long)n));
        show("pre_dec_ge", n, pre_dec_ge((long)n));
        show("pre_inc_lt", n, pre_inc_lt(-3, (int)n - 2));
        show("post_dec_gt", n, post_dec_gt((long)n));
        show("post_inc_lt", n, post_inc_lt(4u, 4u + (unsigned)n));
        show("two_at_once", n, two_at_once(1000ul, (unsigned long)n));
        show("derived", n, derived((unsigned)n, 5u));
        show("with_continue", n, with_continue((unsigned long)n));
        show("with_arms", n, with_arms((unsigned)n));
        show("grid", n, grid((unsigned)n, (unsigned)n % 7u + 1u));
        show("inside_for", n, inside_for((unsigned)n % 5u, (unsigned)n % 9u));
        show("for_inside", n, for_inside((unsigned)n % 11u, bytes, 5u));
        if (n <= 9) {
            for (unsigned i = 0; i < sizeof copy; i++)
                copy[i] = 0xee;
            show("copy_bytes", n, copy_bytes(copy, bytes, (unsigned long)n));
            u64 image = 0;
            for (unsigned i = 0; i < sizeof copy; i++)
                image = mix(image, copy[i]);
            printf("%-22s %6lld: image %016llx\n", "copy_bytes", n, image);
            show("walk_to", n, walk_to(bytes, bytes + n));
            show("with_break", n, with_break(bytes, (unsigned long)n, bytes[n / 2]));
            show("with_break (none)", n, with_break(bytes, (unsigned long)n, 0x100u));
            for (unsigned i = 0; i < sizeof words / sizeof words[0]; i++)
                words[i] = 0xa5a5a5a5u;
            show("fill_back", n, fill_back(words, (unsigned)n, 1000u));
            image = 0;
            for (unsigned i = 0; i < sizeof words / sizeof words[0]; i++)
                image = mix(image, words[i]);
            printf("%-22s %6lld: image %016llx\n", "fill_back", n, image);
        }
    }
    /* Negative and large starts for the signed and the shifting ones. */
    show("pre_dec_ge", -5, pre_dec_ge(-5));
    show("post_dec_gt", -5, post_dec_gt(-5));
    show("pre_inc_lt", 7, pre_inc_lt(7, 3));
    show("halve", 0, halve(0));
    show("halve", 1, halve(1));
    show("halve", 2, halve(2));
    show("halve", -1, halve(~0ULL));
    show("halve", 12345, halve(0x123456789abcdefULL));
    /* A narrow counter that starts at zero leaves all ones and runs no trip;
     * one that starts at its top value runs that many. */
    show("while_post_dec_u8", 255, while_post_dec_u8(255));
    show("while_post_dec_i8", -1, while_post_dec_i8(-1));
    show("while_post_dec_u16", 65535, while_post_dec_u16(65535));

    unsigned odd = 0;
    for (unsigned i = 0; i < 8; i++)
        odd = odd * 257u + odd_seen[i];
    printf("odd %08x\n", odd);
    return 0;
}
