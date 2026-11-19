/* AVX1 masked load/store and streaming store, values checked at runtime.
 *
 * Covers the 11 AVX1-level intrinsics that the masked/non-temporal work adds:
 *   _mm256_maskload_{ps,pd}, _mm256_maskstore_{ps,pd},
 *   _mm_maskload_{ps,pd},    _mm_maskstore_{ps,pd},
 *   _mm256_stream_{ps,pd,si256}.
 *
 * The mask is the per-element sign bit. This fixture checks values: masked-off
 * loads read as zero, masked-off stores keep the old contents. It does not
 * test fault suppression. Output is compared byte-for-byte with GCC 16.2 and
 * Clang 23.1.5.
 */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

static void print_f(const char *tag, const float *v, int n) {
    printf("%s", tag);
    for (int i = 0; i < n; i++) printf(" %.1f", (double)v[i]);
    printf("\n");
}

static void print_d(const char *tag, const double *v, int n) {
    printf("%s", tag);
    for (int i = 0; i < n; i++) printf(" %.1f", v[i]);
    printf("\n");
}

static void print_i(const char *tag, const long long *v, int n) {
    printf("%s", tag);
    for (int i = 0; i < n; i++) printf(" %lld", v[i]);
    printf("\n");
}

/* Lanes with a negative mask load the value; the rest read as zero. */
__attribute__((noinline)) static void maskload_ps256(const float *src, float *dst) {
    __m256i m = _mm256_setr_epi32(-1, 0, -1, 0, -1, 0, -1, -1);
    __m256 v = _mm256_maskload_ps(src, m);
    _mm256_storeu_ps(dst, v);
}

__attribute__((noinline)) static void maskload_pd256(const double *src, double *dst) {
    __m256i m = _mm256_setr_epi64x(-1, 0, -1, 0);
    __m256d v = _mm256_maskload_pd(src, m);
    _mm256_storeu_pd(dst, v);
}

__attribute__((noinline)) static void maskload_ps128(const float *src, float *dst) {
    __m128i m = _mm_setr_epi32(0, -1, 0, -1);
    __m128 v = _mm_maskload_ps(src, m);
    _mm_storeu_ps(dst, v);
}

__attribute__((noinline)) static void maskload_pd128(const double *src, double *dst) {
    __m128i m = _mm_set_epi64x(0, -1);
    __m128d v = _mm_maskload_pd(src, m);
    _mm_storeu_pd(dst, v);
}

/* Masked store: only lanes with a negative mask are written. */
__attribute__((noinline)) static void maskstore_ps256(float *dst, const float *val) {
    __m256i m = _mm256_setr_epi32(0, -1, 0, -1, 0, -1, 0, -1);
    _mm256_maskstore_ps(dst, m, _mm256_loadu_ps(val));
}

__attribute__((noinline)) static void maskstore_pd256(double *dst, const double *val) {
    __m256i m = _mm256_setr_epi64x(-1, 0, 0, -1);
    _mm256_maskstore_pd(dst, m, _mm256_loadu_pd(val));
}

__attribute__((noinline)) static void maskstore_ps128(float *dst, const float *val) {
    __m128i m = _mm_setr_epi32(-1, 0, 0, -1);
    _mm_maskstore_ps(dst, m, _mm_loadu_ps(val));
}

__attribute__((noinline)) static void maskstore_pd128(double *dst, const double *val) {
    __m128i m = _mm_set_epi64x(-1, 0);
    _mm_maskstore_pd(dst, m, _mm_loadu_pd(val));
}

/* Streaming stores: 32-byte aligned destinations, fully written. */
__attribute__((noinline)) static void stream_all(float *fdst, double *ddst, long long *idst,
                                                 const float *fsrc, const double *dsrc,
                                                 const long long *isrc) {
    _mm256_stream_ps(fdst, _mm256_loadu_ps(fsrc));
    _mm256_stream_pd(ddst, _mm256_loadu_pd(dsrc));
    _mm256_stream_si256((__m256i *)idst, _mm256_loadu_si256((const __m256i *)isrc));
    _mm_sfence();
}

int main(void) {
    /* 32-byte aligned sources and destinations. */
    static float fsrc[8] __attribute__((aligned(32)));
    static double dsrc[4] __attribute__((aligned(32)));
    static float fout[8] __attribute__((aligned(32)));
    static double dout[4] __attribute__((aligned(32)));
    static long long lsrc[4] __attribute__((aligned(32)));
    static float fdst_stream[8] __attribute__((aligned(32)));
    static double ddst_stream[4] __attribute__((aligned(32)));
    static long long ldst_stream[4] __attribute__((aligned(32)));

    for (int i = 0; i < 8; i++) fsrc[i] = (float)(100 + i);
    for (int i = 0; i < 4; i++) dsrc[i] = (double)(200 + i);
    for (int i = 0; i < 4; i++) lsrc[i] = 300 + i;

    /* Loads: masked-off lanes must come out zero (and not be read). */
    memset(fout, 0, sizeof fout);
    maskload_ps256(fsrc, fout);
    print_f("maskload_ps256", fout, 8);

    memset(dout, 0, sizeof dout);
    maskload_pd256(dsrc, dout);
    print_d("maskload_pd256", dout, 4);

    memset(fout, 0, sizeof fout);
    maskload_ps128(fsrc, fout);
    print_f("maskload_ps128", fout, 4);

    memset(dout, 0, sizeof dout);
    maskload_pd128(dsrc, dout);
    print_d("maskload_pd128", dout, 2);

    /* Stores: masked-off lanes keep their previous contents. */
    float fdst[8];
    for (int i = 0; i < 8; i++) fdst[i] = -9.0f;
    maskstore_ps256(fdst, fsrc);
    print_f("maskstore_ps256", fdst, 8);

    double ddst[4] = {-9.0, -9.0, -9.0, -9.0};
    maskstore_pd256(ddst, dsrc);
    print_d("maskstore_pd256", ddst, 4);

    float fdst4[4] = {-9.0f, -9.0f, -9.0f, -9.0f};
    maskstore_ps128(fdst4, fsrc);
    print_f("maskstore_ps128", fdst4, 4);

    double ddst2[2] = {-9.0, -9.0};
    maskstore_pd128(ddst2, dsrc);
    print_d("maskstore_pd128", ddst2, 2);

    /* Streaming stores, then read back after the fence. */
    stream_all(fdst_stream, ddst_stream, ldst_stream, fsrc, dsrc, lsrc);
    print_f("stream_ps256", fdst_stream, 8);
    print_d("stream_pd256", ddst_stream, 4);
    print_i("stream_si256", ldst_stream, 4);

    return 0;
}
