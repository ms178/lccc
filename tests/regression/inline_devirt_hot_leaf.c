/* Inliner contracts (zstd match-finder/encoder shapes):
 *  1. a constant-folded function pointer selected by a template parameter is
 *     a direct call, and its small static target executing on every loop
 *     iteration is inlined;
 *  2. the bit-writer loop exercises static-inline helpers in a large caller;
 *     its output must match GCC whichever helpers the size policy inlines;
 *  3. a devirtualized plain-static scorer above the ordinary static limit,
 *     called on every iteration of a loop in a caller that is already past
 *     the caller-size cap, is still inlined (the hot-leaf budget);
 *  4. the same shape through a DIRECT call: a declared-inline scorer is
 *     inlined (hot-leaf budget, direct extension), while a plain-static one
 *     keeps the historical caps and stays a call;
 *  5. a declared-inline helper with its OWN loop (zstd's ZSTD_count: a match
 *     length scan) called inside a loop of a capped caller, on the match path
 *     only, is inlined (hot loop helper); its plain-static twin stays a call. */
#include <stdio.h>
#include <string.h>

typedef int (*pred_t)(const unsigned char *, unsigned);
static int pred_a(const unsigned char *p, unsigned n) { return p[n] == 'a'; }
static int pred_b(const unsigned char *p, unsigned n) { return p[n] == 'b'; }
static inline __attribute__((always_inline)) unsigned
count_gen(const unsigned char *p, unsigned n, int use_a) {
  const pred_t f = use_a ? pred_a : pred_b;
  unsigned c = 0;
  for (unsigned i = 0; i < n; i++) c += (unsigned)f(p, i);
  return c;
}
__attribute__((noinline)) unsigned count_a(const unsigned char *p, unsigned n) { return count_gen(p, n, 1); }
__attribute__((noinline)) unsigned count_b(const unsigned char *p, unsigned n) { return count_gen(p, n, 0); }

typedef struct { unsigned long long c; unsigned n; unsigned char *p, *end; } bits_t;
static __inline void flush(bits_t *s) {
  unsigned nb = s->n >> 3;
  memcpy(s->p, &s->c, 8);
  s->p += nb; s->n &= 7; s->c = nb == 8 ? 0 : s->c >> (nb * 8);
  if (s->p > s->end) s->p = s->end;
}
static __inline void add(bits_t *s, unsigned long long v, unsigned k) {
  s->c |= (v & ((1ull << k) - 1)) << s->n; s->n += k;
}
#define B(x) add(s, (x), 3); add(s, (x) >> 3, 5); flush(s);
__attribute__((noinline)) void encode(bits_t *s, const unsigned *in, int n) {
  for (int i = 0; i < n; i++) {
    unsigned x = in[i];
    B(x) B(x + 1) B(x + 2) B(x + 3) B(x + 4) B(x + 5) B(x * 3) B(x * 5) B(x ^ 7) B(x ^ 9) B(x + 11) B(x + 13)
  }
}

typedef unsigned (*score_t)(const unsigned char *, unsigned);
static unsigned score_a(const unsigned char *p, unsigned n) {
  unsigned x = p[n] * 2654435761u, y = p[n + 1] * 40503u, z = p[n + 2] ^ 0x9e37u;
  x ^= x >> 13; y += x * 7; z ^= y >> 5; x += z << 3; y ^= x >> 11; z += y * 31;
  return (x ^ y ^ z) & 7;
}
static unsigned score_b(const unsigned char *p, unsigned n) {
  unsigned x = p[n] * 40503u, y = p[n + 1] * 2654435761u, z = p[n + 2] ^ 0x7f4au;
  x ^= x >> 11; y += x * 5; z ^= y >> 3; x += z << 5; y ^= x >> 13; z += y * 29;
  return (x ^ y ^ z) & 7;
}
static inline __attribute__((always_inline)) unsigned
sum_gen(const unsigned char *p, unsigned n, int use_a) {
  const score_t f = use_a ? score_a : score_b;
  unsigned c = 0;
  for (unsigned i = 0; i + 2 < n; i++) c += f(p, i);
  return c;
}
/* Straight-line work that puts the callers past the caller-size cap. */
unsigned g_mix[64];
#define MIX(k) g_mix[(k)] = g_mix[(k) + 1] * 33u + (unsigned)p[(k) * 3] + n;
#define MIX8(k) MIX(k) MIX(k + 1) MIX(k + 2) MIX(k + 3) MIX(k + 4) MIX(k + 5) MIX(k + 6) MIX(k + 7)
#define PREAMBLE MIX8(0) MIX8(8) MIX8(16) MIX8(24) MIX8(32) MIX8(40) MIX8(48)
__attribute__((noinline)) unsigned sum_a(const unsigned char *p, unsigned n) { PREAMBLE return sum_gen(p, n, 1); }
__attribute__((noinline)) unsigned sum_b(const unsigned char *p, unsigned n) { PREAMBLE return sum_gen(p, n, 0); }
/* Second, small callers keep the scorers multi-site. */
__attribute__((noinline)) unsigned sum_a2(const unsigned char *p, unsigned n) { return sum_gen(p, n / 2, 1) + 1; }
__attribute__((noinline)) unsigned sum_b2(const unsigned char *p, unsigned n) { return sum_gen(p, n / 2, 0) + 1; }

static inline unsigned score_i(const unsigned char *p, unsigned n) {
  unsigned x = p[n] * 2246822519u, y = p[n + 1] * 3266489917u, z = p[n + 2] ^ 0x5bd1u;
  x ^= x >> 15; y += x * 11; z ^= y >> 7; x += z << 2; y ^= x >> 9; z += y * 37;
  return (x ^ y ^ z) & 7;
}
static unsigned score_s(const unsigned char *p, unsigned n) {
  unsigned x = p[n] * 3266489917u, y = p[n + 1] * 2246822519u, z = p[n + 2] ^ 0x1b87u;
  x ^= x >> 9; y += x * 13; z ^= y >> 6; x += z << 4; y ^= x >> 15; z += y * 41;
  return (x ^ y ^ z) & 7;
}
__attribute__((noinline)) unsigned sum_i(const unsigned char *p, unsigned n) {
  PREAMBLE unsigned c = 0;
  for (unsigned i = 0; i + 2 < n; i++) c += score_i(p, i);
  return c;
}
__attribute__((noinline)) unsigned sum_s(const unsigned char *p, unsigned n) {
  PREAMBLE unsigned c = 0;
  for (unsigned i = 0; i + 2 < n; i++) c += score_s(p, i);
  return c;
}
__attribute__((noinline)) unsigned sum_i2(const unsigned char *p, unsigned n) {
  unsigned c = 1;
  for (unsigned i = 0; i + 2 < n / 2; i++) c += score_i(p, i);
  return c;
}
__attribute__((noinline)) unsigned sum_s2(const unsigned char *p, unsigned n) {
  unsigned c = 1;
  for (unsigned i = 0; i + 2 < n / 2; i++) c += score_s(p, i);
  return c;
}

#define MATCH_LEN_BODY                                                         \
  const unsigned char *const start = a;                                        \
  while (a + 8 <= end) {                                                       \
    unsigned long long x, y;                                                   \
    memcpy(&x, a, 8);                                                          \
    memcpy(&y, b, 8);                                                          \
    if (x != y) return (unsigned)(a - start) + (unsigned)(__builtin_ctzll(x ^ y) >> 3); \
    a += 8;                                                                    \
    b += 8;                                                                    \
  }                                                                            \
  if (a + 4 <= end) {                                                          \
    unsigned x, y;                                                             \
    memcpy(&x, a, 4);                                                          \
    memcpy(&y, b, 4);                                                          \
    if (x == y) { a += 4; b += 4; }                                            \
  }                                                                            \
  if (a + 2 <= end) {                                                          \
    unsigned short x, y;                                                       \
    memcpy(&x, a, 2);                                                          \
    memcpy(&y, b, 2);                                                          \
    if (x == y) { a += 2; b += 2; }                                            \
  }                                                                            \
  if (a < end && *a == *b) a++;                                                \
  return (unsigned)(a - start);
static inline unsigned match_len_i(const unsigned char *a, const unsigned char *b,
                                   const unsigned char *end) { MATCH_LEN_BODY }
static unsigned match_len_s(const unsigned char *a, const unsigned char *b,
                            const unsigned char *end) { MATCH_LEN_BODY }
__attribute__((noinline)) unsigned matches_i(const unsigned char *p, unsigned n) {
  PREAMBLE unsigned c = 0;
  for (unsigned i = 5; i < n; i++)
    if (p[i] == p[i - 5]) c += match_len_i(p + i, p + i - 5, p + n);
  return c;
}
__attribute__((noinline)) unsigned matches_s(const unsigned char *p, unsigned n) {
  PREAMBLE unsigned c = 0;
  for (unsigned i = 5; i < n; i++)
    if (p[i] == p[i - 5]) c += match_len_s(p + i, p + i - 5, p + n);
  return c;
}
__attribute__((noinline)) unsigned matches_i2(const unsigned char *p, unsigned n) {
  return match_len_i(p, p + 1, p + n / 2) + match_len_i(p + 3, p + 7, p + n);
}
__attribute__((noinline)) unsigned matches_s2(const unsigned char *p, unsigned n) {
  return match_len_s(p, p + 1, p + n / 2) + match_len_s(p + 3, p + 7, p + n);
}

int main(void) {
  static unsigned char text[4096];
  static unsigned in[512];
  static unsigned char out[512 * 12 + 64];
  for (unsigned i = 0; i < sizeof text; i++) text[i] = (unsigned char)("abcab"[i % 5] + (i % 97 == 0));
  for (unsigned i = 0; i < 512; i++) in[i] = i * 2654435761u;
  bits_t s = {0, 0, out, out + sizeof out - 8};
  encode(&s, in, 512);
  unsigned long long h = 1469598103934665603ull;
  for (unsigned char *q = out; q < s.p; q++) h = (h ^ *q) * 1099511628211ull;
  printf("%u %u %llu %u\n", count_a(text, sizeof text), count_b(text, sizeof text), h, (unsigned)(s.p - out));
  unsigned sa = sum_a(text, sizeof text), sb = sum_b(text, sizeof text);
  unsigned sa2 = sum_a2(text, sizeof text), sb2 = sum_b2(text, sizeof text), mix = 0;
  for (unsigned i = 0; i < 64; i++) mix = mix * 31u + g_mix[i];
  printf("%u %u %u %u %u\n", sa, sb, sa2, sb2, mix);
  unsigned si = sum_i(text, sizeof text), ss = sum_s(text, sizeof text);
  printf("%u %u %u %u\n", si, ss, sum_i2(text, sizeof text), sum_s2(text, sizeof text));
  printf("%u %u %u %u\n", matches_i(text, sizeof text), matches_s(text, sizeof text),
         matches_i2(text, sizeof text), matches_s2(text, sizeof text));
  return 0;
}
