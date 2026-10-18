// AUD-VABI-4 follow-up: narrow one-lane ext_vector arithmetic must preserve
// signedness across add/sub/mul/neg for both -O0 and -O2, and the small-slot
// (4-byte partitioned) and default (8-byte) frame layouts must agree (A/B
// differential from run_regression.sh).
typedef signed char i8x1 __attribute__((ext_vector_type(1)));
typedef short i16x1 __attribute__((ext_vector_type(1)));
typedef int i32x1 __attribute__((ext_vector_type(1)));
typedef long long i64x1 __attribute__((ext_vector_type(1)));
typedef unsigned char u8x1 __attribute__((ext_vector_type(1)));
typedef unsigned short u16x1 __attribute__((ext_vector_type(1)));
typedef unsigned u32x1 __attribute__((ext_vector_type(1)));
i8x1 i8add(i8x1 a,i8x1 b){return a+b;}
i8x1 i8sub(i8x1 a,i8x1 b){return a-b;}
i8x1 i8neg(i8x1 a){return -a;}
i8x1 i8mul(i8x1 a,i8x1 b){return a*b;}
i16x1 i16add(i16x1 a,i16x1 b){return a+b;}
i16x1 i16sub(i16x1 a,i16x1 b){return a-b;}
i16x1 i16neg(i16x1 a){return -a;}
i16x1 i16mul(i16x1 a,i16x1 b){return a*b;}
i32x1 i32add(i32x1 a,i32x1 b){return a+b;}
i32x1 i32neg(i32x1 a){return -a;}
i64x1 i64add(i64x1 a,i64x1 b){return a+b;}
i64x1 i64neg(i64x1 a){return -a;}
u8x1 u8add(u8x1 a,u8x1 b){return a+b;}
u16x1 u16add(u16x1 a,u16x1 b){return a+b;}
u32x1 u32add(u32x1 a,u32x1 b){return a+b;}
int printf(const char*, ...);
#define I(e,w) do{long long r=(long long)(e); if(r!=(w)){printf("FAIL %s=%lld want %lld\n",#e,r,(long long)(w));return 1;}}while(0)
#define U(e,w) do{unsigned long long r=(unsigned long long)(e); if(r!=(w)){printf("FAIL %s=0x%llx want 0x%llx\n",#e,r,(unsigned long long)(w));return 1;}}while(0)
int main(void){
  I(i8add((i8x1){10},(i8x1){20})[0],30);
  I(i8add((i8x1){-1},(i8x1){1})[0],0);
  I(i8sub((i8x1){50},(i8x1){100})[0],-50);
  I(i8neg((i8x1){42})[0],-42);
  I(i8neg((i8x1){-1})[0],1);
  I(i8mul((i8x1){7},(i8x1){6})[0],42);
  I(i8mul((i8x1){-3},(i8x1){7})[0],-21);
  I(i16add((i16x1){1000},(i16x1){2000})[0],3000);
  I(i16sub((i16x1){0},(i16x1){1})[0],-1);
  I(i16neg((i16x1){30000})[0],-30000);
  I(i16mul((i16x1){100},(i16x1){100})[0],10000);
  I(i32add((i32x1){1000000},(i32x1){234567})[0],1234567);
  I(i32neg((i32x1){-1})[0],1);
  I(i64add((i64x1){-1},(i64x1){2})[0],1);
  I(i64neg((i64x1){-1})[0],1);
  U(u8add((u8x1){200},(u8x1){100})[0],44);
  U(u16add((u16x1){60000},(u16x1){6000})[0],464); /* 66000 truncated to u16 */
  U(u32add((u32x1){0xf0000000u},(u32x1){0x0fffffffu})[0],0xffffffffu);
  printf("OK ext_vector_narrow_arith\n");
  return 0;
}
