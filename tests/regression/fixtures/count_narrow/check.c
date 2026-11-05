/* The freestanding half of the narrow-count check: `test` returns non-zero
 * when the checksum disagrees with the value GCC 16.2 and Clang 23.1.5 measure
 * on this same source, and the start stub exits with that status. */
#include "runtime.c"
int test(void) { return count_narrow_checksum() != 4177401467814966825ull; }
