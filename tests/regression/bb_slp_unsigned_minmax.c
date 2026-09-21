/* bb_slp_unsigned_minmax.c — unsigned-lane clamp correctness at the signed
 * boundary (session S61).
 *
 * SUBJECT. `packed_int_minmax` maps a lane type to a packed min/max
 * intrinsic.  For 16- and 32-bit lanes every available intrinsic lowers to a
 * SIGNED instruction — `vpmaxsd`/`vpminsd`, `vpmaxsw`/`vpminsw`, and the
 * 128-bit `pmaxsw`/`pminsw`.  Mapping an unsigned lane onto one is a
 * miscompile rather than a pessimization: a lane with the top bit set reads
 * as negative, so `max(0x80000000u32, 5u32)` would return 5.  There is no
 * unsigned word or dword packed min/max in the intrinsic vocabulary (SSE4.1's
 * `pmaxud`/`pminud` are encodable but have no `IntrinsicOp`), so
 * `packed_int_minmax` fails closed for those types and the caller falls
 * through to the cmp+blendv composite, which emulates the unsigned predicate
 * with a sign-bit flip.
 *
 * WHY A TEST IF IT IS NOT REACHABLE FROM C TODAY.  C integer promotion means
 * an unsigned sub-word or word compare arrives as `Ugt`/`Ult`, which the
 * min/max fold's spelling table does not accept, so the signed instruction is
 * never selected — the cmp+blendv path takes every unsigned case instead.
 * That is exactly the invariant this file pins: it exercises unsigned clamps
 * whose values straddle the signed boundary at every lane width, so if a
 * future change ever routes an unsigned lane into the signed min/max, these
 * lanes come out wrong and the suite fails instead of shipping a silent
 * miscompile.  Values are chosen so that a signed comparison gives a
 * DIFFERENT answer from the unsigned one in every vector, not merely some.
 *
 * The signed counterparts are included as the positive control: they must
 * keep vectorizing through the min/max fold, so a future tightening that
 * breaks signed lanes shows up here too. */
#include <stdint.h>
#include <stdio.h>

static int fails = 0;
#define EXPECT(name, got, want)                                     \
  do {                                                              \
    unsigned long long g_ = (unsigned long long)(got);              \
    unsigned long long w_ = (unsigned long long)(want);             \
    if (g_ != w_) {                                                 \
      printf("FAIL %s: got %llu want %llu\n", (name), g_, w_);      \
      fails++;                                                      \
    }                                                               \
  } while (0)

/* ── unsigned clamps: every vector straddles the signed boundary ──────── */
void clamp_hi_u32x8(const uint32_t *restrict a, uint32_t *restrict d) {
  for (int i = 0; i < 8; i++) d[i] = a[i] > 200u ? 200u : a[i];
}
void clamp_lo_u32x8(const uint32_t *restrict a, uint32_t *restrict d) {
  for (int i = 0; i < 8; i++) d[i] = a[i] < 0x80000000u ? 7u : a[i];
}
void clamp_both_u32x8(const uint32_t *restrict a, uint32_t *restrict d) {
  for (int i = 0; i < 8; i++) {
    uint32_t t = a[i] > 0xfffffff0u ? 0xfffffff0u : a[i];
    d[i] = t < 16u ? 16u : t;
  }
}
void clamp_hi_u16x16(const uint16_t *restrict a, uint16_t *restrict d) {
  for (int i = 0; i < 16; i++) d[i] = a[i] > 200u ? 200u : a[i];
}
void clamp_both_u16x8(const uint16_t *restrict a, uint16_t *restrict d) {
  for (int i = 0; i < 8; i++) {
    uint16_t t = a[i] > 0xfff0u ? 0xfff0u : a[i];
    d[i] = t < 16u ? 16u : t;
  }
}
void clamp_hi_u8x16(const uint8_t *restrict a, uint8_t *restrict d) {
  for (int i = 0; i < 16; i++) d[i] = a[i] > 200u ? 200u : a[i];
}

/* ── signed positive controls: these SHOULD take the min/max fold ─────── */
void clamp_hi_i32x8(const int32_t *restrict a, int32_t *restrict d) {
  for (int i = 0; i < 8; i++) d[i] = a[i] > 200 ? 200 : a[i];
}
void clamp_both_i16x16(const int16_t *restrict a, int16_t *restrict d) {
  for (int i = 0; i < 16; i++) {
    int16_t t = a[i] > 20000 ? 20000 : a[i];
    d[i] = t < -20000 ? -20000 : t;
  }
}

int main(void) {
  /* Every element is chosen so the signed and unsigned readings of the
   * comparison disagree somewhere in the vector. */
  uint32_t a32[8] = {0u, 5u, 200u, 201u, 0x7fffffffu, 0x80000000u,
                     0xfffffff0u, 4000000000u};
  uint32_t d32[8], e32[8], f32[8];
  uint16_t a16[16], d16[16];
  uint16_t b16[8], e16[8];
  uint8_t a8[16], d8[16];
  int32_t s32[8] = {0, -1, 200, 201, 2147483647, -2147483647 - 1, 100, -100};
  int32_t sd32[8];
  int16_t s16[16], sd16[16];

  clamp_hi_u32x8(a32, d32);
  clamp_lo_u32x8(a32, e32);
  clamp_both_u32x8(a32, f32);
  for (int i = 0; i < 8; i++) {
    uint32_t v = a32[i];
    EXPECT("clamp_hi_u32x8", d32[i], v > 200u ? 200u : v);
    EXPECT("clamp_lo_u32x8", e32[i], v < 0x80000000u ? 7u : v);
    uint32_t t = v > 0xfffffff0u ? 0xfffffff0u : v;
    EXPECT("clamp_both_u32x8", f32[i], t < 16u ? 16u : t);
  }

  /* 16-bit unsigned: the whole upper half, where a signed word compare
   * flips the result. */
  for (int i = 0; i < 16; i++) a16[i] = (uint16_t)(i * 4400);
  clamp_hi_u16x16(a16, d16);
  for (int i = 0; i < 16; i++) {
    unsigned v = a16[i];
    EXPECT("clamp_hi_u16x16", d16[i], v > 200u ? 200u : v);
  }
  static const unsigned b16v[8] = {0u, 15u, 16u, 0x7fffu, 0x8000u,
                                   0xfff0u, 0xfff1u, 0xffffu};
  for (int i = 0; i < 8; i++) b16[i] = (uint16_t)b16v[i];
  clamp_both_u16x8(b16, e16);
  for (int i = 0; i < 8; i++) {
    unsigned v = b16[i];
    unsigned t = v > 0xfff0u ? 0xfff0u : v;
    EXPECT("clamp_both_u16x8", e16[i], t < 16u ? 16u : t);
  }

  /* 8-bit unsigned has a genuine unsigned instruction (pminub/pmaxub), so
   * it is the one width that may legitimately take a min/max fold. */
  for (int i = 0; i < 16; i++) a8[i] = (uint8_t)(i * 17);
  clamp_hi_u8x16(a8, d8);
  for (int i = 0; i < 16; i++) {
    unsigned v = a8[i];
    EXPECT("clamp_hi_u8x16", d8[i], v > 200u ? 200u : v);
  }

  /* signed controls */
  clamp_hi_i32x8(s32, sd32);
  for (int i = 0; i < 8; i++) {
    int v = s32[i];
    EXPECT("clamp_hi_i32x8", sd32[i], v > 200 ? 200 : v);
  }
  for (int i = 0; i < 16; i++) s16[i] = (int16_t)(i * 4400 - 32000);
  clamp_both_i16x16(s16, sd16);
  for (int i = 0; i < 16; i++) {
    int v = s16[i];
    int t = v > 20000 ? 20000 : v;
    EXPECT("clamp_both_i16x16", sd16[i], t < -20000 ? -20000 : t);
  }

  if (fails == 0) printf("bb_slp_unsigned_minmax: all pass (0 fails)\n");
  else printf("bb_slp_unsigned_minmax: %d FAILURES\n", fails);
  return fails != 0;
}
