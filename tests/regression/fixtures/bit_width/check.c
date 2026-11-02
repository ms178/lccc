/* Freestanding differential checksum for single-bit selection.
 *
 * The expected value was measured with GCC 16.2 and Clang 23.1.4 on the same
 * source (both produced 3862114428453411727 for set/clear/test and
 * 2052003996353657299 for the toggle checksum, and 15164872185446587354 for the shared-mask checksum).  Every shift index is below the
 * operand width, so the C is defined; the IR's modulo-W behaviour for larger
 * indices is pinned separately by the matcher/folder unit tests.  The checksum
 * observes signed results *inside* the producing function and through widening
 * returns, so a backend that leaves a narrow result in the wrong canonical
 * register form cannot hide behind a truncating ABI return. */
#include "runtime.c"
int test(void) {
  return bit_width_checksum() != 3862114428453411727ull
      || bit_toggle_checksum() != 2052003996353657299ull
      || bit_shared_checksum() != 15164872185446587354ull;
}
