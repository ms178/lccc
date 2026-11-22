/* -march=haswell -mgeneral-regs-only: POPCNT stays, CRC32 does not.
 *
 * -march= rows are applied after option parsing in GCC, so the general-register
 * handler never sees them: __CRC32__ is absent. POPCNT comes from the post-pass
 * SSE4.2 rule and survives. __MMX__ is withheld until an explicit -mmmx.
 *
 * Flags: -march=haswell -mgeneral-regs-only
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
