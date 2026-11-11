/* A 64-bit result converted to `int` wraps; it does not overflow.
 *
 * C17 6.3.1.3p3 makes the conversion of an out-of-range value to a signed
 * type implementation-defined, and LCCC follows GCC: the value is reduced
 * modulo 2^N.  That is different from the *undefined* overflow of an `int`
 * operation, which the optimizer may assume never happens.  Two passes used
 * to confuse the two at -O2 and -O3 (both wrong on main @ fc8350a46):
 *
 *   1. `iv_widen` filed the truncation of a 64-bit value that depends on a
 *      widened counter -- `(int)((long)a + (long)j)` -- as a *widening* cast
 *      and dropped it together with its re-extension.  `store_sum` stored
 *      2147483649 where the program computes -2147483647.
 *
 *   2. `narrow` rewrote `(int)((long)a + 1L)` to an `int` addition.  The
 *      address peeler then used "signed overflow is undefined" to fold the
 *      `+ 1` into a displacement, and `p[(int)((long)a + 1L)]` became
 *      `movslq %esi, %rsi; movsbl 1(%rdi,%rsi)`: with `a == INT_MAX` it read
 *      `p[2147483648]` instead of `p[-2147483648]`.
 *
 * Every expected value below is computed by `wrap32`, which uses unsigned
 * arithmetic only, from operands read through `volatile` so that neither
 * side is folded.  The last group needs one object larger than 4 GiB so that
 * both the wrapped and the unwrapped element exist; it is mapped without
 * reserving memory and touches five pages, and is skipped where such a
 * mapping is refused.
 */
#include <limits.h>
#include <stddef.h>
#include <stdio.h>

#if defined(__linux__) && __SIZEOF_POINTER__ == 8
#define HAVE_BIG_OBJECT 1
#include <sys/mman.h>
#else
#define HAVE_BIG_OBJECT 0
#endif

#define NOINLINE __attribute__((noinline))

/* The value of `(int)v` for a 64-bit `v`, without a narrowing conversion. */
static NOINLINE long wrap32(long v) {
    unsigned long low = (unsigned long)v & 0xffffffffUL;
    return low >= 0x80000000UL ? (long)low - 0x100000000L : (long)low;
}

/* Likewise for `(short)v`. */
static NOINLINE long wrap16(long v) {
    unsigned long low = (unsigned long)v & 0xffffUL;
    return low >= 0x8000UL ? (long)low - 0x10000L : (long)low;
}

static int failures;

static void expect(const char *what, long got, long want) {
    printf("%-28s %ld\n", what, got);
    if (got != want) {
        printf("  MISMATCH: expected %ld\n", want);
        failures++;
    }
}

/* --- 1. iv_widen: a truncated 64-bit value of the counter ---------------- */

static NOINLINE void store_sum(int a, int n, long *p) {
    for (int j = 0; j < n; j++)
        p[j] = (int)((long)a + (long)j);
}

static NOINLINE void store_long_sum(long a, int n, long *p) {
    for (int j = 0; j < n; j++)
        p[j] = (int)(a + j);
}

static NOINLINE void store_product(int n, long *p) {
    for (int j = 0; j < n; j++)
        p[j] = (int)((long)j * 0x40000000L);
}

static NOINLINE void store_shift(int n, long *p) {
    for (int j = 0; j < n; j++)
        p[j] = (int)((long)j << 30);
}

static NOINLINE void store_difference(int a, int n, long *p) {
    for (int j = 0; j < n; j++)
        p[j] = (int)((long)a - (long)j);
}

static NOINLINE void store_short_sum(int a, int n, long *p) {
    for (int j = 0; j < n; j++)
        p[j] = (short)((long)a + (long)j);
}

static NOINLINE long sum_of_sums(int a, int n, const long *p) {
    long s = 0;
    for (int j = 0; j < n; j++) {
        s += (int)((long)a + (long)j);
        s += p[j];
    }
    return s;
}

/* The sign of the wrapped value picks the element: the unwrapped 64-bit
 * sum is never negative here. */
static NOINLINE long sign_selected(int a, int n, const long *p) {
    long s = 0;
    for (int j = 0; j < n; j++)
        s += p[(int)((long)a + (long)j) < 0];
    return s;
}

/* --- 2. narrow: the truncation outside any loop --------------------------- */

static NOINLINE long add_one(int a) { return (int)((long)a + 1L); }
static NOINLINE long add_pair(int a, int b) { return (int)((long)a + (long)b); }
static NOINLINE long sub_pair(int a, int b) { return (int)((long)a - (long)b); }
static NOINLINE long mul_pair(int a, int b) { return (int)((long)a * (long)b); }
static NOINLINE long shl_const(int a) { return (int)((long)a << 4); }
static NOINLINE long twice(int a) { return (int)((long)a + (long)a); }

static NOINLINE long compound(int r, long l) {
    r += l; /* r = (int)((long)r + l) */
    return r;
}

static int cell;
static NOINLINE long through_memory(int a) {
    cell = (long)a + 1L; /* the store truncates */
    return cell;
}

/* An `int` operation proper stays an `int` operation: in range here. */
static NOINLINE long plain_int(int a, int b) { return a + b; }

/* --- 3. the wrapped value as an address ---------------------------------- */

#if HAVE_BIG_OBJECT
static NOINLINE char at_plus_one(const char *p, int a) { return p[(int)((long)a + 1L)]; }
static NOINLINE char at_sum(const char *p, int a, int b) { return p[(int)((long)a + (long)b)]; }

static NOINLINE long sum_at(const char *p, int a, int n) {
    long s = 0;
    for (int j = 0; j < n; j++)
        s += p[(int)((long)a + (long)j)];
    return s;
}

static void big_object(void) {
    size_t size = ((size_t)1 << 32) + 2 * 4096;
    char *base = mmap(NULL, size, PROT_READ | PROT_WRITE,
                      MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1, 0);
    if (base == MAP_FAILED) {
        /* Not a compiler property; both builds take this path together. */
        return;
    }
    char *p = base + ((size_t)1 << 31);
    volatile int max = INT_MAX;
    p[INT_MIN] = 42;             /* base[0]                             */
    p[INT_MIN + 1] = 7;          /* base[1]                             */
    p[INT_MAX] = 3;              /* base[2^32 - 1]                      */
    p[(long)INT_MAX + 1] = 100;  /* base[2^32]: the unwrapped element   */
    p[(long)INT_MAX + 2] = 100;
    expect("p[(int)(a + 1L)]", at_plus_one(p, max), 42);
    expect("p[(int)(a + b)]", at_sum(p, max, 2), 7);
    expect("sum p[(int)(a + j)]", sum_at(p, max, 3), 3 + 42 + 7);
    munmap(base, size);
}
#endif

int main(void) {
    volatile int n = 4;
    volatile int near_max = INT_MAX - 1, max = INT_MAX, near_min = INT_MIN + 1;
    volatile int near_short_max = SHRT_MAX - 1;
    long out[8];
    char name[40];

    store_sum(near_max, n, out);
    for (int j = 0; j < 4; j++) {
        snprintf(name, sizeof name, "store_sum[%d]", j);
        expect(name, out[j], wrap32((long)near_max + j));
    }
    store_long_sum((long)near_max, n, out);
    for (int j = 0; j < 4; j++) {
        snprintf(name, sizeof name, "store_long_sum[%d]", j);
        expect(name, out[j], wrap32((long)near_max + j));
    }
    store_product(n, out);
    for (int j = 0; j < 4; j++) {
        snprintf(name, sizeof name, "store_product[%d]", j);
        expect(name, out[j], wrap32((long)j * 0x40000000L));
    }
    store_shift(n, out);
    for (int j = 0; j < 4; j++) {
        snprintf(name, sizeof name, "store_shift[%d]", j);
        expect(name, out[j], wrap32((long)j << 30));
    }
    store_difference(near_min, n, out);
    for (int j = 0; j < 4; j++) {
        snprintf(name, sizeof name, "store_difference[%d]", j);
        expect(name, out[j], wrap32((long)near_min - j));
    }
    store_short_sum(near_short_max, n, out);
    for (int j = 0; j < 4; j++) {
        snprintf(name, sizeof name, "store_short_sum[%d]", j);
        expect(name, out[j], wrap16((long)near_short_max + j));
    }

    for (int j = 0; j < 8; j++)
        out[j] = 1L << (8 * j);
    {
        long want = 0;
        for (int j = 0; j < 4; j++)
            want += wrap32((long)near_max + j) + out[j];
        expect("sum_of_sums", sum_of_sums(near_max, n, out), want);
        want = 0;
        for (int j = 0; j < 4; j++)
            want += out[wrap32((long)near_max + j) < 0];
        expect("sign_selected", sign_selected(near_max, n, out), want);
    }

    expect("add_one", add_one(max), wrap32((long)max + 1));
    expect("add_pair", add_pair(max, max), wrap32((long)max + max));
    expect("sub_pair", sub_pair(near_min, max), wrap32((long)near_min - max));
    expect("mul_pair", mul_pair(max, max), wrap32((long)max * max));
    expect("shl_const", shl_const(max), wrap32((long)max << 4));
    expect("twice", twice(max), wrap32((long)max + max));
    expect("compound", compound(max, 5L), wrap32((long)max + 5));
    expect("compound far", compound(7, 0x123456789abL), wrap32(7 + 0x123456789abL));
    expect("through_memory", through_memory(max), wrap32((long)max + 1));
    expect("plain_int", plain_int(near_max, 1), (long)near_max + 1);

#if HAVE_BIG_OBJECT
    big_object();
#endif

    if (failures)
        printf("FAILED: %d\n", failures);
    return failures != 0;
}
