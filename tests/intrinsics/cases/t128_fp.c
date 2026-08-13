// FLAGS: -msse3 -mssse3 -msse4.1 -mfma
/* 128-bit FP intrinsic differential test (SSE/SSE3/SSSE3/SSE4.1/FMA) */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

static void dump128(const char *tag, __m128 v) {
    float out[4];
    _mm_storeu_ps(out, v);
    printf("%s:", tag);
    for (int i = 0; i < 4; i++) printf(" %.6g", (double)out[i]);
    printf("\n");
}
static void dump128d(const char *tag, __m128d v) {
    double out[2];
    _mm_storeu_pd(out, v);
    printf("%s: %.6g %.6g\n", tag, out[0], out[1]);
}

int main(void) {
    __m128 a = _mm_set_ps(4.0f, 3.0f, 2.0f, 1.0f);
    __m128 b = _mm_set_ps(2.0f, 2.0f, 2.0f, 2.0f);
    __m128d ad = _mm_set_pd(4.0, 1.0);
    __m128d bd = _mm_set_pd(2.0, 2.0);

    dump128("add", _mm_add_ps(a, b));
    dump128("sub", _mm_sub_ps(a, b));
    dump128("mul", _mm_mul_ps(a, b));
    dump128("div", _mm_div_ps(a, b));
    dump128("min", _mm_min_ps(a, b));
    dump128("max", _mm_max_ps(a, _mm_set_ps(1.5f, 1.5f, 1.5f, 1.5f)));
    dump128("sqrt", _mm_sqrt_ps(a));
    dump128("rcp", _mm_rcp_ps(b));
    dump128("rsqrt", _mm_rsqrt_ps(b));
    dump128("shuf", _mm_shuffle_ps(a, b, 0x1B));
    dump128("unpcklo", _mm_unpacklo_ps(a, b));
    dump128("unpckhi", _mm_unpackhi_ps(a, b));
    dump128("hadd", _mm_hadd_ps(a, b));
    dump128("hsub", _mm_hsub_ps(a, b));
    dump128("addsub", _mm_addsub_ps(a, b));
    dump128d("addpd", _mm_add_pd(ad, bd));
    dump128d("divpd", _mm_div_pd(ad, bd));
    dump128d("minpd", _mm_min_pd(ad, bd));
    dump128d("sqrtpd", _mm_sqrt_pd(ad));
    dump128d("shufpd", _mm_shuffle_pd(ad, bd, 1));
    dump128d("unpcklpd", _mm_unpacklo_pd(ad, bd));
    dump128d("haddpd", _mm_hadd_pd(ad, bd));

    printf("movemask_ps=%d\n", _mm_movemask_ps(_mm_set_ps(-1.0f, 1.0f, -1.0f, 1.0f)));
    printf("movemask_pd=%d\n", _mm_movemask_pd(_mm_set_pd(-1.0, 1.0)));

    /* compares */
    dump128("cmpeq", _mm_cmpeq_ps(a, _mm_set_ps(4.0f, 3.0f, 2.0f, 1.0f)));
    dump128("cmplt", _mm_cmplt_ps(a, _mm_set_ps(2.0f, 3.0f, 4.0f, 5.0f)));
    dump128("cmpord", _mm_cmpord_ps(a, _mm_set_ps(0.0f, 0.0f, 0.0f, 0.0f)));
    dump128("cmpneq", _mm_cmpneq_ps(a, b));
    dump128d("cmpeqpd", _mm_cmpeq_pd(ad, _mm_set_pd(4.0, 1.0)));
    dump128d("cmpltpd", _mm_cmplt_pd(ad, _mm_set_pd(4.0, 1.0)));

    /* converts */
    __m128i iv = _mm_cvtps_epi32(a);
    int iout[4];
    _mm_storeu_si128((__m128i_u *)iout, iv);
    printf("cvtps_epi32: %d %d %d %d\n", iout[0], iout[1], iout[2], iout[3]);
    iv = _mm_cvttps_epi32(a);
    _mm_storeu_si128((__m128i_u *)iout, iv);
    printf("cvttps_epi32: %d %d %d %d\n", iout[0], iout[1], iout[2], iout[3]);
    dump128("cvtepi32_ps", _mm_cvtepi32_ps(_mm_set_epi32(4, 3, 2, 1)));
    dump128("cvtps_pd", (__m128)_mm_cvtps_pd(a));
    dump128d("cvtpd_ps", (__m128d)_mm_cvtpd_ps(ad));

    /* round/blend/dp */
    dump128("round", _mm_round_ps(a, 0x8));
    dump128d("roundpd", _mm_round_pd(ad, 0x9));
    dump128("blend", _mm_blend_ps(a, b, 0x5));
    dump128d("blendpd", _mm_blend_pd(ad, bd, 1));
    __m128 mask = _mm_set_ps(-0.0f, 0.0f, -0.0f, 0.0f);
    dump128("blendv", _mm_blendv_ps(a, b, mask));
    dump128("dpps", _mm_dp_ps(a, b, 0xF1));
    dump128d("dppd", _mm_dp_pd(ad, bd, 0x31));

    /* insert/extract */
    dump128("insertps", _mm_insert_ps(a, b, 0x10));
    printf("extractps=%d\n", _mm_extract_ps(b, 1));

    /* scalar movs + converts */
    dump128("movess", _mm_move_ss(a, b));
    dump128d("movesd", _mm_move_sd(ad, bd));
    dump128("cvtss_sd", (__m128)_mm_cvtss_sd(ad, b));
    dump128("cvtsd_ss", _mm_cvtsd_ss(a, ad));
    dump128("cvtsi32_ss", _mm_cvtsi32_ss(a, 42));
    dump128("cvtsi64_ss", _mm_cvtsi64_ss(a, 4242424242LL));
    dump128("cvtsi32_sd", (__m128)_mm_cvtsi32_sd(ad, 42));
    printf("cvtss_si32=%d\n", _mm_cvtss_si32(a));
    printf("cvtsd_si32=%d\n", _mm_cvtsd_si32(ad));
    printf("cvttss_si32=%d\n", _mm_cvttss_si32(a));

    /* FMA (with -mfma) */
    dump128("fmadd", _mm_fmadd_ps(a, b, _mm_set_ps(1.0f, 1.0f, 1.0f, 1.0f)));
    dump128d("fmaddpd", _mm_fmadd_pd(ad, bd, _mm_set_pd(1.0, 1.0)));

    /* movddup/sldup/shdup */
    dump128("moveldup", _mm_moveldup_ps(a));
    dump128("movehdup", _mm_movehdup_ps(a));

    printf("done\n");
    return 0;
}
