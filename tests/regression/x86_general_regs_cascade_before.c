/* An SSE cascade before -mgeneral-regs-only leaves POPCNT and CRC32 off.
 *
 * -mno-sse2 withdraws SSE4.2 before the general-register denial runs, so the
 * denial has nothing to carry: GCC defines neither __POPCNT__ nor __CRC32__.
 *
 * Flags: -mavx -mno-sse2 -mgeneral-regs-only
 * The regression runner requires this program's stdout to match GCC 16.2's
 * byte for byte, so the GCC answer is the expectation.
 */
#include <stdio.h>

static void report(void)
{
#ifdef __POPCNT__
    puts("__POPCNT__ on");
#else
    puts("__POPCNT__ off");
#endif
#ifdef __CRC32__
    puts("__CRC32__ on");
#else
    puts("__CRC32__ off");
#endif
#ifdef __MMX__
    puts("__MMX__ on");
#else
    puts("__MMX__ off");
#endif
#ifdef __SSE__
    puts("__SSE__ on");
#else
    puts("__SSE__ off");
#endif
#ifdef __SSE2__
    puts("__SSE2__ on");
#else
    puts("__SSE2__ off");
#endif
#ifdef __SSE4_1__
    puts("__SSE4_1__ on");
#else
    puts("__SSE4_1__ off");
#endif
#ifdef __SSE4_2__
    puts("__SSE4_2__ on");
#else
    puts("__SSE4_2__ off");
#endif
#ifdef __AVX__
    puts("__AVX__ on");
#else
    puts("__AVX__ off");
#endif
#ifdef __AVX2__
    puts("__AVX2__ on");
#else
    puts("__AVX2__ off");
#endif
#ifdef __XSAVE__
    puts("__XSAVE__ on");
#else
    puts("__XSAVE__ off");
#endif
#ifdef __ABM__
    puts("__ABM__ on");
#else
    puts("__ABM__ off");
#endif
#ifdef __LZCNT__
    puts("__LZCNT__ on");
#else
    puts("__LZCNT__ off");
#endif
}

int main(void)
{
    report();
    return 0;
}
