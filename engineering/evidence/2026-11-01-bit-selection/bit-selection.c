/* Bit-set / bit-clear / bit-test / popcount / parity probe.
 *
 * Every function is the smallest C spelling of one idiom, so the instruction
 * count below each entry is exactly what a backend can remove.  The `keep_`
 * functions are neighbouring shapes the folds must not touch. */
typedef unsigned u32;
typedef unsigned long u64;

/* bit set */
u32 bset32(u32 x, u32 n)    { return x | (1u << n); }
u64 bset64(u64 x, u64 n)    { return x | (1ul << n); }
u32 bset_imm(u32 x)         { return x | (1u << 7); }
u32 bset_masked(u32 x, u32 n) { return x | (1u << (n & 31)); }

/* bit clear */
u32 bclr32(u32 x, u32 n)    { return x & ~(1u << n); }
u64 bclr64(u64 x, u64 n)    { return x & ~(1ul << n); }
u32 bclr_imm(u32 x)         { return x & ~(1u << 7); }

/* bit test */
int btest32(u32 x, u32 n)   { return (x & (1u << n)) != 0; }
int btest32z(u32 x, u32 n)  { return (x & (1u << n)) == 0; }
int btest64(u64 x, u64 n)   { return (x & (1ul << n)) != 0; }
int btest_imm(u32 x)        { return (x & (1u << 7)) != 0; }

/* bit ops through memory */
void bset_mem(u32 *p, u32 n)  { *p |= 1u << n; }
void bclr_mem(u32 *p, u32 n)  { *p &= ~(1u << n); }

/* popcount / parity */
int pop32(u32 x)            { return __builtin_popcount(x); }
int pop64(u64 x)            { return __builtin_popcountl(x); }
int parity32(u32 x)         { return __builtin_parity(x); }
int parity64(u64 x)         { return __builtin_parityl(x); }
int pop_and1(u32 x)         { return __builtin_popcount(x) & 1; }

/* neighbours that must keep their current shape */
u32 keep_shl(u32 x, u32 n)  { return (x << n) | x; }
u32 keep_two(u32 x, u32 n)  { return x & (3u << n); }
u32 keep_xor(u32 x, u32 n)  { return x ^ (1u << n); }
int keep_eq4(u32 x, u32 n)  { return (x & (1u << n)) == 4; }
u32 keep_and(u32 x, u32 n)  { return x & (1u << n); }
