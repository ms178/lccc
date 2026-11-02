/* Regression: narrow (32-bit) constant stores into VLA-frame slots must
 * store exactly their operand width.
 *
 * Root cause (BUG-2026-09-03-O2-vla-store-miscompile.md, fixed by the
 * wide-imm memory-relay operand-size hardening): a 32-bit constant outside
 * the signed imm32 window was staged with `movabsq $imm, %rax` and then
 * stored with the relay's DEFAULT S64 width (`movq`), writing 4 bytes past
 * the slot.  In the reproducing shape the offending store was the last
 * element of a 16-byte VLA (b[3] at b+12), so the extra 4 bytes landed on
 * the first element of the neighbouring VLA (a[0]) — zeroing its low half.
 * The checksum chain (g = g*33 + ...) amplified the lost 32 bits by 33^3,
 * which is exactly the delta the bug report measured.
 *
 * This family pins the class at every VLA-tail boundary and width mix:
 * the last element of each narrow array sits directly below the next
 * allocation, so any over-wide store is observable in the checksums.
 */
#include <stdio.h>
#include <stdint.h>

__attribute__((noinline)) void barrier(void) {}
unsigned long long g;

int main(void) {
  unsigned n = 4;
  unsigned long long a[n];
  unsigned int b[n];
  unsigned short c[n];
  /* Immides: each is outside the signed imm32 window where relevant and
   * distinctive per lane, so any over-wide store shows up in g. */
  for (unsigned i = 0; i < n; i++) {
    a[i] = 5ull * 0x9e3779b97f4a7c15ULL + 7ULL + i;   /* 64-bit wide const  */
    b[i] = 6u * 2654435761u + i;                      /* >2^31-1: 3041712678+i */
    c[i] = (unsigned short)(0xB504 + i);              /* narrow const        */
  }
  barrier();
  for (unsigned i = 0; i < n; i++) {
    g = g * 33 + (a[i] + b[i] + (unsigned long long)c[i]);
  }
  printf("%llu\n", g);
  return 0;
}
