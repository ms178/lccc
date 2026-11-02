#include <stdio.h>

#ifndef N
#define N 8000000
#endif
#ifndef SWITCH_LUT_DEFAULT
#define SWITCH_LUT_DEFAULT 0
#endif
#ifndef SWITCH_LUT_LABEL
#define SWITCH_LUT_LABEL "switch_lut_packed"
#endif

__attribute__((noinline)) static int packed(unsigned x) {
    switch (x) {
    case 0: return 3;
    case 1: return 7;
    case 2: return 1;
    case 3: return 12;
    case 4: return 5;
    case 5: return 9;
    case 6: return 2;
    case 7: return 15;
    case 8: return 6;
    case 9: return 11;
    case 10: return 4;
    case 11: return 14;
    case 12: return 0;
    case 13: return 10;
    case 14: return 8;
    case 15: return 13;
    default: return SWITCH_LUT_DEFAULT;
    }
}

int main(void) {
    unsigned seed = 3;
    long sum = 0;
    for (unsigned i = 0; i < N; ++i) {
        seed = seed * 1103515245u + 12345u;
        sum += packed((seed >> 16) & 31);
    }
    printf(SWITCH_LUT_LABEL ": %ld\n", sum);
    return 0;
}
