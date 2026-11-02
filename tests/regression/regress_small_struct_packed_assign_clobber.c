/* Differential-testing regression case, ported from John Regehr's
 * claudes-c-compiler "yarpgen" branch (CC0, https://github.com/regehr/claudes-c-compiler).
 * Reduced by yarpgen/csmith differential testing against gcc. The expected
 * output is asserted by the lccc regression runner via the GCC oracle. */

int printf(const char *, ...);
struct __attribute__((packed)) S { unsigned char b[5]; } g = {{1, 2, 3, 4, 5}};
short marker = 4;
struct S h;
struct S *p = &g;
struct S ret(void) { return h; }
int main(void) {
  *p = ret();
  printf("checksum = %X\n", marker);
}
