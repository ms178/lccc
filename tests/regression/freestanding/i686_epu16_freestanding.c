/* Freestanding i686 runtime check for the unsigned 16-bit min/max intrinsics:
 * _mm256_max_epu16 / _mm256_min_epu16 (AVX2, -DUSE_AVX2) and _mm_max_epu16 /
 * _mm_min_epu16 (SSE4.1, the default). No libc: the program exits 0 when every
 * lane equals the scalar unsigned max/min, and 1 otherwise. The gate script
 * (check_intrinsic_feature_gate.sh) builds it with lccc -m32 -nostdlib and runs
 * it; exit codes 126/127 mean the host cannot execute 32-bit programs at all.
 *
 * Lanes cover the sign-bit boundary (0x7FFF/0x8000), where a signed compare
 * would disagree, plus all-ones and zero, and pseudo-random data. */
#include <immintrin.h>

static unsigned short A[512] __attribute__((aligned(32)));
static unsigned short B[512] __attribute__((aligned(32)));
static unsigned short R[512] __attribute__((aligned(32)));

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
    static const unsigned short edge[8] = {0, 1, 0x7FFF, 0x8000, 0x8001, 0xFFFF, 0xFFFE, 0x00FF};
    for (int i = 0; i < 512; i++) {
        A[i] = (unsigned short)lcg(&seed);
        B[i] = (unsigned short)lcg(&seed);
    }
    for (int i = 0; i < 64; i++) {
        A[i] = edge[i & 7];
        B[i] = edge[(i >> 3) & 7];
    }
    long bad = 0;

#ifdef USE_AVX2
#define STEP 16
#define LOAD(p) _mm256_load_si256((const __m256i *)(p))
#define STORE(p, v) _mm256_store_si256((__m256i *)(p), (v))
#define MAX(a, b) _mm256_max_epu16((a), (b))
#define MIN(a, b) _mm256_min_epu16((a), (b))
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
