// FLAGS: -mavx2
/* Width-changing packed conversions and scalar rounding contracts. */
#include <immintrin.h>
#include <stdio.h>
#include <stdint.h>
#include <string.h>

static void dump(const char *tag, const void *p, unsigned n) {
    const unsigned char *b = p;
    printf("%s", tag);
    for (unsigned i = 0; i < n; ++i) printf(" %02x", b[i]);
    puts("");
}
#define CHECK(tag, ty, expr) do { ty out = (expr); dump(tag, &out, sizeof out); } while (0)
int main(void) {
    float f[8] = {-3.75f, -2.5f, -0.0f, 0.5f, 1.5f, 2.5f, 16777216.0f, 2147483520.0f};
    double d[4] = {-3.75, -2.5, 1.5, 2.5};
    int32_t i[8] = {INT32_MIN, -16777217, -1, 0, 1, 16777217, INT32_MAX, 123456789};
    __m128 s = _mm_loadu_ps(f); __m128d t = _mm_loadu_pd(d);
    __m128i u = _mm_loadu_si128((const __m128i_u *)i);
    __m256 v = _mm256_loadu_ps(f); __m256d w = _mm256_loadu_pd(d);
    __m256i x = _mm256_loadu_si256((const __m256i_u *)i);
    CHECK("ps_dq", __m128i, _mm_cvtps_epi32(s));
    CHECK("tps_dq", __m128i, _mm_cvttps_epi32(s));
    CHECK("dq_ps", __m128, _mm_cvtepi32_ps(u));
    CHECK("ps_pd", __m128d, _mm_cvtps_pd(s));
    CHECK("dq_pd", __m128d, _mm_cvtepi32_pd(u));
    CHECK("pd_ps", __m128, _mm_cvtpd_ps(t));
    CHECK("pd_dq", __m128i, _mm_cvtpd_epi32(t));
    CHECK("tpd_dq", __m128i, _mm_cvttpd_epi32(t));
    CHECK("vps_dq", __m256i, _mm256_cvtps_epi32(v));
    CHECK("vtps_dq", __m256i, _mm256_cvttps_epi32(v));
    CHECK("vdq_ps", __m256, _mm256_cvtepi32_ps(x));
    CHECK("vps_pd", __m256d, _mm256_cvtps_pd(s));
    CHECK("vdq_pd", __m256d, _mm256_cvtepi32_pd(u));
    CHECK("vpd_ps", __m128, _mm256_cvtpd_ps(w));
    CHECK("vpd_dq", __m128i, _mm256_cvtpd_epi32(w));
    CHECK("vtpd_dq", __m128i, _mm256_cvttpd_epi32(w));
    CHECK("i64_ss", __m128, _mm_cvtsi64_ss(s, 4294967297LL));
    CHECK("i64_sd", __m128d, _mm_cvtsi64_sd(t, -4294967297LL));
    CHECK("ss_sd", __m128d, _mm_cvtss_sd(t, s));
    CHECK("sd_ss", __m128, _mm_cvtsd_ss(s, t));
    printf("scalar %d %d %d %d\n", _mm_cvtss_si32(s), _mm_cvttss_si32(s),
           _mm_cvtsd_si32(t), _mm_cvttsd_si32(t));
    return 0;
}
