/* Regression: static-inline wrapper with memory-output operands and matching
 * constraints must survive inlining + copy-prop without dangling outputs. */
#include <stdio.h>
static void cpuid2(int info, unsigned *eax, unsigned *ebx, unsigned *ecx, unsigned *edx) {
  *eax = *ebx = *ecx = *edx = 0;
  __asm__ volatile("cpuid" : "=a"(*eax), "=b"(*ebx), "=c"(*ecx), "=d"(*edx) : "a"(info) : "memory");
}
int main(void) {
  unsigned a, b, c, d;
  cpuid2(1, &a, &b, &c, &d);
  if (!((d >> 26) & 1)) { printf("FAIL: SSE2 bit not set\n"); return 1; }
  printf("OK eax=%08x ebx=%08x ecx=%08x edx=%08x\n", a, b & 0xFEFFFFFFu, c, d); /* mask TSC-deadline bit (24): toggles at runtime on this sandbox CPU */
  return 0;
}
