/* `#pragma GCC target("...")` and the option stack around it.
 *
 * GCC compiles every function defined after `#pragma GCC target("avx2")`
 * as if it carried `__attribute__((target("avx2")))`, until `#pragma GCC
 * pop_options` or `reset_options`. lccc dropped these pragmas, so the
 * functions of such a region were refused as soon as they used the
 * extension the region had asked for ("`_mm256_add_epi32` requires the avx2
 * instruction-set extension"): the way vendored SIMD kernels are written,
 * where no function carries an attribute of its own.
 *
 * Each kernel below is reached only when the CPU has its extension, so the
 * program prints the same on every machine. */
#include <immintrin.h>
#include <stdio.h>

static int plain_sum(const int *v) {
    int s = 0;
    for (int i = 0; i < 8; i++)
        s += v[i] * 2;
    return s;
}

#pragma GCC push_options
#pragma GCC target("avx2")
static int avx2_sum(const int *v) {
    __m256i x = _mm256_loadu_si256((const __m256i *)v);
    x = _mm256_add_epi32(x, x);
    __m128i lo = _mm256_castsi256_si128(x), hi = _mm256_extracti128_si256(x, 1);
    lo = _mm_add_epi32(lo, hi);
    lo = _mm_hadd_epi32(lo, lo);
    lo = _mm_hadd_epi32(lo, lo);
    return _mm_cvtsi128_si32(lo);
}
/* A second pragma adds to the first ... */
#pragma GCC target("bmi2")
static unsigned avx2_bmi2(const int *v, unsigned mask) {
    return _pext_u32((unsigned)avx2_sum(v), mask);
}
#pragma GCC pop_options

/* ... and the pop took both away: this one is plain again. */
static int after_pop(const int *v) { return plain_sum(v) + 1; }

/* Several strings, pasted strings, and no parentheses. */
#pragma GCC push_options
#pragma GCC target("sse4.1", "popcnt")
static int sse41_popcnt(const int *v) {
    __m128i x = _mm_loadu_si128((const __m128i *)v);
    x = _mm_mullo_epi32(x, x);
    return _mm_popcnt_u32((unsigned)_mm_extract_epi32(x, 3));
}
#pragma GCC push_options
#pragma GCC target "ss" "se3"
static int ssse3_abs(const int *v) {
    __m128i x = _mm_loadu_si128((const __m128i *)v);
    x = _mm_abs_epi32(_mm_sub_epi32(_mm_setzero_si128(), x));
    return _mm_cvtsi128_si32(x) + sse41_popcnt(v);
}
#pragma GCC pop_options
#pragma GCC pop_options

/* The pragma first, the function's own attribute on top of it. */
#pragma GCC push_options
#pragma GCC target("avx")
__attribute__((target("avx2,fma"))) static double pragma_and_attribute(const double *d) {
    __m256d x = _mm256_loadu_pd(d);
    x = _mm256_fmadd_pd(x, x, x);
    __m256i n = _mm256_castpd_si256(x);
    n = _mm256_xor_si256(n, n);
    return _mm256_cvtsd_f64(_mm256_add_pd(x, _mm256_castsi256_pd(n)));
}
#pragma GCC reset_options
static int after_reset(const int *v) { return plain_sum(v) + 2; }
/* `reset_options` is not a pop: the push above is still there to pop. */
#pragma GCC pop_options

/* A prototype counts: declared inside a region, or with the attribute, the
 * function is compiled for that target wherever its definition stands and
 * whatever the definition itself says (GCC merges the declarations; the
 * last one that names a target decides). */
#pragma GCC push_options
#pragma GCC target("avx2")
static int declared_in_region(const int *v);
#pragma GCC pop_options
__attribute__((target("bmi2"))) static unsigned declared_with_attribute(unsigned a, unsigned m);

static int declared_in_region(const int *v) {
    __m256i x = _mm256_loadu_si256((const __m256i *)v);
    x = _mm256_sub_epi32(_mm256_setzero_si256(), x);
    return _mm256_extract_epi32(x, 5);
}
static unsigned declared_with_attribute(unsigned a, unsigned m) { return _pdep_u32(a, m); }

int main(void) {
    static const int v[8] = {1, -2, 3, -4, 5, -6, 7, -8};
    static const double d[4] = {1.5, 2.5, 3.5, 4.5};
    __builtin_cpu_init();
    printf("plain %d %d %d\n", plain_sum(v), after_pop(v), after_reset(v));
    printf("avx2 %d\n", __builtin_cpu_supports("avx2") ? avx2_sum(v) : plain_sum(v));
    printf("bmi2 %u\n", __builtin_cpu_supports("avx2") && __builtin_cpu_supports("bmi2")
                            ? avx2_bmi2(v, 0x5555u)
                            : 2u);
    printf("sse4.1 %d\n",
           __builtin_cpu_supports("sse4.1") && __builtin_cpu_supports("popcnt") ? sse41_popcnt(v) : 1);
    printf("ssse3 %d\n", __builtin_cpu_supports("ssse3") && __builtin_cpu_supports("sse4.1") &&
                                 __builtin_cpu_supports("popcnt")
                             ? ssse3_abs(v)
                             : 2);
    printf("fma %g\n", __builtin_cpu_supports("avx2") && __builtin_cpu_supports("fma")
                           ? pragma_and_attribute(d)
                           : 3.75);
    printf("prototype %d %u\n", __builtin_cpu_supports("avx2") ? declared_in_region(v) : 6,
           __builtin_cpu_supports("bmi2") ? declared_with_attribute(0x1fu, 0x5555u) : 341u);
    return 0;
}
