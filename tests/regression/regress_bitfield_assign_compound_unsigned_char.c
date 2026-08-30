/* Differential-testing regression case, ported from John Regehr's
 * claudes-c-compiler "yarpgen" branch (CC0, https://github.com/regehr/claudes-c-compiler).
 * Reduced by yarpgen/csmith differential testing against gcc. The expected
 * output is asserted by the lccc regression runner via the GCC oracle. */

int printf(const char *, ...);
struct { signed f : 28; } g;
int main(void) {
  unsigned char x = 238;
  g.f = (x |= 0);
  printf("%d %u\n", g.f, (unsigned)x);
}
