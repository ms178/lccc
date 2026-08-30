/* Differential-testing regression case, ported from John Regehr's
 * claudes-c-compiler "yarpgen" branch (CC0, https://github.com/regehr/claudes-c-compiler).
 * Reduced by yarpgen/csmith differential testing against gcc. The expected
 * output is asserted by the lccc regression runner via the GCC oracle. */

int printf(const char *, ...);
typedef signed char *P;
union U {
  long long f0;
  P f1;
  unsigned f4;
} g[] = {{{{6}}}};
int main(void) {
  printf("%u\n", g[0].f4);
}
