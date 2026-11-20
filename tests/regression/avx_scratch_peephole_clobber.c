/* A 128-bit result held in %xmm0 by a last-store peephole must not be read
 * after a 256-bit operand load has overwritten %ymm0 (the same bank).
 *
 * Before the fix, `-O2 -mavx` gave a shuffle result of `0 nan 0 2.2e-39`
 * in the xor reproducer, and `_mm_permutevar_ps` gave `0.4999 0.7069 ...`.
 * Both were silent: the value read back was the low half of an unrelated
 * 256-bit operand that the scratch register had been reloaded with.
 *
 * Inputs are loaded from volatile storage so every operation is a runtime
 * operation, not a folded constant. Each check compares against the Intel
 * definition; the exit status is the number of failed checks.
 */
#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

static volatile float vf[8] = {1.0f, 2.0f, 3.0f, 4.0f, 5.0f, 6.0f, 7.0f, 8.0f};
static volatile double vd[8] = {1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0};
static int failures;

static void check_f(const char *name, const float *got, const float *want, int n)
{
    if (memcmp(got, want, (size_t)n * sizeof(float)) != 0) {
        printf("FAIL %s\n", name);
        failures++;
    } else {
        printf("ok   %s\n", name);
    }
}

/* Shuffle result (value A) lives in %xmm0 while a 256-bit xor loads its
 * operand into %ymm0; the shuffle result is stored afterwards. */
__attribute__((noinline)) static void shuffle_across_wide_xor(void)
{
    __m256d a = _mm256_set_pd(vd[7], vd[6], vd[5], vd[4]);
    __m256d b = _mm256_set_pd(vd[3], vd[2], vd[1], vd[0]);
    __m256d ad = _mm256_xor_pd(a, b);
    __m128 vx = _mm_loadu_ps((const float *)vf);
    __m128 pv = _mm_shuffle_ps(vx, vx, 0x1B);
    double od[4];
    float ofl[4];
    _mm256_storeu_pd(od, ad);
    _mm_storeu_ps(ofl, pv);
    /* Lanes reversed: [4 3 2 1]. */
    const float want[4] = {4.0f, 3.0f, 2.0f, 1.0f};
    check_f("shuffle_across_wide_xor", ofl, want, 4);
    /* The xor result: bit pattern of a XOR bit pattern of b, per lane. */
    uint64_t want_bits[4];
    for (int i = 0; i < 4; i++) {
        double x = vd[4 + i], y = vd[i];
        uint64_t ux, uy;
        memcpy(&ux, &x, sizeof ux);
        memcpy(&uy, &y, sizeof uy);
        want_bits[i] = ux ^ uy;
    }
    if (memcmp(od, want_bits, sizeof want_bits) != 0) {
        printf("FAIL xor_pd_wide_result\n");
        failures++;
    } else {
        printf("ok   xor_pd_wide_result\n");
    }
}

/* _mm_permutevar_ps result held in %xmm0 across a 256-bit addsub_pd. */
__attribute__((noinline)) static void permutevar_across_wide_addsub(void)
{
    __m128 vx = _mm_loadu_ps((const float *)vf);
    __m128i idx = _mm_set_epi32(0, 1, 2, 3);
    __m128 pv = _mm_permutevar_ps(vx, idx);
    __m256d a = _mm256_set_pd(vd[7], vd[6], vd[5], vd[4]);
    __m256d b = _mm256_set_pd(vd[3], vd[2], vd[1], vd[0]);
    __m256d sum = _mm256_addsub_pd(a, b);
    double osum[4];
    float ofl[4];
    _mm256_storeu_pd(osum, sum);
    _mm_storeu_ps(ofl, pv);
    const float want[4] = {4.0f, 3.0f, 2.0f, 1.0f};
    check_f("permutevar_across_wide_addsub", ofl, want, 4);
    /* addsub: even lanes a-b, odd lanes a+b; lane 0 = 5-1 = 4. */
    const double dwant[4] = {4.0, 8.0, 4.0, 12.0};
    if (memcmp(osum, dwant, sizeof dwant) != 0) {
        printf("FAIL addsub_across_permutevar\n");
        failures++;
    } else {
        printf("ok   addsub_across_permutevar\n");
    }
}

/* The reverse direction: a 256-bit value held in %ymm0 must not be read
 * after a 128-bit write to %xmm0 has overwritten its low half. */
__attribute__((noinline)) static void wide_held_across_narrow_write(void)
{
    __m256 pa = _mm256_loadu_ps((const float *)vf);
    __m256 pb = _mm256_set1_ps(1.0f);
    __m256 sum = _mm256_add_ps(pa, pb);
    __m128 lo = _mm_loadu_ps((const float *)vf + 4);
    __m128 hi = _mm_shuffle_ps(lo, lo, 0x1B);
    float ofl[8];
    _mm256_storeu_ps(ofl, sum);
    float lanes[4];
    _mm_storeu_ps(lanes, hi);
    /* sum = [2 3 4 5 6 7 8 9]; hi = [8 7 6 5]. */
    const float wsum[8] = {2.0f, 3.0f, 4.0f, 5.0f, 6.0f, 7.0f, 8.0f, 9.0f};
    const float whi[4] = {8.0f, 7.0f, 6.0f, 5.0f};
    check_f("wide_held_across_narrow_write", ofl, wsum, 8);
    check_f("narrow_shuffle_result", lanes, whi, 4);
}

int main(void)
{
    shuffle_across_wide_xor();
    permutevar_across_wide_addsub();
    wide_held_across_narrow_write();
    if (failures != 0) {
        printf("FAILURES: %d\n", failures);
        return 1;
    }
    printf("PASS avx_scratch_peephole_clobber\n");
    return 0;
}
