/* Hardware emission for ops that used to be scalar header loops.
 * Lane-exact vs a scalar reference. Requires SSE2/SSE4.1/AVX2. */
#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>

static int sat_add_u8(int a, int b) {
    int s = a + b;
    return s > 255 ? 255 : s;
}
static int sat_add_i16(int a, int b) {
    int s = a + b;
    if (s > 32767) s = 32767;
    if (s < -32768) s = -32768;
    return s;
}

int main(void) {
    uint8_t a8[16], b8[16];
    for (int i = 0; i < 16; i++) { a8[i] = (uint8_t)(i * 17); b8[i] = (uint8_t)(200 + i); }
    __m128i va8 = _mm_loadu_si128((const __m128i *)a8);
    __m128i vb8 = _mm_loadu_si128((const __m128i *)b8);

    uint8_t r8[16];
    _mm_storeu_si128((__m128i *)r8, _mm_adds_epu8(va8, vb8));
    for (int i = 0; i < 16; i++) if (r8[i] != (uint8_t)sat_add_u8(a8[i], b8[i])) return 1;

    _mm_storeu_si128((__m128i *)r8, _mm_andnot_si128(va8, vb8));
    for (int i = 0; i < 16; i++) if (r8[i] != (uint8_t)((~a8[i]) & b8[i])) return 2;

    int16_t a16[8] = {30000, -30000, 100, -100, 7, -7, 1234, -4321};
    int16_t b16[8] = {5000, -5000, 200, 50, 9, 3, 10, 20};
    __m128i va16 = _mm_loadu_si128((const __m128i *)a16);
    __m128i vb16 = _mm_loadu_si128((const __m128i *)b16);
    int16_t r16[8];
    _mm_storeu_si128((__m128i *)r16, _mm_adds_epi16(va16, vb16));
    for (int i = 0; i < 8; i++) if (r16[i] != (int16_t)sat_add_i16(a16[i], b16[i])) return 3;
    _mm_storeu_si128((__m128i *)r16, _mm_min_epi16(va16, vb16));
    for (int i = 0; i < 8; i++) if (r16[i] != (a16[i] < b16[i] ? a16[i] : b16[i])) return 4;
    _mm_storeu_si128((__m128i *)r16, _mm_cmpeq_epi16(va16, va16));
    for (int i = 0; i < 8; i++) if (r16[i] != (int16_t)0xFFFF) return 5;

    int32_t a32[4] = {1, -2, 3, -4};
    int32_t b32[4] = {0, -3, 3, 5};
    __m128i va32 = _mm_loadu_si128((const __m128i *)a32);
    __m128i vb32 = _mm_loadu_si128((const __m128i *)b32);
    int32_t r32[4];
    _mm_storeu_si128((__m128i *)r32, _mm_cmpgt_epi32(va32, vb32));
    for (int i = 0; i < 4; i++) if (r32[i] != (a32[i] > b32[i] ? -1 : 0)) return 6;

    long long a64[2] = {100, -200};
    long long b64[2] = {7, 9};
    __m128i va64 = _mm_loadu_si128((const __m128i *)a64);
    __m128i vb64 = _mm_loadu_si128((const __m128i *)b64);
    long long r64[2];
    _mm_storeu_si128((__m128i *)r64, _mm_add_epi64(va64, vb64));
    if (r64[0] != 107 || r64[1] != -191) return 7;
    _mm_storeu_si128((__m128i *)r64, _mm_unpacklo_epi64(va64, vb64));
    if (r64[0] != 100 || r64[1] != 7) return 8;

    __m128i z = _mm_setzero_si128();
    _mm_storeu_si128((__m128i *)r64, z);
    if (r64[0] != 0 || r64[1] != 0) return 9;

    if (_mm_testz_si128(z, va64) != 1) return 10;
    if (_mm_testz_si128(va64, va64) != 0) return 11;

    /* AVX2 */
    int32_t A[8] = {1,2,3,4,5,6,7,8};
    int32_t B[8] = {8,7,6,5,4,3,2,1};
    __m256i vA = _mm256_loadu_si256((const __m256i *)A);
    __m256i vB = _mm256_loadu_si256((const __m256i *)B);
    int32_t R[8];
    _mm256_storeu_si256((__m256i *)R, _mm256_mullo_epi32(vA, vB));
    for (int i = 0; i < 8; i++) if (R[i] != A[i] * B[i]) return 12;
    _mm256_storeu_si256((__m256i *)R, _mm256_sub_epi32(vA, vB));
    for (int i = 0; i < 8; i++) if (R[i] != A[i] - B[i]) return 13;
    _mm256_storeu_si256((__m256i *)R, _mm256_andnot_si256(vA, vB));
    for (int i = 0; i < 8; i++) if (R[i] != ((~A[i]) & B[i])) return 14;
    _mm256_storeu_si256((__m256i *)R, _mm256_cmpeq_epi32(vA, vA));
    for (int i = 0; i < 8; i++) if (R[i] != -1) return 15;

    __m128i low = _mm_setr_epi32(10, 20, 30, 40);
    __m256i wide = _mm256_inserti128_si256(_mm256_setzero_si256(), low, 1);
    __m128i back = _mm256_extracti128_si256(wide, 1);
    int32_t br[4];
    _mm_storeu_si128((__m128i *)br, back);
    if (br[0] != 10 || br[1] != 20 || br[2] != 30 || br[3] != 40) return 16;
    __m128i zlow = _mm256_extracti128_si256(wide, 0);
    _mm_storeu_si128((__m128i *)br, zlow);
    if (br[0] || br[1] || br[2] || br[3]) return 17;

    float fa[8] = {1,2,3,4,5,6,7,8};
    float fb[8] = {0.5f,1,1.5f,2,2.5f,3,3.5f,4};
    float fr[8];
    _mm256_storeu_ps(fr, _mm256_add_ps(_mm256_loadu_ps(fa), _mm256_loadu_ps(fb)));
    for (int i = 0; i < 8; i++) if (fr[i] != fa[i] + fb[i]) return 18;
    _mm256_storeu_ps(fr, _mm256_mul_ps(_mm256_loadu_ps(fa), _mm256_loadu_ps(fb)));
    for (int i = 0; i < 8; i++) if (fr[i] != fa[i] * fb[i]) return 19;

    __m256i z256 = _mm256_setzero_si256();
    _mm256_storeu_si256((__m256i *)R, z256);
    for (int i = 0; i < 8; i++) if (R[i] != 0) return 20;

    __m256i zx = _mm256_zextsi128_si256(low);
    _mm256_storeu_si256((__m256i *)R, zx);
    if (R[0] != 10 || R[1] != 20 || R[2] != 30 || R[3] != 40) return 21;
    if (R[4] || R[5] || R[6] || R[7]) return 22;

    printf("OK simd_new_hw_ops\n");
    return 0;
}
