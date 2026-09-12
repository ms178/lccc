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
    double v0 = 22.0653439635;
    double v1 = 346.1963241024;
    double v2 = 71.4012211107;
    double v3 = 350.5432362072;
    double v4 = -367.1843094051;
    double v5 = 234.0868033181;
    const double c0 = 0;
    const double c1 = 1;
    const double c2 = -1;
    const double c3 = 0.5;
    const double c4 = -53.213781713085041;
    const double c5 = 3.6258347746650941;
    const double c6 = 3;
    const double c7 = 2;
    double s_0 = v4;
    s_0 = s_0 * c4 + v4;
    s_0 = s_0 * c4 + v5;
    s_0 = s_0 * c4 + v3;
    s_0 = s_0 * c4 + v2;
    s_0 = s_0 * c4 + v0;
    acc += s_0;
    double s_1 = 0.0;
    for (int k1 = 0; k1 < 85; k1++) { s_1 += v5 * c3; if (k1 & 1) s_1 -= c3 * 0.5; }
    acc += s_1;
    double s_2 = v5;
    if ((uintptr_t)&s_2 & 1) { s_2 += c1; barrier(); } else { s_2 = c1 - v0; }
    acc += s_2 * c1;
    double s_3 = v4 + c5;
    s_3 = opaque(s_3) + c5;
    s_3 = s_3 * c5 - opaque(v5);
    acc += s_3;
    double s_4 = v0;
    s_4 = s_4 * c2 + v0;
    s_4 = s_4 * c2 + v1;
    s_4 = s_4 * c2 + v2;
    acc += s_4;
    double aa5[5] = {v0,v1,v2,v3,v4}; double bb5[5] = {c0+1,c1+2,c2+3,c3+4,c4+5};
    acc += dot(aa5,bb5,5);
    double s_6 = v1;
    s_6 = s_6 * c2 + v1;
    s_6 = s_6 * c2 + v0;
    acc += s_6;
    double s_7 = v1 + c4;
    s_7 = opaque(s_7) + c4;
    s_7 = s_7 * c4 - opaque(v0);
    acc += s_7;
    mixv = 0;
    double u = acc; unsigned long long h = 0x9e3779b97f4a7c15ULL;
    for (int i = 0; i < 64; i++) { h ^= (unsigned long long)(u * (i+1)); u = u*0.999 + 0.001; h = (h<<7)|(h>>57); }
    printf("fpstress %llu\n", (unsigned long long)h);
    return 0;
}
