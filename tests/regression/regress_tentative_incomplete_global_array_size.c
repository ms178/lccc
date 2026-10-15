/* Differential-testing regression case, ported from John Regehr's
 * claudes-c-compiler "yarpgen" branch (CC0, https://github.com/regehr/claudes-c-compiler).
 * Reduced by yarpgen/csmith differential testing against gcc. The expected
 * output is asserted by the lccc regression runner via the GCC oracle. */

int printf(const char *, ...);
int g[];
/* sizeof requires a complete type at the point of use, not just the C
 * end-of-translation-unit completion of a tentative definition. Preserve
 * the initially incomplete declaration and give this size oracle a valid
 * compatible completion before main. */
int g[1];
int main() {
  unsigned crc = sizeof(g);
  printf("checksum = %X\n", crc);
}
