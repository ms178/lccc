#include <stdio.h>
#include <fenv.h>
#ifndef OBSERVE_FENV
#define OBSERVE_FENV 1
#endif
#ifndef N
#define N 250000
#endif
#if OBSERVE_FENV
#pragma STDC FENV_ACCESS ON
#endif
__attribute__((noinline)) double mixed_work(double x, unsigned n) {
    unsigned dead = 1;
    for (unsigned i = 0; i < n; ++i) {
        dead = ((dead * 1664525u + 1013904223u) ^ (i >> 3)) + (dead << 5);
        x = x + 0.25;
    }
    return x;
}
int main(void) {
    int modes[4] = {FE_TONEAREST, FE_DOWNWARD, FE_UPWARD, FE_TOWARDZERO};
    for (int i = 0; i < 4; ++i) {
        if (fesetround(modes[i])) return 2;
        double x = mixed_work(0.0, N);
        if (x != (double)N * 0.25) return 3;
        printf("%.1f\n", x);
    }
    return fesetround(FE_TONEAREST) != 0;
}
