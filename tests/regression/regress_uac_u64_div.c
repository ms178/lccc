/* Differential-testing regression case, ported from John Regehr's
 * claudes-c-compiler "yarpgen" branch (CC0, https://github.com/regehr/claudes-c-compiler).
 * Reduced by yarpgen/csmith differential testing against gcc. The expected
 * output is asserted by the lccc regression runner via the GCC oracle. */

int printf(const char *, ...);
unsigned long long seed;
int tf_0_var_84, tf_0_var_120;
int *tf_0_ptr_1;
void hash(long long *seed, int v) { *seed ^= v; }
int main() {
  tf_0_ptr_1 = &tf_0_var_120;
  *tf_0_ptr_1 = ~(tf_0_var_84 + 0) / (unsigned long)(int)18142741702065582329;
  hash(&seed, tf_0_var_120);
  printf("%llu\n", seed);
}
