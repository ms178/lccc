#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <immintrin.h>
extern void f0(void *,const void *,const void *,unsigned);
extern void f1(void *,const void *,const void *,unsigned);
extern void f2(void *,const void *,const void *,unsigned);
extern void f3(void *,const void *,const void *,unsigned);
extern void f4(void *,const void *,const void *,unsigned);
extern void f5(void *,const void *,const void *,unsigned);
extern void f6(void *,const void *,const void *,unsigned);
extern void f7(void *,const void *,const void *,unsigned);
static const struct { void (*fn)(void *,const void *,const void *,unsigned); unsigned width,zero,cache; } cases[]={
{f0,8,0,0},
{f1,8,0,1},
{f2,8,1,0},
{f3,8,1,1},
{f4,4,0,0},
{f5,4,0,1},
{f6,4,1,0},
{f7,4,1,1},
};
int main(void) {
 unsigned failures=0,checks=0;
 for(unsigned i=0;i<8;++i) for(unsigned k=0;k<16;++k) for(unsigned pattern=0;pattern<12;++pattern) {
  unsigned char src[64],old[64],got[64],expected[64]={0};
  for(unsigned b=0;b<64;++b) { src[b]=(unsigned char)(13*b+pattern+19); old[b]=(unsigned char)(7*b+pattern+91); }
  if(cases[i].cache) memcpy(expected,old,cases[i].width);
  else if(k&1) memcpy(expected,src,cases[i].width);
  else if(!cases[i].zero) memcpy(expected,old,cases[i].width);
  unsigned csr=_mm_getcsr();
  cases[i].fn(got,src,old,k);
  if(memcmp(got,expected,64) || csr!=_mm_getcsr()) { printf("FAIL case=%u mask=%u pattern=%u\n",i,k,pattern);++failures; }
  ++checks;
 }
 printf("masked scalar transfers: %u checks, %u failures\n",checks,failures);
 return failures!=0;
}
