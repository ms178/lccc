/* Differential-testing regression case, ported from John Regehr's
 * claudes-c-compiler "yarpgen" branch (CC0, https://github.com/regehr/claudes-c-compiler).
 * Reduced by yarpgen/csmith differential testing against gcc. The expected
 * output is asserted by the lccc regression runner via the GCC oracle. */

int printf(const char *, ...);
struct {
  unsigned f0;
} g_2034[3] = {{}, {}, 4, 6};
int main() { printf("checksum = %X\n", g_2034[2].f0); }
