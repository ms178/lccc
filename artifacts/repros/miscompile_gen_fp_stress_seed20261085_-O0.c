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
    double v0 = 361.4349219730;
    double v1 = -518.7997175613;
    double v2 = 633.3447687997;
    double v3 = -66.3438490442;
    double v4 = -149.5724982666;
    double v5 = -528.7414648480;
    const double c0 = 0;
    const double c1 = 1;
    const double c2 = -1;
    const double c3 = 0.5;
    const double c4 = 84.39662557989098;
    const double c5 = 85.739885854841077;
    const double c6 = 3;
    const double c7 = 2;
    double s_0 = v4;
    s_0 = s_0 * c7 + v5;
    s_0 = s_0 * c7 + v2;
    acc += s_0;
    double s_1 = v5;
    s_1 = s_1 * c4 + v0;
    s_1 = s_1 * c4 + v0;
    acc += s_1;
    float g2a = (float)v0 + (float)c0;
    float g2b = g2a; for (int w2=0;w2<50;w2++) g2b = g2b*(float)0.5 + (float)c4;
    acc += (double)g2b;
    double s_3 = v4;
    if ((uintptr_t)&s_3 & 1) { s_3 += c3; barrier(); } else { s_3 = c3 - v4; }
    acc += s_3 * c3;
    double s_4 = v1;
    s_4 = s_4 * c5 + v1;
    s_4 = s_4 * c5 + v0;
    s_4 = s_4 * c5 + v1;
    s_4 = s_4 * c5 + v3;
    s_4 = s_4 * c5 + v4;
    s_4 = s_4 * c5 + v2;
    acc += s_4;
    double aa5[5] = {v0,v1,v2,v3,v4}; double bb5[5] = {c0+1,c1+2,c2+3,c3+4,c4+5};
    acc += dot(aa5,bb5,5);
    mixv = 0;
    double u = acc; unsigned long long h = 0x9e3779b97f4a7c15ULL;
    for (int i = 0; i < 64; i++) { h ^= (unsigned long long)(u * (i+1)); u = u*0.999 + 0.001; h = (h<<7)|(h>>57); }
    printf("fpstress %llu\n", (unsigned long long)h);
    return 0;
}
