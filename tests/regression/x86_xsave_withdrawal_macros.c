/* -mno-xsave withdraws AVX and the XSAVE-dependent extensions, but keeps the
 * SHA-512, SM3 and SM4 announcements and USERMSR.
 *
 * GCC 16.2 source (gcc/common/config/i386/i386-common.cc): XSAVE_UNSET clears
 * the ISA AVX mask, and ISA2_XSAVE_UNSET clears AVX2 and AMX-TILE. SM3, SM4 and
 * SHA512 are cleared only by ISA2_AVX_UNSET, which -mno-xsave does not reach.
 * Measured with `-dM -E` on gcc-16.2: `-msha512 -msm3 -msm4 -musermsr -mno-xsave`
 * defines __SHA512__ __SM3__ __SM4__ __USER_MSR__ and no __XSAVE__ or __AVX__.
 *
 * lccc used to drop __SM3__ and __SM4__ here (their requirement included AVX,
 * which -mno-xsave removes) and never defined __USER_MSR__.
 */
#include <stdio.h>

static void report(void)
{
#ifdef __SHA512__
    puts("__SHA512__ on");
#else
    puts("__SHA512__ off");
#endif
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
#ifdef __USER_MSR__
    puts("__USER_MSR__ on");
#else
    puts("__USER_MSR__ off");
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
