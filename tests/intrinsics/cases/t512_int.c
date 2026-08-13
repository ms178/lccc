/* 512-bit integer intrinsic differential test (AVX-512F/BW/VL/VNNI/VBMI2/BITALG/VPCLMULQDQ) */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>
#include <stdint.h>

static void dump512(const char *tag, __m512i v) {
    uint64_t out[8];
    _mm512_storeu_si512(out, v);
    printf("%s:", tag);
    for (int i = 0; i < 8; i++) printf(" %016llx", (unsigned long long)out[i]);
    printf("\n");
}

int main(void) {
    __m512i a = _mm512_set_epi64(8, 7, 6, 5, 4, 3, 2, 1);
    __m512i b = _mm512_set1_epi64(10);
    __m512i z = _mm512_setzero_si512();

    dump512("add", _mm512_add_epi32(a, _mm512_set1_epi32(1)));
    dump512("add64", _mm512_add_epi64(a, b));
    dump512("sub", _mm512_sub_epi32(a, _mm512_set1_epi32(1)));
    dump512("xor", _mm512_xor_si512(a, _mm512_set1_epi64(-1)));
    dump512("and", _mm512_and_si512(a, _mm512_set1_epi64(0xF0F0)));
    dump512("sll", _mm512_slli_epi32(a, 3));
    dump512("srl64", _mm512_srli_epi64(a, 3));
    dump512("shuf", _mm512_shuffle_epi32(a, 0x1B));

    /* SAD + maddubs + madd: adler32-style */
    __m512i u8 = _mm512_set_epi64(0x0807060504030201ull, 0x100f0e0d0c0b0a09ull,
                                  0x1817161514131211ull, 0x201f1e1d1c1b1a19ull,
                                  0x2827262524232221ull, 0x302f2e2d2c2b2a29ull,
                                  0x3837363534333231ull, 0x403f3e3d3c3b3a39ull);
    dump512("sad", _mm512_sad_epu8(u8, z));
    printf("sad_low=%llu\n", (unsigned long long)_mm_cvtsi128_si64(_mm512_extracti64x2_epi64(_mm512_sad_epu8(u8, z), 0)));

    __m512i w1 = _mm512_set1_epi16(0x0100); /* u8 pairs: 0,1 -> 1 */
    __m512i madd = _mm512_maddubs_epi16(u8, w1);
    uint64_t mlo[8];
    _mm512_storeu_si512(mlo, madd);
    printf("maddubs0=%llu\n", (unsigned long long)mlo[0]);

    __m512i w2 = _mm512_set1_epi16(1);
    __m512i madd2 = _mm512_madd_epi16(w2, _mm512_set1_epi16(2));
    _mm512_storeu_si512(mlo, madd2);
    printf("madd0=%llu\n", (unsigned long long)mlo[0]);

    /* ternary logic: xor3 = a^b^c */
    __m512i t = _mm512_ternarylogic_epi32(a, b, _mm512_set1_epi64(5), 0x96);
    dump512("tern", t);
    __m512i t2 = _mm512_xor_si512(_mm512_xor_si512(a, b), _mm512_set1_epi64(5));
    printf("tern_eq_xor3=%d\n", memcmp(&t, &t2, 64) == 0);

    /* mask compares */
    __mmask64 m64 = _mm512_cmpeq_epu8_mask(_mm512_set1_epi8(7), _mm512_set1_epi8(7));
    __mmask64 m64b = _mm512_cmpeq_epu8_mask(_mm512_set1_epi8(7), _mm512_set1_epi8(8));
    printf("mask_eq=%llx mask_neq=%llx\n", (unsigned long long)m64, (unsigned long long)m64b);
    __mmask16 m16 = _mm_cmpeq_epu8_mask(_mm_set1_epi8(3), _mm_set1_epi8(3));
    __mmask32 m32 = _mm256_cmp_epu8_mask(_mm256_set1_epi8(4), _mm256_set1_epi8(4), 0);
    printf("mask16=%x mask32=%x\n", (unsigned)m16, (unsigned)m32);

    /* masked load/store */
    uint8_t src[80], dst[80];
    for (int i = 0; i < 80; i++) src[i] = (uint8_t)(i * 3 + 1);
    memset(dst, 0xAA, sizeof dst);
    __m512i ml = _mm512_maskz_loadu_epi8(0xFFFF0000000000FFull, src); /* bytes 0-7 + 48-63 */
    dump512("mzload", ml);
    __m512i ml2 = _mm512_mask_loadu_epi8(_mm512_set1_epi8(0x55), 0xFFull, src);
    dump512("mload", ml2);
    _mm512_mask_storeu_epi8(dst, 0xFFull, _mm512_set1_epi8(0x42));
    printf("mstore0=%x mstore1=%x\n", dst[0], dst[1]);

    /* insert/extract */
    __m128i lane = _mm_set_epi64x(0xDEADBEEFCAFEBABEull, 0x1234567890ABCDEFull);
    __m512i ins = _mm512_inserti64x2(a, lane, 2);
    dump512("ins", ins);
    __m128i ex = _mm512_extracti64x2_epi64(ins, 2);
    printf("extract=%016llx\n", (unsigned long long)_mm_cvtsi128_si64(ex));
    __m256i ex4 = _mm512_extracti64x4_epi64(a, 0);
    uint64_t e4[4]; _mm256_storeu_si256((__m256i_u *)e4, ex4);
    printf("extract4=%llu,%llu\n", (unsigned long long)e4[0], (unsigned long long)e4[1]);
    __m512i mzi = _mm512_maskz_inserti64x2(0xFF, a, lane, 3);
    dump512("mzins", mzi);

    /* reduce */
    printf("reduce=%d\n", _mm512_reduce_add_epi32(_mm512_set1_epi32(1)));
    printf("reduce2=%d\n", _mm512_reduce_add_epi32(a));

    /* VNNI: vpdpbusd */
    __m512i va = _mm512_set1_epi32(0x01020304u);
    __m512i vb = _mm512_set1_epi32(0x01010101u);
    dump512("vpdpbusd", _mm512_dpbusd_epi32(z, va, vb));

    /* VPCLMULQDQ */
    __m512i cl = _mm512_clmulepi64_epi128(_mm512_set1_epi64(0x1234), _mm512_set1_epi64(0x5678), 0);
    dump512("clmul", cl);

    /* casts/zext */
    __m128i small = _mm_set_epi64x(0, 0x1122334455667788ull);
    __m512i zx = _mm512_zextsi128_si512(small);
    dump512("zext", zx);
    __m256i lo = _mm512_castsi512_si256(a);
    uint64_t l8[4]; _mm256_storeu_si256((__m256i_u *)l8, lo);
    printf("cast256=%llu\n", (unsigned long long)l8[0]);

    /* shuffle_epi8 + palignr */
    __m512i shufc = _mm512_set1_epi64(0x0706050403020100ull);
    dump512("shufb", _mm512_shuffle_epi8(a, shufc));
    dump512("alignr", _mm512_alignr_epi8(a, b, 8));

    /* permutexvar */
    __m512i idx = _mm512_set_epi32(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15);
    dump512("permutex", _mm512_permutexvar_epi32(_mm512_set1_epi32(3), a));

    printf("done\n");
    return 0;
}
