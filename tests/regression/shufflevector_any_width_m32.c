/* i686 leg: every vector result is returned in memory there. -mno-mmx keeps
 * the GCC oracle on the same ABI: with MMX enabled, i386 GCC returns 8-byte
 * integer vectors in %mm registers without EMMS, and the next x87 operation
 * (3.0f + k) then yields the default NaN. */
#include "shufflevector_any_width.c"
