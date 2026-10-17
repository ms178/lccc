/* x86 shifts mask their counts in hardware, so C-level masks that only feed
 * shift counts are redundant on x86-64 (zstd's BIT_lookBits spells them out
 * for portability). The rewrite must drop exactly those masks and keep every
 * mask whose bits are observable. All shifts here are defined for any count. */
#include <stdio.h>

typedef unsigned long long u64;
typedef long long i64;

/* zstd BIT_lookBits: both masks go, 63 - nb becomes ~nb. */
__attribute__((noinline)) u64 look_bits(u64 c, unsigned consumed, unsigned nb) {
  return (c << (consumed & 63)) >> 1 >> ((63 - nb) & 63);
}
/* 64 - n is -n modulo 64. */
__attribute__((noinline)) u64 high_bits(u64 x, unsigned n) { return x >> ((64 - n) & 63); }
/* A wider mask covers the six demanded bits; arithmetic shift. */
__attribute__((noinline)) i64 sar_wide_mask(i64 x, unsigned n) { return x >> (n & 127); }
/* 32-bit shifts read five bits. */
__attribute__((noinline)) unsigned shl32(unsigned x, unsigned n) {
  return (x << (n & 31)) + (x >> ((31 - n) & 31));
}
/* 60 - n is neither -n nor ~n modulo 64: the subtract stays, the mask goes. */
__attribute__((noinline)) u64 non_congruent(u64 x, unsigned n) { return x << ((60 - n) & 63); }

/* Must keep: a 64-bit shift reads bit 5, which `& 31` clears. */
__attribute__((noinline)) u64 keep_narrow_mask(u64 x, unsigned n) { return x << (n & 31); }
/* Must keep: the masked count is also stored. */
__attribute__((noinline)) u64 keep_escaping(u64 x, unsigned n, unsigned *out) {
  unsigned m = n & 63;
  *out = m;
  return x >> m;
}
/* Must keep: the masked count is also an ordinary operand. */
__attribute__((noinline)) u64 keep_arith(u64 x, unsigned n) {
  unsigned m = n & 63;
  return (x >> m) + m;
}

static u64 mix(u64 h, u64 v) { return (h ^ v) * 0x100000001b3ULL; }

int main(void) {
  static const u64 xs[] = {0, 1, 0x8000000000000001ULL, 0xdeadbeefcafef00dULL, ~0ULL,
                           0x0123456789abcdefULL};
  static const unsigned big[] = {0xffffffffu, 0x80000000u, 0x7fffffc0u, 0xffffffe1u};
  u64 h = 0xcbf29ce484222325ULL;
  unsigned sink = 0;
  for (unsigned i = 0; i < sizeof xs / sizeof xs[0]; i++) {
    u64 x = xs[i];
    for (unsigned n = 0; n < 300 + sizeof big / sizeof big[0]; n++) {
      unsigned k = n < 300 ? n : big[n - 300];
      unsigned out = 0;
      h = mix(h, look_bits(x, k, k * 7 + 3));
      h = mix(h, high_bits(x, k));
      h = mix(h, (u64)sar_wide_mask((i64)x, k));
      h = mix(h, shl32((unsigned)x, k));
      h = mix(h, non_congruent(x, k));
      h = mix(h, keep_narrow_mask(x, k));
      h = mix(h, keep_escaping(x, k, &out));
      sink += out;
      h = mix(h, keep_arith(x, k));
    }
  }
  printf("%llu %u\n", h, sink);
  return 0;
}
