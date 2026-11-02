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
    double v0 = -41.0827422068;
    double v1 = -636.5102549083;
    double v2 = 366.9703337307;
    double v3 = 248.4234529341;
    double v4 = 601.3221600008;
    double v5 = -590.4937234517;
    const double c0 = 0;
    const double c1 = 1;
    const double c2 = -1;
    const double c3 = 0.5;
    const double c4 = -89.930576791000519;
    const double c5 = 93.059023844388832;
    const double c6 = 3;
    const double c7 = 2;
    float g0a = (float)v0 + (float)c3;
    float g0b = g0a; for (int w0=0;w0<50;w0++) g0b = g0b*(float)0.5 + (float)c4;
    acc += (double)g0b;
    double aa1[5] = {v0,v1,v2,v3,v4}; double bb1[5] = {c0+1,c1+2,c2+3,c3+4,c4+5};
    acc += dot(aa1,bb1,5);
    double s_2 = v4;
    if ((uintptr_t)&s_2 & 1) { s_2 += c0; barrier(); } else { s_2 = c0 - v0; }
    acc += s_2 * c0;
    double s_3 = 0.0;
    for (int k3 = 0; k3 < 77; k3++) { s_3 += v5 * c2; if (k3 & 1) s_3 -= c2 * 0.5; }
    acc += s_3;
    double s_4 = v0;
    s_4 = s_4 * c5 + v5;
    s_4 = s_4 * c5 + v0;
    s_4 = s_4 * c5 + v4;
    s_4 = s_4 * c5 + v0;
    s_4 = s_4 * c5 + v2;
    acc += s_4;
    double aa5[5] = {v0,v1,v2,v3,v4}; double bb5[5] = {c0+1,c1+2,c2+3,c3+4,c4+5};
    acc += dot(aa5,bb5,5);
    double s_6 = v4 + c1;
    s_6 = opaque(s_6) + c1;
    s_6 = s_6 * c1 - opaque(v3);
    acc += s_6;
    double s_7 = 0.0;
    for (int k7 = 0; k7 < 312; k7++) { s_7 += v0 * c5; if (k7 & 1) s_7 -= c5 * 0.5; }
    acc += s_7;
    double s_8 = v3;
    if ((uintptr_t)&s_8 & 1) { s_8 += c0; barrier(); } else { s_8 = c0 - v1; }
    acc += s_8 * c0;
    mixv = 0;
    double u = acc; unsigned long long h = 0x9e3779b97f4a7c15ULL;
    for (int i = 0; i < 64; i++) { h ^= (unsigned long long)(u * (i+1)); u = u*0.999 + 0.001; h = (h<<7)|(h>>57); }
    printf("fpstress %llu\n", (unsigned long long)h);
    return 0;
}
