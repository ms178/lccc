/* Differential-testing regression case, ported from John Regehr's
 * claudes-c-compiler "yarpgen" branch (CC0, https://github.com/regehr/claudes-c-compiler).
 * Reduced by yarpgen/csmith differential testing against gcc. The expected
 * output is asserted by the lccc regression runner via the GCC oracle. */

int printf(const char *, ...);
int g_130_1, func_39_i, main_l_42 = 7;
int *g_911 = &g_130_1;
long g_1414_4 = 8;
char func_39_p_40;
void func_39(int *p_41) {
  unsigned l_1416[5];
  for (; func_39_i < 5; func_39_i++)
    l_1416[func_39_i] = *g_911 = *p_41;
  func_39_p_40 = g_1414_4 < l_1416[2];
  *g_911 &= func_39_p_40;
}
int main() {
  func_39(&main_l_42);
  unsigned crc = g_130_1;
  printf("checksum = %X\n", crc);
}
