/* Integer 128-bit lane wrappers under -mavx.
 *
 * _mm256_extractf128_si256 and _mm256_insertf128_si256 are AVX1 instructions
 * (vextractf128 / vinsertf128), declared by GCC 16.2 under -mavx. The sibling
 * fixture avx_lane128_float_builtins covers the ps/pd forms by differential
 * comparison; this one pins the integer forms against exact expected values,
 * so it needs no reference compiler and fails on any lane mix-up even if both
 * compilers agreed on the same wrong answer.
 *
 * Each case uses a distinct, recognisable value per lane so a swapped lane, a
 * dropped lane or a wrong element width changes the printed result. The lane
 * selector is a literal at every call site, so the immediate folds into the
 * instruction.
 *
 * Compiled with -O2 -mavx. Exit status is nonzero on any mismatch.
 */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

static int check_ints(const char *what, const int *got, const int *want, int n) {
    if (memcmp(got, want, (size_t)n * sizeof *got) == 0) return 0;
    printf("FAIL %s:", what);
    for (int i = 0; i < n; i++) printf(" %d/%d", got[i], want[i]);
    printf("\n");
    return 1;
}

int main(void) {
    static const int src[8] = {10, 11, 12, 13, 14, 15, 16, 17};
    static const int ins[4] = {90, 91, 92, 93};
    int failures = 0;

    __m256i a = _mm256_loadu_si256((const __m256i *)src);
    __m128i b = _mm_loadu_si128((const __m128i *)ins);

    /* extract: lane 0 is the low half, lane 1 the high half. */
    int got[4];
    _mm_storeu_si128((__m128i *)got, _mm256_extractf128_si256(a, 0));
    failures += check_ints("extract lane0", got, src, 4);
    _mm_storeu_si128((__m128i *)got, _mm256_extractf128_si256(a, 1));
    failures += check_ints("extract lane1", got, src + 4, 4);

    /* insert: replaces one half and leaves the other bit-exact. */
    int out[8];
    _mm256_storeu_si256((__m256i *)out, _mm256_insertf128_si256(a, b, 0));
    {
        int want[8] = {90, 91, 92, 93, 14, 15, 16, 17};
        failures += check_ints("insert lane0", out, want, 8);
    }
    _mm256_storeu_si256((__m256i *)out, _mm256_insertf128_si256(a, b, 1));
    {
        int want[8] = {10, 11, 12, 13, 90, 91, 92, 93};
        failures += check_ints("insert lane1", out, want, 8);
    }

    /* Round-trip: extracting the lane we just inserted returns the input. */
    _mm_storeu_si128((__m128i *)got,
                     _mm256_extractf128_si256(_mm256_insertf128_si256(a, b, 1), 1));
    failures += check_ints("round-trip lane1", got, ins, 4);

    printf("%s (%d failures)\n", failures ? "FAIL" : "PASS", failures);
    return failures != 0;
}
