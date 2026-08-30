/* Differential-testing regression case, ported from John Regehr's
 * claudes-c-compiler "yarpgen" branch (CC0, https://github.com/regehr/claudes-c-compiler).
 * Reduced by yarpgen/csmith differential testing against gcc. The expected
 * output is asserted by the lccc regression runner via the GCC oracle. */

int printf(const char *, ...);
unsigned long long seed;
unsigned tf_3_var_98 = 3357492005;
int tf_3_array_4_2_0;
void hash(long long *seed, int v) { *seed ^= v; }
int main() {
  tf_3_array_4_2_0 = tf_3_var_98 / (long)3;
  hash(&seed, tf_3_array_4_2_0);
  printf("%llu\n", seed);
}
