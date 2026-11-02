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
    double v0 = -324.9253954862;
    double v1 = 497.0298563680;
    double v2 = 815.4002262870;
    double v3 = -599.4059599567;
    double v4 = -364.1991361717;
    double v5 = -86.6888452961;
    const double c0 = 0;
    const double c1 = 1;
    const double c2 = -1;
    const double c3 = 0.5;
    const double c4 = -94.433628355596426;
    const double c5 = -0.048286315468004659;
    const double c6 = 3;
    const double c7 = 2;
    double s_0 = v0;
    s_0 = s_0 * c5 + v2;
    s_0 = s_0 * c5 + v1;
    s_0 = s_0 * c5 + v5;
    s_0 = s_0 * c5 + v1;
    acc += s_0;
    double s_1 = 0.0;
    for (int k1 = 0; k1 < 371; k1++) { s_1 += v2 * c4; if (k1 & 1) s_1 -= c4 * 0.5; }
    acc += s_1;
    double s_2 = v4;
    if ((uintptr_t)&s_2 & 1) { s_2 += c2; barrier(); } else { s_2 = c2 - v0; }
    acc += s_2 * c2;
    float g3a = (float)v0 + (float)c5;
    float g3b = g3a; for (int w3=0;w3<50;w3++) g3b = g3b*(float)0.5 + (float)c4;
    acc += (double)g3b;
    double s_4 = v3 + c6;
    s_4 = opaque(s_4) + c6;
    s_4 = s_4 * c6 - opaque(v4);
    acc += s_4;
    double s_5 = v3;
    s_5 = s_5 * c4 + v2;
    s_5 = s_5 * c4 + v0;
    s_5 = s_5 * c4 + v5;
    s_5 = s_5 * c4 + v3;
    s_5 = s_5 * c4 + v5;
    s_5 = s_5 * c4 + v4;
    acc += s_5;
    mixv = 0;
    double u = acc; unsigned long long h = 0x9e3779b97f4a7c15ULL;
    for (int i = 0; i < 64; i++) { h ^= (unsigned long long)(u * (i+1)); u = u*0.999 + 0.001; h = (h<<7)|(h>>57); }
    printf("fpstress %llu\n", (unsigned long long)h);
    return 0;
}
