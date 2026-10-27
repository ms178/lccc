/* Same packed-switch workload with a default that no table slot encodes.
 * This keeps the out-of-range result select distinct from table extraction. */
#define SWITCH_LUT_DEFAULT 16
#define SWITCH_LUT_LABEL "switch_lut_packed_fallback"
#include "switch_lut_packed_bench.c"
