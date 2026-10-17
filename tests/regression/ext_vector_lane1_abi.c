// AUD-VABI-4 regression: one-lane ext_vector_type must use the right carrier.
// Prior to the fix, f64x1 negation segfaulted and identity bounced via GPRs.
typedef double double1 __attribute__((ext_vector_type(1)));
typedef float float1 __attribute__((ext_vector_type(1)));
typedef unsigned char u8x1 __attribute__((ext_vector_type(1)));
typedef unsigned short u16x1 __attribute__((ext_vector_type(1)));
typedef unsigned int u32x1 __attribute__((ext_vector_type(1)));
typedef unsigned long long u64x1 __attribute__((ext_vector_type(1)));
typedef long long i64x1 __attribute__((ext_vector_type(1)));
typedef signed char i8x1 __attribute__((ext_vector_type(1)));
typedef short i16x1 __attribute__((ext_vector_type(1)));
typedef int i32x1 __attribute__((ext_vector_type(1)));

double1 did(double1 a){return a;}
double1 dadd(double1 a,double1 b){return a+b;}
double1 dsub(double1 a,double1 b){return a-b;}
double1 dmul(double1 a,double1 b){return a*b;}
double1 ddiv(double1 a,double1 b){return a/b;}
double1 dneg(double1 a){return -a;}
float1 fid(float1 a){return a;}
float1 fadd(float1 a,float1 b){return a+b;}
float1 fneg(float1 a){return -a;}
u8x1 u8id(u8x1 a){return a;}
u16x1 u16id(u16x1 a){return a;}
u32x1 u32id(u32x1 a){return a;}
u64x1 u64id(u64x1 a){return a;}
i64x1 i64id(i64x1 a){return a;}
i8x1 i8id(i8x1 a){return a;}
i16x1 i16id(i16x1 a){return a;}
i32x1 i32id(i32x1 a){return a;}

int printf(const char*, ...);
#define D(r,w) do{if(__builtin_fabs((double)(r)-(w))>0.0001){printf("FAIL %s=%g\n",#r,(double)(r));return 1;}}while(0)
#define U(r,w) do{if((unsigned long long)(r)!=(w)){printf("FAIL %s=0x%llx\n",#r,(unsigned long long)(r));return 1;}}while(0)
#define I(r,w) do{if((long long)(r)!=(w)){printf("FAIL %s=%lld\n",#r,(long long)(r));return 1;}}while(0)
int main(void){
  double1 a={3.25},b={1.25};
  float1 fa={2.5f},fb={0.5f};
  u8x1 u8={200}; u16x1 u16={50000}; u32x1 u32={1234567};
  u64x1 u64={0xdeadbeefcafebabeULL};
  i8x1 i8={-50}; i16x1 i16={-20000}; i32x1 i32={-1234567}; i64x1 i64={-42};
  D(did(a)[0],3.25); D(dadd(a,b)[0],4.5); D(dneg(a)[0],-3.25);
  D(dsub(a,b)[0],2.0); D(dmul(a,b)[0],4.0625); D(ddiv(a,b)[0],2.6);
  D(fid(fa)[0],2.5); D(fadd(fa,fb)[0],3.0); D(fneg(fa)[0],-2.5);
  U(u8id(u8)[0],200); U(u16id(u16)[0],50000); U(u32id(u32)[0],1234567);
  U(u64id(u64)[0],0xdeadbeefcafebabeULL);
  I((int)(signed char)i8id(i8)[0],-50); I(i16id(i16)[0],-20000);
  I(i32id(i32)[0],-1234567); I(i64id(i64)[0],-42);
  printf("OK ext_vector_lane1_abi\n");
  return 0;
}
