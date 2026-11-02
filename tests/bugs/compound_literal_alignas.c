/* LCCC vs GCC divergence: _Alignas on compound literals (ISO DR444; EDG
 * Changes 2019-10-22, EDGcpfe/21031,21904 — front end accepts compound
 * literals with _Alignas; the alignment must reach the emitted object).
 *
 * GCC aligns the compound literal to 32 bytes; lccc currently emits the
 * default int alignment.
 *
 * Expected (GCC):   1 3
 * Observed (lccc):  0 3
 */
#include <stdio.h>
int main(void) {
  int *q = (_Alignas(32) int[]){1, 2, 3};
  printf("%d %d\n", ((__UINTPTR_TYPE__)q % 32) == 0, q[2]);
  return 0;
}
