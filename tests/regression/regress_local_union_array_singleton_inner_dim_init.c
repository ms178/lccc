/* Differential-testing regression case, ported from John Regehr's
 * claudes-c-compiler "yarpgen" branch (CC0, https://github.com/regehr/claudes-c-compiler).
 * Reduced by yarpgen/csmith differential testing against gcc. The expected
 * output is asserted by the lccc regression runner via the GCC oracle. */

int printf(const char *, ...);
struct S1 {
  int f0;
  int f1;
};
union U3 {
  struct S1 f0;
};
int main() {
  union U3 a[][1] = {{}, {}, {}, {}, {}, {}, {}, {}, {}, {{{0, 2}}}};
  printf("checksum = %X\n", (unsigned)a[9][0].f0.f1);
}
