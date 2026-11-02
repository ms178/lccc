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
    double v0 = -733.2026849950;
    double v1 = 121.3809775531;
    double v2 = 327.8770697278;
    double v3 = -179.5802481807;
    double v4 = -407.0818358057;
    double v5 = -651.6190007547;
    const double c0 = 0;
    const double c1 = 1;
    const double c2 = -1;
    const double c3 = 0.5;
    const double c4 = 41.501876095695025;
    const double c5 = 90.711007787047009;
    const double c6 = 3;
    const double c7 = 2;
    double aa0[5] = {v0,v1,v2,v3,v4}; double bb0[5] = {c0+1,c1+2,c2+3,c3+4,c4+5};
    acc += dot(aa0,bb0,5);
    float g1a = (float)v0 + (float)c4;
    float g1b = g1a; for (int w1=0;w1<50;w1++) g1b = g1b*(float)0.5 + (float)c0;
    acc += (double)g1b;
    double aa2[5] = {v0,v1,v2,v3,v4}; double bb2[5] = {c0+1,c1+2,c2+3,c3+4,c4+5};
    acc += dot(aa2,bb2,5);
    double s_3 = v5;
    s_3 = s_3 * c0 + v5;
    s_3 = s_3 * c0 + v4;
    acc += s_3;
    double s_4 = v4 + c5;
    s_4 = opaque(s_4) + c5;
    s_4 = s_4 * c5 - opaque(v3);
    acc += s_4;
    double s_5 = v4;
    s_5 = s_5 * c5 + v3;
    s_5 = s_5 * c5 + v0;
    s_5 = s_5 * c5 + v3;
    s_5 = s_5 * c5 + v5;
    s_5 = s_5 * c5 + v3;
    acc += s_5;
    double s_6 = v4;
    if ((uintptr_t)&s_6 & 1) { s_6 += c0; barrier(); } else { s_6 = c0 - v4; }
    acc += s_6 * c0;
    double aa7[5] = {v0,v1,v2,v3,v4}; double bb7[5] = {c0+1,c1+2,c2+3,c3+4,c4+5};
    acc += dot(aa7,bb7,5);
    mixv = 0;
    double u = acc; unsigned long long h = 0x9e3779b97f4a7c15ULL;
    for (int i = 0; i < 64; i++) { h ^= (unsigned long long)(u * (i+1)); u = u*0.999 + 0.001; h = (h<<7)|(h>>57); }
    printf("fpstress %llu\n", (unsigned long long)h);
    return 0;
}
