/* MachInst window-allocator regression, spilled POINTER class: 13
 * simultaneously-live pointers force spill decisions, and every
 * dereference routes through MachInst memory operands (RCX staging for
 * spilled bases). Byte-addressing patterns are the "one spilled pointer
 * demotes the window" class the allocator exists to survive. GCC oracle. */
#include <stdio.h>

long pointer_window_pressure(long *p0, long *p1, long *p2, long *p3,
                             long *p4, long *p5, long *p6, long *p7,
                             long *p8, long *p9, long *p10, long *p11,
                             long *p12, long n) {
    long s = 0;
    for (long i = 0; i < n; i++) {
        s += *p0 + i;
        s ^= *p1 - i;
        s += *p2 * 3;
        s -= *p3 + i;
        s ^= *p4 - i;
        s += *p5 * 5;
        s -= *p6 + i;
        s ^= *p7 - i;
        s += *p8 * 3;
        s -= *p9 + i;
        s ^= *p10 - i;
        s += *p11 * 7;
        s -= *p12 + i;
    }
    return s;
}

int main(void) {
    long v[13] = {1,2,3,4,5,6,7,8,9,10,11,12,13};
    printf("%ld\n", pointer_window_pressure(&v[0], &v[1], &v[2], &v[3], &v[4],
                                            &v[5], &v[6], &v[7], &v[8], &v[9],
                                            &v[10], &v[11], &v[12], 5));
    return 0;
}
