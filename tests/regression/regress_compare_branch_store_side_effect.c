/* Differential-testing regression case, ported from John Regehr's
 * claudes-c-compiler "yarpgen" branch (CC0, https://github.com/regehr/claudes-c-compiler).
 * Reduced by yarpgen/csmith differential testing against gcc. The expected
 * output is asserted by the lccc regression runner via the GCC oracle. */

int printf(const char *, ...);

char g_c;
int g_out;
int *g_ptr;

void f(char *p, int x) {
  for (;;) {
    x = *p <= 1;
    if (x)
      break;
  }
  g_ptr = &x;
  if (x)
    g_out = 7;
}

int main(void) {
  f(&g_c, 0);
  printf("%d\n", g_out);
  return 0;
}
