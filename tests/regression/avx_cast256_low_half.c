/* 256-to-128 casts under -mavx (_mm256_castps256_ps128, _mm256_castpd256_pd128,
 * _mm256_castsi256_si128) across every operand shape the lowering distinguishes:
 *
 *   - a pending 32-byte load (unaligned and 32-byte-aligned): the cast reads the
 *     low 16 bytes of that load directly;
 *   - a register-homed arithmetic result: the cast reads the XMM alias;
 *   - a value reloaded from its stack slot after a call: the full-width path;
 *   - a value whose cast result has two readers: the load must not be elided
 *     into a single-use fold.
 *
 * Each case stores the cast result and the untouched upper half, then compares
 * against constants (not a reference compiler), so the test is self-contained.
 * Compiled with -O2 -mavx; exit status is nonzero on any mismatch.
 */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

static int failures;

static void check_floats(const char *what, const float *got, const float *want, int n) {
    if (memcmp(got, want, (size_t)n * sizeof *got) == 0) return;
    printf("FAIL %s:", what);
    for (int i = 0; i < n; i++) printf(" %g/%g", (double)got[i], (double)want[i]);
    printf("\n");
    failures++;
}

static void check_ints(const char *what, const int *got, const int *want, int n) {
    if (memcmp(got, want, (size_t)n * sizeof *got) == 0) return;
    printf("FAIL %s:", what);
    for (int i = 0; i < n; i++) printf(" %d/%d", got[i], want[i]);
    printf("\n");
    failures++;
}

/* Opaque to the optimizer: forces its argument through a stack slot. */
__attribute__((noinline)) static __m256 through_call(__m256 v) { return v; }

static const float fsrc[8] = {1.5f, -2.25f, 3.0f, 4.75f, 5.5f, -6.0f, 7.25f, 8.0f};
static const int isrc[8] = {10, 11, 12, 13, 14, 15, 16, 17};
static const double dsrc[4] = {0.5, 1.25, -2.0, 3.5};

int main(void) {
    /* 1: pending unaligned 32-byte load, cast reads its low 16 bytes. */
    {
        __m256 a = _mm256_loadu_ps(fsrc);
        float lo[4], full[8];
        _mm_storeu_ps(lo, _mm256_castps256_ps128(a));
        _mm256_storeu_ps(full, a);
        check_floats("ps unaligned load: low half", lo, fsrc, 4);
        check_floats("ps unaligned load: full", full, fsrc, 8);
    }
    /* 2: pending 32-byte-aligned load. */
    {
        _Alignas(32) float al[8];
        memcpy(al, fsrc, sizeof al);
        __m256 a = _mm256_load_ps(al);
        float out[4];
        _mm_storeu_ps(out, _mm256_castps256_ps128(a));
        check_floats("ps aligned load", out, fsrc, 4);
    }
    /* 3: integer pending load. */
    {
        __m256i b = _mm256_loadu_si256((const __m256i *)isrc);
        int out[4];
        _mm_storeu_si128((__m128i *)out, _mm256_castsi256_si128(b));
        check_ints("si load", out, isrc, 4);
    }
    /* 4: double-precision pending load. */
    {
        __m256d d = _mm256_loadu_pd(dsrc);
        double out[2];
        _mm_storeu_pd(out, _mm256_castpd256_pd128(d));
        if (memcmp(out, dsrc, sizeof out) != 0) {
            printf("FAIL pd load\n");
            failures++;
        }
    }
    /* 5: register-homed arithmetic result. */
    {
        __m256 a = _mm256_loadu_ps(fsrc);
        __m256 s = _mm256_add_ps(a, a);
        float out[4], want[4];
        for (int i = 0; i < 4; i++) want[i] = fsrc[i] * 2.0f;
        _mm_storeu_ps(out, _mm256_castps256_ps128(s));
        check_floats("register home", out, want, 4);
    }
    /* 6: slot reload across a call. */
    {
        __m256 k = through_call(_mm256_loadu_ps(fsrc));
        float out[4];
        _mm_storeu_ps(out, _mm256_castps256_ps128(k));
        check_floats("slot reload", out, fsrc, 4);
    }
    /* 7: two readers of the cast result (no single-use fold). */
    {
        __m256 a = _mm256_loadu_ps(fsrc);
        __m128 lo = _mm256_castps256_ps128(a);
        __m128 twice = _mm_add_ps(lo, lo);
        float out[4], want[4];
        for (int i = 0; i < 4; i++) want[i] = fsrc[i] * 2.0f;
        _mm_storeu_ps(out, twice);
        check_floats("two readers", out, want, 4);
    }
    /* 8: cast, then insert the same lane back: round trip is bit-exact. */
    {
        __m256 a = _mm256_loadu_ps(fsrc);
        __m256 r = _mm256_insertf128_ps(a, _mm256_castps256_ps128(a), 1);
        float out[8], want[8];
        memcpy(want, fsrc, sizeof want);
        memcpy(want + 4, fsrc, 4 * sizeof(float));
        _mm256_storeu_ps(out, r);
        check_floats("cast round trip", out, want, 8);
    }
    if (failures) return 1;
    puts("avx_cast256_low_half: ok");
    return 0;
}
