#include <stdint.h>
#include <stdio.h>
#include <inttypes.h>

static uint64_t global_a;
static uint32_t global_b;
static volatile uint32_t observed;

struct pair { uint32_t lo; uint32_t hi; };
struct bits { unsigned a:3; unsigned b:5; unsigned c:8; };

static uint64_t rotl64(uint64_t x, unsigned n) {
  n &= 63u; return n ? ((x << n) | (x >> ((64u - n) & 63u))) : x;
}
static uint64_t mix(uint64_t x, uint64_t y, unsigned n) {
  x ^= rotl64(y + UINT64_C(0x9e3779b97f4a7c15), n);
  x *= UINT64_C(0xbf58476d1ce4e5b9);
  x ^= x >> 29; return x;
}
static struct pair step_pair(struct pair p, uint32_t x) {
  p.lo = (p.lo + x) ^ (p.hi >> 3);
  p.hi = (p.hi * UINT32_C(1664525)) + UINT32_C(1013904223) + p.lo;
  return p;
}
static uint64_t postdec_path(uint32_t n, uint64_t seed) {
  uint64_t sum = seed;
  do { sum = mix(sum, (uint64_t)n + UINT64_C(17), n); } while (n-- != 0);
  return sum;
}
static uint64_t wide_path(uint64_t a, uint64_t b) {
  __int128 x = ((__int128)(uint64_t)a << 32) | (uint32_t)b;
  x = x * 3 + 7;
  return (uint64_t)x ^ (uint64_t)(x >> 64);
}

int main(void) {
  uint64_t acc = UINT64_C(0x61948f45ad3816f1);
  uint64_t salt = UINT64_C(0xfe9358e9e472f72f);
  uint32_t a[16];
  struct pair p = { UINT32_C(1), UINT32_C(2) };
  struct bits bf = { 0, 0, 0 };
  for (unsigned i = 0; i < 16; ++i) a[i] = (uint32_t)(acc >> (i & 31u)) ^ (i * UINT32_C(0x45d9f3b));
  a[2] ^= (uint32_t)(acc >> 25u); acc += a[2];
  p = step_pair(p, (uint32_t)(acc + UINT32_C(0x9f1ae6e0))); acc ^= ((uint64_t)p.hi << 32) | p.lo;
  p = step_pair(p, (uint32_t)(acc + UINT32_C(0xf9cc05af))); acc ^= ((uint64_t)p.hi << 32) | p.lo;
  p = step_pair(p, (uint32_t)(acc + UINT32_C(0x13bbfed6))); acc ^= ((uint64_t)p.hi << 32) | p.lo;
  p = step_pair(p, (uint32_t)(acc + UINT32_C(0x202b2f72))); acc ^= ((uint64_t)p.hi << 32) | p.lo;
  { int32_t sx = -1866; int32_t sy = 18931; int32_t z = sx * 3 + sy * 5; acc ^= (uint64_t)(int64_t)z; }
  p = step_pair(p, (uint32_t)(acc + UINT32_C(0x29c256cb))); acc ^= ((uint64_t)p.hi << 32) | p.lo;
  if ((acc ^ UINT64_C(0x7ba0e497234b6bb4)) & UINT64_C(1)) acc ^= rotl64(salt, 8u); else acc += UINT64_C(0x395d0054871a5083);
  global_a ^= acc + UINT64_C(0xbfa71a2d8f4ac04f); global_b += (uint32_t)(salt >> 38u); acc = mix(acc, global_a ^ global_b, 38u);
  global_a ^= acc + UINT64_C(0xd4ac96d5f19420cc); global_b += (uint32_t)(salt >> 38u); acc = mix(acc, global_a ^ global_b, 38u);
  observed = (uint32_t)(acc ^ UINT32_C(0xab30681d));
  { volatile uint32_t local_v = observed; local_v ^= (uint32_t)salt; observed = local_v; acc ^= observed; }
  acc ^= postdec_path(7u, salt);
  acc ^= wide_path(acc, salt);
  for (unsigned j = 0; j < 16; ++j) {
    unsigned k = (unsigned)((acc + j) & 15u);
    a[k] = (uint32_t)mix(a[k], acc ^ j, j);
    acc ^= ((uint64_t)a[k] << ((j & 7u) * 8u));
  }
  printf("%016" PRIx64 " %08" PRIx32 "\n", acc, observed);
  return 0;
}
