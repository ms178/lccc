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
    double v0 = 299.4669603418;
    double v1 = -406.6931849650;
    double v2 = 59.5122244693;
    double v3 = 244.0229181118;
    double v4 = 989.7920475617;
    double v5 = -580.7371787579;
    const double c0 = 0;
    const double c1 = 1;
    const double c2 = -1;
    const double c3 = 0.5;
    const double c4 = 19.403968311539074;
    const double c5 = 70.092252318958202;
    const double c6 = 3;
    const double c7 = 2;
    double s_0 = v5;
    if ((uintptr_t)&s_0 & 1) { s_0 += c5; barrier(); } else { s_0 = c5 - v4; }
    acc += s_0 * c5;
    double s_1 = 0.0;
    for (int k1 = 0; k1 < 197; k1++) { s_1 += v2 * c7; if (k1 & 1) s_1 -= c7 * 0.5; }
    acc += s_1;
    double s_2 = v1;
    s_2 = s_2 * c6 + v4;
    s_2 = s_2 * c6 + v1;
    acc += s_2;
    double s_3 = v4 + c0;
    s_3 = opaque(s_3) + c0;
    s_3 = s_3 * c0 - opaque(v4);
    acc += s_3;
    float g4a = (float)v0 + (float)c6;
    float g4b = g4a; for (int w4=0;w4<50;w4++) g4b = g4b*(float)0.5 + (float)c4;
    acc += (double)g4b;
    double s_5 = v1;
    s_5 = s_5 * c4 + v2;
    s_5 = s_5 * c4 + v5;
    s_5 = s_5 * c4 + v4;
    s_5 = s_5 * c4 + v4;
    s_5 = s_5 * c4 + v2;
    s_5 = s_5 * c4 + v5;
    acc += s_5;
    mixv = 0;
    double u = acc; unsigned long long h = 0x9e3779b97f4a7c15ULL;
    for (int i = 0; i < 64; i++) { h ^= (unsigned long long)(u * (i+1)); u = u*0.999 + 0.001; h = (h<<7)|(h>>57); }
    printf("fpstress %llu\n", (unsigned long long)h);
    return 0;
}
