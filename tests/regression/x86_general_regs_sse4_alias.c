/* GCC's `-msse4` is an alias for `-msse4.2`, not SSE4.1.  An explicit SSE4.2
 * grant keeps POPCNT and CRC32 through `-mgeneral-regs-only`.
 *
 * Measured on gcc-16.2 with `-dM -E`:
 *   -msse4 -mgeneral-regs-only   -> __CRC32__ __POPCNT__ (no SSE macros)
 *   -msse4.1 -mgeneral-regs-only -> no __POPCNT__, no __CRC32__
 * lccc dropped both POPCNT and CRC32 for `-msse4`: the hand-kept arm for the
 * spelling set SSE4.1 but never recorded the explicit SSE4.2 grant.
 */
#include <stdio.h>

int main(void)
{
#ifdef __CRC32__
    puts("__CRC32__ on");
#else
    puts("__CRC32__ off");
#endif
#ifdef __POPCNT__
    puts("__POPCNT__ on");
#else
    puts("__POPCNT__ off");
#endif
    return 0;
}
