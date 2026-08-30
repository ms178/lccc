/* Differential-testing regression case, ported from John Regehr's
 * claudes-c-compiler "yarpgen" branch (CC0, https://github.com/regehr/claudes-c-compiler).
 * Reduced by yarpgen/csmith differential testing against gcc. The expected
 * output is asserted by the lccc regression runner via the GCC oracle. */

int printf(const char *, ...);
struct S0 {
  unsigned f0;
  signed f1;
} g_91, *g_1734 = &g_91;
int main() {
  struct S0 l_3197[][1][1] = {{{{8, 7}}}};
  *g_1734 = l_3197[0][0][0];
  unsigned crc = g_91.f1;
  printf("checksum = %X\n", crc);
}
