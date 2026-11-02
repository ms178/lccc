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
    double v0 = 105.1555984883;
    double v1 = -80.2754363573;
    double v2 = 696.6341124324;
    double v3 = -829.2585141934;
    double v4 = 557.2549980007;
    double v5 = 440.1832890927;
    const double c0 = 0;
    const double c1 = 1;
    const double c2 = -1;
    const double c3 = 0.5;
    const double c4 = -53.872513106035228;
    const double c5 = 7.2107424659943291;
    const double c6 = 3;
    const double c7 = 2;
    double s_0 = v4 + c7;
    s_0 = opaque(s_0) + c7;
    s_0 = s_0 * c7 - opaque(v4);
    acc += s_0;
    double s_1 = v1;
    if ((uintptr_t)&s_1 & 1) { s_1 += c3; barrier(); } else { s_1 = c3 - v5; }
    acc += s_1 * c3;
    double s_2 = v0;
    s_2 = s_2 * c4 + v2;
    s_2 = s_2 * c4 + v3;
    s_2 = s_2 * c4 + v0;
    s_2 = s_2 * c4 + v1;
    acc += s_2;
    float g3a = (float)v0 + (float)c4;
    float g3b = g3a; for (int w3=0;w3<50;w3++) g3b = g3b*(float)0.5 + (float)c4;
    acc += (double)g3b;
    float g4a = (float)v0 + (float)c0;
    float g4b = g4a; for (int w4=0;w4<50;w4++) g4b = g4b*(float)0.5 + (float)c2;
    acc += (double)g4b;
    double s_5 = 0.0;
    for (int k5 = 0; k5 < 349; k5++) { s_5 += v1 * c6; if (k5 & 1) s_5 -= c6 * 0.5; }
    acc += s_5;
    double s_6 = v0;
    s_6 = s_6 * c4 + v3;
    s_6 = s_6 * c4 + v2;
    s_6 = s_6 * c4 + v1;
    s_6 = s_6 * c4 + v1;
    s_6 = s_6 * c4 + v2;
    s_6 = s_6 * c4 + v3;
    acc += s_6;
    mixv = 0;
    double u = acc; unsigned long long h = 0x9e3779b97f4a7c15ULL;
    for (int i = 0; i < 64; i++) { h ^= (unsigned long long)(u * (i+1)); u = u*0.999 + 0.001; h = (h<<7)|(h>>57); }
    printf("fpstress %llu\n", (unsigned long long)h);
    return 0;
}
