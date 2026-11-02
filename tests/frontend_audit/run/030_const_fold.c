#include <stdio.h>
#define P(e) printf("%-40s %lld\n", #e, (long long)(e))
int main(void) {
    P(0x7fffffffffffffffLL);
    P(-0x7fffffffffffffffLL - 1);
    P(0xffffffffffffffffULL);
    P(1ULL << 63);
    P((1ULL << 63) >> 63);
    P(0xffffffffu * 0xffffffffu);
    P(1000000 * 1000000);
    P(10000000000);
    P(-7 / 2); P(-7 % 2); P(7 / -2); P(7 % -2);
    P(-7u / 2); P(-7u % 2);
    P(1 << 30); P(3 << 20); P(-1 >> 5); P(1u << 31);
    P(0x100000000 >> 32);
    P((char)-1); P((unsigned char)-1); P((short)-1); P((unsigned short)-1);
    P((int)3.99); P((int)-3.99); P((long long)1e18);
    P('A' << 1); P('ab'); P('\377'); P('\x80');
    P(1 ? 2 : 3); P(0 && (1/0)); P(1 || 1/0);
    P(sizeof(int) * 8 - 1);
    P(__LINE__); P(__STDC__);
    return 0;
}
