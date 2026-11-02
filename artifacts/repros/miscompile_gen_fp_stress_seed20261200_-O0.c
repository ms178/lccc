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
    double v0 = -610.4814612113;
    double v1 = -230.8383307093;
    double v2 = -971.0513910498;
    double v3 = 522.2263211697;
    double v4 = -872.1553232976;
    double v5 = 480.8467817207;
    const double c0 = 0;
    const double c1 = 1;
    const double c2 = -1;
    const double c3 = 0.5;
    const double c4 = -91.881196779222648;
    const double c5 = -72.207781449122436;
    const double c6 = 3;
    const double c7 = 2;
    double s_0 = v0;
    s_0 = s_0 * c4 + v3;
    s_0 = s_0 * c4 + v4;
    s_0 = s_0 * c4 + v2;
    s_0 = s_0 * c4 + v5;
    s_0 = s_0 * c4 + v2;
    acc += s_0;
    double s_1 = 0.0;
    for (int k1 = 0; k1 < 271; k1++) { s_1 += v4 * c6; if (k1 & 1) s_1 -= c6 * 0.5; }
    acc += s_1;
    double s_2 = v5;
    if ((uintptr_t)&s_2 & 1) { s_2 += c1; barrier(); } else { s_2 = c1 - v3; }
    acc += s_2 * c1;
    double s_3 = 0.0;
    for (int k3 = 0; k3 < 343; k3++) { s_3 += v1 * c2; if (k3 & 1) s_3 -= c2 * 0.5; }
    acc += s_3;
    double aa4[5] = {v0,v1,v2,v3,v4}; double bb4[5] = {c0+1,c1+2,c2+3,c3+4,c4+5};
    acc += dot(aa4,bb4,5);
    double s_5 = v3 + c3;
    s_5 = opaque(s_5) + c3;
    s_5 = s_5 * c3 - opaque(v4);
    acc += s_5;
    double aa6[5] = {v0,v1,v2,v3,v4}; double bb6[5] = {c0+1,c1+2,c2+3,c3+4,c4+5};
    acc += dot(aa6,bb6,5);
    double s_7 = v5;
    if ((uintptr_t)&s_7 & 1) { s_7 += c1; barrier(); } else { s_7 = c1 - v5; }
    acc += s_7 * c1;
    mixv = 0;
    double u = acc; unsigned long long h = 0x9e3779b97f4a7c15ULL;
    for (int i = 0; i < 64; i++) { h ^= (unsigned long long)(u * (i+1)); u = u*0.999 + 0.001; h = (h<<7)|(h>>57); }
    printf("fpstress %llu\n", (unsigned long long)h);
    return 0;
}
