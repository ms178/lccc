// FLAGS: -mavx2
/* Deferred (lazy) vector stores.
 *
 * `emit_avx_fp_256_op` may leave a 256-bit result in %ymm0 with its home
 * slot unwritten, letting the next op in the chain consume it from the
 * register. Every consumer of that value must then either read the register
 * or commit the store first; reading the slot yields zeros or stale data
 * (the `_mm256_cvtps_pd` / `_mm256_cvtepi32_pd` regression that returned
 * all-zero results).
 *
 * Each kernel below is a chain, so the intermediate value is exactly the
 * deferred one, and each is followed by consumers of a DIFFERENT shape:
 * widening, narrowing, a horizontal reduction, an integer convert, and a
 * store. Printed values are compared against GCC.
 */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

static void p256(const char *t, __m256 v) {
    float o[8];
    _mm256_storeu_ps(o, v);
    printf("%s %g %g %g %g %g %g %g %g\n", t, o[0], o[1], o[2], o[3],
           o[4], o[5], o[6], o[7]);
}
static void pd(const char *t, __m256d v) {
    double o[4];
    _mm256_storeu_pd(o, v);
    printf("%s %.17g %.17g %.17g %.17g\n", t, o[0], o[1], o[2], o[3]);
}
static void pi(const char *t, __m256i v) {
    int o[8];
    _mm256_storeu_si256((__m256i *)o, v);
    printf("%s %d %d %d %d %d %d %d %d\n", t, o[0], o[1], o[2], o[3],
           o[4], o[5], o[6], o[7]);
}

static __m256 seeds(int k) {
    float b[8];
    for (int i = 0; i < 8; ++i) b[i] = (float)(i * 3 + k) * 0.5f;
    return _mm256_loadu_ps(b);
}

/* A long clamp chain: the shape `avx_fp_private` used, where keeping the
 * intermediate in a register is the whole point. */
__attribute__((noinline)) static __m256 clamp_chain(int k) {
    __m256 v = seeds(k);
    __m256 lo = _mm256_set1_ps(-2.0f);
    __m256 hi = _mm256_set1_ps(2.0f);
    for (int i = 0; i < 6; ++i) {
        v = _mm256_max_ps(v, lo);
        v = _mm256_min_ps(v, hi);
        v = _mm256_add_ps(v, _mm256_set1_ps(0.25f));
    }
    return v;
}

/* Deferred value feeding a WIDENING convert (reads the low 128 bits). */
__attribute__((noinline)) static __m256d widen_after_chain(int k) {
    __m256 v = clamp_chain(k);
    return _mm256_cvtps_pd(_mm256_castps256_ps128(v));
}

/* Deferred value feeding an INTEGER convert with truncation. */
__attribute__((noinline)) static __m256i cvt_after_chain(int k) {
    __m256 v = clamp_chain(k);
    return _mm256_cvttps_epi32(v);
}

/* Deferred value feeding another convert family. */
__attribute__((noinline)) static __m256d cvtepi32_after_chain(int k) {
    __m256i iv = _mm256_cvttps_epi32(clamp_chain(k));
    return _mm256_cvtepi32_pd(_mm256_castsi256_si128(iv));
}

/* Deferred value feeding a horizontal reduction (reads all lanes). */
__attribute__((noinline)) static float hsum_after_chain(int k) {
    __m256 v = clamp_chain(k);
    __m128 lo = _mm256_castps256_ps128(v);
    __m128 hi = _mm256_extractf128_ps(v, 1);
    __m128 s = _mm_add_ps(lo, hi);
    s = _mm_hadd_ps(s, s);
    s = _mm_hadd_ps(s, s);
    float o[4];
    _mm_storeu_ps(o, s);
    return o[0];
}

/* Deferred value written straight to memory. */
__attribute__((noinline)) static void store_after_chain(int k, float *out) {
    __m256 v = clamp_chain(k);
    _mm256_storeu_ps(out, v);
}

/* Two independent chains interleaved: the deferred value must not be
 * confused between them. */
__attribute__((noinline)) static __m256 two_chains(int k) {
    __m256 a = clamp_chain(k);
    __m256 b = clamp_chain(k + 100);
    return _mm256_add_ps(_mm256_mul_ps(a, b), _mm256_set1_ps(1.0f));
}

/* The deferred value is also read a second time AFTER another op, so it
 * must have reached its slot. */
__attribute__((noinline)) static __m256 reuse_after_chain(int k) {
    __m256 v = clamp_chain(k);
    __m256 w = _mm256_sub_ps(v, _mm256_set1_ps(0.5f));
    return _mm256_add_ps(v, w);
}

int main(void) {
    p256("clamp0", clamp_chain(0));
    p256("clamp7", clamp_chain(7));
    pd("widen", widen_after_chain(1));
    pi("cvtt", cvt_after_chain(2));
    pd("cvtepi32", cvtepi32_after_chain(3));
    printf("hsum %.9g\n", hsum_after_chain(4));
    float out[8];
    store_after_chain(5, out);
    printf("store %g %g %g %g %g %g %g %g\n", out[0], out[1], out[2],
           out[3], out[4], out[5], out[6], out[7]);
    p256("two", two_chains(6));
    p256("reuse", reuse_after_chain(8));
    return 0;
}
