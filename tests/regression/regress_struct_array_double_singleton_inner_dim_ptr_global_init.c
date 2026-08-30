/* Differential-testing regression case, ported from John Regehr's
 * claudes-c-compiler "yarpgen" branch (CC0, https://github.com/regehr/claudes-c-compiler).
 * Reduced by yarpgen/csmith differential testing against gcc. The expected
 * output is asserted by the lccc regression runner via the GCC oracle. */

int printf(const char *, ...);
struct {
  unsigned char f0;
  char *f1;
} g_284[][1][1] = {{{{4}}}};
int main() {
  printf("checksum = %X\n", (unsigned)g_284[0][0][0].f0);
}
