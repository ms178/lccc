#include <stdio.h>

#ifndef N
#define N 8000000
#endif

__attribute__((noinline)) static int classify(unsigned x) {
    switch (x) {
    case 10: case 12: case 15: case 20: return 42;
    case 11: case 14: return 7;
    default: return 7;
    }
}

int main(void) {
    unsigned seed = 1;
    long sum = 0;
    for (unsigned i = 0; i < N; ++i) {
        seed = seed * 1664525u + 1013904223u;
        sum += classify((seed >> 16) & 31);
    }
    printf("switch_lut_two_val: %ld\n", sum);
    return 0;
}
