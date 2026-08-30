/* Differential-testing regression case, ported from John Regehr's
 * claudes-c-compiler "yarpgen" branch (CC0, https://github.com/regehr/claudes-c-compiler).
 * Reduced by yarpgen/csmith differential testing against gcc. The expected
 * output is asserted by the lccc regression runner via the GCC oracle. */

int printf(const char *, ...);
struct {
  short f2;
  int f3;
} g_1180[][1] = {{{0, 1}}};
int main() {
  unsigned crc = g_1180[0][0].f3;
  printf("checksum = %X\n", crc);
}
