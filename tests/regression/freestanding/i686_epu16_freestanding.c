/* Freestanding i686 runtime check for the 16-bit packed min/max intrinsics.
 * Modes (select with -D):
 *   (default)                  _mm_max_epu16 / _mm_min_epu16    (SSE4.1)
 *   -DUSE_AVX2                 _mm256_max_epu16 / _mm256_min_epu16 (AVX2)
 *   -DSIGNED16                 _mm_max_epi16 / _mm_min_epi16    (SSE2)
 *   -DSIGNED16 -DUSE_AVX2      _mm256_max_epi16 / _mm256_min_epi16 (AVX2)
 * No libc: the program exits 0 when every lane equals the scalar max/min of
 * the selected signedness, and 1 otherwise. The gate script
 * (check_intrinsic_feature_gate.sh) builds each mode with lccc -m32 -nostdlib
 * and runs it; exit codes 126/127 mean the host cannot execute 32-bit programs.
 *
 * Lanes cover the sign-bit boundary (0x7FFF/0x8000), where a signed and an
 * unsigned compare disagree, plus all-ones, zero, and pseudo-random data. */
#include <immintrin.h>

#ifdef SIGNED16
typedef short elem_t;
#else
typedef unsigned short elem_t;
#endif

static elem_t A[512] __attribute__((aligned(32)));
static elem_t B[512] __attribute__((aligned(32)));
static elem_t R[512] __attribute__((aligned(32)));

static void sys_exit(long code) {
    __asm__ volatile("int $0x80" : : "a"(1L), "b"(code) : "memory");
    for (;;) {
    }
}

static unsigned lcg(unsigned *s) {
    *s = *s * 1103515245u + 12345u;
    return *s >> 8;
}

void _start(void) {
    unsigned seed = 0x1234567u;
    static const elem_t edge[8] = {0, 1, 0x7FFF, (elem_t)0x8000, (elem_t)0x8001, (elem_t)0xFFFF,
                                   (elem_t)0xFFFE, 0x00FF};
    for (int i = 0; i < 512; i++) {
        A[i] = (elem_t)lcg(&seed);
        B[i] = (elem_t)lcg(&seed);
    }
    for (int i = 0; i < 64; i++) {
        A[i] = edge[i & 7];
        B[i] = edge[(i >> 3) & 7];
    }
    long bad = 0;

#if defined(USE_AVX2) && defined(SIGNED16)
#define STEP 16
#define LOAD(p) _mm256_load_si256((const __m256i *)(p))
#define STORE(p, v) _mm256_store_si256((__m256i *)(p), (v))
#define MAX(a, b) _mm256_max_epi16((a), (b))
#define MIN(a, b) _mm256_min_epi16((a), (b))
#elif defined(USE_AVX2)
#define STEP 16
#define LOAD(p) _mm256_load_si256((const __m256i *)(p))
#define STORE(p, v) _mm256_store_si256((__m256i *)(p), (v))
#define MAX(a, b) _mm256_max_epu16((a), (b))
#define MIN(a, b) _mm256_min_epu16((a), (b))
#elif defined(SIGNED16)
#define STEP 8
#define LOAD(p) _mm_load_si128((const __m128i *)(p))
#define STORE(p, v) _mm_store_si128((__m128i *)(p), (v))
#define MAX(a, b) _mm_max_epi16((a), (b))
#define MIN(a, b) _mm_min_epi16((a), (b))
#else
#define STEP 8
#define LOAD(p) _mm_load_si128((const __m128i *)(p))
#define STORE(p, v) _mm_store_si128((__m128i *)(p), (v))
#define MAX(a, b) _mm_max_epu16((a), (b))
#define MIN(a, b) _mm_min_epu16((a), (b))
#endif

    for (int i = 0; i < 512; i += STEP)
        STORE(&R[i], MAX(LOAD(&A[i]), LOAD(&B[i])));
    for (int i = 0; i < 512; i++)
        if (R[i] != (A[i] > B[i] ? A[i] : B[i]))
            bad++;
    for (int i = 0; i < 512; i += STEP)
        STORE(&R[i], MIN(LOAD(&A[i]), LOAD(&B[i])));
    for (int i = 0; i < 512; i++)
        if (R[i] != (A[i] < B[i] ? A[i] : B[i]))
            bad++;

    sys_exit(bad ? 1 : 0);
}
