/* Differential-testing regression case, ported from John Regehr's
 * claudes-c-compiler "yarpgen" branch (CC0, https://github.com/regehr/claudes-c-compiler).
 * Reduced by yarpgen/csmith differential testing against gcc. The expected
 * output is asserted by the lccc regression runner via the GCC oracle. */

int printf(const char *, ...);
unsigned long long seed;
int var_35 = 80976248578483579;
void hash(long long *seed, int v) { *seed ^= v; }
int main(void) {
  var_35 &= ({
    __typeof__(0) _a = ~0;
    __typeof__(({
      __typeof__(4ULL) _a;
      _a < _a;
    })) _b = 0;
    _a < _b;
  });
  hash(&seed, var_35);
  printf("%llu\n", seed);
  return 0;
}
