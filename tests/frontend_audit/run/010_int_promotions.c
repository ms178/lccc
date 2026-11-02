#include <stdio.h>
#define P(e) printf("%-42s %lld\n", #e, (long long)(e))
int main(void) {
    unsigned char uc = 200; signed char sc = -56; unsigned short us = 65535;
    P(uc + uc); P(uc * 2); P(sc - 1); P(us + us); P(us * us);
    P(-uc); P(~uc); P(~sc); P(~us);
    P((unsigned char)(uc + 100)); P((signed char)(uc + 100));
    P(-1 < 1u); P(-1 < 1); P(-1L < 1u); P(-1LL < 1u);
    P(sizeof(uc + uc)); P(sizeof(+uc)); P(sizeof(-uc));
    P((long long)uc << 1); P(1u - 2u); P(1 - 2u);
    P(0x7fffffff + 1u); P(0x7fffffff + 1);
    P(-2147483647 - 1); P((unsigned)-1 >> 1); P(-1 >> 1);
    P((short)0x8000); P((unsigned short)0x8000);
    P('a' + 1); P(sizeof('a')); P(sizeof(L'a'));
    return 0;
}
