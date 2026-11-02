#include <stdio.h>
#define P(e) printf("%-34s %lld\n", #e, (long long)(e))
int main(void) {
    P(1 << 0); P(1u << 0); P(0 << 31); P(1u << 31 >> 31);
    P((unsigned)1 << 31); P(2147483647 >> 1);
    P((long long)1 << 62); P((unsigned long long)1 << 63);
    P(-8 >> 1); P(-8 / 2); P(-8 % 3); P(8 % -3);
    P(-9223372036854775807LL - 1);
    P((unsigned)-1 / 3); P((unsigned)-1 % 3);
    P(0x80000000 >> 31); P((int)0x80000000u >> 31);
    int x = -17; P(x / 4); P(x % 4); P(x >> 2);
    unsigned y = 0x80000000u; P(y >> 31); P(y / 2);
    P((long long)x * 0x100000000LL);
    P(1.0 != 1); P(-0.0 == 0.0); P(0.0 / 1.0);
    return 0;
}
