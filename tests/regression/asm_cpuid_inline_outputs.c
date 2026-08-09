/* Regression: inline-asm output store-back corrupted RSP-relative addressing
 * after a scratch pushq (wrong slot), and copy-prop left asm outputs dangling.
 * Previously: zeros at -O1+ and SIGSEGV at -O0 with __cpuid-style wrappers. */
#include <cpuid.h>
#include <stdio.h>
int main(void) {
  unsigned a = 0, b = 0, c = 0, d = 0;
  __cpuid(1, a, b, c, d);           /* cpu features leaf */
  /* On any x86-64 the vendor string leaf (0) must be non-zero in ebx/ecx/edx,
   * and leaf 1 must set edx bit 26 (SSE2 is mandatory on x86-64). */
  unsigned v0a, v0b, v0c, v0d;
  __cpuid(0, v0a, v0b, v0c, v0d);
  if ((v0b | v0c | v0d) == 0) { printf("FAIL: leaf0 vendor zero\n"); return 1; }
  if (!((d >> 26) & 1)) { printf("FAIL: SSE2 bit not set\n"); return 1; }
  printf("OK eax=%08x ebx=%08x ecx=%08x edx=%08x\n", a, b & 0xFEFFFFFFu, c, d); /* mask TSC-deadline bit (24): toggles at runtime on this sandbox CPU */
  return 0;
}
