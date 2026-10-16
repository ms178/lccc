#ifndef NO_FENV_PRAGMA
#pragma STDC FENV_ACCESS ON
#endif
#include <fenv.h>
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <float.h>
#define NI __attribute__((noinline))
NI double literal_sum(void) { return 0x1p0 + 0x1p-53; }
NI double literal_overflow(void) { return DBL_MAX * 2.0; }
NI double runtime_sum(double a,double b) { return a+b; }
static uint64_t bits(double x) { uint64_t u; memcpy(&u,&x,8); return u; }
int main(void) {
    volatile double one=1.0, half_ulp=0x1p-53;
    if(fesetround(FE_UPWARD))return 99;
    double expected=runtime_sum(one,half_ulp),actual=literal_sum();
    printf("round=%llx/%llx ",(unsigned long long)bits(expected),(unsigned long long)bits(actual));
    feclearexcept(FE_ALL_EXCEPT);
    volatile double overflow=literal_overflow();(void)overflow;
    int raised=fetestexcept(FE_OVERFLOW);
    printf("overflow=%d\n",raised!=0);
    return !(bits(expected)==bits(actual) && raised);
}
