/* vec_arx boundary absorption: runtime differential for the entry-copy
 * and feed-forward absorbers (src/passes/vec_arx.rs).
 *
 * The transform rewrites chacha20_core's entry copy loop into an
 * element-wise register pack and replaces the feed-forward loop
 * `out[i] = x[i] + in[i]` with vector adds behind a RUNTIME overlap
 * guard (fast path iff out == in, out below in, or disjoint; the
 * scalar loop remains for the semantics-changing overlap window
 * out = in + 1..15 elements).
 *
 * Every guard region is exercised, with the overlap scenarios allowed
 * to feed their clobber forward call-to-call so a divergence anywhere
 * cascades into the checksum:
 *   disjoint   - fast path, the benchmark shape
 *   exact      - out == in (distance 0: fast path, read-before-clobber)
 *   above      - out = in + 1 element (the UNSAFE window: scalar path)
 *   below      - out = in - 1 element (fast path, stores trail reads)
 *   far        - out = in + 16 elements (boundary: first safe distance)
 *
 * The outputs are compared against a GCC build and against an
 * lccc build with the pass disabled (CCC_DISABLE_PASSES=vec_arx), so
 * both the guard routing and the absorbed fast path must reproduce
 * the scalar semantics bit-for-bit.
 */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#define ROUNDS_PER_SCENARIO 1024

__attribute__((noinline)) static void chacha20_core(uint32_t out[16],
                                                    const uint32_t in[16]) {
  uint32_t x[16];
  for (int i = 0; i < 16; i++)
    x[i] = in[i];
  for (int r = 0; r < 10; r++) {
#define QR(a, b, c, d)                                                        \
  x[a] += x[b];                                                               \
  x[d] ^= x[a];                                                               \
  x[d] = (x[d] << 16) | (x[d] >> 16);                                         \
  x[c] += x[d];                                                               \
  x[b] ^= x[c];                                                               \
  x[b] = (x[b] << 12) | (x[b] >> 20);                                         \
  x[a] += x[b];                                                               \
  x[d] ^= x[a];                                                               \
  x[d] = (x[d] << 8) | (x[d] >> 24);                                          \
  x[c] += x[d];                                                               \
  x[b] ^= x[c];                                                               \
  x[b] = (x[b] << 7) | (x[b] >> 25);
    QR(0, 4, 8, 12)
    QR(1, 5, 9, 13)
    QR(2, 6, 10, 14)
    QR(3, 7, 11, 15)
    QR(0, 5, 10, 15)
    QR(1, 6, 11, 12)
    QR(2, 7, 8, 13)
    QR(3, 4, 9, 14)
#undef QR
  }
  for (int i = 0; i < 16; i++)
    out[i] = x[i] + in[i];
}

static uint64_t fnv1a(uint64_t h, const uint32_t *w, int n) {
  for (int i = 0; i < n; i++) {
    h ^= w[i];
    h *= 0x100000001b3ULL;
  }
  return h;
}

/* RFC 7539 section 2.3.2 test-vector state. */
static const uint32_t seed[16] = {
    0x61707865u, 0x3320646eu, 0x79622d32u, 0x6b206574u,
    0x03020100u, 0x07060504u, 0x0b0a0908u, 0x0f0e0d0cu,
    0x13121110u, 0x17161514u, 0x1b1a1918u, 0x1f1e1d1cu,
    0x00000001u, 0x09000000u, 0x4a000000u, 0x00000000u};

static uint64_t run_scenario(uint32_t *in, uint32_t *out) {
  /* (Re)seed the input region; overlap scenarios then let each call's
   * clobber of `in` carry into the next call. */
  memcpy(in, seed, sizeof seed);
  uint64_t h = 0xcbf29ce484222325ULL;
  for (uint32_t i = 0; i < ROUNDS_PER_SCENARIO; i++) {
    in[12] = seed[12] + i;      /* the RFC 7539 counter word */
    in[4] ^= i * 0x9e3779b9u;   /* second hot word, like the benchmark */
    chacha20_core(out, in);
    h = fnv1a(h, out, 16);
  }
  return h;
}

int main(void) {
  static uint32_t arena[64];

  /* Known-answer check first: one block of the RFC 7539 vector. */
  uint32_t kat[16];
  chacha20_core(kat, seed);
  if (kat[0] != 0xe4e7f110u || kat[1] != 0x15593bd1u ||
      kat[14] != 0xe883d0cbu || kat[15] != 0x4e3c50a2u) {
    puts("KAT FAILED");
    return 1;
  }

  memset(arena, 0, sizeof arena);
  printf("disjoint %016llx\n",
         (unsigned long long)run_scenario(arena, arena + 32));
  memset(arena, 0, sizeof arena);
  printf("exact    %016llx\n", (unsigned long long)run_scenario(arena, arena));
  memset(arena, 0, sizeof arena);
  printf("above    %016llx\n",
         (unsigned long long)run_scenario(arena, arena + 1));
  memset(arena, 0, sizeof arena);
  printf("below    %016llx\n",
         (unsigned long long)run_scenario(arena + 1, arena));
  memset(arena, 0, sizeof arena);
  printf("far      %016llx\n",
         (unsigned long long)run_scenario(arena, arena + 16));
  return 0;
}
