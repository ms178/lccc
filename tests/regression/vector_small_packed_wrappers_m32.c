/* i686 leg: every vector is returned in memory there, so no packed carrier
 * exists. `__builtin_shuffle` on a vector of at most 8 bytes still packed its
 * result, and every consumer dereferenced the lane bits as an address. The
 * GCC oracle keeps MMX enabled (the fixture calls MMX intrinsics), so it
 * returns 8-byte vectors in %mm registers without EMMS; the fixture does no
 * x87 arithmetic, so that cannot perturb its result. */
#include "vector_small_packed_wrappers.c"
