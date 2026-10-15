#include <fenv.h>
#include <stdint.h>
#ifndef NO_FENV_PRAGMA
#pragma STDC FENV_ACCESS ON
#endif
__attribute__((noinline)) double positive(void) { return 1.0 + 0x1p-53; }
__attribute__((noinline)) double negative(void) { return -1.0 - 0x1p-53; }
__attribute__((noinline)) float converted(void) { return (float)16777217; }
/* Constant-return analysis must retain the exception-producing body too. */
__attribute__((noinline)) static int discarded(double zero) {
    double result=1.0/zero; (void)result; return 7;
}
int main(void) {
    const int modes[]={FE_TONEAREST, FE_UPWARD, FE_DOWNWARD, FE_TOWARDZERO};
    for(int i=0;i<4;i++) {
        if(fesetround(modes[i]))return 10+i;
        union { double d; uint64_t u; } p,n;
        union { float f; uint32_t u; } c;
        p.d=positive();n.d=negative();c.f=converted();
        if(p.u != (i==1 ? UINT64_C(0x3ff0000000000001) : UINT64_C(0x3ff0000000000000)))return 20+i;
        if(n.u != (i==2 ? UINT64_C(0xbff0000000000001) : UINT64_C(0xbff0000000000000)))return 30+i;
        if(c.u != (i==1 ? UINT32_C(0x4b800001) : UINT32_C(0x4b800000)))return 40+i;
    }
#ifndef ROUNDING_ONLY
    if(feclearexcept(FE_ALL_EXCEPT))return 50;
    if(discarded(0.0)!=7)return 51;
    if(!(fetestexcept(FE_DIVBYZERO)&FE_DIVBYZERO))return 52;
#endif
    return fesetround(FE_TONEAREST)!=0;
}
