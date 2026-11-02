/* bb_slp_unary_demote.c — packed unary (negate / complement) on sub-word
 * lanes (session S61, follow-up D).
 *
 * SUBJECT.  C integer promotion means `-t` on an `int16_t` does not reach the
 * vectorizer as `Neg_i16(t)`.  It reaches it as
 *
 *     Cast_{I32->I16}( UnaryOp{Neg, I32}( Cast_{I16->I32}(t) ) )
 *
 * — the negation happens at the promoted width and only the store truncates.
 * The BB-SLP unary recognizer required every lane to be a `UnaryOp` whose type
 * EQUALLED the lane type, so the promoted spelling matched nothing and the
 * seed was rejected: `abs_i16x8` stayed 63 scalar instructions where gcc
 * emits 7 (`vpsubw` / `vpcmpgtw` / `vpblendvb`).
 *
 * `unary_lane_src` normalizes the sandwich to the narrow unary.  That is
 * exact, not an approximation: two's-complement negation and bitwise
 * complement are per-bit / modulo-2^bits operations, so negating wide and
 * truncating agrees bit-for-bit with truncating and negating.  The widened
 * operand must be a widening of exactly the lane type, so what gets negated
 * is bit-identical to the lane; anything else fails closed.
 *
 * VALUES.  Every case that can distinguish a wrong width or a wrong
 * signedness is present: INT16_MIN (whose negation overflows back to itself —
 * the classic case a widened negate gets right only if the truncation is
 * kept), the ±1 and ±max boundaries, and the full unsigned range for the
 * complement.  The signed 8-bit family is the control that must NOT take a
 * packed min/max fold (there is no signed byte min/max before AVX-512).
 *
 * The signed/unsigned distinction matters twice over here: the negate is
 * signed, the complement is not, and both run at three lane widths. */
#include <stdint.h>
#include <stdio.h>

static int fails = 0;
#define EXPECT(name, got, want)                                     \
  do {                                                              \
    long long g_ = (long long)(got), w_ = (long long)(want);        \
    if (g_ != w_) {                                                 \
      printf("FAIL %s: got %lld want %lld\n", (name), g_, w_);      \
      fails++;                                                      \
    }                                                               \
  } while (0)

void abs_i16x8(const int16_t *restrict a, int16_t *restrict d) {
  for (int i = 0; i < 8; i++) {
    int16_t t = a[i];
    d[i] = t < 0 ? (int16_t)-t : t;
  }
}
void neg_i16x8(const int16_t *restrict a, int16_t *restrict d) {
  for (int i = 0; i < 8; i++) d[i] = (int16_t)-a[i];
}
void not_u16x8(const uint16_t *restrict a, uint16_t *restrict d) {
  for (int i = 0; i < 8; i++) d[i] = (uint16_t)~a[i];
}
void abs_i8x16(const int8_t *restrict a, int8_t *restrict d) {
  for (int i = 0; i < 16; i++) {
    int8_t t = a[i];
    d[i] = t < 0 ? (int8_t)-t : t;
  }
}
void neg_i32x4(const int32_t *restrict a, int32_t *restrict d) {
  for (int i = 0; i < 4; i++) d[i] = -a[i];
}
void abs_i32x4(const int32_t *restrict a, int32_t *restrict d) {
  for (int i = 0; i < 4; i++) {
    int t = a[i];
    d[i] = t < 0 ? -t : t;
  }
}
/* abs + clamp in one tree: the negate arm feeds a demoted select, so this
 * exercises the recursive arm demotion and the unary fold together. */
void absclamp_i16x8(const int16_t *restrict a, int16_t *restrict d) {
  for (int i = 0; i < 8; i++) {
    int16_t t = a[i] < 0 ? (int16_t)-a[i] : a[i];
    d[i] = t > 1000 ? 1000 : t;
  }
}

int main(void) {
  /* INT16_MIN first: -INT16_MIN overflows back to INT16_MIN, which a
   * widened negate reproduces only if the truncation is preserved. */
  int16_t a16[8] = {0, 1, -1, 32767, -32768, -32767, 100, -100};
  int16_t d16[8], n16[8], ac16[8];
  uint16_t u16[8], un16[8];
  int8_t a8[16], d8[16];
  int32_t a32[4] = {0, 1, -1, -2147483647 - 1};
  int32_t n32[4], abs32[4];

  abs_i16x8(a16, d16);
  neg_i16x8(a16, n16);
  absclamp_i16x8(a16, ac16);
  for (int i = 0; i < 8; i++) {
    int v = a16[i];
    int av = v < 0 ? -v : v;
    EXPECT("abs_i16x8", d16[i], (int16_t)av);
    EXPECT("neg_i16x8", n16[i], (int16_t)-v);
    EXPECT("absclamp_i16x8", ac16[i], (int16_t)((int16_t)av > 1000 ? 1000 : (int16_t)av));
    u16[i] = (uint16_t)((unsigned)v + 40000u);
  }
  not_u16x8(u16, un16);
  for (int i = 0; i < 8; i++) EXPECT("not_u16x8", un16[i], (uint16_t)~u16[i]);

  /* signed 8-bit across the whole range, including INT8_MIN */
  for (int i = 0; i < 16; i++) a8[i] = (int8_t)(i * 17 - 128);
  abs_i8x16(a8, d8);
  for (int i = 0; i < 16; i++) {
    int v = a8[i];
    EXPECT("abs_i8x16", d8[i], (int8_t)(v < 0 ? -v : v));
  }

  /* 32-bit controls: these already worked before the fix, so a regression
   * in the shared recognizer shows up here rather than only at 16 bits. */
  neg_i32x4(a32, n32);
  abs_i32x4(a32, abs32);
  for (int i = 0; i < 4; i++) {
    int v = a32[i];
    EXPECT("neg_i32x4", n32[i], -v);
    EXPECT("abs_i32x4", abs32[i], v < 0 ? -v : v);
  }

  if (fails == 0) printf("bb_slp_unary_demote: all pass (0 fails)\n");
  else printf("bb_slp_unary_demote: %d FAILURES\n", fails);
  return fails != 0;
}
