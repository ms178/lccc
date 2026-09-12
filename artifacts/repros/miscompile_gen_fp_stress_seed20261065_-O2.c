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
    double v0 = -213.3495514793;
    double v1 = 185.9586313879;
    double v2 = 345.7059251876;
    double v3 = -553.8374599576;
    double v4 = -758.4921652439;
    double v5 = -601.5515031144;
    const double c0 = 0;
    const double c1 = 1;
    const double c2 = -1;
    const double c3 = 0.5;
    const double c4 = -77.925778517789453;
    const double c5 = -16.602725502555685;
    const double c6 = 3;
    const double c7 = 2;
    double s_0 = v1;
    s_0 = s_0 * c4 + v4;
    s_0 = s_0 * c4 + v1;
    s_0 = s_0 * c4 + v5;
    s_0 = s_0 * c4 + v5;
    s_0 = s_0 * c4 + v5;
    s_0 = s_0 * c4 + v4;
    acc += s_0;
    double s_1 = v2;
    s_1 = s_1 * c4 + v3;
    s_1 = s_1 * c4 + v1;
    s_1 = s_1 * c4 + v0;
    s_1 = s_1 * c4 + v5;
    acc += s_1;
    double s_2 = 0.0;
    for (int k2 = 0; k2 < 42; k2++) { s_2 += v4 * c7; if (k2 & 1) s_2 -= c7 * 0.5; }
    acc += s_2;
    float g3a = (float)v0 + (float)c5;
    float g3b = g3a; for (int w3=0;w3<50;w3++) g3b = g3b*(float)0.5 + (float)c7;
    acc += (double)g3b;
    double s_4 = 0.0;
    for (int k4 = 0; k4 < 104; k4++) { s_4 += v3 * c5; if (k4 & 1) s_4 -= c5 * 0.5; }
    acc += s_4;
    double s_5 = 0.0;
    for (int k5 = 0; k5 < 363; k5++) { s_5 += v4 * c3; if (k5 & 1) s_5 -= c3 * 0.5; }
    acc += s_5;
    double s_6 = v5;
    s_6 = s_6 * c4 + v2;
    s_6 = s_6 * c4 + v3;
    s_6 = s_6 * c4 + v4;
    s_6 = s_6 * c4 + v4;
    acc += s_6;
    mixv = 0;
    double u = acc; unsigned long long h = 0x9e3779b97f4a7c15ULL;
    for (int i = 0; i < 64; i++) { h ^= (unsigned long long)(u * (i+1)); u = u*0.999 + 0.001; h = (h<<7)|(h>>57); }
    printf("fpstress %llu\n", (unsigned long long)h);
    return 0;
}
