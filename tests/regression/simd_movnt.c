/* non-temporal vector stores (Movntdq/Movntpd) correctness and
 * the register-aware loaders feeding them. Also covers _mm_storel_epi64 /
 * _mm_loadl_epi64 (the raw movq helpers). _Alignas(32) guarantees the
 * runtime-aligned slot (>16), which movntdq/movntpd require. */
#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>

static int fail = 0;
static void chk(int cond, const char *what) {
  if (!cond) { printf("FAIL %s\n", what); fail = 1; }
}

int main(void) {
  __m128i v = _mm_setr_epi32(0xDEADBEEF, 0x12345678, 0x0BADF00D, 0xCAFEBABE);
  _Alignas(32) uint8_t dst[64];
  for (int i = 0; i < 64; i++) dst[i] = 0xAA;
  _mm_stream_si128((__m128i*)dst, v);
  chk((uint32_t)_mm_cvtsi128_si32(_mm_loadu_si128((const __m128i*)dst)) == 0xDEADBEEFu, "movntdq lo");
  chk((uint32_t)_mm_extract_epi32(_mm_loadu_si128((const __m128i*)dst), 1) == 0x12345678u, "movntdq hi");
  chk(dst[16] == 0xAA && dst[63] == 0xAA, "movntdq neighbours");

  __m128d pd = _mm_setr_pd(3.25, -9.5);
  _Alignas(32) double pdst[4] = { 0.0, 0.0, 0.0, 0.0 };
  _mm_stream_pd(pdst, pd);
  chk(pdst[0] == 3.25 && pdst[1] == -9.5, "movntpd vals");
  chk(pdst[2] == 0.0 && pdst[3] == 0.0, "movntpd neighbours");

  __m128i base = _mm_set1_epi32(7);
  for (int i = 0; i < 100; i++) {
    base = _mm_add_epi32(base, _mm_set1_epi32(3));
    _mm_stream_si128((__m128i*)dst, base);
    __m128i r = _mm_loadu_si128((const __m128i*)dst);
    chk((uint32_t)_mm_extract_epi32(r, 0) == 7u + (uint32_t)(i + 1) * 3u, "stream-chain");
  }

  __m128i q = _mm_set_epi64x(0x99AABBCCDDEEFF00ull, 0x1122334455667788ull);
  uint64_t lo = 0;
  _mm_storel_epi64((__m128i*)&lo, q);
  chk(lo == 0x1122334455667788ull, "storel_epi64");
  uint64_t src = 0xDEADBEEFCAFEF00Dull;
  __m128i loaded = _mm_loadl_epi64((const __m128i*)&src);
  chk((uint64_t)_mm_cvtsi128_si64(loaded) == 0xDEADBEEFCAFEF00Dull, "loadl_epi64");

  if (fail) return 1;
  printf("OK simd_movnt\n");
  return 0;
}
