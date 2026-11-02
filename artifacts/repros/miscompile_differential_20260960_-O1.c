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
  uint64_t acc = UINT64_C(0x9b1ca4bb8ba90418);
  uint64_t salt = UINT64_C(0x41915f14909e0e04);
  uint32_t a[16];
  struct pair p = { UINT32_C(1), UINT32_C(2) };
  struct bits bf = { 0, 0, 0 };
  for (unsigned i = 0; i < 16; ++i) a[i] = (uint32_t)(acc >> (i & 31u)) ^ (i * UINT32_C(0x45d9f3b));
  a[5] ^= (uint32_t)(acc >> 13u); acc += a[5];
  p = step_pair(p, (uint32_t)(acc + UINT32_C(0xdaa0c744))); acc ^= ((uint64_t)p.hi << 32) | p.lo;
  { int32_t sx = 14513; int32_t sy = -864; int32_t z = sx * 3 + sy * 5; acc ^= (uint64_t)(int64_t)z; }
  if ((acc ^ UINT64_C(0x22175d81b5d7799a)) & UINT64_C(1)) acc ^= rotl64(salt, 60u); else acc += UINT64_C(0xadb99f283515569a);
  acc = mix(acc + UINT64_C(0xe43e21b51518440a), salt ^ UINT64_C(0x2fa393d7041482b9), 8u);
  { int32_t sx = 4948; int32_t sy = 9875; int32_t z = sx * 3 + sy * 5; acc ^= (uint64_t)(int64_t)z; }
  a[3] ^= (uint32_t)(acc >> 13u); acc += a[3];
  p = step_pair(p, (uint32_t)(acc + UINT32_C(0x8a9e5aa3))); acc ^= ((uint64_t)p.hi << 32) | p.lo;
  if ((acc ^ UINT64_C(0x50d880063ee74660)) & UINT64_C(1)) acc ^= rotl64(salt, 20u); else acc += UINT64_C(0x772598dc4e62cbf3);
  global_a ^= acc + UINT64_C(0x3413fe7a6d2f20a7); global_b += (uint32_t)(salt >> 10u); acc = mix(acc, global_a ^ global_b, 10u);
  if ((acc ^ UINT64_C(0xf40cfca85a92ed85)) & UINT64_C(1)) acc ^= rotl64(salt, 16u); else acc += UINT64_C(0xb976f48c083d358d);
  observed = (uint32_t)(acc ^ UINT32_C(0x26f6a511));
  { volatile uint32_t local_v = observed; local_v ^= (uint32_t)salt; observed = local_v; acc ^= observed; }
  acc ^= postdec_path(0u, salt);
  acc ^= wide_path(acc, salt);
  for (unsigned j = 0; j < 16; ++j) {
    unsigned k = (unsigned)((acc + j) & 15u);
    a[k] = (uint32_t)mix(a[k], acc ^ j, j);
    acc ^= ((uint64_t)a[k] << ((j & 7u) * 8u));
  }
  printf("%016" PRIx64 " %08" PRIx32 "\n", acc, observed);
  return 0;
}
