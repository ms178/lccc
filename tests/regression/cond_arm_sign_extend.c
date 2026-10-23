// int-producing expressions must sign-extend when converted to
// unsigned long, through every transparent wrapper and conversion
// boundary. The lowering reports two types per expression: the
// widened storage type (an int literal is "I64" on LP64) and the
// type of the value actually produced (I32 for `1 << s` or `x - 3`).
// Conversion boundaries that trusted the storage type as the cast's
// source type emitted an I64->U64 identity cast over I32 bits and
// zero-extended INT_MIN. The constant-condition ternary path, the
// value-type queries through comma/_Generic/__builtin_choose_expr,
// and __builtin_expect's raw passthrough (its prototype returns
// long, so the argument converts on the way in) all had the defect.
#include <stdio.h>
#include <limits.h>

typedef unsigned long ul;

static volatile int vs = 31;
static volatile int vx; /* INT_MIN + 3, so vx - 3 == INT_MIN exactly */

static ul as_arg(ul x) { return x; }
static ul as_ret_shift(int s) { return 1 << s; }
static ul as_ret_sub(int x) { return x - 3; }
static double as_ret_d(int s) { return 1 << s; }

/* 128-bit constant widening is by value, not by bit pattern: a
   negative signed initializer sign-extends to all 128 bits. The
   constant coercion used to treat the 64-bit pattern as the value,
   so these globals came out as 2^64-1 / 2^64-2^31 (high half 0). */
static unsigned __int128 g_u128_neg1 = -1;
static unsigned __int128 g_u128_shift = 1 << 31;
static __int128 g_i128_shift = 1 << 31;

static int fails = 0;
#define CHECK(v) do { \
    ul got_ = (v); \
    if (got_ != (ul)INT_MIN) { \
        printf("BAD %s -> %lu\n", #v, got_); \
        fails++; \
    } \
} while (0)

int main(void)
{
    vx = INT_MIN + 3;

    /* producer-shape coverage: shift and literal-subtraction forms */
    CHECK((ul)(1 << vs));
    CHECK((ul)(vx - 3));

    /* ternary: constant condition, both arms, and variable condition */
    CHECK((1) ? (1 << vs) : 0ul);
    CHECK((1) ? (vx - 3) : 0ul);
    CHECK((0) ? 0ul : (1 << vs));
    CHECK((vs == 31) ? (1 << vs) : 0ul);
    CHECK(((1) ? (1 << vs) : (vs * 16ul)));

    /* transparent wrappers */
    CHECK((0, 1 << vs));
    CHECK((0, vx - 3));
    CHECK(_Generic(1, int: (1 << vs), default: 0));
    CHECK(__builtin_choose_expr(1, 1 << vs, 0));
    CHECK(__builtin_choose_expr(0, 0, vx - 3));
    CHECK(__builtin_expect(1 << vs, 0));
    CHECK(__builtin_expect(vx - 3, 0));
    CHECK(__builtin_expect_with_probability(1 << vs, 0, 0.5));

    /* assignment / argument / return / initializer boundaries */
    ul assigned;
    assigned = 1 << vs;
    CHECK(assigned);
    CHECK(as_arg(1 << vs));
    CHECK(as_ret_shift(vs));
    CHECK(as_ret_sub(vx));
    ul arr[1] = { 1 << vs };
    CHECK(arr[0]);
    struct { ul m; } st = { vx - 3 };
    CHECK(st.m);

    /* same boundary into double: the sign must survive */
    if (as_ret_d(vs) != -2147483648.0) {
        printf("BAD double %f\n", as_ret_d(vs));
        fails++;
    }

    if (g_u128_neg1 != ~(unsigned __int128)0) { printf("BAD g_u128_neg1\n"); fails++; }
    if (g_u128_shift != (unsigned __int128)(__int128)INT_MIN) { printf("BAD g_u128_shift\n"); fails++; }
    if (g_i128_shift != (__int128)INT_MIN) { printf("BAD g_i128_shift\n"); fails++; }

    printf(fails ? "FAIL %d\n" : "OK\n", fails);
    return fails;
}
