/* AVX1 typed float logic on directed bit patterns (-O2 -mavx, no AVX2).
 *
 * _mm256_and/or/xor/andnot_ps and _pd are pure bit operations, so every lane
 * must match an integer-only oracle exactly. The inputs are the bit patterns
 * that the logic ops are most likely to mishandle: +0.0 and -0.0 (only the
 * sign bit differs), quiet and signaling NaNs with payloads and both signs,
 * infinities, denormals, and all-ones. Every ordered pair of patterns is used,
 * so an operand swap in ANDNOT (which computes ~x & y) changes the count.
 * Vectors come through memcpy from noinline kernels, so no lane is folded.
 *
 * The program prints the mismatch count per family. GCC 16.2 (-O2 -mavx)
 * must print the same line, and the exit status is 0 only when both are 0. */
#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

static const uint32_t pat[] = {
    0x00000000u, /* +0.0 */
    0x80000000u, /* -0.0 */
    0x7fc00000u, /* qNaN */
    0xffc00000u, /* -qNaN */
    0x7f800001u, /* sNaN, payload 1 */
    0xff800001u, /* -sNaN, payload 1 */
    0x7fffffffu, /* qNaN, all payload bits */
    0x00000001u, /* smallest denormal */
    0x7f800000u, /* +inf */
    0xff800000u, /* -inf */
    0x3f800000u, /* 1.0 */
    0xbf800000u, /* -1.0 */
    0xffffffffu, /* all ones */
    0x12345678u,
    0x80000001u, /* -denormal */
    0xfffffffeu,
};
#define NP ((unsigned)(sizeof pat / sizeof pat[0]))

__attribute__((noinline)) static void k_ps(const void *a, const void *b, void *o) {
    __m256 x, y;
    memcpy(&x, a, 32);
    memcpy(&y, b, 32);
    __m256 r0 = _mm256_and_ps(x, y);
    __m256 r1 = _mm256_or_ps(x, y);
    __m256 r2 = _mm256_xor_ps(x, y);
    __m256 r3 = _mm256_andnot_ps(x, y);
    memcpy((char *)o + 0, &r0, 32);
    memcpy((char *)o + 32, &r1, 32);
    memcpy((char *)o + 64, &r2, 32);
    memcpy((char *)o + 96, &r3, 32);
}

__attribute__((noinline)) static void k_pd(const void *a, const void *b, void *o) {
    __m256d x, y;
    memcpy(&x, a, 32);
    memcpy(&y, b, 32);
    __m256d r0 = _mm256_and_pd(x, y);
    __m256d r1 = _mm256_or_pd(x, y);
    __m256d r2 = _mm256_xor_pd(x, y);
    __m256d r3 = _mm256_andnot_pd(x, y);
    memcpy((char *)o + 0, &r0, 32);
    memcpy((char *)o + 32, &r1, 32);
    memcpy((char *)o + 64, &r2, 32);
    memcpy((char *)o + 96, &r3, 32);
}

int main(void) {
    uint32_t a[8], b[8], got[32], want[32];
    uint64_t da[4], db[4], dgot[16], dwant[16];
    unsigned bad_ps = 0, bad_pd = 0;

    for (unsigned p = 0; p < NP; p++) {
        for (unsigned q = 0; q < NP; q++) {
            for (unsigned k = 0; k < 8; k++) {
                a[k] = pat[(p + k) % NP];
                b[k] = pat[(q + 2 * k + 1) % NP];
            }
            k_ps(a, b, got);
            for (unsigned k = 0; k < 8; k++) {
                want[0 + k] = a[k] & b[k];
                want[8 + k] = a[k] | b[k];
                want[16 + k] = a[k] ^ b[k];
                want[24 + k] = ~a[k] & b[k];
            }
            if (memcmp(got, want, sizeof want) != 0)
                bad_ps++;

            for (unsigned k = 0; k < 4; k++) {
                da[k] = ((uint64_t)a[2 * k] << 32) | a[2 * k + 1];
                db[k] = ((uint64_t)b[2 * k] << 32) | b[2 * k + 1];
            }
            k_pd(da, db, dgot);
            for (unsigned k = 0; k < 4; k++) {
                dwant[0 + k] = da[k] & db[k];
                dwant[4 + k] = da[k] | db[k];
                dwant[8 + k] = da[k] ^ db[k];
                dwant[12 + k] = ~da[k] & db[k];
            }
            if (memcmp(dgot, dwant, sizeof dwant) != 0)
                bad_pd++;
        }
    }
    printf("float_logic_bits ps_mismatch=%u pd_mismatch=%u\n", bad_ps, bad_pd);
    return (bad_ps || bad_pd) ? 1 : 0;
}
