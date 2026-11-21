#include <immintrin.h>
#include <stdio.h>

/* Two global 32-byte vectors read by a 256-bit VPCMPB into a mask register.
 * Each __m256i local is a copy of a loadu from a global. The mask compare
 * must read the globals' bytes directly (vpcmpb $0, B(%rip), %ymm0). Before
 * the fix the locals stayed in memory: the runtime code grew by 2.3 KB on the
 * 485-case AVX-512VL matrix (5136 -> 7440 bytes; GCC 16.2: 5308). */
static unsigned char A[64] __attribute__((aligned(32)));
static unsigned char B[64] __attribute__((aligned(32)));

__attribute__((noinline)) static unsigned long long eq_bytes(void) {
    __m256i a = _mm256_loadu_si256((const __m256i *)(const void *)A);
    __m256i b = _mm256_loadu_si256((const __m256i *)(const void *)B);
    return (unsigned long long)((__mmask32)_mm256_cmpeq_epi8_mask(a, b));
}

int main(void) {
    /* Runtime execution is capability-gated, as in avx10_1_avx512_runtime.c:
     * on a host without AVX-512 F/BW/VL the program prints the same skip line
     * the GCC reference prints, so the two outputs still compare equal. */
    __builtin_cpu_init();
    if (!__builtin_cpu_supports("avx512f") || !__builtin_cpu_supports("avx512bw") ||
        !__builtin_cpu_supports("avx512vl")) {
        printf("SKIP host lacks AVX-512 F/BW/VL\n");
        return 0;
    }
    for (int i = 0; i < 64; i++) {
        A[i] = (unsigned char)(i * 7 + 11);
        B[i] = (unsigned char)((i % 3) ? A[i] : (A[i] ^ 0x5A));
    }
    printf("%016llx\n", eq_bytes());
    B[0] = A[0];
    printf("%016llx\n", eq_bytes());
    return 0;
}
