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
  uint64_t acc = UINT64_C(0xb04e24f389c6f464);
  uint64_t salt = UINT64_C(0xa5ba440e6bf97592);
  uint32_t a[16];
  struct pair p = { UINT32_C(1), UINT32_C(2) };
  struct bits bf = { 0, 0, 0 };
  for (unsigned i = 0; i < 16; ++i) a[i] = (uint32_t)(acc >> (i & 31u)) ^ (i * UINT32_C(0x45d9f3b));
  { int32_t sx = 15661; int32_t sy = 7824; int32_t z = sx * 3 + sy * 5; acc ^= (uint64_t)(int64_t)z; }
  p = step_pair(p, (uint32_t)(acc + UINT32_C(0xa2fbcffe))); acc ^= ((uint64_t)p.hi << 32) | p.lo;
  if ((acc ^ UINT64_C(0x9c24110b529375de)) & UINT64_C(1)) acc ^= rotl64(salt, 11u); else acc += UINT64_C(0xee44c31a4a02d260);
  bf.a = (unsigned)(acc >> 10u); bf.b = (unsigned)(salt >> 17u); bf.c = bf.a + bf.b; acc += (uint64_t)(bf.a | (bf.b << 3) | (bf.c << 8));
  a[4] ^= (uint32_t)(acc >> 11u); acc += a[4];
  p = step_pair(p, (uint32_t)(acc + UINT32_C(0x7825468a))); acc ^= ((uint64_t)p.hi << 32) | p.lo;
  p = step_pair(p, (uint32_t)(acc + UINT32_C(0xd7494a1b))); acc ^= ((uint64_t)p.hi << 32) | p.lo;
  a[1] ^= (uint32_t)(acc >> 46u); acc += a[1];
  if ((acc ^ UINT64_C(0x61066af3bd4ec6ef)) & UINT64_C(1)) acc ^= rotl64(salt, 55u); else acc += UINT64_C(0x808eaad24641035f);
  acc = mix(acc + UINT64_C(0xae0d5eebf370021b), salt ^ UINT64_C(0xeb4efe23cadab0c3), 15u);
  observed = (uint32_t)(acc ^ UINT32_C(0x7c7d0ef8));
  { volatile uint32_t local_v = observed; local_v ^= (uint32_t)salt; observed = local_v; acc ^= observed; }
  acc ^= postdec_path(6u, salt);
  acc ^= wide_path(acc, salt);
  for (unsigned j = 0; j < 16; ++j) {
    unsigned k = (unsigned)((acc + j) & 15u);
    a[k] = (uint32_t)mix(a[k], acc ^ j, j);
    acc ^= ((uint64_t)a[k] << ((j & 7u) * 8u));
  }
  printf("%016" PRIx64 " %08" PRIx32 "\n", acc, observed);
  return 0;
}
