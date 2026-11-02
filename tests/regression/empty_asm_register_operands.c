/* An empty asm template still stages its REGISTER operands: "+d"(y) loads y
 * into %rdx (and stores it back), "S"/"D" inputs load %rsi/%rdi, a "0" tie
 * delivers its value.  Liveness may therefore stop treating an empty-template
 * asm as a call point only when every operand is memory or immediate
 * (inline_asm_is_register_inert).  Eight values are kept live across each
 * statement so that, with the call point dropped, one of them is homed in the
 * staged register -- the "+d"/"+S"/"+D"/inputs probes returned wrong sums when
 * it was.  Output must match GCC at every level and on both x86 targets. */
#include <stdio.h>
#define LIVE8 int a = p[0] * 3, b = p[1] * 5, c = p[2] * 7, d = p[3] * 9, \
                  e = p[4] * 11, f = p[5] * 13, g = p[6] * 15, h = p[7] * 17
#define SUM (a + 2 * b + 3 * c + 4 * d + 5 * e + 6 * f + 7 * g + 8 * h)
#define PROBE(name, cons) \
  __attribute__((noinline)) int name(const int *p, int y) { \
    LIVE8; __asm__ volatile("" : cons(y)); return SUM * 31 + y; }
PROBE(pc, "+c") PROBE(pd, "+d") PROBE(pS, "+S") PROBE(pD, "+D")
PROBE(pa, "+a") PROBE(pb, "+b") PROBE(pr, "+r")
__attribute__((noinline)) int ptie(const int *p, int y) {
  LIVE8; int z; __asm__ volatile("" : "=c"(z) : "0"(y)); return SUM * 31 + z; }
__attribute__((noinline)) int pin(const int *p, int y) {
  LIVE8; __asm__ volatile("" : : "c"(y), "d"(y + 1), "S"(y + 2), "D"(y + 3)); return SUM * 31 + y; }
__attribute__((noinline)) long ploop(const int *p, int n) {
  long s = 0; for (int i = 0; i < n; i++) { int a = p[i & 7] * 3, b = p[(i + 1) & 7];
    int y = i; __asm__ volatile("" : "+c"(y)); __asm__ volatile("" : "+d"(y));
    s += a * 7 + b * 5 + y; } return s; }
int main(int argc, char **argv) {
  (void)argv; int v[8]; for (int i = 0; i < 8; i++) v[i] = argc * (i + 2) + i * i;
  int (*fs[])(const int *, int) = {pc, pd, pS, pD, pa, pb, pr, ptie, pin};
  long acc = 0; for (unsigned k = 0; k < sizeof fs / sizeof *fs; k++) { int r = fs[k](v, argc + 40); printf("%u %d\n", k, r); acc += r; }
  printf("loop %ld\n", ploop(v, 1000 + argc)); return 0; }
