// FLAGS: -mavx512f -mavx512bw -mavx512vl -mavx512dq
/* EVEX register widths, full narrowing results, and exact-width casts. */
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
    float f[16] = {-3.75f,-2.5f,-0.0f,0.5f,1.5f,2.5f,16777216.0f,2147483520.0f,
                   -128,127,0,1,4096,-4096,65536,-65536};
    double d[8] = {-3.75,-2.5,-0.0,0.5,1.5,2.5,16777217,2147483647};
    int32_t i[16] = {INT32_MIN,-16777217,-1,0,1,16777217,INT32_MAX,123456789,
                    -128,127,0,1,4096,-4096,65536,-65536};
    __m512 s; __m512d t; __m512i u; __m256 h; __m256i j;
    memcpy(&s,f,64); memcpy(&t,d,64); memcpy(&u,i,64);
    memcpy(&h,f,32); memcpy(&j,i,32);
    CHECK("ps_dq", __m512i, _mm512_cvtps_epi32(s));
    CHECK("tps_dq", __m512i, _mm512_cvttps_epi32(s));
    CHECK("dq_ps", __m512, _mm512_cvtepi32_ps(u));
    CHECK("ps_pd", __m512d, _mm512_cvtps_pd(h));
    CHECK("dq_pd", __m512d, _mm512_cvtepi32_pd(j));
    CHECK("pd_ps", __m256, _mm512_cvtpd_ps(t));
    CHECK("pd_dq", __m256i, _mm512_cvtpd_epi32(t));
    CHECK("tpd_dq", __m256i, _mm512_cvttpd_epi32(t));
    __m128i low = _mm_set_epi64x(0x0123456789abcdefLL, -123456789LL);
    CHECK("zero_extend", __m512i, _mm512_zextsi128_si512(low));
    /* Only the low 128 cast bits are defined; never compare the upper bits. */
    __m512i cast = _mm512_castsi128_si512(low);
    dump("cast_low", &cast, 16);
    __m512i cast_from256 = _mm512_castsi256_si512(j);
    dump("cast_from256_low", &cast_from256, 32);
    CHECK("cast256", __m256i, _mm512_castsi512_si256(u));
    unsigned char bytes[64]; for (unsigned k=0;k<64;++k) bytes[k]=(unsigned char)(k*7+3);
    __m128i old128 = _mm_set1_epi8(0x55);
    __m256i old256 = _mm256_set1_epi8(0x33);
    CHECK("merge128", __m128i, _mm_mask_loadu_epi8(old128, 0xa55a, bytes));
    CHECK("merge256", __m256i, _mm256_mask_loadu_epi8(old256, 0xa55a9669, bytes));
    return 0;
}
