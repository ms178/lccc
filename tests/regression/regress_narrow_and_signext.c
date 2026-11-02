/* Differential-testing regression case, ported from John Regehr's
 * claudes-c-compiler "yarpgen" branch (CC0, https://github.com/regehr/claudes-c-compiler).
 * Reduced by yarpgen/csmith differential testing against gcc. The expected
 * output is asserted by the lccc regression runner via the GCC oracle. */

int printf(const char *, ...);
unsigned u = 41136287;
int main(void) {
  int a = u << 10;
  long b = a & -4;
  printf("%llu\n", (unsigned long long)b);
}
