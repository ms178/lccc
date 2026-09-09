/*
 * SSE2-baseline twin of vec_signed_range_fusion.c.
 *
 * The fusion is lane-width- and ISA-agnostic (it produces only `Add` and a
 * signed `Slt`, both of which the 128-bit path lowers as `paddd`/`pcmpgtd`),
 * so the same vectors must pass with the 4/8-lane vectorizer.  The body is
 * included rather than copied so the two gates can never drift.
 */
#include "vec_signed_range_fusion.c"
