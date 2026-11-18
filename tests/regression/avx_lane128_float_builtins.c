/* 128-bit lane builtins for float, double and integer-256 vectors under -mavx.
 *
 * vextractf128 and vinsertf128 are AVX1 instructions. GCC 16.2 declares
 * __builtin_ia32_vextractf128_{ps,pd,si}256 and __builtin_ia32_vinsertf128_
 * {ps,pd,si}256, each taking a one-bit lane selector. The integer forms take
 * and return the untyped __v8si / __v4si views, so the result needs a cast to
 * __m128i / __m256i (GCC rejects an implicit conversion).
 *
 * Each (builtin, lane) pair is checked against the _mm256_ wrapper that must
 * select the same lane, and the untouched lane of every insert must come
 * through bit-exact. The output is a checksum over the raw bytes of every
 * result plus the integer lanes, so a lane mix-up, a dropped lane or a wrong
 * element width changes the printed value. The program is differential:
 * lccc's stdout must equal GCC 16.2's, byte for byte.
 *
 * The fixture deliberately avoids _mm256_setr_ps and _mm256_setr_pd, which
 * lccc does not declare under -mavx. Vector literals and casts express the
 * same values. The integer wrappers _mm256_extractf128_si256 and
 * _mm256_insertf128_si256 are covered by avx_lane128_int_builtins.
 *
 * The lane selector is always a literal at the builtin call site (macros
 * below), so the immediate never depends on inlining. Compiled with -O2 -mavx.
 */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

typedef int v8si_t __attribute__((vector_size(32)));

static unsigned fold(const void *p, unsigned n, unsigned h) {
    const unsigned char *b = (const unsigned char *)p;
    for (unsigned i = 0; i < n; i++)
        h = (h ^ b[i]) * 16777619u;
    return h;
}

/* Builtins with a literal lane selector. */
#define EXT_PS(a, L) __builtin_ia32_vextractf128_ps256((a), (L))
#define EXT_PD(a, L) __builtin_ia32_vextractf128_pd256((a), (L))
#define EXT_SI(a, L) ((__m128i)__builtin_ia32_vextractf128_si256((__v8si)(a), (L)))
#define INS_PS(a, b, L) __builtin_ia32_vinsertf128_ps256((a), (b), (L))
#define INS_PD(a, b, L) __builtin_ia32_vinsertf128_pd256((a), (b), (L))
#define INS_SI(a, b, L) \
    ((__m256i)__builtin_ia32_vinsertf128_si256((__v8si)(a), (__v4si)(b), (L)))

int main(void) {
    const __m256 fa = { 1.5f, -2.25f, 3.0f, 4.75f, 5.5f, -6.0f, 7.25f, 8.0f };
    const __m256d da = { 10.5, -11.25, 12.0, 13.75 };
    const __m256i ia = (__m256i)(__v8si){ 10, 11, 12, 13, 14, 15, 16, 17 };
    const __m128 fb = _mm_setr_ps(-100.0f, 101.0f, -102.0f, 103.0f);
    const __m128d db = _mm_setr_pd(-200.0, 201.0);
    const __m128i ib = _mm_setr_epi32(90, 91, 92, 93);
    unsigned h = 2166136261u;
    int mismatches = 0;

    /* Lane 0: extract, then insert into the high half would be the other
     * lane; each block below handles one literal selector. */
    {
        __m128 ef = EXT_PS(fa, 0);
        __m128 efw = _mm256_extractf128_ps(fa, 0);
        __m128d ed = EXT_PD(da, 0);
        __m128d edw = _mm256_extractf128_pd(da, 0);
        __m128i ei = EXT_SI(ia, 0);
        __m128i eiw = (__m128i)_mm256_extractf128_ps((__m256)ia, 0);
        if (memcmp(&ef, &efw, sizeof ef) != 0) mismatches++;
        if (memcmp(&ed, &edw, sizeof ed) != 0) mismatches++;
        if (memcmp(&ei, &eiw, sizeof ei) != 0) mismatches++;
        h = fold(&ef, sizeof ef, h);
        h = fold(&ed, sizeof ed, h);
        h = fold(&ei, sizeof ei, h);

        __m256 inf = INS_PS(fa, fb, 0);
        __m256 infw = _mm256_insertf128_ps(fa, fb, 0);
        __m256d ind = INS_PD(da, db, 0);
        __m256d indw = _mm256_insertf128_pd(da, db, 0);
        __m256i ini = INS_SI(ia, ib, 0);
        __m256i iniw = (__m256i)_mm256_insertf128_ps((__m256)ia, (__m128)ib, 0);
        if (memcmp(&inf, &infw, sizeof inf) != 0) mismatches++;
        if (memcmp(&ind, &indw, sizeof ind) != 0) mismatches++;
        if (memcmp(&ini, &iniw, sizeof ini) != 0) mismatches++;
        h = fold(&inf, sizeof inf, h);
        h = fold(&ind, sizeof ind, h);
        h = fold(&ini, sizeof ini, h);

        /* Inserting at lane 0 must keep the high half of the original. */
        __m128 hi_f = _mm256_extractf128_ps(fa, 1);
        __m128 hi_out = _mm256_extractf128_ps(inf, 1);
        if (memcmp(&hi_f, &hi_out, sizeof hi_f) != 0) mismatches++;
        h = fold(&hi_out, sizeof hi_out, h);
    }

    /* Lane 1: same checks with the other selector. */
    {
        __m128 ef = EXT_PS(fa, 1);
        __m128 efw = _mm256_extractf128_ps(fa, 1);
        __m128d ed = EXT_PD(da, 1);
        __m128d edw = _mm256_extractf128_pd(da, 1);
        __m128i ei = EXT_SI(ia, 1);
        __m128i eiw = (__m128i)_mm256_extractf128_ps((__m256)ia, 1);
        if (memcmp(&ef, &efw, sizeof ef) != 0) mismatches++;
        if (memcmp(&ed, &edw, sizeof ed) != 0) mismatches++;
        if (memcmp(&ei, &eiw, sizeof ei) != 0) mismatches++;
        h = fold(&ef, sizeof ef, h);
        h = fold(&ed, sizeof ed, h);
        h = fold(&ei, sizeof ei, h);

        __m256 inf = INS_PS(fa, fb, 1);
        __m256 infw = _mm256_insertf128_ps(fa, fb, 1);
        __m256d ind = INS_PD(da, db, 1);
        __m256d indw = _mm256_insertf128_pd(da, db, 1);
        __m256i ini = INS_SI(ia, ib, 1);
        __m256i iniw = (__m256i)_mm256_insertf128_ps((__m256)ia, (__m128)ib, 1);
        if (memcmp(&inf, &infw, sizeof inf) != 0) mismatches++;
        if (memcmp(&ind, &indw, sizeof ind) != 0) mismatches++;
        if (memcmp(&ini, &iniw, sizeof ini) != 0) mismatches++;
        h = fold(&inf, sizeof inf, h);
        h = fold(&ind, sizeof ind, h);
        h = fold(&ini, sizeof ini, h);

        /* Inserting at lane 1 must keep the low half of the original. */
        __m128 lo_f = _mm256_extractf128_ps(fa, 0);
        __m128 lo_out = _mm256_extractf128_ps(inf, 0);
        if (memcmp(&lo_f, &lo_out, sizeof lo_f) != 0) mismatches++;
        h = fold(&lo_out, sizeof lo_out, h);

        int li[8];
        memcpy(li, &ini, sizeof li);
        printf("ints %d %d %d %d | %d %d %d %d\n",
               li[0], li[1], li[2], li[3], li[4], li[5], li[6], li[7]);
    }

    printf("mismatches %d\n", mismatches);
    printf("%08x\n", h);
    return mismatches != 0;
}
