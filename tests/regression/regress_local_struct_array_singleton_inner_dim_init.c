/* Differential-testing regression case, ported from John Regehr's
 * claudes-c-compiler "yarpgen" branch (CC0, https://github.com/regehr/claudes-c-compiler).
 * Reduced by yarpgen/csmith differential testing against gcc. The expected
 * output is asserted by the lccc regression runner via the GCC oracle. */

int printf(const char *, ...);
struct S1 {
  signed f1;
  signed f2;
};
char g_2;
int main() {
  struct S1 l_17[][1] = {{}, {}, {}, {}, {}, {}, {}, {{9, 4}}};
  l_17[7][0].f2 && printf("checksum = %X\n", g_2);
}
