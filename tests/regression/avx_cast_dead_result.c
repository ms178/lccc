#include <immintrin.h>
#include <stdio.h>

/* A 256-to-128 cast whose result is never read. The operand is a zero
 * compound literal, so the cast reads a 32-byte home that the zero store
 * fills. The 16-byte result home must stay in memory. Promoting it left a
 * dead SSA value with no register and no slot, and codegen aborted with
 * "value N has no register, stack slot, Copy, or GlobalAddr definition"
 * (the _mm256_castpd256_pd128 row of check_avx_intrinsic_isa.py). */
__attribute__((noinline)) static void dead_cast(void) {
    (void)_mm256_castpd256_pd128((__m256d){0});
}

__attribute__((noinline)) static double live_cast(__m256d x) {
    return _mm_cvtsd_f64(_mm256_castpd256_pd128(x));
}

int main(void) {
    dead_cast();
    __m256d v = _mm256_set_pd(4.0, 3.0, 2.0, 1.5);
    printf("%.2f\n", live_cast(v));
    dead_cast();
    printf("%.2f\n", _mm256_cvtsd_f64(_mm256_castpd128_pd256(_mm_set_sd(7.25))));
    return 0;
}
