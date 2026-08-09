/* v5 regression: SIMD insert/extract family correctness (register-aware loaders).
 * Exercises Pinsrw/Pinsrd/Pinsrb/Pinsrq, Pextrw/Pextrd/Pextrb/Pextrq,
 * Cvtsi128Si32/Si64, Pmovmskb128/256 and Pabsb against scalar references.
 * Extract/insert indices are compile-time immediates (pextr and pinsr require
 * an immediate; a runtime index is invalid). */
#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>

static uint32_t ref_crc = 0;
static void chk(int cond, const char *what) {
  if (!cond) { printf("FAIL %s\n", what); ref_crc = 1; }
}

int main(void) {
  __m128i a = _mm_setr_epi32(0x11111111, 0x22222222, 0x33333333, 0x44444444);
  __m128i t = _mm_insert_epi32(a, 0xDEADBEEF, 2);
  chk((uint32_t)_mm_extract_epi32(t, 0) == 0x11111111u, "pextrd0");
  chk((uint32_t)_mm_extract_epi32(t, 1) == 0x22222222u, "pextrd1");
  chk((uint32_t)_mm_extract_epi32(t, 2) == 0xDEADBEEFu, "pextrd2");
  chk((uint32_t)_mm_extract_epi32(t, 3) == 0x44444444u, "pextrd3");

  __m128i b = _mm_setr_epi16(0x0102, 0x0304, 0x0506, 0x0708, 0x090A, 0x0B0C, 0x0D0E, 0x0F10);
  __m128i u = _mm_insert_epi16(b, 0xCAFE, 5);
  chk((uint32_t)_mm_extract_epi16(u, 0) == 0x0102u, "pextrw0");
  chk((uint32_t)_mm_extract_epi16(u, 5) == 0xCAFEu, "pextrw5");
  chk((uint32_t)_mm_extract_epi16(u, 7) == 0x0F10u, "pextrw7");

  __m128i c = _mm_setr_epi8(1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16);
  __m128i v = _mm_insert_epi8(c, 0x7F, 3);
  chk((uint32_t)_mm_extract_epi8(v, 3) == 0x7Fu, "pextrb3");
  chk((uint32_t)_mm_extract_epi8(v, 15) == 16u, "pextrb15");

  __m128i d = _mm_set_epi64x(0xFEDCBA9876543210ull, 0x0123456789ABCDEFull);
  __m128i w = _mm_insert_epi64(d, 0x1122334455667788ull, 0);
  chk((uint64_t)_mm_extract_epi64(w, 0) == 0x1122334455667788ull, "pextrq0");
  chk((uint64_t)_mm_extract_epi64(w, 1) == 0xFEDCBA9876543210ull, "pextrq1");

  chk((uint32_t)_mm_cvtsi128_si32(a) == 0x11111111u, "cvtsi32");
  chk((uint64_t)_mm_cvtsi128_si64(d) == 0x0123456789ABCDEFull, "cvtsi64");

  __m128i neg = _mm_setr_epi8(-1, -2, -3, -4, -5, -6, -7, -8, 1, 2, 3, 4, 5, 6, 7, 8);
  __m128i pa = _mm_abs_epi8(neg);
  chk((uint32_t)_mm_extract_epi8(pa, 0) == 1u, "pabsb128-0");
  chk((uint32_t)_mm_extract_epi8(pa, 1) == 2u, "pabsb128-1");
  chk((uint32_t)_mm_extract_epi8(pa, 2) == 3u, "pabsb128-2");
  chk((uint32_t)_mm_extract_epi8(pa, 3) == 4u, "pabsb128-3");
  chk((uint32_t)_mm_extract_epi8(pa, 4) == 5u, "pabsb128-4");
  chk((uint32_t)_mm_extract_epi8(pa, 5) == 6u, "pabsb128-5");
  chk((uint32_t)_mm_extract_epi8(pa, 6) == 7u, "pabsb128-6");
  chk((uint32_t)_mm_extract_epi8(pa, 7) == 8u, "pabsb128-7");

  __m256i neg256 = _mm256_setr_epi8(
      -1, -2, -3, -4, -5, -6, -7, -8, -9, -10, -11, -12, -13, -14, -15, -16,
      -17, -18, -19, -20, -21, -22, -23, -24, -25, -26, -27, -28, -29, -30, -31, -32);
  __m256i pa256 = _mm256_abs_epi8(neg256);
  uint8_t buf[32];
  _mm256_storeu_si256((__m256i*)buf, pa256);
  for (int i = 0; i < 32; i++)
    chk(buf[i] == (uint8_t)(i + 1), "pabsb256");

  __m128i sign128 = _mm_setr_epi8(0x80, 0x00, 0x80, 0x00, 0x80, 0x00, 0x80, 0x00,
                                  0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00);
  chk((uint32_t)_mm_movemask_epi8(sign128) == 0x0055u, "pmovmskb128");
  __m256i sign256 = _mm256_set_m128i(sign128, sign128);
  chk((uint32_t)_mm256_movemask_epi8(sign256) == 0x00550055u, "pmovmskb256");

  __m128i x = _mm_set1_epi32(0x100);
  for (int j = 0; j < 250; j++) {
    x = _mm_add_epi32(x, _mm_set1_epi32(1));
    x = _mm_insert_epi32(x, j * 4 + 0, 0);
    x = _mm_add_epi32(x, _mm_set1_epi32(1));
    x = _mm_insert_epi32(x, j * 4 + 1, 1);
    x = _mm_add_epi32(x, _mm_set1_epi32(1));
    x = _mm_insert_epi32(x, j * 4 + 2, 2);
    x = _mm_add_epi32(x, _mm_set1_epi32(1));
    x = _mm_insert_epi32(x, j * 4 + 3, 3);
  }
  chk((uint32_t)_mm_extract_epi32(x, 0) == 999u, "chain-lane0");
  chk((uint32_t)_mm_extract_epi32(x, 1) == 999u, "chain-lane1");
  chk((uint32_t)_mm_extract_epi32(x, 2) == 999u, "chain-lane2");
  chk((uint32_t)_mm_extract_epi32(x, 3) == 999u, "chain-lane3");
  if (ref_crc) return 1;
  printf("OK simd_insert_extract\n");
  return 0;
}
