#include <stdio.h>
#define P(e) printf("%-40s %lld\n", #e, (long long)(e))
#define PU(e) printf("%-40s %llu\n", #e, (unsigned long long)(e))
int main(void) {
    int i = -1; unsigned u = 1; long l = -1; unsigned long ul = 1;
    P(i < u); P(i < (int)u); P(l < u); P(i < ul); P(l < ul);
    P(i == -1); PU((unsigned)i); PU((unsigned long)i);
    PU((unsigned long long)-1); P((int)(unsigned short)-1);
    P((long long)(int)0x80000000u); PU((unsigned long long)(int)-1);
    P(sizeof(i + u)); P(sizeof(l + u)); P(sizeof(i + ul));
    P(sizeof(l + u) == sizeof(long));
    P((char)300); P((signed char)(unsigned char)300);
    P(1 ? -1 : 1u); PU(1 ? -1 : 1u); P(0 ? -1 : 1u);
    P(-1 < 0ULL); P((long long)-1 < 0ULL);
    /* GNU `a ?: b` takes the same common type as `a ? a : b`. */
    PU(-1 ?: 1U); P(0 ?: -1L); PU(0 ?: 1u);
    /* An array bound spelled as a folded conditional. */
    P(sizeof(char[(1 ? -1 : 1u) ? 3 : 1]));
    return 0;
}
