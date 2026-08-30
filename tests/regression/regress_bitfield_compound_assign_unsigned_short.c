/* Differential-testing regression case, ported from John Regehr's
 * claudes-c-compiler "yarpgen" branch (CC0, https://github.com/regehr/claudes-c-compiler).
 * Reduced by yarpgen/csmith differential testing against gcc. The expected
 * output is asserted by the lccc regression runner via the GCC oracle. */

int printf(const char *, ...);
struct { signed f : 18; } g;
char c = -97;
int main(void) {
  unsigned short x = c;
  g.f ^= x;
  printf("%d\n", g.f);
}
