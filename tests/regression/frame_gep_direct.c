/* Constant-offset addresses of an escaped local must be used as direct frame
 * operands, not reloaded from a spilled pointer copy in the hot loop. */
#include <stdio.h>
typedef struct { unsigned long state; const unsigned short *table; } fse_t;
typedef struct { unsigned long bits; unsigned consumed; fse_t s[8]; } seq_t;
__attribute__((noinline)) static void init(fse_t *s, unsigned long bits, const unsigned short *t) {
  s->state = bits & 63; s->table = t; }
static inline unsigned step(fse_t *s, unsigned long *bits) {
  unsigned v = s->table[s->state]; s->state = (v >> 4) & 63; *bits = (*bits >> 3) | ((unsigned long)v << 50); return v & 15; }
__attribute__((noinline)) static unsigned long decode(const unsigned short *t, unsigned long seed, int n) {
  seq_t q; q.bits = seed; q.consumed = 0;
  init(&q.s[0], seed, t); init(&q.s[1], seed >> 6, t); init(&q.s[2], seed >> 12, t); init(&q.s[3], seed >> 18, t);
  init(&q.s[4], seed >> 24, t); init(&q.s[5], seed >> 30, t); init(&q.s[6], seed >> 36, t); init(&q.s[7], seed >> 42, t);
  unsigned long acc = 0;
  for (int i = 0; i < n; i++) {
    unsigned a = step(&q.s[0], &q.bits), b = step(&q.s[1], &q.bits), c = step(&q.s[2], &q.bits), d = step(&q.s[3], &q.bits);
    unsigned e = step(&q.s[4], &q.bits), f = step(&q.s[5], &q.bits), g = step(&q.s[6], &q.bits), h = step(&q.s[7], &q.bits);
    acc = acc * 31 + a * 7 + b * 3 + c + d * 5 + e * 11 + f * 13 + g * 17 + h * 19 + (q.bits & 255);
    q.consumed += a + b + c + d + e + f + g + h; }
  unsigned long r = acc ^ q.consumed;
  for (int k = 0; k < 8; k++) r ^= q.s[k].state << (k * 6);
  return r;
}
int main(void) {
  static unsigned short t[64];
  for (int i = 0; i < 64; i++) t[i] = (unsigned short)(i * 2654435761u >> 7);
  printf("%lu\n", decode(t, 0x123456789abcdefUL, 100000));
  return 0;
}
