/* Differential-testing regression case, ported from John Regehr's
 * claudes-c-compiler "yarpgen" branch (CC0, https://github.com/regehr/claudes-c-compiler).
 * Reduced by yarpgen/csmith differential testing against gcc. The expected
 * output is asserted by the lccc regression runner via the GCC oracle. */

int printf(const char *, ...);
int g, t;
long f(short p, int *q) {
  ((*q = 10) || 60) & 0;
  if (*q)
    return p;
  return t;
}
int main() {
  short out = f(1, &g);
  printf("%d\n", out);
}
