/*
 * SSE2-baseline twin of vec_range_mask_fusion.c.
 *
 * The fusion is lane-width- and ISA-agnostic (it produces only `Add` and a
 * signed `Slt`, both of which the 128-bit path lowers as `paddd`/`pcmpgtd`),
 * so the SAME exhaustive vectors must pass with the 4-lane vectorizer.  The
 * body is included rather than copied so the two gates can never drift.
 */
#include "vec_range_mask_fusion.c"
