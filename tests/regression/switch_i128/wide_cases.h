/* 128-bit switch dispatch, shared by the switch_i128_wide_cases*.c wrappers
 * (one per optimization level: the defects below showed at -O0, -O1 and -Os
 * while -O2 happened to take other paths).
 *
 *   * A case value is converted to the controlling expression's type by
 *     value (C11 6.8.4.2p5).  The frontend kept case values as i64, so
 *     `case 0xffffffffffffffffull:` became `case -1:` (2^128 - 1 to a
 *     signed or unsigned __int128), `case 0x8000000000000000ull:` became
 *     INT64_MIN, and `case (__int128)1 << 64:` became `case 0:`.  Such a
 *     case is now compared at full width.
 *   * A case that is the sign extension of its low half matches only when
 *     the selector's high half is that sign extension: AArch64 and RISC-V
 *     compared the low half alone, so `(__int128)1 << 64` took `case 0:`
 *     (gcc.c-torture execute/pr122943.c).
 *   * x86-64 had that guard, but when `default:` was the block laid out
 *     next, the non-negative dispatch fell through into the negative one:
 *     2^64 - 1 (high half 0, low half -1) took `case -1:`.
 *   * Dense case sets (jump tables), mixed-sign dense sets (one table per
 *     sign class), GNU case ranges with narrow and wide bounds, and a
 *     selector the compiler can see as a constant.
 *
 * Every input comes from a volatile table and every expected answer is
 * written out by hand, so nothing is folded away and no reference shares
 * the code under test.  Exit status 0 is success; abort() otherwise. */
extern void abort(void);

typedef __int128 i128;
typedef unsigned __int128 u128;

#define U64MAX 0xffffffffffffffffull
#define TWO64 ((i128)1 << 64)
#define I128MAX ((i128)(((u128)1 << 127) - 1))
#define I128MIN (-I128MAX - 1)

__attribute__((noinline)) static int wide_signed(i128 v)
{
    switch (v) {
    case 0: return 1;
    case -1: return 2;
    case 0xffffffffffffffffull: return 3; /* 2^64 - 1 */
    case TWO64: return 4;
    case -TWO64: return 5;
    case 0x8000000000000000ull: return 6; /* +2^63 */
    case -0x7fffffffffffffffll - 1: return 7; /* -2^63 */
    case I128MAX: return 8;
    default: return 0;
    }
}

__attribute__((noinline)) static int wide_unsigned(u128 v)
{
    switch (v) {
    case 0: return 1;
    case -1: return 2; /* 2^128 - 1 */
    case 0xffffffffffffffffull: return 3;
    case (u128)1 << 64: return 4;
    case (u128)1 << 127: return 5;
    case 7: return 6;
    default: return 0;
    }
}

/* `default:` first, so it is the block laid out after the dispatch. */
__attribute__((noinline)) static int default_first(i128 v)
{
    int r;
    switch (v) {
    default: r = 9; break;
    case -1: r = 1; break;
    case 1: r = 2; break;
    }
    return r;
}

__attribute__((noinline)) static int default_first_unsigned(u128 v)
{
    int r;
    switch (v) {
    default: r = 9; break;
    case 5: r = 1; break;
    case 0x7fffffffffffffffull: r = 2; break;
    }
    return r;
}

__attribute__((noinline)) static int dense_nonneg(i128 v)
{
    switch (v) {
    case 0: return 11; case 1: return 12; case 2: return 13; case 3: return 14;
    case 4: return 15; case 5: return 16; case 6: return 17; case 7: return 18;
    case 8: return 19; case 9: return 20;
    default: return 0;
    }
}

__attribute__((noinline)) static int dense_negative(i128 v)
{
    switch (v) {
    case -1: return 21; case -2: return 22; case -3: return 23; case -4: return 24;
    case -5: return 25; case -6: return 26; case -7: return 27; case -8: return 28;
    default: return 0;
    }
}

__attribute__((noinline)) static int dense_mixed(i128 v)
{
    switch (v) {
    case -5: return 31; case -4: return 32; case -3: return 33; case -2: return 34;
    case -1: return 35; case 0: return 36; case 1: return 37; case 2: return 38;
    case 3: return 39; case 4: return 40; case 5: return 41;
    default: return 0;
    }
}

__attribute__((noinline)) static int dense_unsigned(u128 v)
{
    switch (v) {
    case 0: return 51; case 1: return 52; case 2: return 53; case 3: return 54;
    case 4: return 55; case 5: return 56; case 6: return 57; case 7: return 58;
    default: return 0;
    }
}

/* Sign-extended cases of an unsigned selector: `case -1:` is 2^128 - 1. */
__attribute__((noinline)) static int mixed_unsigned(u128 v)
{
    switch (v) {
    case -1: return 1;
    case 1: return 2;
    case -2: return 3;
    default: return 0;
    }
}

__attribute__((noinline)) static int ranged(i128 v)
{
    switch (v) {
    case 1 ... 5: return 1;
    case -5 ... -1: return 2;
    case TWO64 ... TWO64 + 10: return 3;
    case 100: return 4;
    case -TWO64 - 3 ... -TWO64 + 3: return 5;
    default: return 0;
    }
}

__attribute__((noinline)) static int ranged_unsigned(u128 v)
{
    switch (v) {
    case 0xfffffffffffffff0ull ... ((u128)1 << 64) + 0x10: return 1;
    case 3 ... 9: return 2;
    case (u128)-4 ... (u128)-1: return 3;
    default: return 0;
    }
}

/* The selector is a constant after propagation (at -O1 and up). */
static inline int folded(void)
{
    i128 v = TWO64;
    switch (v) {
    case 0: return 1;
    case TWO64: return 2;
    default: return 3;
    }
}

static inline int folded_low(void)
{
    i128 v = (i128)U64MAX;
    switch (v) {
    case -1: return 1;
    case 0: return 2;
    default: return 3;
    }
}

/* Called with constant arguments: an interprocedural evaluator may answer
 * these calls at compile time. */
__attribute__((noinline)) static int pure_select(i128 v)
{
    switch (v) {
    case 0: return 1;
    case -1: return 2;
    default: return 3;
    }
}

struct signed_case { i128 in; int want; };
struct unsigned_case { u128 in; int want; };

static volatile const struct signed_case signed_cases[] = {
    {0, 1}, {-1, 2}, {(i128)U64MAX, 3}, {TWO64, 4}, {-TWO64, 5},
    {(i128)0x8000000000000000ull, 6}, {-(i128)0x8000000000000000ull, 7},
    {I128MAX, 8}, {I128MIN, 0}, {TWO64 + 1, 0}, {-TWO64 + 1, 0},
    {1, 0}, {-2, 0}, {(i128)U64MAX - 1, 0}, {TWO64 - 2, 0},
};

static volatile const struct unsigned_case unsigned_cases[] = {
    {0, 1}, {~(u128)0, 2}, {U64MAX, 3}, {(u128)1 << 64, 4}, {(u128)1 << 127, 5},
    {7, 6}, {((u128)1 << 64) + 7, 0}, {((u128)1 << 127) + 7, 0},
    {~(u128)0 - 1, 0}, {(u128)U64MAX << 64, 0}, {U64MAX - 1, 0},
};

static volatile const struct signed_case default_first_cases[] = {
    {-1, 1}, {1, 2}, {(i128)U64MAX, 9}, {TWO64 + 1, 9}, {-TWO64 + 1, 9},
    {-TWO64 - 1, 9}, {0, 9}, {-2, 9}, {2, 9},
};

static volatile const struct unsigned_case default_first_unsigned_cases[] = {
    {5, 1}, {0x7fffffffffffffffull, 2}, {((u128)1 << 64) + 5, 9},
    {((u128)U64MAX << 64) | 5, 9}, {U64MAX, 9}, {0, 9},
};

static volatile const struct signed_case dense_nonneg_cases[] = {
    {0, 11}, {3, 14}, {9, 20}, {10, 0}, {-1, 0}, {TWO64 + 3, 0},
    {-TWO64 + 3, 0}, {TWO64, 0}, {(i128)U64MAX, 0},
};

static volatile const struct signed_case dense_negative_cases[] = {
    {-1, 21}, {-4, 24}, {-8, 28}, {-9, 0}, {0, 0}, {TWO64 - 5, 0},
    {-TWO64 - 5, 0}, {(i128)U64MAX, 0}, {TWO64 - 1, 0},
};

static volatile const struct signed_case dense_mixed_cases[] = {
    {-5, 31}, {-1, 35}, {0, 36}, {5, 41}, {6, 0}, {-6, 0},
    {(i128)U64MAX - 2, 0}, {-TWO64 + 3, 0}, {TWO64 + 2, 0}, {-TWO64 - 3, 0},
};

static volatile const struct unsigned_case dense_unsigned_cases[] = {
    {0, 51}, {7, 58}, {8, 0}, {((u128)1 << 64) | 3, 0}, {(u128)1 << 127, 0},
    {~(u128)0, 0},
};

static volatile const struct unsigned_case mixed_unsigned_cases[] = {
    {~(u128)0, 1}, {1, 2}, {~(u128)0 - 1, 3}, {U64MAX, 0}, {U64MAX - 1, 0},
    {((u128)U64MAX << 64) | 1, 0}, {0, 0},
};

static volatile const struct signed_case ranged_cases[] = {
    {1, 1}, {5, 1}, {-5, 2}, {-1, 2}, {TWO64, 3}, {TWO64 + 10, 3}, {100, 4},
    {-TWO64 - 3, 5}, {-TWO64, 5}, {-TWO64 + 3, 5}, {0, 0}, {6, 0}, {-6, 0},
    {2 * TWO64 + 3, 0}, {TWO64 + 11, 0}, {TWO64 - 1, 0},
    {TWO64 + 100, 0}, {-TWO64 + 4, 0}, {-TWO64 - 4, 0},
};

static volatile const struct unsigned_case ranged_unsigned_cases[] = {
    {0xfffffffffffffff0ull, 1}, {U64MAX, 1}, {(u128)1 << 64, 1},
    {((u128)1 << 64) + 0x10, 1}, {((u128)1 << 64) + 0x11, 0},
    {0xffffffffffffffefull, 0}, {3, 2}, {9, 2}, {10, 0}, {~(u128)0, 3},
    {~(u128)0 - 3, 3}, {~(u128)0 - 4, 0}, {((u128)1 << 64) + 5, 1},
    {((u128)2 << 64) + 5, 0},
};

#define COUNT(a) (sizeof(a) / sizeof((a)[0]))
#define CHECK(fn, table)                                   \
    for (unsigned i = 0; i < COUNT(table); i++)            \
        if (fn(table[i].in) != table[i].want)              \
            abort();

int main(void)
{
    CHECK(wide_signed, signed_cases);
    CHECK(wide_unsigned, unsigned_cases);
    CHECK(default_first, default_first_cases);
    CHECK(default_first_unsigned, default_first_unsigned_cases);
    CHECK(dense_nonneg, dense_nonneg_cases);
    CHECK(dense_negative, dense_negative_cases);
    CHECK(dense_mixed, dense_mixed_cases);
    CHECK(dense_unsigned, dense_unsigned_cases);
    CHECK(mixed_unsigned, mixed_unsigned_cases);
    CHECK(ranged, ranged_cases);
    CHECK(ranged_unsigned, ranged_unsigned_cases);
    if (folded() != 2 || folded_low() != 3)
        abort();
    if (pure_select(TWO64) != 3 || pure_select((i128)U64MAX) != 3 || pure_select(-1) != 2)
        abort();
    return 0;
}
