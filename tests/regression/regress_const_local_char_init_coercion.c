/* Differential-testing regression case, ported from John Regehr's
 * claudes-c-compiler "yarpgen" branch (CC0, https://github.com/regehr/claudes-c-compiler).
 * Reduced by yarpgen/csmith differential testing against gcc. The expected
 * output is asserted by the lccc regression runner via the GCC oracle. */

int printf(const char *, ...);
int g_27;

int main(void) {
  const char l_17 = 220;
  l_17 < 0 || (g_27 = 9);
  printf("%u\n", (unsigned)g_27);
}
