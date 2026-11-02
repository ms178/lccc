/* Differential-testing regression case, ported from John Regehr's
 * claudes-c-compiler "yarpgen" branch (CC0, https://github.com/regehr/claudes-c-compiler).
 * Reduced by yarpgen/csmith differential testing against gcc. The expected
 * output is asserted by the lccc regression runner via the GCC oracle. */

int printf(const char *, ...);
struct {
  int;
  char f4;
} g_1521[] = {3};
int main() { printf("checksum = %X\n", g_1521[0].f4); }
