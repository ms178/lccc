/* AVX2 256-bit intrinsics vs scalar reference — 256-bit loads,
 * arithmetic, comparisons, shuffles, broadcasts, inserts/extracts. */
#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>

static int feq(float a, float b) { return a == b || (a != a && b != b); }

int main(void) {
    int32_t a[8] = {1, 2, 3, 4, 5, 6, 7, 8};
    int32_t b[8] = {8, 7, 6, 5, 4, 3, 2, 1};
    __m256i va = _mm256_loadu_si256((const __m256i*)a);
    __m256i vb = _mm256_loadu_si256((const __m256i*)b);

    int32_t r[8];
    _mm256_storeu_si256((__m256i*)r, _mm256_add_epi32(va, vb));
    for (int i = 0; i < 8; i++) if (r[i] != a[i] + b[i]) return 1;
    _mm256_storeu_si256((__m256i*)r, _mm256_sub_epi32(va, vb));
    for (int i = 0; i < 8; i++) if (r[i] != a[i] - b[i]) return 2;
    _mm256_storeu_si256((__m256i*)r, _mm256_mullo_epi32(va, vb));
    for (int i = 0; i < 8; i++) if (r[i] != a[i] * b[i]) return 3;
    _mm256_storeu_si256((__m256i*)r, _mm256_xor_si256(va, vb));
    for (int i = 0; i < 8; i++) if (r[i] != (a[i] ^ b[i])) return 4;

    /* byte compare -> 32-bit movemask */
    uint8_t x[32], y[32];
    for (int i = 0; i < 32; i++) { x[i] = (uint8_t)(i * 5); y[i] = (uint8_t)(i * 5 + 1); }
    __m256i vx = _mm256_loadu_si256((const __m256i*)x);
    __m256i vy = _mm256_loadu_si256((const __m256i*)y);
    unsigned mask = (unsigned)_mm256_movemask_epi8(_mm256_cmpeq_epi8(vx, vy));
    if (mask != 0) return 5;
    unsigned nmask = (unsigned)_mm256_movemask_epi8(_mm256_cmpgt_epi8(vy, vx));
    if (nmask != 0xFFFFFFFFu) return 6;

    /* 256-bit float */
    float fa[8] = {1,2,3,4,5,6,7,8}, fb[8] = {0.5f,1,1.5f,2,2.5f,3,3.5f,4};
    __m256 vfa = _mm256_loadu_ps(fa);
    __m256 vfb = _mm256_loadu_ps(fb);
    float fr[8];
    _mm256_storeu_ps(fr, _mm256_add_ps(vfa, vfb));
    for (int i = 0; i < 8; i++) if (!feq(fr[i], fa[i]+fb[i])) return 7;
    _mm256_storeu_ps(fr, _mm256_mul_ps(vfa, vfb));
    for (int i = 0; i < 8; i++) if (!feq(fr[i], fa[i]*fb[i])) return 8;

    /* broadcast + insert/extract (128<->256) */
    __m128i low = _mm_setr_epi32(100, 200, 300, 400);
    __m256i wide = _mm256_inserti128_si256(_mm256_setzero_si256(), low, 0);
    _mm256_storeu_si256((__m256i*)r, wide);
    if (r[0]!=100 || r[1]!=200 || r[2]!=300 || r[3]!=400) return 9;
    if (r[4]!=0 || r[5]!=0 || r[6]!=0 || r[7]!=0) return 10;
    __m256i wide2 = _mm256_inserti128_si256(_mm256_setzero_si256(), low, 1);
    _mm256_storeu_si256((__m256i*)r, wide2);
    if (r[4]!=100 || r[5]!=200 || r[6]!=300 || r[7]!=400) return 11;
    __m128i back = _mm256_extracti128_si256(wide2, 1);
    int32_t br[4];
    _mm_storeu_si128((__m128i*)br, back);
    if (br[0]!=100 || br[1]!=200 || br[2]!=300 || br[3]!=400) return 12;

    /* broadcast 128 -> 256 */
    __m256i bcast = _mm256_broadcastsi128_si256(low);
    _mm256_storeu_si256((__m256i*)r, bcast);
    for (int i = 0; i < 8; i++) if (r[i] != (i < 4 ? (int[]){100,200,300,400}[i] : (int[]){100,200,300,400}[i-4])) return 13;

    /* zero extension */
    __m256i z = _mm256_zextsi128_si256(low);
    _mm256_storeu_si256((__m256i*)r, z);
    if (r[4]!=0 || r[5]!=0 || r[6]!=0 || r[7]!=0) return 14;

    /* set1 */
    __m256i s1 = _mm256_set1_epi32(0x12345678);
    _mm256_storeu_si256((__m256i*)r, s1);
    for (int i = 0; i < 8; i++) if (r[i] != 0x12345678) return 15;

    /* pshufb 256 */
    uint8_t sh[32], idx[32];
    for (int i = 0; i < 32; i++) sh[i] = (uint8_t)i;
    for (int i = 0; i < 32; i++) idx[i] = (uint8_t)(31 - i);   /* reverse (per 16) */
    __m256i vsh = _mm256_loadu_si256((const __m256i*)sh);
    __m256i vidx = _mm256_loadu_si256((const __m256i*)idx);
    uint8_t out[32];
    _mm256_storeu_si256((__m256i*)out, _mm256_shuffle_epi8(vsh, vidx));
    for (int i = 0; i < 32; i++) {
        int lane = i & 15;
        uint8_t want = (uint8_t)((i & 16) + (15 - lane));
        if (out[i] != want) return 16;
    }

    printf("OK simd_avx2_256\n");
    return 0;
}
