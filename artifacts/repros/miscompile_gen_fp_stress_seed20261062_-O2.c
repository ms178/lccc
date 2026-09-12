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
    double v0 = -253.6014686727;
    double v1 = -773.8908463479;
    double v2 = -98.8809571566;
    double v3 = -925.7986322541;
    double v4 = 633.3504441508;
    double v5 = 766.0425576065;
    const double c0 = 0;
    const double c1 = 1;
    const double c2 = -1;
    const double c3 = 0.5;
    const double c4 = -9.6322425840097878;
    const double c5 = 62.358221137352956;
    const double c6 = 3;
    const double c7 = 2;
    double s_0 = v1;
    s_0 = s_0 * c5 + v2;
    s_0 = s_0 * c5 + v0;
    s_0 = s_0 * c5 + v1;
    s_0 = s_0 * c5 + v4;
    s_0 = s_0 * c5 + v5;
    s_0 = s_0 * c5 + v1;
    acc += s_0;
    double s_1 = v0;
    if ((uintptr_t)&s_1 & 1) { s_1 += c2; barrier(); } else { s_1 = c2 - v3; }
    acc += s_1 * c2;
    double aa2[5] = {v0,v1,v2,v3,v4}; double bb2[5] = {c0+1,c1+2,c2+3,c3+4,c4+5};
    acc += dot(aa2,bb2,5);
    double s_3 = 0.0;
    for (int k3 = 0; k3 < 249; k3++) { s_3 += v3 * c0; if (k3 & 1) s_3 -= c0 * 0.5; }
    acc += s_3;
    double aa4[5] = {v0,v1,v2,v3,v4}; double bb4[5] = {c0+1,c1+2,c2+3,c3+4,c4+5};
    acc += dot(aa4,bb4,5);
    float g5a = (float)v0 + (float)c2;
    float g5b = g5a; for (int w5=0;w5<50;w5++) g5b = g5b*(float)0.5 + (float)c5;
    acc += (double)g5b;
    mixv = 0;
    double u = acc; unsigned long long h = 0x9e3779b97f4a7c15ULL;
    for (int i = 0; i < 64; i++) { h ^= (unsigned long long)(u * (i+1)); u = u*0.999 + 0.001; h = (h<<7)|(h>>57); }
    printf("fpstress %llu\n", (unsigned long long)h);
    return 0;
}
