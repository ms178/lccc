/*
 * SSE2-baseline twin of vec_byte_runtime_invariant.c: exercises the
 * `movd`+`punpcklbw`+`punpcklwd`+`pshufd` byte splat and the compare+blend
 * fallback where `pminsb`/`pmaxsb` (SSE4.1) are unavailable.
 */
#include "vec_byte_runtime_invariant.c"
