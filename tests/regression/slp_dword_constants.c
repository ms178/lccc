#include <stdint.h>
#include <stdio.h>
#define MUL(name, k) \
__attribute__((noinline)) static void name(const uint32_t *a, uint32_t *b) { \
 uint32_t x=a[0], y=a[1], z=a[2], w=a[3]; \
 b[0]=x*(k); b[1]=y*(k); b[2]=z*(k); b[3]=w*(k); }
MUL(lcg,1664525U)
MUL(mul_signbit,0x80000000U)
MUL(nearmax,0xfffffffeU)
MUL(allones,0xffffffffU)
MUL(seventeen,17U)
int main(void) {
 void (*const f[])(const uint32_t *,uint32_t *)={lcg,mul_signbit,nearmax,allones,seventeen};
 const uint32_t c[]={1664525U,0x80000000U,0xfffffffeU,0xffffffffU,17U};
 const uint32_t edges[]={0,1,0xffffffffU,0x80000000U,0x7fffffffU,255,65535};
 uint32_t state=0x512137abU; uint64_t hash=0;
 for(unsigned i=0;i<65536;++i) {
  uint32_t a[4],b[4];
  for(unsigned j=0;j<4;++j) {
   state^=state<<13; state^=state>>17; state^=state<<5;
   a[j]=i<7 ? edges[(i+j)%7] : state;
  }
  for(unsigned k=0;k<5;++k) {
   f[k](a,b);
   for(unsigned j=0;j<4;++j) {
    uint32_t want=(uint32_t)((uint64_t)a[j]*c[k]);
    if(b[j]!=want) { printf("FAIL %u %u %u\n",i,k,j);return 1; }
    hash=(hash*1099511628211ULL)^b[j];
   }
  }
 }
 printf("PASS slp constants %016llx\n",(unsigned long long)hash);
 return 0;
}
