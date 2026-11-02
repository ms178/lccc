#include <stdio.h>
#define TYPE(x) _Generic((x), int: 1, unsigned: 2, long: 3, unsigned long: 4, \
                         long long: 5, unsigned long long: 6, char: 7, \
                         signed char: 8, unsigned char: 9, short: 10, \
                         unsigned short: 11, float: 12, double: 13, \
                         long double: 14, char *: 15, const char *: 16, \
                         int *: 17, default: 0)
int main(void) {
    char c = 0; signed char sc = 0; unsigned char uc = 0;
    short s = 0; unsigned short us = 0;
    int i = 0; unsigned u = 0; long l = 0; unsigned long ul = 0;
    long long ll = 0; unsigned long long ull = 0;
    float f = 0; double d = 0; long double ld = 0;
    const char *cs = 0; char *p = 0; int *ip = 0;
    printf("%d %d %d %d %d %d\n", TYPE(c), TYPE(sc), TYPE(uc), TYPE(s), TYPE(us), TYPE(i));
    printf("%d %d %d %d %d %d\n", TYPE(u), TYPE(l), TYPE(ul), TYPE(ll), TYPE(ull), TYPE(f));
    printf("%d %d %d %d %d %d\n", TYPE(d), TYPE(ld), TYPE(cs), TYPE(p), TYPE(ip), TYPE(1 + 1));
    printf("%d %d %d\n", TYPE(1u + 1), TYPE(1L + 1), TYPE(1.0f + 1));
    printf("%d %d\n", TYPE('a'), TYPE("lit"));
    __typeof__(i) ti = 5; __typeof__(p) tp = 0;
    printf("%d %d %zu\n", ti, tp == 0, sizeof(ti));
    printf("%zu %zu\n", sizeof(typeof(u)), sizeof(__typeof__(1.0f)));
    return 0;
}
