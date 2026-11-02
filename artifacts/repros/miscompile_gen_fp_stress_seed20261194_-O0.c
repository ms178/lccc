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
    double v0 = 530.4172555086;
    double v1 = -604.9003036836;
    double v2 = 312.6820141136;
    double v3 = 130.8165761236;
    double v4 = 271.6837399183;
    double v5 = -591.1116038688;
    const double c0 = 0;
    const double c1 = 1;
    const double c2 = -1;
    const double c3 = 0.5;
    const double c4 = -88.720243902687471;
    const double c5 = 76.467106671830237;
    const double c6 = 3;
    const double c7 = 2;
    double s_0 = 0.0;
    for (int k0 = 0; k0 < 312; k0++) { s_0 += v3 * c6; if (k0 & 1) s_0 -= c6 * 0.5; }
    acc += s_0;
    float g1a = (float)v0 + (float)c2;
    float g1b = g1a; for (int w1=0;w1<50;w1++) g1b = g1b*(float)0.5 + (float)c3;
    acc += (double)g1b;
    double s_2 = v5 + c4;
    s_2 = opaque(s_2) + c4;
    s_2 = s_2 * c4 - opaque(v3);
    acc += s_2;
    double s_3 = v1;
    s_3 = s_3 * c5 + v2;
    s_3 = s_3 * c5 + v1;
    s_3 = s_3 * c5 + v4;
    s_3 = s_3 * c5 + v1;
    s_3 = s_3 * c5 + v5;
    s_3 = s_3 * c5 + v2;
    acc += s_3;
    float g4a = (float)v0 + (float)c3;
    float g4b = g4a; for (int w4=0;w4<50;w4++) g4b = g4b*(float)0.5 + (float)c3;
    acc += (double)g4b;
    mixv = 0;
    double u = acc; unsigned long long h = 0x9e3779b97f4a7c15ULL;
    for (int i = 0; i < 64; i++) { h ^= (unsigned long long)(u * (i+1)); u = u*0.999 + 0.001; h = (h<<7)|(h>>57); }
    printf("fpstress %llu\n", (unsigned long long)h);
    return 0;
}
