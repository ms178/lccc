/* SSE2 baseline build (default -O2) of vector_guard_sum_equal_width.c: the
 * guarded equal-width sums lower to VecMaskedAddValueI32x4 (pcmpgtd + pand
 * + paddd, interleaved, run-time thresholds splatted once in the preheader)
 * instead of the AVX2 VecMaskedAddI32x8. */
#include "vector_guard_sum_equal_width.c"
