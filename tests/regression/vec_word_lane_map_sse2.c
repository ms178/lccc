/*
 * SSE2-baseline twin of vec_word_lane_map.c (VF=8): exercises `pminsw`/
 * `pmaxsw` (baseline) and the compare+blend fallback where `pminuw`
 * (SSE4.1) is unavailable.
 */
#include "vec_word_lane_map.c"
