#include <stdio.h>
#include <stdint.h>
__attribute__((noinline)) double opaque(double x){ volatile int q=(int)x; (void)q; return x*1.0000000001 + 0.25; }
__attribute__((noinline)) void barrier(void){ volatile int q=0; (void)q; }
static double acc;
static unsigned long long mixv;
__attribute__((noinline)) static double dot(double *a, double *b, int n){
    double s = 0.0;
    for (int i = 0; i < n; i++) s += a[i]*b[i];
    return s;
}
int main(void){
    double v0 = 384.9218040120;
    double v1 = -722.5885272947;
    double v2 = 613.1894321593;
    double v3 = -343.4279689416;
    double v4 = 153.1446071539;
    double v5 = -961.9255316053;
    const double c0 = 0;
    const double c1 = 1;
    const double c2 = -1;
    const double c3 = 0.5;
    const double c4 = 94.333300881796674;
    const double c5 = -17.775265151007133;
    const double c6 = 3;
    const double c7 = 2;
    double aa0[5] = {v0,v1,v2,v3,v4}; double bb0[5] = {c0+1,c1+2,c2+3,c3+4,c4+5};
    acc += dot(aa0,bb0,5);
    float g1a = (float)v0 + (float)c3;
    float g1b = g1a; for (int w1=0;w1<50;w1++) g1b = g1b*(float)0.5 + (float)c7;
    acc += (double)g1b;
    double s_2 = v1;
    if ((uintptr_t)&s_2 & 1) { s_2 += c4; barrier(); } else { s_2 = c4 - v2; }
    acc += s_2 * c4;
    double s_3 = v4;
    if ((uintptr_t)&s_3 & 1) { s_3 += c3; barrier(); } else { s_3 = c3 - v4; }
    acc += s_3 * c3;
    double aa4[5] = {v0,v1,v2,v3,v4}; double bb4[5] = {c0+1,c1+2,c2+3,c3+4,c4+5};
    acc += dot(aa4,bb4,5);
    double s_5 = v0;
    s_5 = s_5 * c0 + v4;
    s_5 = s_5 * c0 + v0;
    acc += s_5;
    double s_6 = v2;
    s_6 = s_6 * c4 + v0;
    s_6 = s_6 * c4 + v0;
    s_6 = s_6 * c4 + v1;
    s_6 = s_6 * c4 + v1;
    s_6 = s_6 * c4 + v4;
    s_6 = s_6 * c4 + v5;
    acc += s_6;
    float g7a = (float)v0 + (float)c6;
    float g7b = g7a; for (int w7=0;w7<50;w7++) g7b = g7b*(float)0.5 + (float)c0;
    acc += (double)g7b;
    float g8a = (float)v0 + (float)c6;
    float g8b = g8a; for (int w8=0;w8<50;w8++) g8b = g8b*(float)0.5 + (float)c7;
    acc += (double)g8b;
    mixv = 0;
    double u = acc; unsigned long long h = 0x9e3779b97f4a7c15ULL;
    for (int i = 0; i < 64; i++) { h ^= (unsigned long long)(u * (i+1)); u = u*0.999 + 0.001; h = (h<<7)|(h>>57); }
    printf("fpstress %llu\n", (unsigned long long)h);
    return 0;
}
