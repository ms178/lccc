/* Freestanding entry for the browser bit-index checksum (separate objects).
 *
 * 2932816357278149644 is the value GCC 16.2 and Clang 23.1.5 both compute at
 * -O0, -O1, -O2, -O3 and -Os (hosted x86-64 build of browser_bits.c and
 * browser_driver.c), and that a -fsanitize=undefined,bounds trap build of both
 * compilers reproduces without trapping.  The driver folds only width-neutral
 * observations, so the same constant is pinned on all four targets. */
unsigned long long browser_bits_checksum(void);
int test(void) { return browser_bits_checksum() != 2932816357278149644ull; }
