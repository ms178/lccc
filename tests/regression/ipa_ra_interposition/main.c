#include <stdio.h>
long use_pragma(long, long, long, long);
long use_late(long, long, long, long);
long use_asm(long, long, long, long);
long use_static(long, long, long, long);
int main(void) {
    /* 3*5 + (7^11) + leaf(3) + leaf(5) + 3+5+7+11 = 15+12+10+16+26 = 79 */
    printf("%ld %ld %ld %ld\n", use_pragma(3, 5, 7, 11), use_late(3, 5, 7, 11),
           use_asm(3, 5, 7, 11), use_static(3, 5, 7, 11));
    return 0;
}
