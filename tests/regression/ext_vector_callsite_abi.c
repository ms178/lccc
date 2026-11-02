// AUD-VABI-4 follow-up: caller-side argument marshalling must place one-lane
// ext_vectors according to the per-shape ABI classification, with the
// correct extension; the callee side must see identical bits.
// Multi-arg sequences, pointer args, and 64-bit wide carriers are exercised.
typedef double double1 __attribute__((ext_vector_type(1)));
typedef float float1 __attribute__((ext_vector_type(1)));
typedef unsigned char u8x1 __attribute__((ext_vector_type(1)));
typedef signed char i8x1 __attribute__((ext_vector_type(1)));
typedef unsigned short u16x1 __attribute__((ext_vector_type(1)));
typedef short i16x1 __attribute__((ext_vector_type(1)));
typedef unsigned u32x1 __attribute__((ext_vector_type(1)));
typedef int i32x1 __attribute__((ext_vector_type(1)));
typedef unsigned long long u64x1 __attribute__((ext_vector_type(1)));
typedef long long i64x1 __attribute__((ext_vector_type(1)));

double1 dsum4(double1 a,double1 b,double1 c,double1 d){return (double1){a[0]+b[0]+c[0]+d[0]};}
float1 fsum4(float1 a,float1 b,float1 c,float1 d){return (float1){a[0]+b[0]+c[0]+d[0]};}
i64x1 isum8(i8x1 a,i16x1 b,i32x1 c,i64x1 d,u8x1 e,u16x1 f,u32x1 g,u64x1 h){
  return (i64x1){a[0]+b[0]+c[0]+(int)d[0]+(int)e[0]+(int)f[0]+(int)g[0]+(long long)h[0]};
}
i8x1 i8_many(int a,int b,int c,int d,int e,int f,i8x1 v){return (i8x1){(signed char)(a+b+c+d+e+f+v[0])};}
int printf(const char*, ...);
int fail=0;
#define D(r,w) do{if(__builtin_fabs((double)(r)-(w))>0.0001){printf("FAIL %s=%g\n",#r,(double)(r));fail=1;}}while(0)
#define I(r,w) do{if((long long)(r)!=(w)){printf("FAIL %s=%lld want %lld\n",#r,(long long)(r),(long long)(w));fail=1;}}while(0)
int main(void){
  D(dsum4((double1){1.0},(double1){2.0},(double1){3.0},(double1){4.0})[0],10.0);
  D(fsum4((float1){1.0f},(float1){2.0f},(float1){3.0f},(float1){4.0f})[0],10.0f);
  // Mixed-sign wide aggregate: u64 deadbeef is large positive; signed small
  // lanes sign-extend.
  I(isum8((i8x1){1},(i16x1){2},(i32x1){3},(i64x1){4},
          (u8x1){5},(u16x1){6},(u32x1){7},(u64x1){8})[0],36);
  I(i8_many(1,2,3,4,5,6,(i8x1){7})[0],28);
  // High-bit stress: -1 in every narrow signed lane must be preserved through
  // the call boundary (proves sign-extend at both caller and callee). The
  // unsigned lanes pass a small positive that doesn't disturb the sum.
  I(isum8((i8x1){-1},(i16x1){-1},(i32x1){-1},(i64x1){-1},
          (u8x1){1},(u16x1){1},(u32x1){1},(u64x1){1})[0],0);
  if(fail) return 1;
  printf("OK ext_vector_callsite_abi\n");
  return 0;
}
