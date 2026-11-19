/* AVX1-only (-mavx, no AVX2) non-temporal and masked memory intrinsics.
 *
 * Under -mavx the 128-bit streaming stores and the streaming load must use the
 * VEX.128 spellings (VMOVNT*, VMOVNTDQA xmm, VMOVDQU, VMOVUPS). A legacy SSE
 * instruction there would mix with the dirty upper YMM state of the 256-bit
 * code in the same function. This fixture exercises every 128-bit streaming
 * store, the 128-bit streaming load, and the AVX1 float/double masked forms,
 * and compares every printed value with GCC 16.2.
 *
 * Fault suppression is checked with a PROT_NONE guard page: masked-off lanes
 * sit on it, so any access to them would fault instead of exiting 0.
 *
 * Built with -O2 -mavx (see avx1_nontemporal_masked_vex.flags). */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

static int fails;

#define CHECK(cond, what)                                                   \
    do {                                                                    \
        if (!(cond)) {                                                      \
            printf("FAIL: %s (line %d)\n", (what), __LINE__);               \
            fails++;                                                        \
        }                                                                   \
    } while (0)

/* 16-byte aligned destinations: the 128-bit streaming stores fault otherwise. */
static _Alignas(32) float g_f[8];
static _Alignas(32) double g_d[4];
static _Alignas(32) int g_i[8];

__attribute__((noinline)) static void stream128(float *pf, double *pd, int *pi)
{
    _mm_stream_ps(pf, _mm_setr_ps(1.5f, -2.5f, 3.5f, -4.5f));
    _mm_stream_pd(pd, _mm_setr_pd(6.25, -7.75));
    _mm_stream_si128((__m128i *)pi, _mm_setr_epi32(8, -9, 10, -11));
    _mm_sfence();
}

__attribute__((noinline)) static __m128i load128(const __m128i *p)
{
    return _mm_stream_load_si128((__m128i *)p);
}

__attribute__((noinline)) static void stream256_ps(float *p, __m256 v)
{
    _mm256_stream_ps(p, v);
    _mm_sfence();
}

__attribute__((noinline)) static __m128 maskload_ps128(const float *p, __m128i m)
{
    return _mm_maskload_ps(p, m);
}

__attribute__((noinline)) static __m128d maskload_pd128(const double *p, __m128i m)
{
    return _mm_maskload_pd(p, m);
}

__attribute__((noinline)) static __m256 maskload_ps256(const float *p, __m256i m)
{
    return _mm256_maskload_ps(p, m);
}

__attribute__((noinline)) static __m256d maskload_pd256(const double *p, __m256i m)
{
    return _mm256_maskload_pd(p, m);
}

__attribute__((noinline)) static void maskstore_ps128(float *p, __m128i m, __m128 v)
{
    _mm_maskstore_ps(p, m, v);
}

__attribute__((noinline)) static void maskstore_pd256(double *p, __m256i m, __m256d v)
{
    _mm256_maskstore_pd(p, m, v);
}

int main(void)
{
    /* Streaming stores, then a streaming load of the same data. */
    memset(g_f, 0, sizeof g_f);
    memset(g_d, 0, sizeof g_d);
    memset(g_i, 0, sizeof g_i);
    stream128(g_f, g_d, g_i);
    printf("stream_ps %g %g %g %g\n", (double)g_f[0], (double)g_f[1],
           (double)g_f[2], (double)g_f[3]);
    printf("stream_pd %g %g\n", g_d[0], g_d[1]);
    printf("stream_si128 %d %d %d %d\n", g_i[0], g_i[1], g_i[2], g_i[3]);
    CHECK(g_f[0] == 1.5f && g_f[3] == -4.5f, "stream_ps values");
    CHECK(g_d[0] == 6.25 && g_d[1] == -7.75, "stream_pd values");
    CHECK(g_i[0] == 8 && g_i[3] == -11, "stream_si128 values");

    __m128i v = load128((const __m128i *)g_i);
    int back[4];
    _mm_storeu_si128((__m128i *)back, v);
    printf("stream_load_si128 %d %d %d %d\n", back[0], back[1], back[2], back[3]);
    CHECK(back[0] == 8 && back[1] == -9 && back[2] == 10 && back[3] == -11,
          "stream_load_si128");

    /* 256-bit float stream, 32-byte aligned. */
    _Alignas(32) float s8[8];
    stream256_ps(s8, _mm256_setr_ps(1, 2, 3, 4, 5, 6, 7, 8));
    printf("stream256_ps %g %g %g %g %g %g %g %g\n", (double)s8[0], (double)s8[1],
           (double)s8[2], (double)s8[3], (double)s8[4], (double)s8[5],
           (double)s8[6], (double)s8[7]);
    CHECK(s8[0] == 1.0f && s8[7] == 8.0f, "stream256_ps values");

    /* Masked loads with masked-off lanes on a guard page. */
    long page = sysconf(_SC_PAGESIZE);
    char *base = mmap(NULL, 2 * (size_t)page, PROT_READ | PROT_WRITE,
                      MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (base == MAP_FAILED) {
        printf("FAIL: mmap\n");
        return 1;
    }
    if (mprotect(base + page, (size_t)page, PROT_NONE) != 0) {
        printf("FAIL: mprotect\n");
        return 1;
    }

    /* 128-bit float: one readable lane before the guard page. */
    float *f1 = (float *)(base + page) - 1;
    f1[0] = 31.0f;
    float r4[4];
    _mm_storeu_ps(r4, maskload_ps128(f1, _mm_setr_epi32(-1, 0, 0, 0)));
    printf("maskload_ps128 %g %g %g %g\n", (double)r4[0], (double)r4[1],
           (double)r4[2], (double)r4[3]);
    CHECK(r4[0] == 31.0f && r4[1] == 0.0f && r4[3] == 0.0f, "maskload_ps128");

    /* 128-bit double: lane 0 readable, lane 1 on the guard page. */
    double *d1 = (double *)(base + page) - 1;
    d1[0] = -3.5;
    double r2[2];
    _mm_storeu_pd(r2, maskload_pd128(d1, _mm_set_epi64x(0, -1)));
    printf("maskload_pd128 %g %g\n", r2[0], r2[1]);
    CHECK(r2[0] == -3.5 && r2[1] == 0.0, "maskload_pd128");

    /* 256-bit float: three readable lanes, five on the guard page. */
    float *f3 = (float *)(base + page) - 3;
    f3[0] = 41.0f;
    f3[1] = 42.0f;
    f3[2] = 43.0f;
    float r8[8];
    _mm256_storeu_ps(r8, maskload_ps256(f3, _mm256_setr_epi32(-1, -1, -1, 0, 0, 0, 0, 0)));
    printf("maskload_ps256 %g %g %g %g\n", (double)r8[0], (double)r8[1],
           (double)r8[2], (double)r8[3]);
    CHECK(r8[0] == 41.0f && r8[2] == 43.0f && r8[3] == 0.0f && r8[7] == 0.0f,
          "maskload_ps256");

    /* 256-bit double: two readable lanes, two on the guard page. */
    double *d2 = (double *)(base + page) - 2;
    d2[0] = 1.25;
    d2[1] = -2.5;
    double r4d[4];
    _mm256_storeu_pd(r4d, maskload_pd256(d2, _mm256_setr_epi64x(-1, -1, 0, 0)));
    printf("maskload_pd256 %g %g %g %g\n", r4d[0], r4d[1], r4d[2], r4d[3]);
    CHECK(r4d[0] == 1.25 && r4d[1] == -2.5 && r4d[2] == 0.0 && r4d[3] == 0.0,
          "maskload_pd256");

    /* Masked stores: unselected lanes keep their sentinel value. */
    float ms[4] = {-9.0f, -9.0f, -9.0f, -9.0f};
    maskstore_ps128(ms, _mm_setr_epi32(0, -1, 0, -1), _mm_setr_ps(1, 2, 3, 4));
    printf("maskstore_ps128 %g %g %g %g\n", (double)ms[0], (double)ms[1],
           (double)ms[2], (double)ms[3]);
    CHECK(ms[0] == -9.0f && ms[1] == 2.0f && ms[2] == -9.0f && ms[3] == 4.0f,
          "maskstore_ps128");

    /* 256-bit double store: lanes 0 and 1 readable, 2 and 3 on the guard. */
    double *d3 = (double *)(base + page) - 2;
    d3[0] = 0.0;
    d3[1] = 0.0;
    maskstore_pd256(d3, _mm256_setr_epi64x(-1, 0, 0, 0),
                    _mm256_setr_pd(-7.5, 8.5, 9.5, 10.5));
    printf("maskstore_pd256 %g %g\n", d3[0], d3[1]);
    CHECK(d3[0] == -7.5 && d3[1] == 0.0, "maskstore_pd256");

    munmap(base, 2 * (size_t)page);
    printf("%s (%d failures)\n", fails ? "FAILED" : "PASSED", fails);
    return fails ? 1 : 0;
}
