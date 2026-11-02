/* Bit toggle: `x ^ (1u << n)`, the fourth and last member of the single-bit
   family (`|` sets, `& ~` clears, `&` tests).  Measured at -O2 against GCC
   16.2 and Clang 23.1.4; the `CCC_NO_BIT_SELECT=1` build is the pre-fold
   spelling, which is what the "before" column records. */
typedef unsigned u32;
typedef unsigned long u64;

u32 btog32(u32 x, u32 n)         { return x ^ (1u << n); }
u64 btog64(u64 x, u64 n)         { return x ^ (1ul << n); }
u32 btog_masked(u32 x, u32 n)    { return x ^ (1u << (n & 31)); }
int btog_imm(u32 x)              { return (int)(x ^ (1u << 5)); }
/* Memory base: the base is loaded, not a parameter. */
u32 btog_mem(u32 *p, u32 n)      { u32 v = *p; v ^= 1u << n; return v; }
/* The involutive spelling: the same bit toggled twice. */
u32 btog_twice(u32 x, u32 n)     { return x ^ (1u << n) ^ (1u << n); }
/* The shift is the first operand. */
u32 btog_swapped(u32 x, u32 n)   { return (1u << n) ^ x; }
/* The index is an expression, not the parameter: `n + 1` is the bit toggled. */
u32 btog_offset(u32 x, u32 n)    { return x ^ (1u << (n + 1)); }

/* Controls: not a single-bit operation on the base. */
u32 keep_xor_two(u32 x, u32 n)   { return x ^ (3u << n); }
u32 keep_xor_not(u32 x, u32 n)   { return x ^ ~(1u << n); }
u32 keep_xor_hi(u32 x, u64 n)    { return x ^ (1ul << n); }
