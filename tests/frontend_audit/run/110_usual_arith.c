#include <stdio.h>
#define P(e) printf("%-40s %lld\n", #e, (long long)(e))
#define PU(e) printf("%-40s %llu\n", #e, (unsigned long long)(e))
#define T(e) printf("%-40s %d\n", #e, (int)(e))
static long long ll_ret(int x) { return x; }
static unsigned long long ull_ret(unsigned x) { return x; }
int main(void) {
    unsigned u = 0xdeadbeefu; int i = -1; long l = -1; unsigned long ul = 3;
    PU(u * i); PU(u + i); P(l * u); PU(l * u);
    P(i ? u : l); PU(i ? u : l); P(0 ? u : l);
    PU(i ? 1 : -1u); PU(l ? 1 : -1u); PU(0 ? 1 : -1u);
    PU(i + 1u); P(i + 1L); PU(i + 1UL);
    T(sizeof(u ? 1 : 1L)); T(sizeof(u ? 1u : 1L));
    P((char)i + (unsigned char)i);
    P((short)i * (unsigned short)i);
    PU(ll_ret(i)); PU(ull_ret(u));
    double d = u; P((long long)d); P((long long)(float)u);
    P((long long)(double)i);
    unsigned long long q = i; PU(q);
    signed char sc = -1; unsigned char uc = sc; P(uc);
    PU((unsigned)(short)-1); PU((unsigned)(char)-1);
    PU(-1 % 2u);
    long double ld = -1; PU((unsigned long long)ld);
    return 0;
}
