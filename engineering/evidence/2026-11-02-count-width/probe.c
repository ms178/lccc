/* The shapes §20 of SCALAR-PREDICATE-FOLDS.md is about, one function each.
 * Compile with `-O2 -S` and count the body of each (count.py). */
typedef unsigned char u8;
typedef unsigned short u16;
typedef unsigned u32;
typedef unsigned long long u64;

/* Shape 1: the low bit of a count, with the mask narrowed to `int`. */
int pc64_and1(u64 x) { return (int)(__builtin_popcountll(x) & 1); }

/* The same question spelled as the builtin: the two must converge. */
int par64ll(u64 x) { return __builtin_parityll(x); }

/* Shape 2: a promoted narrow count, which can run at its own width. */
int par_u8(u8 x)  { return __builtin_parity(x); }
int par_u16(u16 x){ return __builtin_parity(x); }
int pc_u8(u8 x)   { return __builtin_popcount(x); }
int pc_u16(u16 x) { return __builtin_popcount(x); }

/* Controls: same-width shapes, which the simplifier owns and this rule must
 * not touch, and shapes no rule in this family reaches. */
int pc32_and1(u32 x) { return __builtin_popcount(x) & 1; }
int np2c(u32 x)      { return 1u << (32 - __builtin_clz(x - 1)); }
