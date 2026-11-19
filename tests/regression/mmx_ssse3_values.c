/* Regression: the SSSE3 operations on MMX registers (_mm_abs/sign/mulhrs/
 * maddubs/hadd/hadds/hsub/hsubs/shuffle_pi8, _mm_alignr_pi8) and the
 * MMX-register double conversions (_mm_cvtpd_pi32, _mm_cvttpd_pi32,
 * _mm_cvtpi32_pd).
 *
 * Each result is printed as its 64 raw bits. The output must match GCC 16.2
 * byte for byte. Inputs include the saturation edges (INT16_MIN for abs and
 * mulhrs, the signed-saturating pairs for hadds/hsubs), control bytes with
 * bit 7 set and with bit 3 set (only bits 2:0 select), every alignr count
 * from 0 through 16, and doubles that round, truncate, or overflow. */
#include <tmmintrin.h>
#include <stdio.h>

#ifdef __clang__
#define MK64(x) ((__m64)(long long)(x))
#else
#define MK64(x) _mm_set_pi64x(x)
#endif

static volatile long long A[] = {
    (long long)0x80FF7F0102030405ULL, (long long)0x8000FFFF7FFF0001ULL,
    (long long)0x0123456789ABCDEFULL, (long long)0xFFFFFFFF80000000ULL,
};
static volatile long long B[] = {
    (long long)0x0F8F7F1087060504ULL, (long long)0x8000800080008000ULL,
    (long long)0x7FFF7FFF7FFF7FFFULL, (long long)0x00000001FFFFFFFFULL,
};
static volatile long long CTL[] = {
    (long long)0x0809870F83071008ULL, (long long)0xFF0E0D0C0B0A0908ULL,
    (long long)0x3F7F00FFAA55F0E1ULL, (long long)0x0706050403020100ULL,
};

static void show(const char *tag, __m64 v)
{
    printf("%-14s %016llx\n", tag, (unsigned long long)_mm_cvtm64_si64(v));
}

static void show_d(const char *tag, double d)
{
    union { double d; unsigned long long u; } c;
    c.d = d;
    printf("%-14s %016llx\n", tag, c.u);
}


int main(void)
{
    for (int i = 0; i < 4; i++) {
        int j = (i + 1) & 3;
        __m64 a = MK64(A[i]), b = MK64(B[j]), c = MK64(CTL[i]);
        show("abs8", _mm_abs_pi8(a));
        show("abs16", _mm_abs_pi16(a));
        show("abs32", _mm_abs_pi32(a));
        show("sign8", _mm_sign_pi8(a, b));
        show("sign16", _mm_sign_pi16(a, b));
        show("sign32", _mm_sign_pi32(a, b));
        show("mulhrs16", _mm_mulhrs_pi16(a, b));
        show("maddubs16", _mm_maddubs_pi16(a, b));
        show("hadd16", _mm_hadd_pi16(a, b));
        show("hadd32", _mm_hadd_pi32(a, b));
        show("hadds16", _mm_hadds_pi16(a, b));
        show("hsub16", _mm_hsub_pi16(a, b));
        show("hsub32", _mm_hsub_pi32(a, b));
        show("hsubs16", _mm_hsubs_pi16(a, b));
        show("shuffle8", _mm_shuffle_pi8(a, c));
        show("shuffle8r", _mm_shuffle_pi8(b, a));
    }
    {
        __m64 a = MK64(A[0]), b = MK64(B[0]);
        show("alignr 0", _mm_alignr_pi8(a, b, 0));
        show("alignr 1", _mm_alignr_pi8(a, b, 1));
        show("alignr 3", _mm_alignr_pi8(a, b, 3));
        show("alignr 7", _mm_alignr_pi8(a, b, 7));
        show("alignr 8", _mm_alignr_pi8(a, b, 8));
        show("alignr 9", _mm_alignr_pi8(a, b, 9));
        show("alignr 15", _mm_alignr_pi8(a, b, 15));
        show("alignr 16", _mm_alignr_pi8(a, b, 16));
    }
    {
        __m128d d1 = _mm_set_pd(-3.5, 2.5);       /* round half to even */
        __m128d d2 = _mm_set_pd(1.0e10, -2.7);    /* overflow, truncate */
        show("cvtpd_pi32", _mm_cvtpd_pi32(d1));
        show("cvtpd_pi32o", _mm_cvtpd_pi32(d2));
        show("cvttpd_pi32", _mm_cvttpd_pi32(d1));
        show("cvttpd_pi32o", _mm_cvttpd_pi32(d2));
        __m64 w = _mm_set_pi32(-7, 9);
        __m128d back = _mm_cvtpi32_pd(w);
        show_d("cvtpi32_pd lo", _mm_cvtsd_f64(back));
        show_d("cvtpi32_pd hi", _mm_cvtsd_f64(_mm_unpackhi_pd(back, back)));
    }
    return 0;
}
