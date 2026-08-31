/* Regression (RA-09): register self-op miscompile in emit_alu_reg_direct.
 *
 * Pattern: a memory load whose result is consumed by a commutative ALU
 * (Xor) whose dest AND rhs both get the SAME callee-saved register, while
 * the load is materialized through the accumulator (%rax). Under O3/Os the
 * HOT_LOOP phase-1 allocator homed the shift result and the xor dest on the
 * same register; the old emitter then ran the "save lhs to dest, re-read rhs"
 * dance which clobbered rhs with lhs and emitted a self-op:
 *
 *     movl 168(%rsp), %ebx     ; load a[14]  (materialized via dest reg)
 *     xorl %ebx, %ebx          ; a[14] ^= a[14]  -- WRONG, was ^= acc>>31
 *
 * Emitter fix: when lhs is already in %rax and rhs's home IS the dest
 * register, leave lhs in %rax and let the 2-operand tail emit
 * `op %rax, %dest` (= rhs op lhs == lhs op rhs for commutative ALU).
 *
 * Differential vs GCC. Deterministic: reproduces on -O0/-O2/-O3.
 */
#include <stdint.h>
#include <stdio.h>
#include <inttypes.h>

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
  uint64_t acc = UINT64_C(0xdcf4bb99f4bea973);
  uint64_t salt = UINT64_C(0xd95bafc8f2a4d27b);
  uint32_t a[16];
  struct pair p = { UINT32_C(1), UINT32_C(2) };
  struct bits bf = { 0, 0, 0 };
  for (unsigned i = 0; i < 16; ++i) a[i] = (uint32_t)(acc >> (i & 31u)) ^ (i * UINT32_C(0x45d9f3b));
  bf.a = (unsigned)(acc >> 53u); bf.b = (unsigned)(salt >> 60u); bf.c = bf.a + bf.b;
  acc += (uint64_t)(bf.a | (bf.b << 3) | (bf.c << 8));
  a[14] ^= (uint32_t)(acc >> 31u); acc += a[14];
  p = step_pair(p, (uint32_t)(acc + UINT32_C(0x47733e84))); acc ^= ((uint64_t)p.hi << 32) | p.lo;
  acc ^= postdec_path(7u, salt);
  acc ^= wide_path(acc, salt);
  for (unsigned j = 0; j < 16; ++j) {
    unsigned k = (unsigned)((acc + j) & 15u);
    a[k] = (uint32_t)mix(a[k], acc ^ j, j);
    acc ^= ((uint64_t)a[k] << ((j & 7u) * 8u));
  }
  printf("%016" PRIx64 " %08" PRIx32 "\n", acc, observed);
}
