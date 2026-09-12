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
    double v0 = -10.0075290245;
    double v1 = -259.3549184152;
    double v2 = -101.3278343749;
    double v3 = -257.7434147013;
    double v4 = 905.5690626826;
    double v5 = -34.6739137594;
    const double c0 = 0;
    const double c1 = 1;
    const double c2 = -1;
    const double c3 = 0.5;
    const double c4 = -43.423921265050105;
    const double c5 = 61.199540285266096;
    const double c6 = 3;
    const double c7 = 2;
    double s_0 = 0.0;
    for (int k0 = 0; k0 < 191; k0++) { s_0 += v3 * c6; if (k0 & 1) s_0 -= c6 * 0.5; }
    acc += s_0;
    double s_1 = 0.0;
    for (int k1 = 0; k1 < 366; k1++) { s_1 += v2 * c2; if (k1 & 1) s_1 -= c2 * 0.5; }
    acc += s_1;
    double s_2 = 0.0;
    for (int k2 = 0; k2 < 266; k2++) { s_2 += v1 * c5; if (k2 & 1) s_2 -= c5 * 0.5; }
    acc += s_2;
    double s_3 = v4;
    s_3 = s_3 * c5 + v4;
    s_3 = s_3 * c5 + v1;
    s_3 = s_3 * c5 + v5;
    acc += s_3;
    float g4a = (float)v0 + (float)c5;
    float g4b = g4a; for (int w4=0;w4<50;w4++) g4b = g4b*(float)0.5 + (float)c7;
    acc += (double)g4b;
    double s_5 = v4;
    s_5 = s_5 * c5 + v2;
    s_5 = s_5 * c5 + v4;
    s_5 = s_5 * c5 + v2;
    s_5 = s_5 * c5 + v4;
    s_5 = s_5 * c5 + v4;
    s_5 = s_5 * c5 + v5;
    acc += s_5;
    double aa6[5] = {v0,v1,v2,v3,v4}; double bb6[5] = {c0+1,c1+2,c2+3,c3+4,c4+5};
    acc += dot(aa6,bb6,5);
    double s_7 = v0;
    s_7 = s_7 * c4 + v4;
    s_7 = s_7 * c4 + v4;
    s_7 = s_7 * c4 + v1;
    s_7 = s_7 * c4 + v2;
    acc += s_7;
    double s_8 = v5 + c7;
    s_8 = opaque(s_8) + c7;
    s_8 = s_8 * c7 - opaque(v4);
    acc += s_8;
    mixv = 0;
    double u = acc; unsigned long long h = 0x9e3779b97f4a7c15ULL;
    for (int i = 0; i < 64; i++) { h ^= (unsigned long long)(u * (i+1)); u = u*0.999 + 0.001; h = (h<<7)|(h>>57); }
    printf("fpstress %llu\n", (unsigned long long)h);
    return 0;
}
