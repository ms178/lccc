/* Nested-diamond if-conversion battery (session S61).
 *
 * Pins the two defects fixed together:
 *
 *   (A) the load-speculation coverage gate only consulted the diamond's own
 *       pred block, so a NESTED conditional — whose inner head holds nothing
 *       but the compare — could never prove its arm load safe, even though
 *       the covering access sits in the outer head that dominates it;
 *
 *   (B) `detect_diamond` required each arm to be ONE block branching straight
 *       to the merge, so once the inner level converted, the outer false arm
 *       was a two-block chain (`head -> inner_merge`) and the outer level
 *       stayed branchy forever.
 *
 * Every check is a VALUE check, so the tri-config differential (if-conversion
 * on / disabled / gcc) is bit-exact by construction.  The negative controls
 * are runtime-fatal if speculation happens: they pass pointers that would
 * fault when dereferenced on the path that must not dereference them.
 */
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

/* ── 1. the shape the fix exists for: clamp with the covering load in the
 *     OUTER head, two levels of diamond ------------------------------- */
int nested_covered(const int *p, int *out) {
  int v = *p; /* dominates both diamond levels */
  int r;
  if (v > 100)
    r = 100;
  else if (v < 0)
    r = *p; /* same address: provably covered by the dominating load */
  else
    r = v;
  *out = r;
  return r;
}

/* ── 2. NEGATIVE control: the arm load is NOT covered anywhere on the
 *     other path.  Speculating it would dereference NULL. ------------- */
int half_guard(const int *p, int sel, int *out) {
  int r = 0;
  if (sel) {
    r = *p; /* p is NULL whenever sel == 0 */
  }
  *out = r;
  return r;
}

/* ── 3. NEGATIVE control, nested: the INNER arm's load is uncovered. The
 *     dominating chain must not manufacture coverage out of an unrelated
 *     address. -------------------------------------------------------- */
int nested_uncovered(const int *covered, const int *maybe_null, int sel,
                     int *out) {
  int v = *covered; /* dominates, but covers a DIFFERENT address */
  int r;
  if (v > 100)
    r = 100;
  else if (sel)
    r = *maybe_null; /* NULL when sel == 0 */
  else
    r = v;
  *out = r;
  return r;
}

/* ── 4. side effects in an arm: must never be speculated ------------- */
volatile int side_sink;
int side_effect_arms(const int *a, int *out) {
  int v = *a;
  int r;
  if (v > 10) {
    side_sink = 1;
    r = 10;
  } else if (v < -10) {
    side_sink = 2;
    r = -10;
  } else {
    side_sink = 3;
    r = v;
  }
  *out = r;
  return r;
}

/* ── 5. volatile load in an arm: never speculatable ------------------ */
int volatile_arm(volatile int *p, int sel, int *out) {
  int r;
  if (sel)
    r = *p;
  else
    r = 42;
  *out = r;
  return r;
}

/* ── 6. the vectorizable family: clamps at every lane width ---------- */
void clamp_u8x16(const uint8_t *restrict a, uint8_t *restrict d) {
  for (int i = 0; i < 16; i++)
    d[i] = a[i] > 200 ? 200 : (a[i] < 16 ? 16 : a[i]);
}
void clamp_i32x4(const int32_t *restrict a, int32_t *restrict d) {
  for (int i = 0; i < 4; i++)
    d[i] = a[i] > 200 ? 200 : (a[i] < 16 ? 16 : a[i]);
}
void clamp3_i32x4(const int32_t *restrict a, int32_t *restrict d) {
  for (int i = 0; i < 4; i++) {
    int t = a[i];
    d[i] = t > 200 ? 200 : (t < 16 ? 16 : (t == 100 ? -1 : t));
  }
}
void clamp_dyn_i32x4(const int32_t *restrict a, const int32_t *restrict b,
                     int32_t *restrict d) {
  for (int i = 0; i < 4; i++) {
    int hi = b[i], lo = -b[i];
    d[i] = a[i] > hi ? hi : (a[i] < lo ? lo : a[i]);
  }
}
void clamp_u16x8(const uint16_t *restrict a, uint16_t *restrict d) {
  for (int i = 0; i < 8; i++)
    d[i] = a[i] > 60000 ? 60000 : (a[i] < 100 ? 100 : a[i]);
}
void clamp_i16x8(const int16_t *restrict a, int16_t *restrict d) {
  for (int i = 0; i < 8; i++)
    d[i] = a[i] > 30000 ? 30000 : (a[i] < -30000 ? -30000 : a[i]);
}

/* ── 6b. the TEMPORARY spelling of the same clamps.  C promotes the whole
 *     tree either way, but the front end materializes the intermediate
 *     variable differently: the compare stays PROMOTED here (it is narrowed
 *     in the single-expression form above) and the twice-read `a[i]` becomes
 *     a `Copy` of the load.  `build_demoted_select_arm` has to accept both
 *     compare spellings and `build_pack` has to see through the copy, so
 *     these are a distinct code path from section 6 and are value-checked
 *     here, not only inspected as assembly. ------------------------------ */
void clamp_lo_u8x16(const uint8_t *restrict a, uint8_t *restrict d) {
  for (int i = 0; i < 16; i++) d[i] = a[i] < 16 ? 16 : a[i];
}
void clamp_tmp_u8x16(const uint8_t *restrict a, uint8_t *restrict d) {
  for (int i = 0; i < 16; i++) {
    uint8_t t = a[i] > 200 ? 200 : a[i];
    d[i] = t < 16 ? 16 : t;
  }
}
void clamp_tmp_i8x16(const int8_t *restrict a, int8_t *restrict d) {
  for (int i = 0; i < 16; i++) {
    int8_t t = a[i] > 100 ? 100 : a[i];
    d[i] = t < -100 ? -100 : t;
  }
}
void clamp_tmp_u16x8(const uint16_t *restrict a, uint16_t *restrict d) {
  for (int i = 0; i < 8; i++) {
    uint16_t t = a[i] > 32000 ? 32000 : a[i];
    d[i] = t < 4 ? 4 : t;
  }
}
void clamp_tmp_i16x8(const int16_t *restrict a, int16_t *restrict d) {
  for (int i = 0; i < 8; i++) {
    int16_t t = a[i] > 20000 ? 20000 : a[i];
    d[i] = t < -20000 ? -20000 : t;
  }
}
void clamp_tmp_i32x4(const int32_t *restrict a, int32_t *restrict d) {
  for (int i = 0; i < 4; i++) {
    int32_t t = a[i] > 200 ? 200 : a[i];
    d[i] = t < 16 ? 16 : t;
  }
}

/* ── 6c. two more negative controls, on the STORE side.  `half_store` only
 *     assigns on one path of the second level (the `i > 20` arm is dead for
 *     i < 16), so the store set is half-covered and the diamond must survive;
 *     `volatile_store_arm` writes a volatile in one arm, which is a side
 *     effect and must never be speculated. ------------------------------ */
volatile int vsink;
void half_store(const uint8_t *restrict a, uint8_t *restrict d) {
  for (int i = 0; i < 16; i++) {
    if (a[i] & 1) d[i] = 7;
    else if (i > 20) d[i] = 9;
  }
}
void volatile_store_arm(const uint8_t *restrict a, uint8_t *restrict d) {
  for (int i = 0; i < 16; i++) {
    if (a[i] > 100) { vsink = 3; d[i] = 200; }
    else if (a[i] < 16) d[i] = 16;
    else d[i] = a[i];
  }
}

/* ── 7. a loop the fix must hand to the loop vectorizer --------------- */
void clamp_loop_u8(const uint8_t *restrict a, uint8_t *restrict d, int n) {
  for (int i = 0; i < n; i++)
    d[i] = a[i] > 200 ? 200 : (a[i] < 16 ? 16 : a[i]);
}

/* ── 8. an arm chain that must NOT be followed into a join block ----- */
int join_in_chain(const int *a, int n, int *out) {
  int v = a[0];
  int r;
  if (v > 5) {
    r = 5;
  } else {
    /* two-way inner split whose false arm rejoins a block also reached
     * from outside the diamond — the chain walk must stop there */
    if (v > 0)
      r = v * 2;
    else
      r = n;
    if (r > 1000) r = 1000;
  }
  *out = r;
  return r;
}

int main(void) {
  int out = -12345;

  /* (1) covering load in the dominating outer head */
  {
    int buf[4];
    int vals[] = {-50, 0, 50, 999};
    long long want[] = {-50, 0, 50, 100};
    for (int i = 0; i < 4; i++) {
      buf[0] = vals[i];
      EXPECT("nested_covered", nested_covered(buf, &out), want[i]);
      EXPECT("nested_covered out", out, want[i]);
    }
  }

  /* (2) the uncovered arm load must not be speculated: NULL + sel==0 */
  EXPECT("half_guard null", half_guard((const int *)0, 0, &out), 0);
  EXPECT("half_guard null out", out, 0);
  {
    int v = 77;
    EXPECT("half_guard set", half_guard(&v, 1, &out), 77);
    EXPECT("half_guard set out", out, 77);
  }

  /* (3) nested uncovered: dominating block covers a DIFFERENT address */
  {
    int cov = 10;
    EXPECT("nested_uncovered nosel", nested_uncovered(&cov, (const int *)0, 0, &out), 10);
    int p = 5;
    EXPECT("nested_uncovered sel", nested_uncovered(&cov, &p, 1, &out), 5);
    cov = 500;
    EXPECT("nested_uncovered hi", nested_uncovered(&cov, (const int *)0, 0, &out), 100);
  }

  /* (4) side effects: the arm that runs must be the one that wrote */
  {
    int a[4];
    int vals[] = {50, -50, 3};
    long long want[] = {10, -10, 3};
    long long sinkw[] = {1, 2, 3};
    for (int i = 0; i < 3; i++) {
      a[0] = vals[i];
      side_sink = 0;
      EXPECT("side_effect_arms", side_effect_arms(a, &out), want[i]);
      EXPECT("side_effect_arms sink", side_sink, sinkw[i]);
    }
  }

  /* (5) volatile arm */
  {
    volatile int v = 9;
    EXPECT("volatile_arm set", volatile_arm(&v, 1, &out), 9);
    EXPECT("volatile_arm unset", volatile_arm(&v, 0, &out), 42);
  }

  /* (6) lane-width clamps, boundary-dense */
  {
    /* 16 entries: the u8 kernel below reads exactly 16 (a 14-entry table
     * here was undefined behavior — gcc -Waggressive-loop-optimizations
     * caught it at delivery; the battery must be warning-free). */
    static const int probes[16] = {0,   1,    15,   16,  17,  99,  100, 101,
                                   199, 200,  201,  255, -1,  -1000, 128, 64};
    uint8_t a8[16], d8[16];
    int32_t a32[4], d32[4], d3[4], dyn[4], b32[4];
    uint16_t a16[8], d16[8];
    int16_t as16[8], ds16[8];
    for (int i = 0; i < 16; i++) a8[i] = (uint8_t)probes[i];
    clamp_u8x16(a8, d8);
    for (int i = 0; i < 16; i++) {
      unsigned v = a8[i];
      unsigned w = v > 200 ? 200 : (v < 16 ? 16 : v);
      EXPECT("clamp_u8x16", d8[i], w);
    }
    for (int i = 0; i < 4; i++) { a32[i] = probes[i]; b32[i] = 50; }
    clamp_i32x4(a32, d32);
    clamp3_i32x4(a32, d3);
    clamp_dyn_i32x4(a32, b32, dyn);
    for (int i = 0; i < 4; i++) {
      int v = a32[i];
      EXPECT("clamp_i32x4", d32[i], v > 200 ? 200 : (v < 16 ? 16 : v));
      EXPECT("clamp3_i32x4", d3[i],
             v > 200 ? 200 : (v < 16 ? 16 : (v == 100 ? -1 : v)));
      int hi = b32[i], lo = -hi;
      EXPECT("clamp_dyn_i32x4", dyn[i], v > hi ? hi : (v < lo ? lo : v));
    }
    for (int i = 0; i < 8; i++) a16[i] = (uint16_t)(i * 9000 + 5);
    clamp_u16x8(a16, d16);
    for (int i = 0; i < 8; i++) {
      unsigned v = a16[i];
      EXPECT("clamp_u16x8", d16[i], v > 60000 ? 60000 : (v < 100 ? 100 : v));
    }
    for (int i = 0; i < 8; i++) as16[i] = (int16_t)(i * 9000 - 32000);
    clamp_i16x8(as16, ds16);
    for (int i = 0; i < 8; i++) {
      int v = as16[i];
      EXPECT("clamp_i16x8", ds16[i],
             v > 30000 ? 30000 : (v < -30000 ? -30000 : v));
    }
  }

  /* (7) the loop form (the runtime win: 41x measured) */
  {
    uint8_t a[64], d[64];
    for (int i = 0; i < 64; i++) a[i] = (uint8_t)((i * 37) & 0xFF);
    clamp_loop_u8(a, d, 64);
    for (int i = 0; i < 64; i++) {
      unsigned v = a[i];
      EXPECT("clamp_loop_u8", d[i], v > 200 ? 200 : (v < 16 ? 16 : v));
    }
  }

  /* (8) join inside the arm chain */
  {
    int a[4];
    int vals[] = {50, 3, -7, 0};
    long long want[] = {5, 6, 1000, 1000};
    for (int i = 0; i < 4; i++) {
      a[0] = vals[i];
      EXPECT("join_in_chain", join_in_chain(a, 1000, &out), want[i]);
    }
  }

  /* (6b) the temporary spelling: promoted inner compare + a copy of the
   * load for the twice-read operand.  Boundary-dense on purpose — the
   * demotion remaps signed/unsigned predicates and truncates the compare
   * constants, so the values at and around every bound are the ones that
   * would expose an off-by-one or a signedness flip. */
  {
    static const int probes[16] = {0,   1,    3,    4,    5,    15,   16,  17,
                                   99,  100,  101,  199,  200,  201,  255, 128};
    uint8_t a8[16], d8[16], lo8[16];
    int8_t as8[16], ds8[16];
    uint16_t a16[8], d16[8];
    int16_t as16[8], ds16[8];
    int32_t a32[4], d32[4];
    for (int i = 0; i < 16; i++) a8[i] = (uint8_t)probes[i];
    clamp_lo_u8x16(a8, lo8);
    clamp_tmp_u8x16(a8, d8);
    for (int i = 0; i < 16; i++) {
      unsigned v = a8[i];
      EXPECT("clamp_lo_u8x16", lo8[i], v < 16 ? 16 : v);
      unsigned t = v > 200 ? 200 : v;
      EXPECT("clamp_tmp_u8x16", d8[i], t < 16 ? 16 : t);
    }
    /* signed 8-bit: straddle both bounds, including the exact values */
    static const int s8[16] = {0, 1, -1, 99, 100, 101, -99, -100,
                               -101, 127, -128, 50, -50, 64, -64, 2};
    for (int i = 0; i < 16; i++) as8[i] = (int8_t)s8[i];
    clamp_tmp_i8x16(as8, ds8);
    for (int i = 0; i < 16; i++) {
      int v = as8[i];
      int t = v > 100 ? 100 : v;
      EXPECT("clamp_tmp_i8x16", ds8[i], t < -100 ? -100 : t);
    }
    /* unsigned 16-bit: include the whole upper half, where a signed
     * predicate would read the top bit as a sign and invert the clamp */
    static const unsigned u16p[8] = {0, 3, 4, 5, 100, 31999, 32000, 65535};
    for (int i = 0; i < 8; i++) a16[i] = (uint16_t)u16p[i];
    clamp_tmp_u16x8(a16, d16);
    for (int i = 0; i < 8; i++) {
      unsigned v = a16[i];
      unsigned t = v > 32000 ? 32000 : v;
      EXPECT("clamp_tmp_u16x8", d16[i], t < 4 ? 4 : t);
    }
    static const int s16p[8] = {0, -1, 1, 19999, 20000, 20001, -20000, -32768};
    for (int i = 0; i < 8; i++) as16[i] = (int16_t)s16p[i];
    clamp_tmp_i16x8(as16, ds16);
    for (int i = 0; i < 8; i++) {
      int v = as16[i];
      int t = v > 20000 ? 20000 : v;
      EXPECT("clamp_tmp_i16x8", ds16[i], t < -20000 ? -20000 : t);
    }
    for (int i = 0; i < 4; i++) a32[i] = probes[i];
    clamp_tmp_i32x4(a32, d32);
    for (int i = 0; i < 4; i++) {
      int v = a32[i];
      int t = v > 200 ? 200 : v;
      EXPECT("clamp_tmp_i32x4", d32[i], t < 16 ? 16 : t);
    }
  }

  /* (6c) store-side negative controls: value-checked, and the gate asserts
   * they still BRANCH. */
  {
    uint8_t a[16], d[16], e[16];
    for (int i = 0; i < 16; i++) { a[i] = (uint8_t)(i * 17); d[i] = 0xEE; e[i] = 0xEE; }
    half_store(a, d);
    for (int i = 0; i < 16; i++)
      EXPECT("half_store", d[i], (a[i] & 1) ? 7 : 0xEE);
    /* The kernel consumes all 16 lanes per call, so call it ONCE on the
     * whole array.  `vsink` is a single global written by every lane that
     * takes the first arm, so it is checked once for the array as a whole,
     * not per lane. */
    for (int i = 0; i < 16; i++) { a[i] = (uint8_t)(i * 17); e[i] = 0xEE; }
    vsink = 0;
    volatile_store_arm(a, e);
    int any_over = 0;
    for (int i = 0; i < 16; i++) {
      unsigned v = a[i];
      EXPECT("volatile_store_arm", e[i],
             v > 100 ? 200 : (v < 16 ? 16 : v));
      if (v > 100) any_over = 1;
    }
    EXPECT("volatile_store_arm sink", vsink, any_over ? 3 : 0);
  }

  if (fails == 0)
    printf("bb_slp_nested_ifconv: all pass (0 fails)\n");
  else
    printf("bb_slp_nested_ifconv: %d fails\n", fails);
  return fails != 0;
}
