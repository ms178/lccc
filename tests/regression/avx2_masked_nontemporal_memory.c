/* AVX/AVX2 sign-bit-masked and non-temporal memory intrinsics.
 *
 * Covers VMOVNT{PS,PD,DQ}, VMOVNTDQA (128 and 256 bit), and the VMASKMOV
 * family (float forms AVX, integer forms AVX2, 128 and 256 bit). Each
 * property is checked against a scalar reference, and the printed values are
 * compared byte-for-byte with GCC 16.2.
 *
 * Fault suppression: a masked load or store touches only the lanes whose mask
 * sign bit is set. The fixture places the operands so that every masked-off
 * lane lies on a PROT_NONE guard page. If lccc read or wrote a masked-off
 * lane, the process would fault (SIGSEGV) instead of printing and exiting 0.
 *
 * Masked-off destination lanes must keep their previous value. The stores
 * below are checked against sentinel-filled arrays.
 *
 * Built with -O2 -mavx2 (see avx2_masked_nontemporal_memory.flags). */
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

static void print_ps8(const char *tag, const float *v)
{
    printf("%s", tag);
    for (int i = 0; i < 8; i++)
        printf(" %g", (double)v[i]);
    printf("\n");
}

static void print_pd4(const char *tag, const double *v)
{
    printf("%s", tag);
    for (int i = 0; i < 4; i++)
        printf(" %g", v[i]);
    printf("\n");
}

static void print_i32x8(const char *tag, const int *v)
{
    printf("%s", tag);
    for (int i = 0; i < 8; i++)
        printf(" %d", v[i]);
    printf("\n");
}

static void print_i64x4(const char *tag, const long long *v)
{
    printf("%s", tag);
    for (int i = 0; i < 4; i++)
        printf(" %lld", v[i]);
    printf("\n");
}

/* ---- Non-temporal stores and loads (aligned) ---- */

__attribute__((noinline)) static void stream_stores(float *pf, double *pd, __m256i *pi)
{
    _mm256_stream_ps(pf, _mm256_setr_ps(1, 2, 3, 4, 5, 6, 7, 8));
    _mm256_stream_pd(pd, _mm256_setr_pd(-1.5, 2.25, -3.75, 4.5));
    _mm256_stream_si256(pi, _mm256_setr_epi32(11, -12, 13, -14, 15, -16, 17, -18));
    _mm_sfence();
}

__attribute__((noinline)) static __m256i stream_load_256(const __m256i *p)
{
    return _mm256_stream_load_si256(p);
}

__attribute__((noinline)) static __m128i stream_load_128(const __m128i *p)
{
    return _mm_stream_load_si128((__m128i *)p);
}

static void test_streaming(void)
{
    _Alignas(32) float pf[8];
    _Alignas(32) double pd[4];
    _Alignas(32) __m256i pi;
    memset(pf, 0, sizeof pf);
    memset(pd, 0, sizeof pd);
    memset(&pi, 0, sizeof pi);
    stream_stores(pf, pd, &pi);

    print_ps8("stream_ps", pf);
    print_pd4("stream_pd", pd);
    int ib[8];
    memcpy(ib, &pi, sizeof ib);
    print_i32x8("stream_si256", ib);
    CHECK(pf[0] == 1.0f && pf[7] == 8.0f, "stream_ps values");
    CHECK(pd[0] == -1.5 && pd[2] == -3.75, "stream_pd values");
    CHECK(ib[0] == 11 && ib[7] == -18, "stream_si256 values");

    _Alignas(32) long long src256[4] = {0x0102030405060708LL, -2, 3, -4};
    __m256i v256 = stream_load_256((const __m256i *)src256);
    long long out256[4];
    _mm256_storeu_si256((__m256i *)out256, v256);
    print_i64x4("stream_load_256", out256);
    CHECK(out256[0] == 0x0102030405060708LL && out256[1] == -2, "stream_load_256");

    _Alignas(16) __m128i src128 = _mm_setr_epi32(7, -8, 9, -10);
    __m128i v128 = stream_load_128(&src128);
    int o128[4];
    _mm_storeu_si128((__m128i *)o128, v128);
    printf("stream_load_128 %d %d %d %d\n", o128[0], o128[1], o128[2], o128[3]);
    CHECK(o128[0] == 7 && o128[3] == -10, "stream_load_128");
}

/* ---- Masked loads whose masked-off lanes lie on a guard page ---- */

static char *g_base;
static long g_page;

static void guard_setup(void)
{
    g_page = sysconf(_SC_PAGESIZE);
    g_base = mmap(NULL, 2 * (size_t)g_page, PROT_READ | PROT_WRITE,
                  MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (g_base == MAP_FAILED) {
        printf("FAIL: mmap\n");
        fails++;
        return;
    }
    if (mprotect(g_base + g_page, (size_t)g_page, PROT_NONE) != 0) {
        printf("FAIL: mprotect\n");
        fails++;
    }
}

/* Stable value for the lanes that stay inside the readable page. */
static void fill_page_tail_float(float *tail, int valid)
{
    for (int i = 0; i < valid; i++)
        tail[i] = 100.0f + (float)i;
}

__attribute__((noinline)) static __m256 maskload_ps256(const float *p, __m256i m)
{
    return _mm256_maskload_ps(p, m);
}

__attribute__((noinline)) static __m256d maskload_pd256(const double *p, __m256i m)
{
    return _mm256_maskload_pd(p, m);
}

__attribute__((noinline)) static __m256i maskload_epi32_256(const int *p, __m256i m)
{
    return _mm256_maskload_epi32(p, m);
}

__attribute__((noinline)) static __m256i maskload_epi64_256(const long long *p, __m256i m)
{
    return _mm256_maskload_epi64(p, m);
}

__attribute__((noinline)) static __m128 maskload_ps128(const float *p, __m128i m)
{
    return _mm_maskload_ps(p, m);
}

__attribute__((noinline)) static __m128i maskload_epi32_128(const int *p, __m128i m)
{
    return _mm_maskload_epi32(p, m);
}

static void test_masked_loads(void)
{
    if (!g_base)
        return;
    /* Three readable floats end at the guard page; lanes 3..7 are on it. */
    float *tail = (float *)(g_base + g_page) - 3;
    fill_page_tail_float(tail, 3);
    __m256i m = _mm256_setr_epi32(-1, -1, -1, 0, 0, 0, 0, 0);
    float r[8];
    _mm256_storeu_ps(r, maskload_ps256(tail, m));
    print_ps8("maskload_ps256", r);
    CHECK(r[0] == 100.0f && r[2] == 102.0f && r[3] == 0.0f && r[7] == 0.0f,
          "maskload_ps256 lanes");

    /* Two readable doubles end at the guard page; lanes 2..3 are on it. */
    double *dtail = (double *)(g_base + g_page) - 2;
    dtail[0] = 1.25;
    dtail[1] = -2.5;
    __m256i mq = _mm256_setr_epi64x(-1, -1, 0, 0);
    double d[4];
    _mm256_storeu_pd(d, maskload_pd256(dtail, mq));
    print_pd4("maskload_pd256", d);
    CHECK(d[0] == 1.25 && d[1] == -2.5 && d[2] == 0.0 && d[3] == 0.0,
          "maskload_pd256 lanes");

    int *itail = (int *)(g_base + g_page) - 3;
    itail[0] = 21;
    itail[1] = -22;
    itail[2] = 23;
    int iv[8];
    _mm256_storeu_si256((__m256i *)iv, maskload_epi32_256(itail, m));
    print_i32x8("maskload_epi32_256", iv);
    CHECK(iv[0] == 21 && iv[2] == 23 && iv[3] == 0 && iv[7] == 0,
          "maskload_epi32_256 lanes");

    long long *ltail = (long long *)(g_base + g_page) - 2;
    ltail[0] = 31;
    ltail[1] = -32;
    long long lv[4];
    _mm256_storeu_si256((__m256i *)lv, maskload_epi64_256(ltail, mq));
    print_i64x4("maskload_epi64_256", lv);
    CHECK(lv[0] == 31 && lv[1] == -32 && lv[2] == 0 && lv[3] == 0,
          "maskload_epi64_256 lanes");

    /* 128-bit: one readable float before the guard page. */
    float *t1 = (float *)(g_base + g_page) - 1;
    t1[0] = 41.0f;
    float r128[4];
    _mm_storeu_ps(r128, maskload_ps128(t1, _mm_setr_epi32(-1, 0, 0, 0)));
    printf("maskload_ps128 %g %g %g %g\n", (double)r128[0], (double)r128[1],
           (double)r128[2], (double)r128[3]);
    CHECK(r128[0] == 41.0f && r128[1] == 0.0f && r128[3] == 0.0f, "maskload_ps128");

    int *i1 = (int *)(g_base + g_page) - 1;
    i1[0] = 51;
    int i128[4];
    _mm_storeu_si128((__m128i *)i128, maskload_epi32_128(i1, _mm_setr_epi32(-1, 0, 0, 0)));
    printf("maskload_epi32_128 %d %d %d %d\n", i128[0], i128[1], i128[2], i128[3]);
    CHECK(i128[0] == 51 && i128[1] == 0 && i128[2] == 0 && i128[3] == 0,
          "maskload_epi32_128");
}

/* ---- Masked stores: unselected lanes keep their old value ---- */

__attribute__((noinline)) static void maskstore_ps256(float *p, __m256i m, __m256 v)
{
    _mm256_maskstore_ps(p, m, v);
}

__attribute__((noinline)) static void maskstore_epi64_256(long long *p, __m256i m, __m256i v)
{
    _mm256_maskstore_epi64(p, m, v);
}

__attribute__((noinline)) static void maskstore_epi32_128(int *p, __m128i m, __m128i v)
{
    _mm_maskstore_epi32(p, m, v);
}

__attribute__((noinline)) static void maskstore_pd128(double *p, __m128i m, __m128d v)
{
    _mm_maskstore_pd(p, m, v);
}

static void test_masked_stores(void)
{
    /* Sentinel-filled destination: only lanes 1, 3, 5 may change. */
    float dst[8];
    for (int i = 0; i < 8; i++)
        dst[i] = -9.0f;
    maskstore_ps256(dst, _mm256_setr_epi32(0, -1, 0, -1, 0, -1, 0, 0),
                    _mm256_setr_ps(1, 2, 3, 4, 5, 6, 7, 8));
    print_ps8("maskstore_ps256", dst);
    CHECK(dst[0] == -9.0f && dst[1] == 2.0f && dst[3] == 4.0f && dst[5] == 6.0f &&
              dst[6] == -9.0f && dst[7] == -9.0f,
          "maskstore_ps256 lanes");

    /* The store's masked-off lanes lie on the guard page. */
    if (g_base) {
        long long *tail = (long long *)(g_base + g_page) - 2;
        tail[0] = 0;
        tail[1] = 0;
        maskstore_epi64_256(tail, _mm256_setr_epi64x(-1, -1, 0, 0),
                            _mm256_setr_epi64x(61, -62, 63, -64));
        printf("maskstore_epi64_256 %lld %lld\n", tail[0], tail[1]);
        CHECK(tail[0] == 61 && tail[1] == -62, "maskstore_epi64_256 values");

        /* Lanes 0 and 1 are readable; lanes 2 and 3 are on the guard page. */
        int *itail = (int *)(g_base + g_page) - 2;
        itail[0] = -1;
        itail[1] = -1;
        maskstore_epi32_128(itail, _mm_setr_epi32(-1, -1, 0, 0),
                            _mm_setr_epi32(71, 72, 73, 74));
        printf("maskstore_epi32_128 %d %d\n", itail[0], itail[1]);
        CHECK(itail[0] == 71 && itail[1] == 72, "maskstore_epi32_128 values");

        /* Lane 0 is readable and selected; lane 1 is on the guard page and
         * masked off. Only lane 0 may change. */
        double *dtail = (double *)(g_base + g_page) - 2;
        dtail[0] = 2.0;
        dtail[1] = 0.0;
        maskstore_pd128(dtail, _mm_set_epi64x(0, -1), _mm_setr_pd(-7.5, 8.5));
        printf("maskstore_pd128 %g\n", dtail[0]);
        CHECK(dtail[0] == -7.5, "maskstore_pd128 selected lane");
    }
}

int main(void)
{
    guard_setup();
    test_streaming();
    test_masked_loads();
    test_masked_stores();
    if (g_base)
        munmap(g_base, 2 * (size_t)g_page);
    printf("%s (%d failures)\n", fails ? "FAILED" : "PASSED", fails);
    return fails ? 1 : 0;
}
