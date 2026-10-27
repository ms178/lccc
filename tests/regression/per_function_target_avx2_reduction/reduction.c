/* Regression gate for the per-function AVX2 middle-end ISA overlay.
 *
 * This source lives in a subdirectory because the regression corpus runner
 * compiles every top-level tests/regression/*.c as a complete executable.
 * check_vectorize_isa_gate.sh supplies a separate driver for runtime checks.
 *
 * The signed i32 sum is an ordinary vectorizable reduction. The target
 * function must receive AVX2 YMM vectorization without a translation-unit
 * -mavx2 flag, while the neighboring baseline function must not inherit that
 * permission. Both bodies are retained as named functions for assembly checks.
 */
__attribute__((target("avx2"), noinline))
int sum_i32_avx2(const int *values, int count) {
    int sum = 0;
    for (int i = 0; i < count; ++i)
        sum += values[i];
    return sum;
}

__attribute__((noinline))
int sum_i32_baseline(const int *values, int count) {
    int sum = 0;
    for (int i = 0; i < count; ++i)
        sum += values[i];
    return sum;
}
