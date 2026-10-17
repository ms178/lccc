// AUD-VABI-4 exhaustive regression: one-lane FP vectors (f32x1/f64x1)
// return/pass in xmm0 without GPR bounce; arithmetic, fptrs, volatile,
// memcpy, NaN/Inf/subnormal chains all work at O0/O1/O2.
#include <stdio.h>
#include <string.h>
typedef double double1 __attribute__((ext_vector_type(1)));
typedef float float1 __attribute__((ext_vector_type(1)));
typedef unsigned char u8x1 __attribute__((ext_vector_type(1)));
typedef unsigned short u16x1 __attribute__((ext_vector_type(1)));
typedef unsigned u32x1 __attribute__((ext_vector_type(1)));
typedef unsigned long long u64x1 __attribute__((ext_vector_type(1)));
typedef double1 (*D1D1)(double1);
typedef float1 (*F1F1)(float1);
typedef u64x1 (*U64U64)(u64x1);
__attribute__((noinline)) double1 did(double1 a){return a;}
__attribute__((noinline)) float1 fid(float1 a){return a;}
__attribute__((noinline)) u8x1 u8id(u8x1 a){return a;}
__attribute__((noinline)) u16x1 u16id(u16x1 a){return a;}
__attribute__((noinline)) u32x1 u32id(u32x1 a){return a;}
__attribute__((noinline)) u64x1 u64id(u64x1 a){return a;}
__attribute__((noinline)) double1 dadd(double1 a,double1 b){return a+b;}
__attribute__((noinline)) double1 dsub(double1 a,double1 b){return a-b;}
__attribute__((noinline)) double1 dmul(double1 a,double1 b){return a*b;}
__attribute__((noinline)) double1 ddiv(double1 a,double1 b){return a/b;}
__attribute__((noinline)) double1 dneg(double1 a){return -a;}
__attribute__((noinline)) float1 fadd(float1 a,float1 b){return a+b;}
__attribute__((noinline)) float1 fsub(float1 a,float1 b){return a-b;}
__attribute__((noinline)) float1 fmul(float1 a,float1 b){return a*b;}
__attribute__((noinline)) float1 fdiv(float1 a,float1 b){return a/b;}
__attribute__((noinline)) float1 fneg(float1 a){return -a;}
__attribute__((noinline)) double1 call_via_d(D1D1 p,double1 a){return p(a);}
__attribute__((noinline)) float1 call_via_f(F1F1 p,float1 a){return p(a);}
static double as_d(double1 v){double r;memcpy(&r,&v,8);return r;}
static float as_f(float1 v){float r;memcpy(&r,&v,4);return r;}
static unsigned char as_u8(u8x1 v){unsigned char r;memcpy(&r,&v,1);return r;}
static unsigned short as_u16(u16x1 v){unsigned short r;memcpy(&r,&v,2);return r;}
static unsigned as_u32(u32x1 v){unsigned r;memcpy(&r,&v,4);return r;}
static unsigned long long as_u64(u64x1 v){unsigned long long r;memcpy(&r,&v,8);return r;}
int fails=0;
#define DCK(E,W) do{double e=E;if(e!=(W)){fprintf(stderr,"FAIL: %s line %d got %g want %g\n",#E,__LINE__,e,(double)(W));fails++;}}while(0)
#define FCK(E,W) do{float e=E;if(e!=(W)){fprintf(stderr,"FAIL: %s line %d got %g want %g\n",#E,__LINE__,(double)e,(double)(W));fails++;}}while(0)
#define UCK(E,W) do{unsigned long long e=E;if(e!=(unsigned long long)(W)){fprintf(stderr,"FAIL: %s line %d got 0x%llx want 0x%llx\n",#E,__LINE__,e,(unsigned long long)(W));fails++;}}while(0)
int main(){
  double1 dx=(double1){3.25}; float1 fx=(float1){2.5f};
  DCK(as_d(did(dx)),3.25);FCK(as_f(fid(fx)),2.5f);
  UCK(as_u8(u8id((u8x1){0xABu})),0xABu);
  UCK(as_u16(u16id((u16x1){0xABCDu})),0xABCDu);
  UCK(as_u32(u32id((u32x1){0xDEADBEEFu})),0xDEADBEEFu);
  UCK(as_u64(u64id((u64x1){0xCAFEBABEDEADBEEFull})),0xCAFEBABEDEADBEEFull);
  D1D1 p=did; F1F1 fp=fid; U64U64 up=u64id;
  DCK(as_d(p(dx)),3.25);FCK(as_f(fp(fx)),2.5f);
  UCK(as_u64(up((u64x1){0x1234ull})),0x1234ull);
  DCK(as_d(call_via_d(did,dx)),3.25);FCK(as_f(call_via_f(fid,fx)),2.5f);
  volatile double1 vdx=(double1){9.75}; volatile float1 vfx=(float1){1.25f};
  DCK(as_d(did(vdx)),9.75);FCK(as_f(fid(vfx)),1.25f);
  DCK(as_d(dadd(dx,dx)),6.5);FCK(as_f(fadd(fx,fx)),5.0f);
  DCK(as_d(dsub(dx,dx)),0.0);FCK(as_f(fsub(fx,fx)),0.0f);
  DCK(as_d(dmul(dx,(double1){2.0})),6.5);FCK(as_f(fmul(fx,(float1){2.0f})),5.0f);
  DCK(as_d(ddiv(dx,(double1){2.0})),1.625);FCK(as_f(fdiv(fx,(float1){2.0f})),1.25f);
  DCK(as_d(dneg(dx)),-3.25);FCK(as_f(fneg(fx)),-2.5f);
  /* chains */
  DCK(as_d(dneg(ddiv(dmul(dsub(dadd(did(dx),(double1){0.5}),(double1){0.5}),(double1){2.0}),(double1){2.0}))),-3.25);
  FCK(as_f(fneg(fdiv(fmul(fsub(fadd(fid(fx),(float1){0.5f}),(float1){0.5f}),(float1){2.0f}),(float1){2.0f}))),-2.5f);
  DCK(as_d(dneg(ddiv(dmul(dsub(dadd(did(dx),(double1){1.0}),(double1){0.25}),(double1){2.0}),(double1){2.0}))),-4.0);
  FCK(as_f(fneg(fdiv(fmul(fsub(fadd(fid(fx),(float1){1.0f}),(float1){0.25f}),(float1){2.0f}),(float1){2.0f}))),-3.25f);
  /* NaN / Inf / subnormal */
  DCK(!!(as_d(did((double1){(0.0/0.0)}))!=as_d(did((double1){(0.0/0.0)}))),1);
  DCK(as_d(dadd((double1){1.0/0.0},(double1){-1.0/0.0}))!=as_d(dadd((double1){1.0/0.0},(double1){-1.0/0.0})),1);
  {double1 sub; unsigned long long b=0x1000000ULL; memcpy(&sub,&b,8);
    double1 d=did(sub); unsigned long long rb; memcpy(&rb,&d,8); UCK(rb,b);}
  fprintf(stderr,"fails=%d\n",fails); return fails;
}