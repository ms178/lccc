/* -mno-avx withdraws SM3 and SM4 (GCC ISA2_AVX_UNSET), while -mno-avx2 keeps
 * them and -mno-avx leaves XSAVE standing (GCC's AVX_UNSET is ISA-only).
 *
 * Measured on gcc-16.2 with `-dM -E`:
 *   -msm3 -msm4 -mno-avx          -> __XSAVE__ only (no __SM3__, __SM4__, __AVX__)
 *   -msm3 -msm4 -mavx2 -mno-avx2  -> __SM3__ __SM4__ __AVX__ __XSAVE__
 * lccc kept SM3/SM4 under -mno-avx2 and dropped them under -mno-avx before the
 * requirement change; this fixture pins both halves of the cascade.
 */
#include <stdio.h>

static void report(void)
{
#ifdef __SM3__
    puts("__SM3__ on");
#else
    puts("__SM3__ off");
#endif
#ifdef __SM4__
    puts("__SM4__ on");
#else
    puts("__SM4__ off");
#endif
#ifdef __XSAVE__
    puts("__XSAVE__ on");
#else
    puts("__XSAVE__ off");
#endif
#ifdef __AVX__
    puts("__AVX__ on");
#else
    puts("__AVX__ off");
#endif
}

int main(void)
{
    report();
    return 0;
}
