// AUD-VABI-4 regression: one-lane ext_vector_type must use the right carrier
// class AND sign-extend narrow-signed returns correctly.
//
//  * FP and integer lane values survive direct call-result indexing;
//  * signed narrow lanes retain their sign, unsigned lanes retain high bits.
// Register locations are determined by the ABI classifier, not lane type:
// ext f32x1 is INTEGER bits, ext f64x1 is a stack argument/SSE return,
// and ext u64x1 uses SSE on x86-64 (Clang 23.1.4).
//
// Previously i8x1 identity emitted `movsbl %dil,%edi; movzbl %dil,%eax`,
// returning 255 for -1; i64x1 returned the alloca address as a pointer.

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
typedef signed char sc_t;
typedef unsigned char uc_t;
typedef short ss_t;
typedef unsigned short us_t;

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
sc_t scid(sc_t a){return a;}
uc_t ucid(uc_t a){return a;}
ss_t ssid(ss_t a){return a;}
us_t usid(us_t a){return a;}

int printf(const char*, ...);
#define D(r,w) do{if(__builtin_fabs((double)(r)-(w))>0.0001){printf("FAIL %s=%g\n",#r,(double)(r));return 1;}}while(0)
#define U(r,w) do{if((unsigned long long)(r)!=(w)){printf("FAIL %s=0x%llx want 0x%llx\n",#r,(unsigned long long)(r),(unsigned long long)(w));return 1;}}while(0)
#define I(r,w) do{if((long long)(r)!=(w)){printf("FAIL %s=%lld want %lld\n",#r,(long long)(r),(long long)(w));return 1;}}while(0)
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
  // Check narrow SIGNED returns sign-extend (high-bit regression).
  I(i8id((i8x1){-1})[0],-1);
  I(i8id((i8x1){-128})[0],-128);
  I(i8id(i8)[0],-50);
  I(i16id((i16x1){-1})[0],-1);
  I(i16id((i16x1){-32768})[0],-32768);
  I(i16id(i16)[0],-20000);
  I(i32id((i32x1){-1})[0],-1);
  I(i32id(i32)[0],-1234567);
  I(i64id((i64x1){-1})[0],-1);
  I(i64id(i64)[0],-42);
  U(u8id((u8x1){255})[0],255);
  U(u8id((u8x1){0})[0],0);
  // Scalar narrow idents must likewise sign/zero extend correctly.
  I((int)scid(-1),-1);
  I((int)scid(-128),-128);
  I((int)ssid(-1),-1);
  I((int)ssid(-32768),-32768);
  I((int)ucid(255),255);
  I((int)usid(65535),65535);
  printf("OK ext_vector_lane1_abi\n");
  return 0;
}
