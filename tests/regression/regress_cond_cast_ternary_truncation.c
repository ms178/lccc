/* Differential-testing regression case, ported from John Regehr's
 * claudes-c-compiler "yarpgen" branch (CC0, https://github.com/regehr/claudes-c-compiler).
 * Reduced by yarpgen/csmith differential testing against gcc. The expected
 * output is asserted by the lccc regression runner via the GCC oracle. */

int printf(const char *, ...);
unsigned long long seed;
char tf_4_array_6_2_3, tf_4_array_6_2_5_3;
int main() {
  if (8 ^ tf_4_array_6_2_5_3)
    if ((char)(tf_4_array_6_2_3 ? 512 : 512))
      printf("%llu\n", seed);
}
