/* AVX2 signed 16->8 pack fixture (_mm256_packs_epi16, VPACKSSWB, 256-bit).
 *
 * Pins the value semantics of the 256-bit signed-saturating pack that was
 * undeclared under -mavx2 before this change:
 *   - each 128-bit half packs independently: result bytes 0..7 come from a's
 *     low half, 8..15 from b's low half, 16..23 from a's high half, 24..31
 *     from b's high half;
 *   - saturation at both ends: values <= -129 clamp to -128, values >= 128
 *     clamp to 127, and -128..127 pass through unchanged.
 * The vector result is checked lane-by-lane against a scalar reference, so the
 * binary is self-checking; it also must print the same bytes as GCC 16.2.
 * Built with -mavx2 (see avx2_packs_epi16_values.flags). */
#include <immintrin.h>
#include <stdio.h>

static void reference_packs(const short *a, const short *b, signed char *out)
{
    /* out[0..7]=a[0..7], out[8..15]=b[0..7], out[16..23]=a[8..15], out[24..31]=b[8..15]. */
    for (int half = 0; half < 2; half++) {
        for (int i = 0; i < 8; i++) {
            short sa = a[half * 8 + i];
            short sb = b[half * 8 + i];
            out[half * 16 + i] = (signed char)(sa > 127 ? 127 : sa < -128 ? -128 : sa);
            out[half * 16 + 8 + i] = (signed char)(sb > 127 ? 127 : sb < -128 ? -128 : sb);
        }
    }
}

int main(void)
{
    /* Boundary-heavy inputs: saturation edges, sign flips, and lane-unique values. */
    short A[16] = {-32768, -129, -128, -127, -1, 0, 1, 127,
                   128, 255, 256, 32767, -300, 42, -42, 7};
    short B[16] = {100, -100, 5, -5, 200, -200, 32000, -32000,
                   127, 128, -128, -129, 9, -9, 77, -77};
    signed char want[32];
    signed char got[32];

    reference_packs(A, B, want);

    __m256i a = _mm256_loadu_si256((const __m256i *)A);
    __m256i b = _mm256_loadu_si256((const __m256i *)B);
    _mm256_storeu_si256((__m256i *)got, _mm256_packs_epi16(a, b));

    int mismatches = 0;
    printf("packs_epi16");
    for (int i = 0; i < 32; i++) {
        printf(" %d", got[i]);
        if (got[i] != want[i])
            mismatches++;
    }
    printf("\n");

    /* Second pattern: all-saturated and all-zero halves, to catch a pack that
     * swaps the two sources or drops the high 128-bit lane. */
    short C[16], D[16];
    for (int i = 0; i < 16; i++) {
        C[i] = (short)(i < 8 ? -1000 : 1000);
        D[i] = (short)(i & 1 ? 0 : -1);
    }
    reference_packs(C, D, want);
    _mm256_storeu_si256((__m256i *)got,
                        _mm256_packs_epi16(_mm256_loadu_si256((const __m256i *)C),
                                           _mm256_loadu_si256((const __m256i *)D)));
    printf("packs_epi16_sat");
    for (int i = 0; i < 32; i++) {
        printf(" %d", got[i]);
        if (got[i] != want[i])
            mismatches++;
    }
    printf("\n");

    printf("mismatches %d\n", mismatches);
    return mismatches != 0;
}
