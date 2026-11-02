/* Diagnostic-only LCCC translation unit: compile the fixture body unchanged
 * into an object. A small GCC-built signal-handler wrapper is linked around
 * it by trace_vector_small_packed_wrappers_m32.sh so failures in LCCC's own
 * signal-handler code cannot hide the original register state. */
#define main vector_small_packed_wrappers_test_main
#include "../vector_small_packed_wrappers_m32.c"
#undef main
