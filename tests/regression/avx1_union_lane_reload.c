/* Same corpus compiled for AVX-without-AVX2: the lane rewrite is gated on AVX2
 * (vpbroadcastd/vbroadcastss-from-register), so this build must stay correct
 * with the memory path. */
#include "avx_union_lane_reload.c"
