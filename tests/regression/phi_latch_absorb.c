/* Regression: self-loop latch absorption in phi elimination.
 *
 * A block that is its own phi target is always a "critical edge" predecessor,
 * so the old code unconditionally split the backedge into a copy-only latch
 * block, adding one unconditional branch per iteration. Absorption appends the
 * phi copies to the block instead when every destination is invisible outside
 * it and each copy satisfies the ordinary non-interference rule.
 *
 * These loops deliberately span both sides of that decision:
 *   - bottom_tested / accumulate: next-iteration values are distinct and
 *     computed last, so absorption applies;
 *   - escapes_on_exit: the accumulator is live out through the exit phi, so the
 *     latch MUST still be split. Getting this wrong silently changes the
 *     returned sum; it is the shape that regressed ra09_selfop_xor.
 *
 * Differential vs GCC. Deterministic.
 */
#include <stdint.h>
#include <stdio.h>

static volatile uint32_t sink32;

__attribute__((noinline)) static uint32_t bottom_tested(uint32_t n) {
  uint32_t s = 0;
  for (uint32_t i = 0; i < n; ++i) s += i * 3u;
  return s;
}

__attribute__((noinline)) static uint64_t accumulate(uint32_t n, uint64_t seed) {
  uint64_t s = seed;
  uint32_t i = 0;
  do {
    s = (s ^ (i + 1u)) * UINT64_C(0x9e3779b97f4a7c15);
    s ^= s >> 29;
    ++i;
  } while (i < n);
  return s;
}

/* The accumulator leaves the loop live, so its phi destination is read by the
 * exit block and the backedge may not be absorbed. */
__attribute__((noinline)) static uint32_t escapes_on_exit(uint32_t n) {
  uint32_t a = 1u, b = 2u;
  while (n-- != 0) {
    uint32_t t = a + b;
    a = b;
    b = t;
  }
  return a * 3u + b;
}

__attribute__((noinline)) static uint32_t two_state(uint32_t n) {
  uint32_t x = 3u, y = 5u;
  for (uint32_t i = 0; i < n; ++i) {
    uint32_t nx = y ^ (x << 1);
    uint32_t ny = x + y + i;
    x = nx;
    y = ny;
  }
  return x ^ y;
}

int main(void) {
  uint64_t acc = 0;
  for (uint32_t k = 0; k < 8; ++k) {
    acc += bottom_tested(k * 37u + 1u);
    acc += accumulate(k + 3u, 0xabcdef01u + k);
    acc += escapes_on_exit(k * 5u);
    acc += two_state(k * 11u + 1u);
  }
  sink32 = (uint32_t)acc;
  printf("%016llx %08x\n", (unsigned long long)acc, sink32);
  return 0;
}
