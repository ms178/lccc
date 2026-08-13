// FLAGS: -mavx -mfma
/* 256-bit FP intrinsic differential test (AVX/AVX2/FMA) */
#include <immintrin.h>
#include <stdio.h>

static void dump256(const char *tag, __m256 v) {
    float out[8];
    _mm256_storeu_ps(out, v);
    printf("%s:", tag);
    for (int i = 0; i < 8; i++) printf(" %.6g", (double)out[i]);
    printf("\n");
}
static void dump256d(const char *tag, __m256d v) {
    double out[4];
    _mm256_storeu_pd(out, v);
    printf("%s: %.6g %.6g %.6g %.6g\n", tag, out[0], out[1], out[2], out[3]);
}

int main(void) {
    __m256 a = _mm256_set_ps(8, 7, 6, 5, 4, 3, 2, 1);
    __m256 b = _mm256_set1_ps(2.0f);
    __m256d ad = _mm256_set_pd(4.0, 3.0, 2.0, 1.0);
    __m256d bd = _mm256_set1_pd(2.0);

    dump256("add", _mm256_add_ps(a, b));
    dump256("sub", _mm256_sub_ps(a, b));
    dump256("mul", _mm256_mul_ps(a, b));
    dump256("div", _mm256_div_ps(a, b));
    dump256("min", _mm256_min_ps(a, b));
    dump256("max", _mm256_max_ps(a, _mm256_set1_ps(5.0f)));
    dump256("sqrt", _mm256_sqrt_ps(a));
    dump256("shuf", _mm256_shuffle_ps(a, b, 0x1B));
    dump256("unpcklo", _mm256_unpacklo_ps(a, b));
    dump256("unpckhi", _mm256_unpackhi_ps(a, b));
    dump256("hadd", _mm256_hadd_ps(a, b));
    dump256("hsub", _mm256_hsub_ps(a, b));
    dump256("addsub", _mm256_addsub_ps(a, b));
    dump256d("addpd", _mm256_add_pd(ad, bd));
    dump256d("divpd", _mm256_div_pd(ad, bd));
    dump256d("minpd", _mm256_min_pd(ad, bd));
    dump256d("sqrtpd", _mm256_sqrt_pd(ad));
    dump256d("shufpd", _mm256_shuffle_pd(ad, bd, 0x5));
    dump256d("unpcklpd", _mm256_unpacklo_pd(ad, bd));
    dump256d("haddpd", _mm256_hadd_pd(ad, bd));

    printf("movemask_ps=%d\n", _mm256_movemask_ps(_mm256_set_ps(-1, 1, -1, 1, -1, 1, -1, 1)));
    printf("movemask_pd=%d\n", _mm256_movemask_pd(_mm256_set_pd(-1.0, 1.0, -1.0, 1.0)));

    dump256("cmpeq", _mm256_cmp_ps(a, _mm256_set_ps(8, 7, 6, 5, 4, 3, 2, 1), 0));
    dump256("cmplt", _mm256_cmp_ps(a, _mm256_set1_ps(5.0f), 1));
    dump256("cmpord", _mm256_cmp_ps(a, _mm256_setzero_ps(), 7));
    dump256("cmpneq", _mm256_cmp_ps(a, b, 4));
    dump256d("cmpeqpd", _mm256_cmp_pd(ad, _mm256_set_pd(4.0, 3.0, 2.0, 1.0), 0));
    dump256d("cmpltpd", _mm256_cmp_pd(ad, _mm256_set1_pd(3.0), 1));

    /* converts */
    __m256i iv = _mm256_cvtps_epi32(a);
    int iout[8];
    _mm256_storeu_si256((__m256i_u *)iout, iv);
    printf("cvtps_epi32: %d %d %d %d %d %d %d %d\n",
           iout[0], iout[1], iout[2], iout[3], iout[4], iout[5], iout[6], iout[7]);
    iv = _mm256_cvttps_epi32(a);
    _mm256_storeu_si256((__m256i_u *)iout, iv);
    printf("cvttps_epi32: %d %d %d %d\n", iout[0], iout[1], iout[2], iout[3]);
    dump256("cvtepi32_ps", _mm256_cvtepi32_ps(_mm256_set_epi32(8, 7, 6, 5, 4, 3, 2, 1)));
    dump256d("cvtps_pd", _mm256_cvtps_pd(_mm256_castps256_ps128(a)));
    __m128 cpd = _mm256_cvtpd_ps(ad);
    float cp[4]; _mm_storeu_ps(cp, cpd);
    printf("cvtpd_ps: %.6g %.6g %.6g %.6g\n", (double)cp[0], (double)cp[1], (double)cp[2], (double)cp[3]);

    /* round/blend/permute */
    dump256("round", _mm256_round_ps(a, 0x8));
    dump256d("roundpd", _mm256_round_pd(ad, 0x9));
    dump256("blend", _mm256_blend_ps(a, b, 0x55));
    dump256d("blendpd", _mm256_blend_pd(ad, bd, 0x5));
    dump256("permute", _mm256_permute_ps(a, 0x1B));
    dump256("permutevar", _mm256_permutevar_ps(a, _mm256_set_epi32(7, 6, 5, 4, 3, 2, 1, 0)));
    __m256 mask = _mm256_set_ps(-0.0f, 0.0f, -0.0f, 0.0f, -0.0f, 0.0f, -0.0f, 0.0f);
    dump256("blendv", _mm256_blendv_ps(a, b, mask));
    dump256d("blendvpd", _mm256_blendv_pd(ad, bd, _mm256_set_pd(-0.0, 0.0, -0.0, 0.0)));

    /* permute2f128 / insert / extract */
    dump256("perm2f128", _mm256_permute2f128_ps(a, b, 0x20));
    dump256("insertf128", _mm256_insertf128_ps(a, _mm_set_ps(9, 9, 9, 9), 1));
    __m128 ex = _mm256_extractf128_ps(a, 1);
    float eo[4]; _mm_storeu_ps(eo, ex);
    printf("extractf128: %.0f %.0f %.0f %.0f\n", (double)eo[0], (double)eo[1], (double)eo[2], (double)eo[3]);

    /* FMA */
    dump256("fmadd", _mm256_fmadd_ps(a, b, _mm256_set1_ps(1.0f)));
    dump256d("fmaddpd", _mm256_fmadd_pd(ad, bd, _mm256_set1_pd(1.0)));

    printf("done\n");
    return 0;
}
