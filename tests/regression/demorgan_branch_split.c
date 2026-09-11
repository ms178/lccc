// De Morgan branch split: `if (CmpA && CmpB)` / `if (CmpA || CmpB)` where
// both comparisons are single-use must compile to cmpA; jcc; cmpB; jcc
// (short-circuit) instead of two boolean materializations plus and/or/test.
// The loop-carried shapes below mirror lz4's fill_source/compress_block
// sites (the frontend keeps them as And/Or+branch IR; see the census).
// Exit status 0 + "OK demorgan_branch_split <checksum>" on success.
#include <stdio.h>

static unsigned int rng_state = 0x5a1b3c7dU;
static unsigned char buf[512];

static void fill(void) {
  unsigned int i;
  for (i = 0; i < sizeof(buf); i++) {
    rng_state = rng_state * 1664525U + 1013904223U;
    // AND site: must fire the split (both Cmps single-use, same block).
    if ((rng_state & 0x0fU) < 6 && i >= 128)
      buf[i] = buf[i - 128 + (rng_state & 0x3fU)];
    else
      buf[i] = (unsigned char)(rng_state >> 24);
  }
}

// OR site needs call arms: pure-arithmetic arms get if-converted to a
// Select (cmov), and direct `||` is control-flow-lowered by the frontend
// and never merged back — so the materialized `|`+Cast+branch shape below
// (via a noinline sink) is what exercises the Or passthru path.
__attribute__((noinline)) static void sink(unsigned int x, unsigned int i) {
  buf[(x + i) & 511] ^= (unsigned char)x;
}

static unsigned long scan(void) {
  unsigned int i;
  unsigned long acc = 0;
  for (i = 0; i < sizeof(buf); i++) {
    unsigned int v = buf[i];
    // OR site: must fire the split (short-circuit on true). The `|`
    // materializes an Or binop, the `unsigned` assignment adds a Cast
    // passthru, and the calls keep a real CondBranch.
    unsigned int c = (v < 16) | (i > 400);
    if (c)
      sink(v * 3 + i, i);
    else
      sink(v + 7, i);
    acc += buf[i] + i;
  }
  return acc;
}

int main(void) {
  unsigned long h;
  unsigned int i;
  unsigned long mix = 0;
  fill();
  h = scan();
  for (i = 0; i < sizeof(buf); i++)
    mix = mix * 31 + buf[i];
  mix ^= h * 0x9e3779b1UL;
  printf("OK demorgan_branch_split %lu\n", mix);
  return 0;
}
