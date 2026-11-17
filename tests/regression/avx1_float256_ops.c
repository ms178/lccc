#include <stdio.h>
#include <string.h>
/* Regression: 256-bit FLOAT and float<->int conversion operations under -mavx
 * alone. Each kernel is a 32-byte vector op that the AVX1 permission now
 * lowers to VEX.256 code: lane compares feeding an integer-mask select, a
 * reversing shuffle, division with addition, and both directions of
 * __builtin_convertvector. The output of every call is folded into one FNV
 * hash; the program is differential against GCC 16.2 (`-O2 -mavx`), so any
 * lane that differs changes the printed hash. The mask select is written
 * with integer bit operations because C does not allow the vector ?: form
 * outside C++. Inputs are finite, so NaN payload rules do not apply. */
typedef float v8sf __attribute__((vector_size(32)));
typedef double v4df __attribute__((vector_size(32)));
typedef int v8si __attribute__((vector_size(32)));
void k1(float *o, const float *a, const float *b) { v8sf x, y; memcpy(&x,a,32); memcpy(&y,b,32); v8si m = (v8si)(x < y); v8sf r = (v8sf)((m & (v8si)x) | (~m & (v8si)y)); memcpy(o,&r,32); }
void k2(float *o, const float *a, const float *b) { v8sf x, y; memcpy(&x,a,32); memcpy(&y,b,32); v8si m = (v8si)(x > y); v8sf r = (v8sf)((m & (v8si)x) | (~m & (v8si)y)); memcpy(o,&r,32); }
void k3(float *o, const float *a) { v8sf x; memcpy(&x,a,32); v8sf r = __builtin_shufflevector(x, x, 7,6,5,4,3,2,1,0); memcpy(o,&r,32); }
void k4(double *o, const double *a) { v4df x; memcpy(&x,a,32); v4df r = __builtin_shufflevector(x, x, 3,2,1,0); memcpy(o,&r,32); }
void k5(float *o, const float *a, const float *b) { v8sf x, y; memcpy(&x,a,32); memcpy(&y,b,32); v8sf r = x / y + x * y; memcpy(o,&r,32); }
void k6(int *o, const float *a) { v8sf x; memcpy(&x,a,32); v8si r = __builtin_convertvector(x, v8si); memcpy(o,&r,32); }
void k7(float *o, const int *a) { v8si x; memcpy(&x,a,32); v8sf r = __builtin_convertvector(x, v8sf); memcpy(o,&r,32); }
void k8(double *o, const double *a, const double *b) { v4df x, y; memcpy(&x,a,32); memcpy(&y,b,32); typedef long long v4di __attribute__((vector_size(32))); v4di m = (v4di)(x < y); v4df r = (v4df)((m & (v4di)x) | (~m & (v4di)y)); memcpy(o,&r,32); }
void k9(float *o, const float *a) { v8sf x; memcpy(&x,a,32); v8sf z = {0}; v8sf r = x == z; memcpy(o,&r,32); }
#include <string.h>
void k1(float *o, const float *a, const float *b);
void k2(float *o, const float *a, const float *b);
void k3(float *o, const float *a);
void k4(double *o, const double *a);
void k5(float *o, const float *a, const float *b);
void k6(int *o, const float *a);
void k7(float *o, const int *a);
void k8(double *o, const double *a, const double *b);
void k9(float *o, const float *a);
static unsigned long long fold(const void *p, unsigned n, unsigned long long h){ const unsigned char *c=p; for(unsigned i=0;i<n;i++) h=(h^c[i])*1099511628211ull; return h; }
int main(void){
  float a[8], b[8], o[8]; double da[4], db[4], od[4]; int ia[8], oi[8];
  unsigned long long h=1469598103934665603ull; unsigned s=7;
  for (int t=0;t<50;t++){
    for(int i=0;i<8;i++){ s=s*1664525u+1013904223u; a[i]=(float)((int)(s>>8)-(1<<23))/1024.0f; s=s*1664525u+1013904223u; b[i]=(float)((int)(s>>8)-(1<<23))/512.0f; ia[i]=(int)(s>>9)-(1<<21);}
    for(int i=0;i<4;i++){ da[i]=a[i]*3.5; db[i]=b[i]-1.25; }
    k1(o,a,b); h=fold(o,32,h); k2(o,a,b); h=fold(o,32,h); k3(o,a); h=fold(o,32,h);
    k4(od,da); h=fold(od,32,h); k5(o,a,b); h=fold(o,32,h); k6(oi,a); h=fold(oi,32,h);
    k7(o,ia); h=fold(o,32,h); k8(od,da,db); h=fold(od,32,h); k9(o,a); h=fold(o,32,h);
  }
  printf("wide_ops %016llx\n", h); return 0;
}
