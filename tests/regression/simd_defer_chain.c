/* deferred single-use vector stores (accumulator renaming).
 * Chains where each fresh result is consumed by the immediately-following
 * intrinsic (as args[0] AND as args[1]) must produce exact results — the
 * store->reload round trip is eliminated, so any stale-slot read shows up
 * here. Also stresses the generalized last-store peephole (non-xmm0
 * producers via pblendvb). */
#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>

static int fail = 0;
static void chk(int cond, const char *what) {
  if (!cond) { printf("FAIL %s\n", what); fail = 1; }
}

int main(void) {
  __m128i acc = _mm_set1_epi32(0);
  for (int i = 0; i < 500; i++) {
    __m128i x = _mm_set1_epi32(i);
    acc = _mm_xor_si128(x, acc);          /* fresh result `x` is args[0] */
  }
  uint32_t exp = 0;
  for (int i = 0; i < 500; i++) exp ^= (uint32_t)i;
  chk((uint32_t)_mm_extract_epi32(acc, 0) == exp, "defer-arg0");

  acc = _mm_set1_epi32(0);
  for (int i = 0; i < 500; i++) {
    __m128i x = _mm_set1_epi32(i * 3);
    acc = _mm_xor_si128(acc, x);          /* fresh result `x` is args[1] */
  }
  exp = 0;
  for (int i = 0; i < 500; i++) exp ^= (uint32_t)(i * 3);
  chk((uint32_t)_mm_extract_epi32(acc, 0) == exp, "defer-arg1");

  __m128i t = _mm_set1_epi32(1);
  for (int i = 0; i < 300; i++) {
    __m128i a = _mm_add_epi32(t, _mm_set1_epi32(5));
    __m128i b = _mm_mullo_epi32(a, _mm_set1_epi32(3));
    t = _mm_xor_si128(b, a);              /* b = args[0], a = args[1] */
  }
  uint32_t v = 1;
  for (int i = 0; i < 300; i++) {
    uint32_t a = v + 5;
    uint32_t b = a * 3;
    v = b ^ a;
  }
  chk((uint32_t)_mm_extract_epi32(t, 0) == v, "defer-deep-chain");

  __m128i m = _mm_setr_epi8(0xFF, 0, 0xFF, 0, 0xFF, 0, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0, 0);
  __m128i A = _mm_set1_epi8(0x11);
  __m128i B = _mm_set1_epi8(0x22);
  __m128i bl = _mm_blendv_epi8(A, B, m);              /* mask ? B : A */
  __m128i r = _mm_xor_si128(bl, _mm_set1_epi8(0x33)); /* consume the xmm2 result */
  chk((uint32_t)_mm_extract_epi8(r, 0) == (0x22 ^ 0x33), "pblendvb-xmm2-consume");
  chk((uint32_t)_mm_extract_epi8(r, 2) == (0x22 ^ 0x33), "pblendvb-xmm2-consume2");
  chk((uint32_t)_mm_extract_epi8(r, 8) == (0x11 ^ 0x33), "pblendvb-xmm2-consume3");

  if (fail) return 1;
  printf("OK simd_defer_chain\n");
  return 0;
}
