/* Kernels for the boolean-return checksum (bool_return_check.c).
 *
 * Every kernel is a separate, opaque function in its own object, so it is
 * compiled with its real calling convention: the IR fold that turns
 * `if (c) return 1; return 0;` into `return c != 0;` (and the inverted
 * form into `return c == 0;`) is observed through narrow and wide return
 * types, and the x86 extension-chain collapse (`movzbl; movslq` ->
 * `movzbl`, `movsbl; movslq` -> `movsbq`) through the `*_ext` kernels.
 * The constants that are not a 0/1 pair, the ordered float predicates
 * (whose negation is not a predicate under NaN) and the pointer tests pin
 * the shapes the fold must leave alone or handle exactly. */
typedef unsigned char u8;
typedef signed char s8;
typedef unsigned short u16;
typedef short s16;
typedef unsigned u32;
typedef int s32;
typedef unsigned long long u64;
typedef long long s64;

#define PAIR(name, T, cond)                                                   \
  __attribute__((noinline)) int name##_t(T a, T b) { if (cond) return 1; return 0; } \
  __attribute__((noinline)) int name##_f(T a, T b) { if (cond) return 0; return 1; }

PAIR(slt, s32, a < b)
PAIR(sle, s32, a <= b)
PAIR(sgt, s32, a > b)
PAIR(sge, s32, a >= b)
PAIR(eq, s32, a == b)
PAIR(ne, s32, a != b)
PAIR(ult, u32, a < b)
PAIR(ule, u32, a <= b)
PAIR(ugt, u32, a > b)
PAIR(uge, u32, a >= b)
PAIR(slt64, s64, a < b)
PAIR(ugt64, u64, a > b)
PAIR(eq64, u64, a == b)
PAIR(slt8, s8, a < b)
PAIR(ult16, u16, a < b)
PAIR(flt, double, a < b)
PAIR(fle, double, a <= b)
PAIR(fgt, double, a > b)
PAIR(fge, double, a >= b)
PAIR(feq, double, a == b)
PAIR(fne, double, a != b)
PAIR(fgt32, float, a > b)
PAIR(fne32, float, a != b)

/* The returned boolean at every return width. */
__attribute__((noinline)) long ret_long(s32 a, s32 b) { if (a < b) return 1; return 0; }
__attribute__((noinline)) u8 ret_u8(s32 a, s32 b) { if (a < b) return 0; return 1; }
__attribute__((noinline)) _Bool ret_bool(u64 a, u64 b) { if (a > b) return 1; return 0; }
__attribute__((noinline)) s16 ret_s16(s32 a, s32 b) { if (a == b) return 1; return 0; }
__attribute__((noinline)) u32 ret_u32(s32 a, s32 b) { if (a >= b) return 1; return 0; }
__attribute__((noinline)) s64 ret_s64(u32 a, u32 b) { if (a <= b) return 0; return 1; }
__attribute__((noinline)) s8 ret_s8(s32 a, s32 b) { if (a != b) return 1; return 0; }
__attribute__((noinline)) u8 ret_u8_wrap(s32 a, s32 b) { if (a < b) return (u8)257; return 0; }

/* Single-bit tests (the IR BitTest condition). */
__attribute__((noinline)) int bt64_t(u64 x, u32 i) { if ((x >> (i & 63)) & 1) return 1; return 0; }
__attribute__((noinline)) int bt64_f(u64 x, u32 i) { if ((x >> (i & 63)) & 1) return 0; return 1; }
__attribute__((noinline)) int bt32_t(u32 x, u32 i) { if (x & (1u << (i & 31))) return 1; return 0; }
__attribute__((noinline)) int bt32_f(u32 x, u32 i) { if (x & (1u << (i & 31))) return 0; return 1; }
__attribute__((noinline)) long bt64_long(u64 x, u32 i) { if ((x >> (i & 63)) & 1) return 0; return 1; }
/* sqlite3's MASKBIT guard: `i` is a column index, never negative. */
__attribute__((noinline)) int maskbit(u64 mask, s32 i) {
  if (i >= 64 || (mask & (1ull << i)) == 0) return 0;
  return 1;
}

/* Integer truthiness: the zero test the branch itself performs. */
__attribute__((noinline)) int nz32_t(s32 x) { if (x) return 1; return 0; }
__attribute__((noinline)) int nz32_f(s32 x) { if (x) return 0; return 1; }
__attribute__((noinline)) int nz64_t(u64 x) { if (x) return 1; return 0; }
__attribute__((noinline)) int nz8_f(u8 x) { if (x) return 0; return 1; }
__attribute__((noinline)) int nz16_t(s16 x) { if (x) return 1; return 0; }
__attribute__((noinline)) int and_f(u32 x, u32 m) { if (x & m) return 0; return 1; }
__attribute__((noinline)) int ptr_t(const void *p) { if (p) return 1; return 0; }
__attribute__((noinline)) int ptr_f(const void *p) { if (p) return 0; return 1; }

/* Not a 0/1 pair: the branch (or an equivalent select) must stay exact. */
__attribute__((noinline)) int neg_t(s32 a, s32 b) { if (a < b) return -1; return 0; }
__attribute__((noinline)) int two_t(s32 a, s32 b) { if (a < b) return 2; return 0; }
__attribute__((noinline)) int same_t(s32 a, s32 b) { if (a < b) return 1; return 1; }

/* Extension chains: an int widened to long after a narrow extension. */
__attribute__((noinline)) long zext_b_ext(const u8 *p) { s32 x = *p; return x; }
__attribute__((noinline)) long zext_w_ext(const u16 *p) { s32 x = *p; return x; }
__attribute__((noinline)) long sext_b_ext(const s8 *p) { s32 x = *p; return x; }
__attribute__((noinline)) long sext_w_ext(const s16 *p) { s32 x = *p; return x; }
__attribute__((noinline)) s64 zext_reg_ext(u8 c) { s32 x = c; return x; }
__attribute__((noinline)) s64 sext_reg_ext(s8 c) { s16 y = c; s32 x = y; return x; }
__attribute__((noinline)) s64 zext_w_reg_ext(u16 c) { s32 x = c; return x; }
__attribute__((noinline)) u64 zext_sum_ext(u8 c, s32 k) { s32 x = c; return (u64)(s64)x + (u64)(s64)k; }
__attribute__((noinline)) long cmp_long_ext(s32 a, s32 b) { return a < b; }
__attribute__((noinline)) s64 cmp_s64_ext(u64 a, u64 b) { return a == b; }
