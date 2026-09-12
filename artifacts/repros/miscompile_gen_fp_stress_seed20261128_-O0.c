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
    double v0 = -78.1320167402;
    double v1 = 468.0717570377;
    double v2 = 560.4069098145;
    double v3 = 976.2199672210;
    double v4 = 279.5255987922;
    double v5 = -219.4393968596;
    const double c0 = 0;
    const double c1 = 1;
    const double c2 = -1;
    const double c3 = 0.5;
    const double c4 = 86.631939632329136;
    const double c5 = 72.788874353150305;
    const double c6 = 3;
    const double c7 = 2;
    double aa0[5] = {v0,v1,v2,v3,v4}; double bb0[5] = {c0+1,c1+2,c2+3,c3+4,c4+5};
    acc += dot(aa0,bb0,5);
    double s_1 = v0;
    s_1 = s_1 * c5 + v1;
    s_1 = s_1 * c5 + v3;
    s_1 = s_1 * c5 + v2;
    s_1 = s_1 * c5 + v2;
    s_1 = s_1 * c5 + v5;
    acc += s_1;
    float g2a = (float)v0 + (float)c0;
    float g2b = g2a; for (int w2=0;w2<50;w2++) g2b = g2b*(float)0.5 + (float)c1;
    acc += (double)g2b;
    double aa3[5] = {v0,v1,v2,v3,v4}; double bb3[5] = {c0+1,c1+2,c2+3,c3+4,c4+5};
    acc += dot(aa3,bb3,5);
    mixv = 0;
    double u = acc; unsigned long long h = 0x9e3779b97f4a7c15ULL;
    for (int i = 0; i < 64; i++) { h ^= (unsigned long long)(u * (i+1)); u = u*0.999 + 0.001; h = (h<<7)|(h>>57); }
    printf("fpstress %llu\n", (unsigned long long)h);
    return 0;
}
