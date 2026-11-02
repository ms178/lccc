/* Gate: the x86 compare-replay must compare in the operands' own homes.
 *
 * When a Cmp's single use is the block's CondBranch, the emitter skips the Cmp
 * and re-emits it at the branch. It used to stage both operands through
 * %rax/%rcx first, which is one move per operand per iteration -- and a later
 * peephole rewrote the staged compare back onto the operand homes while
 * leaving the %rax load behind as dead code inside the loop.
 *
 * The operand homes are valid at the branch: operand_links extends a
 * register-homed operand's interval to the consumer (prologue.rs IS-09) and the
 * redefinition guard in compute_cmp_replay_scan rejects any Cmp whose operand
 * an intervening instruction rewrites.
 *
 * Differential vs GCC. Deterministic.
 */
#include <stdint.h>
#include <stdio.h>

__attribute__((noinline)) static uint32_t counted(uint32_t n) {
  uint32_t s = 0;
  for (uint32_t i = 0; i < n; ++i) s += i * 3u;
  return s;
}

__attribute__((noinline)) static uint64_t strided(const uint32_t *a, uint32_t n) {
  uint64_t s = 0;
  for (uint32_t i = 0; i < n; ++i) s += a[i] * (i + 1u);
  return s;
}

int main(void) {
  uint32_t a[64];
  for (uint32_t i = 0; i < 64; ++i) a[i] = i * 2654435761u;
  uint64_t acc = 0;
  for (uint32_t k = 0; k < 8; ++k) {
    acc += counted(k * 97u + 1u);
    acc += strided(a, k * 7u + 1u);
  }
  printf("%016llx\n", (unsigned long long)acc);
  return 0;
}
