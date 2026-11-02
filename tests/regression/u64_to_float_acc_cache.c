/* The U64 -> double/float conversion takes a "shift and round" path for
 * values with bit 63 set, and that path REWRITES %rax (`shrq $1`, `orq
 * %rcx, %rax`). The emitter did not invalidate the accumulator cache, which
 * still claimed the converted value was live in %rax, so any later use of
 * that value took the cache fast path and read the mangled conversion
 * operand instead of the value:
 *
 *     x = ptr ^ (1ULL << 63);      // bit 63 set: the big path
 *     d = (double)x;               // shrq/orq rewrite %rax
 *     e = (double)x;               // cache hit: cvtsi2sdq on the leftover
 *
 * d != e, and `x` itself also came back wrong. Measured at -O1 on the release
 * build (the value stays in %rax there); the debug-assertions build refuses
 * the stale accumulator read outright, which is how the shape was found.
 *
 * A stack address has bit 63 clear on Linux, so the plain
 * `(double)(uintptr_t)&local` shape never took the big path in tests: the
 * register must be forced through the >= 2^63 route, which is what the
 * XOR with 1<<63 below does. All values are runtime (derived from a local's
 * address), so no const-folding can hide the conversion.
 *
 * Expected: exit 0. GCC and Clang agree.
 */
#include <stdio.h>
#include <stdint.h>

#define HIGH_BIT (1ULL << 63)

/* double: two conversions of the same value must agree, and the value must
 * survive the first conversion intact */
static int t_double_repeat(void) {
    int s = 0;
    uintptr_t x = (uintptr_t)&s ^ HIGH_BIT;
    double d = (double)x;
    double e = (double)x;
    return d == e && (uintptr_t)x == ((uintptr_t)&s ^ HIGH_BIT);
}

/* the same for float (the addss / addsd halves of the big path) */
static int t_float_repeat(void) {
    int s = 0;
    uintptr_t x = (uintptr_t)&s ^ HIGH_BIT;
    float d = (float)x;
    float e = (float)x;
    return d == e && (uintptr_t)x == ((uintptr_t)&s ^ HIGH_BIT);
}

/* the value is used by an integer operation after the conversion: the
 * consumer must see the original bits, not the conversion's scratch */
static int t_int_use_after(void) {
    int s = 0;
    uintptr_t x = (uintptr_t)&s ^ HIGH_BIT;
    volatile double d = (double)x;
    (void)d;
    uintptr_t y = x;
    return y == ((uintptr_t)&s ^ HIGH_BIT) && (y >> 63) == 1;
}

/* interleaved: convert, use the integer, convert again */
static int t_interleaved(void) {
    int s = 0;
    uintptr_t x = (uintptr_t)&s ^ HIGH_BIT;
    double d = (double)x;
    uintptr_t y = x ^ 1;
    double e = (double)x;
    return d == e && y == (((uintptr_t)&s ^ HIGH_BIT) ^ 1);
}

/* a sub-64-bit unsigned source (U32 -> double) must stay unaffected: it rides
 * the narrow cvt form and must not pay for the big-path bookkeeping */
static int t_u32(void) {
    int s = 0;
    unsigned int x = (unsigned int)(uintptr_t)&s ^ 0x80000000u;
    double d = (double)x;
    double e = (double)x;
    return d == e && x == ((unsigned int)(uintptr_t)&s ^ 0x80000000u);
}

/* the signed route (I64 -> double) for a negative value, which must also
 * survive a repeat conversion */
static int t_i64_negative(void) {
    int s = 0;
    int64_t x = (int64_t)((uintptr_t)&s ^ HIGH_BIT);
    double d = (double)x;
    double e = (double)x;
    return x < 0 && d == e;
}

struct test {
    const char *name;
    int (*fn)(void);
};

int main(void) {
    static const struct test tests[] = {
        {"double_repeat", t_double_repeat},
        {"float_repeat", t_float_repeat},
        {"int_use_after", t_int_use_after},
        {"interleaved", t_interleaved},
        {"u32", t_u32},
        {"i64_negative", t_i64_negative},
    };
    int failures = 0;

    for (unsigned i = 0; i < sizeof tests / sizeof tests[0]; i++) {
        if (!tests[i].fn()) {
            printf("FAIL %s\n", tests[i].name);
            failures++;
        } else {
            printf("ok %s\n", tests[i].name);
        }
    }
    printf(failures ? "FAILURES: %d\n" : "OK u64_to_float_acc_cache\n", failures);
    return failures != 0;
}
