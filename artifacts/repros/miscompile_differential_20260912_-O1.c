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
  uint64_t acc = UINT64_C(0xaef9c97a0fe8f576);
  uint64_t salt = UINT64_C(0x36fe6b9d6e6d7582);
  uint32_t a[16];
  struct pair p = { UINT32_C(1), UINT32_C(2) };
  struct bits bf = { 0, 0, 0 };
  for (unsigned i = 0; i < 16; ++i) a[i] = (uint32_t)(acc >> (i & 31u)) ^ (i * UINT32_C(0x45d9f3b));
  acc = mix(acc + UINT64_C(0xf6eca5f99f23daa7), salt ^ UINT64_C(0x92c33820b360f560), 10u);
  a[1] ^= (uint32_t)(acc >> 8u); acc += a[1];
  if ((acc ^ UINT64_C(0xb0edbe70cf3badd6)) & UINT64_C(1)) acc ^= rotl64(salt, 5u); else acc += UINT64_C(0x896e8da7fd61a658);
  p = step_pair(p, (uint32_t)(acc + UINT32_C(0xfd79dbce))); acc ^= ((uint64_t)p.hi << 32) | p.lo;
  p = step_pair(p, (uint32_t)(acc + UINT32_C(0x1152aede))); acc ^= ((uint64_t)p.hi << 32) | p.lo;
  a[4] ^= (uint32_t)(acc >> 13u); acc += a[4];
  a[14] ^= (uint32_t)(acc >> 13u); acc += a[14];
  if ((acc ^ UINT64_C(0x0db26c5050d7287e)) & UINT64_C(1)) acc ^= rotl64(salt, 54u); else acc += UINT64_C(0x5389943609c8779e);
  p = step_pair(p, (uint32_t)(acc + UINT32_C(0x11d365ae))); acc ^= ((uint64_t)p.hi << 32) | p.lo;
  a[2] ^= (uint32_t)(acc >> 24u); acc += a[2];
  a[6] ^= (uint32_t)(acc >> 53u); acc += a[6];
  observed = (uint32_t)(acc ^ UINT32_C(0x0b1fc704));
  { volatile uint32_t local_v = observed; local_v ^= (uint32_t)salt; observed = local_v; acc ^= observed; }
  acc ^= postdec_path(8u, salt);
  acc ^= wide_path(acc, salt);
  for (unsigned j = 0; j < 16; ++j) {
    unsigned k = (unsigned)((acc + j) & 15u);
    a[k] = (uint32_t)mix(a[k], acc ^ j, j);
    acc ^= ((uint64_t)a[k] << ((j & 7u) * 8u));
  }
  printf("%016" PRIx64 " %08" PRIx32 "\n", acc, observed);
  return 0;
}
