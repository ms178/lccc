/* Differential-testing regression case, ported from John Regehr's
 * claudes-c-compiler "yarpgen" branch (CC0, https://github.com/regehr/claudes-c-compiler).
 * Reduced by yarpgen/csmith differential testing against gcc. The expected
 * output is asserted by the lccc regression runner via the GCC oracle. */

int printf(const char *, ...);
int side;
int rhs(void) {
  side = 1;
  return 1;
}
int main(void) {
  const unsigned u = 0xFFFFFFFFu;
  ~u && rhs();
  printf("%d\n", side);
}
