/*
 * Predicate materialization probe.
 *
 * Each SHAPE below is a boolean spelled as a select over 1/0; each CONTROL is
 * a near miss that must keep its select (or its two separate selectors).  The
 * counts in `predicate-counts.txt` are `-O2 -S` instruction lines in the
 * function body, for LCCC with the fold on, LCCC with
 * `CCC_NO_PREDICATE_MATERIALIZE=1`, gcc 16.2.0 and clang 23.1.4.
 */
typedef unsigned u32;
typedef unsigned long u64;

/* ---- shapes ------------------------------------------------------------- */
int from_cmp(int a, int b)      { return a != b ? 1 : 0; }
int from_cmp_neg(int a, int b)  { return a != b ? 0 : 1; }
int from_lt(int a, int b)       { return a < b ? 1 : 0; }
int from_lt_neg(int a, int b)   { return a < b ? 0 : 1; }
u32 zero_or_one(int c)          { return c ? 1u : 0u; }
int one_or_zero(int c)          { return c ? 0 : 1; }
u32 mask_of_sign(int x)         { return x < 0 ? 0u : 1u; }
u64 zero_or_one64(long long c)  { return c ? 1ull : 0ull; }
int or_or(int a, int b)         { return (a != 0) | (b != 0); }
int and_eq(int a, int b)        { return (a == 0) & (b == 0); }
int or3(int a, int b, int c)    { return (a != 0) | (b != 0) | (c != 0); }

/* ---- controls ----------------------------------------------------------- */
int keep_and_ne(int a, int b)   { return (a != 0) & (b != 0); }
int keep_or_eq(int a, int b)    { return (a == 0) | (b == 0); }
int keep_xor(int a, int b)      { return (a != 0) ^ (b != 0); }
int keep_three_two(int c)       { return c ? 3 : 2; }
int keep_one_two(int c)         { return c ? 1 : 2; }
int g_shared;
int keep_shared(int a, int b)   { int t = a != b; g_shared = t; return t ? 0 : 1; }
