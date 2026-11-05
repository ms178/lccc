/* Freestanding checksum over the boolean-return kernels (bool_return.c).
 *
 * 5265056886320220448 is the value GCC 16.2 and Clang 23.1.5 both compute
 * at -O0, -O1, -O2, -O3 and -Os (hosted x86-64 build of both files), and
 * that a -fsanitize=undefined trap build of both compilers reproduces
 * without trapping.  Every observation is width-neutral (a boolean, or a
 * value that fits the narrowest `long`), so the constant holds on all four
 * targets.  The float inputs include NaN and both zeros: an ordered
 * predicate inverted into its opposite, instead of negated, changes the
 * NaN rows. */
typedef unsigned char u8;
typedef signed char s8;
typedef unsigned short u16;
typedef short s16;
typedef unsigned u32;
typedef int s32;
typedef unsigned long long u64;
typedef long long s64;

#define DECL(name, T) int name##_t(T, T); int name##_f(T, T);
DECL(slt, s32) DECL(sle, s32) DECL(sgt, s32) DECL(sge, s32) DECL(eq, s32) DECL(ne, s32)
DECL(ult, u32) DECL(ule, u32) DECL(ugt, u32) DECL(uge, u32)
DECL(slt64, s64) DECL(ugt64, u64) DECL(eq64, u64) DECL(slt8, s8) DECL(ult16, u16)
DECL(flt, double) DECL(fle, double) DECL(fgt, double) DECL(fge, double)
DECL(feq, double) DECL(fne, double) DECL(fgt32, float) DECL(fne32, float)
long ret_long(s32, s32);
u8 ret_u8(s32, s32);
_Bool ret_bool(u64, u64);
s16 ret_s16(s32, s32);
u32 ret_u32(s32, s32);
s64 ret_s64(u32, u32);
s8 ret_s8(s32, s32);
u8 ret_u8_wrap(s32, s32);
int bt64_t(u64, u32);
int bt64_f(u64, u32);
int bt32_t(u32, u32);
int bt32_f(u32, u32);
long bt64_long(u64, u32);
int maskbit(u64, s32);
int nz32_t(s32);
int nz32_f(s32);
int nz64_t(u64);
int nz8_f(u8);
int nz16_t(s16);
int and_f(u32, u32);
int ptr_t(const void *);
int ptr_f(const void *);
int neg_t(s32, s32);
int two_t(s32, s32);
int same_t(s32, s32);
long zext_b_ext(const u8 *);
long zext_w_ext(const u16 *);
long sext_b_ext(const s8 *);
long sext_w_ext(const s16 *);
s64 zext_reg_ext(u8);
s64 sext_reg_ext(s8);
s64 zext_w_reg_ext(u16);
u64 zext_sum_ext(u8, s32);
long cmp_long_ext(s32, s32);
s64 cmp_s64_ext(u64, u64);

static u64 h = 1469598103934665603ull;
static void mix(u64 v) {
  for (int i = 0; i < 8; i++) {
    h ^= (v >> (8 * i)) & 255u;
    h *= 1099511628211ull;
  }
}
static volatile double zero = 0.0;

u64 bool_return_checksum(void) {
  static const s32 ints[] = {
      (s32)0x80000000u, (s32)0x80000001u, -65, -2, -1, 0, 1, 2, 63, 64,
      127, 128, 255, 256, 0x55555555, 0x7ffffffe, 0x7fffffff};
  static const u64 wides[] = {
      0x8000000000000000ull, ~0ull, 0, 1, 2, 0x100000000ull,
      0xffffffffull, 0x7fffffffffffffffull, 0x8000000000000001ull};
  static const s8 bytes[] = {-128, -127, -1, 0, 1, 126, 127};
  static const u16 halves[] = {0, 1, 0x7f, 0x80, 0xff, 0x100, 0x7fff, 0x8000, 0xffff};
  static const u64 masks[] = {
      0, ~0ull, 0x8000000000000001ull, 0xaaaaaaaaaaaaaaaaull,
      0x5555555555555555ull, 0xdeadbeefcafebabeull, 1};
  const unsigned nints = sizeof ints / sizeof ints[0];
  const unsigned nwides = sizeof wides / sizeof wides[0];
  const unsigned nbytes = sizeof bytes / sizeof bytes[0];
  const unsigned nhalves = sizeof halves / sizeof halves[0];
  const unsigned nmasks = sizeof masks / sizeof masks[0];
  double nan = zero / zero;
  const double reals[] = {-1.0 / zero, -1.5, -0.0, 0.0, 1.0, 2.5, 1.0 / zero, nan};
  const unsigned nreals = sizeof reals / sizeof reals[0];

  for (unsigned i = 0; i < nints; i++) {
    for (unsigned j = 0; j < nints; j++) {
      s32 a = ints[i], b = ints[j];
      u32 ua = (u32)a, ub = (u32)b;
      mix((u64)(slt_t(a, b) | slt_f(a, b) << 1 | sle_t(a, b) << 2 | sle_f(a, b) << 3));
      mix((u64)(sgt_t(a, b) | sgt_f(a, b) << 1 | sge_t(a, b) << 2 | sge_f(a, b) << 3));
      mix((u64)(eq_t(a, b) | eq_f(a, b) << 1 | ne_t(a, b) << 2 | ne_f(a, b) << 3));
      mix((u64)(ult_t(ua, ub) | ult_f(ua, ub) << 1 | ule_t(ua, ub) << 2 | ule_f(ua, ub) << 3));
      mix((u64)(ugt_t(ua, ub) | ugt_f(ua, ub) << 1 | uge_t(ua, ub) << 2 | uge_f(ua, ub) << 3));
      mix((u64)(s64)ret_long(a, b));
      mix((u64)ret_u8(a, b));
      mix((u64)ret_s16(a, b));
      mix((u64)ret_u32(a, b));
      mix((u64)ret_s64(ua, ub));
      mix((u64)(s64)ret_s8(a, b));
      mix((u64)ret_u8_wrap(a, b));
      mix((u64)(s64)neg_t(a, b));
      mix((u64)(two_t(a, b) | same_t(a, b) << 4));
      mix((u64)(s64)cmp_long_ext(a, b));
      mix((u64)and_f(ua, ub));
    }
    mix((u64)(nz32_t(ints[i]) | nz32_f(ints[i]) << 1 | nz16_t((s16)ints[i]) << 2 |
              nz8_f((u8)ints[i]) << 3));
  }
  for (unsigned i = 0; i < nwides; i++) {
    for (unsigned j = 0; j < nwides; j++) {
      u64 a = wides[i], b = wides[j];
      mix((u64)(slt64_t((s64)a, (s64)b) | slt64_f((s64)a, (s64)b) << 1));
      mix((u64)(ugt64_t(a, b) | ugt64_f(a, b) << 1 | eq64_t(a, b) << 2 | eq64_f(a, b) << 3));
      mix((u64)ret_bool(a, b));
      mix((u64)cmp_s64_ext(a, b));
    }
    mix((u64)nz64_t(wides[i]));
  }
  for (unsigned i = 0; i < nbytes; i++)
    for (unsigned j = 0; j < nbytes; j++)
      mix((u64)(slt8_t(bytes[i], bytes[j]) | slt8_f(bytes[i], bytes[j]) << 1));
  for (unsigned i = 0; i < nhalves; i++)
    for (unsigned j = 0; j < nhalves; j++)
      mix((u64)(ult16_t(halves[i], halves[j]) | ult16_f(halves[i], halves[j]) << 1));
  for (unsigned i = 0; i < nreals; i++) {
    for (unsigned j = 0; j < nreals; j++) {
      double a = reals[i], b = reals[j];
      float fa = (float)a, fb = (float)b;
      mix((u64)(flt_t(a, b) | flt_f(a, b) << 1 | fle_t(a, b) << 2 | fle_f(a, b) << 3));
      mix((u64)(fgt_t(a, b) | fgt_f(a, b) << 1 | fge_t(a, b) << 2 | fge_f(a, b) << 3));
      mix((u64)(feq_t(a, b) | feq_f(a, b) << 1 | fne_t(a, b) << 2 | fne_f(a, b) << 3));
      mix((u64)(fgt32_t(fa, fb) | fgt32_f(fa, fb) << 1 | fne32_t(fa, fb) << 2 |
                fne32_f(fa, fb) << 3));
    }
  }
  for (unsigned m = 0; m < nmasks; m++) {
    for (u32 i = 0; i < 80; i++) {
      u64 x = masks[m];
      mix((u64)(bt64_t(x, i) | bt64_f(x, i) << 1 | bt32_t((u32)x, i) << 2 |
                bt32_f((u32)(x >> 32), i) << 3));
      mix((u64)(s64)bt64_long(x, i));
    }
    for (s32 i = 0; i < 100; i++)
      mix((u64)maskbit(masks[m], i));
  }
  mix((u64)(ptr_t(&h) | ptr_f(&h) << 1 | ptr_t((const void *)0) << 2 |
            ptr_f((const void *)0) << 3));
  for (unsigned v = 0; v < 256; v++) {
    u8 c = (u8)v;
    s8 sc = (s8)v;
    mix((u64)(s64)zext_b_ext(&c));
    mix((u64)(s64)sext_b_ext(&sc));
    mix((u64)zext_reg_ext(c));
    mix((u64)sext_reg_ext(sc));
    mix(zext_sum_ext(c, ints[v % nints]));
  }
  for (unsigned i = 0; i < nhalves; i++) {
    u16 w = halves[i];
    s16 sw = (s16)halves[i];
    mix((u64)(s64)zext_w_ext(&w));
    mix((u64)(s64)sext_w_ext(&sw));
    mix((u64)zext_w_reg_ext(w));
  }
  return h;
}

int test(void) { return bool_return_checksum() != 5265056886320220448ull; }
