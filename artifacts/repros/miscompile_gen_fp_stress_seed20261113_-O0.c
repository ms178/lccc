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
    double v0 = -642.6968124770;
    double v1 = 668.0950094852;
    double v2 = -271.8439412252;
    double v3 = 274.8799757920;
    double v4 = -980.7559779594;
    double v5 = -868.6149937605;
    const double c0 = 0;
    const double c1 = 1;
    const double c2 = -1;
    const double c3 = 0.5;
    const double c4 = -73.498795784959441;
    const double c5 = -90.315298152761244;
    const double c6 = 3;
    const double c7 = 2;
    float g0a = (float)v0 + (float)c1;
    float g0b = g0a; for (int w0=0;w0<50;w0++) g0b = g0b*(float)0.5 + (float)c4;
    acc += (double)g0b;
    double s_1 = 0.0;
    for (int k1 = 0; k1 < 27; k1++) { s_1 += v5 * c4; if (k1 & 1) s_1 -= c4 * 0.5; }
    acc += s_1;
    double s_2 = v4;
    s_2 = s_2 * c4 + v2;
    s_2 = s_2 * c4 + v0;
    s_2 = s_2 * c4 + v3;
    s_2 = s_2 * c4 + v3;
    s_2 = s_2 * c4 + v2;
    acc += s_2;
    double s_3 = v1;
    s_3 = s_3 * c4 + v0;
    s_3 = s_3 * c4 + v2;
    s_3 = s_3 * c4 + v3;
    acc += s_3;
    mixv = 0;
    double u = acc; unsigned long long h = 0x9e3779b97f4a7c15ULL;
    for (int i = 0; i < 64; i++) { h ^= (unsigned long long)(u * (i+1)); u = u*0.999 + 0.001; h = (h<<7)|(h>>57); }
    printf("fpstress %llu\n", (unsigned long long)h);
    return 0;
}
