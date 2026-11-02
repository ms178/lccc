/* Differential-testing regression case, ported from John Regehr's
 * claudes-c-compiler "yarpgen" branch (CC0, https://github.com/regehr/claudes-c-compiler).
 * Reduced by yarpgen/csmith differential testing against gcc. The expected
 * output used to be "asserted via the GCC oracle", but GCC rejects the
 * multi-level braces around a scalar ("braces around scalar initializer";
 * C11 6.7.9p11 allows one optional level), so the runner could only ever
 * SKIP-COMPARE it. The case therefore checks itself: Clang 23.1.4, which
 * accepts the extension, prints 6 (f0 = 6, read back through f4). */

int printf(const char *, ...);
typedef signed char *P;
union U {
  long long f0;
  P f1;
  unsigned f4;
} g[] = {{{{6}}}};
int main(void) {
  printf("%u\n", g[0].f4);
  return g[0].f4 != 6;
}
