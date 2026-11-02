#include <stdio.h>
#include <float.h>
#define P(e) printf("%-30s %.17g\n", #e, (double)(e))
#define PF(e) printf("%-30s %.9g\n", #e, (double)(e))
int main(void) {
    P(0.1); P(1.0/3.0); P(1e308); P(1e-308); P(1e309 == 1.0/0.0 ? 1.0 : 0.0);
    P(0x1p-3); P(0x1.8p1); P(0x1.fffffffffffffp1023);
    PF(0.1f); PF(1.0f/3.0f);
    P((double)0.1f); P((double)(float)0.1);
    P(1.0L == 1.0); P(3.0 * 0.5);
    P(16777217.0f == 16777216.0f);
    P(1.0e-40f == 0.0f);
    printf("%d %d %d\n", FLT_RADIX, DBL_MANT_DIG, DBL_MAX_EXP);
    printf("%d %d\n", LDBL_MANT_DIG, LDBL_MAX_EXP);
    return 0;
}
