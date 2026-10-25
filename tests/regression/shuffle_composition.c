/* Fixed dword permutations; scalar oracle never uses vector shuffles. */
#ifdef SHUFFLE_KERNELS_ONLY
typedef unsigned int uint32_t;
#else
#include <stdint.h>
#include <stdio.h>
#endif
typedef uint32_t u4 __attribute__((vector_size(16), may_alias));
typedef float f4 __attribute__((vector_size(16), may_alias));
#define SH(x,y,a,b,c,d) __builtin_shufflevector(x,y,a,b,c,d)
__attribute__((noinline)) void shuffle_0(const uint32_t *a,const uint32_t *b,uint32_t *o) {
  u4 x=*(const u4*)a,y=*(const u4*)b;
  u4 p=SH(x,y,3,2,1,0);
  *(u4*)o=SH(p,p,3,2,1,0);
}
__attribute__((noinline)) void shuffle_1(const uint32_t *a,const uint32_t *b,uint32_t *o) {
  u4 x=*(const u4*)a,y=*(const u4*)b;
  u4 p=SH(x,y,1,2,3,0);
  *(u4*)o=SH(p,p,2,3,0,1);
}
__attribute__((noinline)) void shuffle_2(const uint32_t *a,const uint32_t *b,uint32_t *o) {
  u4 x=*(const u4*)a,y=*(const u4*)b;
  u4 p=SH(x,y,2,3,0,1);
  *(u4*)o=SH(p,p,1,2,3,0);
}
__attribute__((noinline)) void shuffle_3(const uint32_t *a,const uint32_t *b,uint32_t *o) {
  u4 x=*(const u4*)a,y=*(const u4*)b;
  u4 p=SH(x,y,0,0,0,0);
  *(u4*)o=SH(p,p,3,3,3,3);
}
__attribute__((noinline)) void shuffle_4(const uint32_t *a,const uint32_t *b,uint32_t *o) {
  u4 x=*(const u4*)a,y=*(const u4*)b;
  u4 p=SH(x,y,3,3,3,3);
  *(u4*)o=SH(p,p,0,0,0,0);
}
__attribute__((noinline)) void shuffle_5(const uint32_t *a,const uint32_t *b,uint32_t *o) {
  u4 x=*(const u4*)a,y=*(const u4*)b;
  u4 p=SH(x,y,0,1,2,3);
  *(u4*)o=SH(p,p,3,2,1,0);
}
__attribute__((noinline)) void shuffle_6(const uint32_t *a,const uint32_t *b,uint32_t *o) {
  u4 x=*(const u4*)a,y=*(const u4*)b;
  u4 p=SH(x,y,3,2,1,0);
  *(u4*)o=SH(p,p,0,1,2,3);
}
__attribute__((noinline)) void shuffle_7(const uint32_t *a,const uint32_t *b,uint32_t *o) {
  u4 x=*(const u4*)a,y=*(const u4*)b;
  u4 p=SH(x,y,1,0,3,2);
  *(u4*)o=SH(p,p,2,3,0,1);
}
__attribute__((noinline)) void shuffle_8(const uint32_t *a,const uint32_t *b,uint32_t *o) {
  u4 x=*(const u4*)a,y=*(const u4*)b;
  u4 p=SH(x,y,2,0,3,1);
  *(u4*)o=SH(p,p,1,2,3,0);
}
__attribute__((noinline)) void shuffle_9(const uint32_t *a,const uint32_t *b,uint32_t *o) {
  u4 x=*(const u4*)a,y=*(const u4*)b;
  u4 p=SH(x,y,3,0,2,1);
  *(u4*)o=SH(p,p,1,2,0,3);
}
__attribute__((noinline)) void shuffle_10(const uint32_t *a,const uint32_t *b,uint32_t *o) {
  u4 x=*(const u4*)a,y=*(const u4*)b;
  u4 p=SH(x,y,2,0,1,0);
  *(u4*)o=SH(p,p,0,3,0,3);
}
__attribute__((noinline)) void shuffle_11(const uint32_t *a,const uint32_t *b,uint32_t *o) {
  u4 x=*(const u4*)a,y=*(const u4*)b;
  u4 p=SH(x,y,0,0,3,3);
  *(u4*)o=SH(p,p,3,3,0,0);
}
__attribute__((noinline)) void shuffle_12(const uint32_t *a,const uint32_t *b,uint32_t *o) {
  u4 x=*(const u4*)a,y=*(const u4*)b;
  u4 p=SH(x,y,1,1,1,1);
  *(u4*)o=SH(p,p,2,2,2,2);
}
__attribute__((noinline)) void shuffle_13(const uint32_t *a,const uint32_t *b,uint32_t *o) {
  u4 x=*(const u4*)a,y=*(const u4*)b;
  u4 p=SH(x,y,6,6,6,6);
  *(u4*)o=SH(p,p,1,1,1,1);
}
__attribute__((noinline)) void shuffle_14(const uint32_t *a,const uint32_t *b,uint32_t *o) {
  f4 x=*(const f4*)a,y=*(const f4*)b;
  f4 p=SH(x,y,3,2,1,0);
  *(f4*)o=SH(p,p,1,2,3,0);
}
__attribute__((noinline)) void shuffle_15(const uint32_t *a,const uint32_t *b,uint32_t *o) {
  f4 x=*(const f4*)a,y=*(const f4*)b;
  f4 p=SH(x,y,1,2,3,0);
  *(f4*)o=SH(p,p,3,2,1,0);
}
__attribute__((noinline)) void shuffle_16(const uint32_t *a,const uint32_t *b,uint32_t *o) {
  u4 x=*(const u4*)a,y=*(const u4*)b;
  *(u4*)o=SH(x,x,3,2,1,0) + SH(y,y,3,2,1,0);
}
__attribute__((noinline)) void shuffle_17(const uint32_t *a,const uint32_t *b,uint32_t *o) {
  u4 x=*(const u4*)a,y=*(const u4*)b;
  *(u4*)o=SH(x,x,3,2,1,0) - SH(y,y,3,2,1,0);
}
__attribute__((noinline)) void shuffle_18(const uint32_t *a,const uint32_t *b,uint32_t *o) {
  u4 x=*(const u4*)a,y=*(const u4*)b;
  *(u4*)o=SH(x,x,3,2,1,0) & SH(y,y,3,2,1,0);
}
__attribute__((noinline)) void shuffle_19(const uint32_t *a,const uint32_t *b,uint32_t *o) {
  u4 x=*(const u4*)a,y=*(const u4*)b;
  *(u4*)o=SH(x,x,3,2,1,0) | SH(y,y,3,2,1,0);
}
__attribute__((noinline)) void shuffle_20(const uint32_t *a,const uint32_t *b,uint32_t *o) {
  u4 x=*(const u4*)a,y=*(const u4*)b;
  *(u4*)o=SH(x,x,3,2,1,0) ^ SH(y,y,3,2,1,0);
}
__attribute__((noinline)) void shuffle_21(const uint32_t*a,const uint32_t*b,uint32_t*o){u4 x=*(const u4*)a,y=*(const u4*)b; *(u4*)o=SH(x,y,0,5,2,7);}
__attribute__((noinline)) void shuffle_22(const uint32_t*a,const uint32_t*b,uint32_t*o){u4 x=*(const u4*)a,y=*(const u4*)b,p=SH(x,x,3,2,1,0); *(u4*)o=p+SH(y,y,0,1,2,3);}
__attribute__((noinline)) void shuffle_23(const uint32_t*a,const uint32_t*b,uint32_t*o){u4 x=*(const u4*)a,y=*(const u4*)b,p=SH(x,x,3,2,1,0); *(u4*)o=SH(p,p,3,2,1,0)+p+y;}
__attribute__((noinline)) void shuffle_24(const uint32_t*a,const uint32_t*b,uint32_t*o){u4 x=*(const u4*)a,y=*(const u4*)b,p=SH(x,x,3,2,1,0),q=SH(y,y,3,2,1,0); *(u4*)o=(p+q)^p;}
__attribute__((noinline)) void shuffle_25(const uint32_t*a,const uint32_t*b,uint32_t*o){u4 x=*(const u4*)a,y=*(const u4*)b,p=SH(x,x,3,2,1,0); *(u4*)o=SH(p,p,3,2,1,0)^y;}
#ifndef SHUFFLE_KERNELS_ONLY
typedef void (*kernel)(const uint32_t*,const uint32_t*,uint32_t*);
static kernel kernels[]={shuffle_0,shuffle_1,shuffle_2,shuffle_3,shuffle_4,shuffle_5,shuffle_6,shuffle_7,shuffle_8,shuffle_9,shuffle_10,shuffle_11,shuffle_12,shuffle_13,shuffle_14,shuffle_15,shuffle_16,shuffle_17,shuffle_18,shuffle_19,shuffle_20,shuffle_21,shuffle_22,shuffle_23,shuffle_24,shuffle_25};
static const unsigned masks[16][2]={{27,27},{57,78},{78,57},{0,255},{255,0},{228,27},{27,228},{177,78},{114,57},{99,201},{18,204},{240,15},{85,170},{170,85},{27,57},{57,27}};
static volatile unsigned calls;
static u4 operand(const uint32_t *p){calls++; return *(const u4*)p;}
static int effects(const uint32_t*a,const uint32_t*b){
  u4 x=SH(operand(a),operand(b),3,2,1,0);
  u4 y=SH(operand(a),operand(b),7,6,5,4);
  u4 z=SH(operand(a),operand(b),0,5,2,7);
  return calls==6 && x[0]==a[3] && y[0]==b[3] && z[1]==b[1];
}
int main(void){
 uint32_t a[4] __attribute__((aligned(16))),b[4] __attribute__((aligned(16))),o[4] __attribute__((aligned(16)));
 uint32_t seed=0x94a5bc7d; uint64_t sum=0; unsigned n,k,l;
 const uint32_t edge[]={0,0xffffffff,0x80000000,0x7fffffff,0x7f800001,0x7fc12345,0xff800001,1,0x55555555,0xaaaaaaaa};
 for(n=0;n<4096;n++){
  for(l=0;l<4;l++){seed=seed*1664525u+1013904223u;a[l]=n<10?edge[(n+l)%10]:seed;seed=seed*1664525u+1013904223u;b[l]=n<10?edge[(n+l+3)%10]:seed;}
  for(k=0;k<26;k++){
   kernels[k](a,b,o);
   for(l=0;l<4;l++){
    uint32_t r=0,x=a[3-l],y=b[3-l];
    if(k<16){unsigned q=(masks[k][1]>>(2*l))&3,ix=(masks[k][0]>>(2*q))&3;r=k==13?b[ix]:a[ix];}
    else if(k==16)r=x+y; else if(k==17)r=x-y;else if(k==18)r=x&y;else if(k==19)r=x|y;else if(k==20)r=x^y;
    else if(k==21)r=(l&1)?b[l]:a[l]; else if(k==22)r=x+b[l];else if(k==23)r=a[l]+x+b[l];else if(k==24)r=(x+y)^x;else r=a[l]^b[l];
    if(o[l]!=r){printf("mismatch %u %u %u %x %x\n",n,k,l,o[l],r);return 1;}
    sum=(sum^o[l])*0x100000001b3ULL;
   }
  }
 }
 if(!effects(a,b))return 2;
 printf("%llx %u\n",(unsigned long long)sum,4096u*26u*4u);return 0;
}
#endif
