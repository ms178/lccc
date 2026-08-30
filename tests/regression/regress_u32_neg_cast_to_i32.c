/* Differential-testing regression case, ported from John Regehr's
 * claudes-c-compiler "yarpgen" branch (CC0, https://github.com/regehr/claudes-c-compiler).
 * Reduced by yarpgen/csmith differential testing against gcc. The expected
 * output is asserted by the lccc regression runner via the GCC oracle. */

int printf(const char *, ...);
unsigned long long seed;
int tf_2_var_94, tf_2_var_134;
unsigned tf_2_struct_obj_2_1;
void hash(long long *seed, int v) { *seed ^= v; }
int main() {
  tf_2_struct_obj_2_1 = 9035291;
  tf_2_var_134 = 0 > ((int)-tf_2_struct_obj_2_1 | (long)tf_2_var_94);
  hash(&seed, tf_2_var_134);
  printf("%llu\n", seed);
}
