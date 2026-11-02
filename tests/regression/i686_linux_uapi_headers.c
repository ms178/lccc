/* Debian multilib shares the macro-selected asm/ headers under the host
 * multiarch tuple; -m32 must retain that explicitly 32-bit-capable tree. */
#include <linux/types.h>
#include <asm/posix_types.h>
_Static_assert(sizeof(__u64) == 8, "kernel u64 width");
_Static_assert(sizeof(__kernel_long_t) == 4, "i686 kernel long width");
int main(void) { return 0; }
