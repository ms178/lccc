/* The same lane-mask provenance matrix at the baseline SSE2 target: the
 * blend lowering there is and/andn/or rather than pblendvb, and the compare
 * set differs (no native unsigned compares), so it is a distinct code path.
 * See vec_mask_provenance.c for the full rationale. */
#include "vec_mask_provenance.c"
