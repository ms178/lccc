/* XMM register allocation for vector values.
 * `t` is a multi-use 128-bit value: the backend may keep it in an XMM
 * register for its whole live range (CCC_ENABLE_VECREG) or in its slot —
 * both must produce identical, correct results. */
#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>

static int fail = 0;
static void chk(int cond, const char *what) {
  if (!cond) { printf("FAIL %s\n", what); fail = 1; }
}

static uint32_t multius(__m128i x, __m128i y) {
  __m128i t = _mm_add_epi32(x, y);
  __m128i u = _mm_add_epi32(t, t);
  __m128i w = _mm_add_epi32(u, t);
  __m128i q = _mm_sub_epi32(w, t);
  return (uint32_t)_mm_cvtsi128_si32(q);
}

static uint32_t loopcarry(const uint32_t *data, int n) {
  __m128i acc = _mm_setzero_si128();
  for (int i = 0; i < n; i += 4) {
    __m128i v = _mm_loadu_si128((const __m128i*)(data + i));
    acc = _mm_add_epi32(acc, v);
    acc = _mm_xor_si128(acc, _mm_srli_epi32(v, 1));
  }
  return (uint32_t)_mm_cvtsi128_si32(acc);
}

int main(void) {
  __m128i x = _mm_set_epi32(1, 2, 3, 4);
  __m128i y = _mm_set_epi32(10, 20, 30, 40);
  /* x = {4,3,2,1}, y = {40,30,20,10}: t={44,33,22,11}, u={88,66,44,22},
     w={132,99,66,33}, q=w-t={88,66,44,22} -> lane0 = 88 */
  chk(multius(x, y) == 88u, "vecreg-multius");

  uint32_t data[64];
  for (int i = 0; i < 64; i++) data[i] = (uint32_t)(i * 2654435761u);
  uint32_t acc = 0;
  for (int i = 0; i < 64; i += 4) {
    acc = acc + data[i];
    acc = acc ^ (data[i] >> 1);
  }
  chk(loopcarry(data, 64) == acc, "vecreg-loopcarry");

  if (fail) return 1;
  printf("OK simd_vecreg\n");
  return 0;
}
