/* Exact packed shift contracts with an independent unsigned scalar oracle. */
#ifdef MASK_KERNELS_ONLY
typedef unsigned short uint16_t; typedef unsigned int uint32_t; typedef unsigned long long uint64_t;
#else
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#endif
typedef unsigned short u16_128 __attribute__((vector_size(16),may_alias));
typedef short s16_128 __attribute__((vector_size(16),may_alias));
__attribute__((noinline)) void packed_shl_chain_16_128(const void*ap,const void*bp,void*op) {
  u16_128 a=*(const u16_128*)ap,b=*(const u16_128*)bp;
  *(u16_128*)op=(a << 3) << 4;
}
__attribute__((noinline)) void packed_shl_zero_16_128(const void*ap,const void*bp,void*op) {
  u16_128 a=*(const u16_128*)ap,b=*(const u16_128*)bp;
  *(u16_128*)op=(a << 15) << 1;
}
__attribute__((noinline)) void packed_lshr_chain_16_128(const void*ap,const void*bp,void*op) {
  u16_128 a=*(const u16_128*)ap,b=*(const u16_128*)bp;
  *(u16_128*)op=(a >> 3) >> 4;
}
__attribute__((noinline)) void packed_lshr_zero_16_128(const void*ap,const void*bp,void*op) {
  u16_128 a=*(const u16_128*)ap,b=*(const u16_128*)bp;
  *(u16_128*)op=(a >> 15) >> 1;
}
__attribute__((noinline)) void packed_ashr_chain_16_128(const void*ap,const void*bp,void*op) {
  u16_128 a=*(const u16_128*)ap,b=*(const u16_128*)bp;
  *(u16_128*)op=(u16_128)(((s16_128)a >> 3) >> 4);
}
__attribute__((noinline)) void packed_ashr_saturate_16_128(const void*ap,const void*bp,void*op) {
  u16_128 a=*(const u16_128*)ap,b=*(const u16_128*)bp;
  *(u16_128*)op=(u16_128)(((s16_128)a >> 14) >> 4);
}
__attribute__((noinline)) void packed_mask_direct_16_128(const void*ap,const void*bp,void*op) {
  u16_128 a=*(const u16_128*)ap,b=*(const u16_128*)bp;
  u16_128 m=(u16_128)(a<b);
  *(u16_128*)op=(u16_128)((s16_128)m >> 7);
}
__attribute__((noinline)) void packed_mask_reextend_16_128(const void*ap,const void*bp,void*op) {
  u16_128 a=*(const u16_128*)ap,b=*(const u16_128*)bp;
  u16_128 m=(u16_128)(a<b),p=m<<7;
  *(u16_128*)op=(u16_128)((s16_128)p>>7);
}
__attribute__((noinline)) void packed_mask_tree_16_128(const void*ap,const void*bp,void*op) {
  u16_128 a=*(const u16_128*)ap,b=*(const u16_128*)bp;
  u16_128 m0=(u16_128)(a<b),m1=(u16_128)((a^3)<(b|1)),m2=(u16_128)((a+7)<b),
    m3=(u16_128)((a^128)<b),m4=(u16_128)((a-1)<(b+1)),m5=(u16_128)((a|5)<b),m6=(u16_128)((a&(uint16_t)~1u)<b);
  u16_128 m=((((((m0|m1)&m2)^m3)&m4)|m5)&m6),p=m<<7;
  *(u16_128*)op=(u16_128)((s16_128)p>>7);
}
__attribute__((noinline)) void packed_nonmask_control_16_128(const void*ap,const void*bp,void*op) {
  u16_128 a=*(const u16_128*)ap,b=*(const u16_128*)bp;
  *(u16_128*)op=(u16_128)((s16_128)(a<<7)>>7);
}
__attribute__((noinline)) void packed_shared_control_16_128(const void*ap,const void*bp,void*op) {
  u16_128 a=*(const u16_128*)ap,b=*(const u16_128*)bp;
  u16_128 m=(u16_128)(a<b),p=m<<7;
  *(u16_128*)op=(u16_128)((s16_128)p>>7) ^ p;
}
typedef unsigned int u32_128 __attribute__((vector_size(16),may_alias));
typedef int s32_128 __attribute__((vector_size(16),may_alias));
__attribute__((noinline)) void packed_shl_chain_32_128(const void*ap,const void*bp,void*op) {
  u32_128 a=*(const u32_128*)ap,b=*(const u32_128*)bp;
  *(u32_128*)op=(a << 3) << 4;
}
__attribute__((noinline)) void packed_shl_zero_32_128(const void*ap,const void*bp,void*op) {
  u32_128 a=*(const u32_128*)ap,b=*(const u32_128*)bp;
  *(u32_128*)op=(a << 31) << 1;
}
__attribute__((noinline)) void packed_lshr_chain_32_128(const void*ap,const void*bp,void*op) {
  u32_128 a=*(const u32_128*)ap,b=*(const u32_128*)bp;
  *(u32_128*)op=(a >> 3) >> 4;
}
__attribute__((noinline)) void packed_lshr_zero_32_128(const void*ap,const void*bp,void*op) {
  u32_128 a=*(const u32_128*)ap,b=*(const u32_128*)bp;
  *(u32_128*)op=(a >> 31) >> 1;
}
__attribute__((noinline)) void packed_ashr_chain_32_128(const void*ap,const void*bp,void*op) {
  u32_128 a=*(const u32_128*)ap,b=*(const u32_128*)bp;
  *(u32_128*)op=(u32_128)(((s32_128)a >> 3) >> 4);
}
__attribute__((noinline)) void packed_ashr_saturate_32_128(const void*ap,const void*bp,void*op) {
  u32_128 a=*(const u32_128*)ap,b=*(const u32_128*)bp;
  *(u32_128*)op=(u32_128)(((s32_128)a >> 30) >> 4);
}
__attribute__((noinline)) void packed_mask_direct_32_128(const void*ap,const void*bp,void*op) {
  u32_128 a=*(const u32_128*)ap,b=*(const u32_128*)bp;
  u32_128 m=(u32_128)(a<b);
  *(u32_128*)op=(u32_128)((s32_128)m >> 7);
}
__attribute__((noinline)) void packed_mask_reextend_32_128(const void*ap,const void*bp,void*op) {
  u32_128 a=*(const u32_128*)ap,b=*(const u32_128*)bp;
  u32_128 m=(u32_128)(a<b),p=m<<7;
  *(u32_128*)op=(u32_128)((s32_128)p>>7);
}
__attribute__((noinline)) void packed_mask_tree_32_128(const void*ap,const void*bp,void*op) {
  u32_128 a=*(const u32_128*)ap,b=*(const u32_128*)bp;
  u32_128 m0=(u32_128)(a<b),m1=(u32_128)((a^3)<(b|1)),m2=(u32_128)((a+7)<b),
    m3=(u32_128)((a^128)<b),m4=(u32_128)((a-1)<(b+1)),m5=(u32_128)((a|5)<b),m6=(u32_128)((a&(uint32_t)~1u)<b);
  u32_128 m=((((((m0|m1)&m2)^m3)&m4)|m5)&m6),p=m<<7;
  *(u32_128*)op=(u32_128)((s32_128)p>>7);
}
__attribute__((noinline)) void packed_nonmask_control_32_128(const void*ap,const void*bp,void*op) {
  u32_128 a=*(const u32_128*)ap,b=*(const u32_128*)bp;
  *(u32_128*)op=(u32_128)((s32_128)(a<<7)>>7);
}
__attribute__((noinline)) void packed_shared_control_32_128(const void*ap,const void*bp,void*op) {
  u32_128 a=*(const u32_128*)ap,b=*(const u32_128*)bp;
  u32_128 m=(u32_128)(a<b),p=m<<7;
  *(u32_128*)op=(u32_128)((s32_128)p>>7) ^ p;
}
typedef unsigned long long u64_128 __attribute__((vector_size(16),may_alias));
typedef long long s64_128 __attribute__((vector_size(16),may_alias));
__attribute__((noinline)) void packed_shl_chain_64_128(const void*ap,const void*bp,void*op) {
  u64_128 a=*(const u64_128*)ap,b=*(const u64_128*)bp;
  *(u64_128*)op=(a << 3) << 4;
}
__attribute__((noinline)) void packed_shl_zero_64_128(const void*ap,const void*bp,void*op) {
  u64_128 a=*(const u64_128*)ap,b=*(const u64_128*)bp;
  *(u64_128*)op=(a << 63) << 1;
}
__attribute__((noinline)) void packed_lshr_chain_64_128(const void*ap,const void*bp,void*op) {
  u64_128 a=*(const u64_128*)ap,b=*(const u64_128*)bp;
  *(u64_128*)op=(a >> 3) >> 4;
}
__attribute__((noinline)) void packed_lshr_zero_64_128(const void*ap,const void*bp,void*op) {
  u64_128 a=*(const u64_128*)ap,b=*(const u64_128*)bp;
  *(u64_128*)op=(a >> 63) >> 1;
}
#ifndef ONLY128
typedef unsigned short u16_256 __attribute__((vector_size(32),may_alias));
typedef short s16_256 __attribute__((vector_size(32),may_alias));
__attribute__((noinline)) void packed_shl_chain_16_256(const void*ap,const void*bp,void*op) {
  u16_256 a=*(const u16_256*)ap,b=*(const u16_256*)bp;
  *(u16_256*)op=(a << 3) << 4;
}
__attribute__((noinline)) void packed_shl_zero_16_256(const void*ap,const void*bp,void*op) {
  u16_256 a=*(const u16_256*)ap,b=*(const u16_256*)bp;
  *(u16_256*)op=(a << 15) << 1;
}
__attribute__((noinline)) void packed_lshr_chain_16_256(const void*ap,const void*bp,void*op) {
  u16_256 a=*(const u16_256*)ap,b=*(const u16_256*)bp;
  *(u16_256*)op=(a >> 3) >> 4;
}
__attribute__((noinline)) void packed_lshr_zero_16_256(const void*ap,const void*bp,void*op) {
  u16_256 a=*(const u16_256*)ap,b=*(const u16_256*)bp;
  *(u16_256*)op=(a >> 15) >> 1;
}
__attribute__((noinline)) void packed_ashr_chain_16_256(const void*ap,const void*bp,void*op) {
  u16_256 a=*(const u16_256*)ap,b=*(const u16_256*)bp;
  *(u16_256*)op=(u16_256)(((s16_256)a >> 3) >> 4);
}
__attribute__((noinline)) void packed_ashr_saturate_16_256(const void*ap,const void*bp,void*op) {
  u16_256 a=*(const u16_256*)ap,b=*(const u16_256*)bp;
  *(u16_256*)op=(u16_256)(((s16_256)a >> 14) >> 4);
}
__attribute__((noinline)) void packed_mask_direct_16_256(const void*ap,const void*bp,void*op) {
  u16_256 a=*(const u16_256*)ap,b=*(const u16_256*)bp;
  u16_256 m=(u16_256)(a<b);
  *(u16_256*)op=(u16_256)((s16_256)m >> 7);
}
__attribute__((noinline)) void packed_mask_reextend_16_256(const void*ap,const void*bp,void*op) {
  u16_256 a=*(const u16_256*)ap,b=*(const u16_256*)bp;
  u16_256 m=(u16_256)(a<b),p=m<<7;
  *(u16_256*)op=(u16_256)((s16_256)p>>7);
}
__attribute__((noinline)) void packed_mask_tree_16_256(const void*ap,const void*bp,void*op) {
  u16_256 a=*(const u16_256*)ap,b=*(const u16_256*)bp;
  u16_256 m0=(u16_256)(a<b),m1=(u16_256)((a^3)<(b|1)),m2=(u16_256)((a+7)<b),
    m3=(u16_256)((a^128)<b),m4=(u16_256)((a-1)<(b+1)),m5=(u16_256)((a|5)<b),m6=(u16_256)((a&(uint16_t)~1u)<b);
  u16_256 m=((((((m0|m1)&m2)^m3)&m4)|m5)&m6),p=m<<7;
  *(u16_256*)op=(u16_256)((s16_256)p>>7);
}
__attribute__((noinline)) void packed_nonmask_control_16_256(const void*ap,const void*bp,void*op) {
  u16_256 a=*(const u16_256*)ap,b=*(const u16_256*)bp;
  *(u16_256*)op=(u16_256)((s16_256)(a<<7)>>7);
}
__attribute__((noinline)) void packed_shared_control_16_256(const void*ap,const void*bp,void*op) {
  u16_256 a=*(const u16_256*)ap,b=*(const u16_256*)bp;
  u16_256 m=(u16_256)(a<b),p=m<<7;
  *(u16_256*)op=(u16_256)((s16_256)p>>7) ^ p;
}
typedef unsigned int u32_256 __attribute__((vector_size(32),may_alias));
typedef int s32_256 __attribute__((vector_size(32),may_alias));
__attribute__((noinline)) void packed_shl_chain_32_256(const void*ap,const void*bp,void*op) {
  u32_256 a=*(const u32_256*)ap,b=*(const u32_256*)bp;
  *(u32_256*)op=(a << 3) << 4;
}
__attribute__((noinline)) void packed_shl_zero_32_256(const void*ap,const void*bp,void*op) {
  u32_256 a=*(const u32_256*)ap,b=*(const u32_256*)bp;
  *(u32_256*)op=(a << 31) << 1;
}
__attribute__((noinline)) void packed_lshr_chain_32_256(const void*ap,const void*bp,void*op) {
  u32_256 a=*(const u32_256*)ap,b=*(const u32_256*)bp;
  *(u32_256*)op=(a >> 3) >> 4;
}
__attribute__((noinline)) void packed_lshr_zero_32_256(const void*ap,const void*bp,void*op) {
  u32_256 a=*(const u32_256*)ap,b=*(const u32_256*)bp;
  *(u32_256*)op=(a >> 31) >> 1;
}
__attribute__((noinline)) void packed_ashr_chain_32_256(const void*ap,const void*bp,void*op) {
  u32_256 a=*(const u32_256*)ap,b=*(const u32_256*)bp;
  *(u32_256*)op=(u32_256)(((s32_256)a >> 3) >> 4);
}
__attribute__((noinline)) void packed_ashr_saturate_32_256(const void*ap,const void*bp,void*op) {
  u32_256 a=*(const u32_256*)ap,b=*(const u32_256*)bp;
  *(u32_256*)op=(u32_256)(((s32_256)a >> 30) >> 4);
}
__attribute__((noinline)) void packed_mask_direct_32_256(const void*ap,const void*bp,void*op) {
  u32_256 a=*(const u32_256*)ap,b=*(const u32_256*)bp;
  u32_256 m=(u32_256)(a<b);
  *(u32_256*)op=(u32_256)((s32_256)m >> 7);
}
__attribute__((noinline)) void packed_mask_reextend_32_256(const void*ap,const void*bp,void*op) {
  u32_256 a=*(const u32_256*)ap,b=*(const u32_256*)bp;
  u32_256 m=(u32_256)(a<b),p=m<<7;
  *(u32_256*)op=(u32_256)((s32_256)p>>7);
}
__attribute__((noinline)) void packed_mask_tree_32_256(const void*ap,const void*bp,void*op) {
  u32_256 a=*(const u32_256*)ap,b=*(const u32_256*)bp;
  u32_256 m0=(u32_256)(a<b),m1=(u32_256)((a^3)<(b|1)),m2=(u32_256)((a+7)<b),
    m3=(u32_256)((a^128)<b),m4=(u32_256)((a-1)<(b+1)),m5=(u32_256)((a|5)<b),m6=(u32_256)((a&(uint32_t)~1u)<b);
  u32_256 m=((((((m0|m1)&m2)^m3)&m4)|m5)&m6),p=m<<7;
  *(u32_256*)op=(u32_256)((s32_256)p>>7);
}
__attribute__((noinline)) void packed_nonmask_control_32_256(const void*ap,const void*bp,void*op) {
  u32_256 a=*(const u32_256*)ap,b=*(const u32_256*)bp;
  *(u32_256*)op=(u32_256)((s32_256)(a<<7)>>7);
}
__attribute__((noinline)) void packed_shared_control_32_256(const void*ap,const void*bp,void*op) {
  u32_256 a=*(const u32_256*)ap,b=*(const u32_256*)bp;
  u32_256 m=(u32_256)(a<b),p=m<<7;
  *(u32_256*)op=(u32_256)((s32_256)p>>7) ^ p;
}
typedef unsigned long long u64_256 __attribute__((vector_size(32),may_alias));
typedef long long s64_256 __attribute__((vector_size(32),may_alias));
__attribute__((noinline)) void packed_shl_chain_64_256(const void*ap,const void*bp,void*op) {
  u64_256 a=*(const u64_256*)ap,b=*(const u64_256*)bp;
  *(u64_256*)op=(a << 3) << 4;
}
__attribute__((noinline)) void packed_shl_zero_64_256(const void*ap,const void*bp,void*op) {
  u64_256 a=*(const u64_256*)ap,b=*(const u64_256*)bp;
  *(u64_256*)op=(a << 63) << 1;
}
__attribute__((noinline)) void packed_lshr_chain_64_256(const void*ap,const void*bp,void*op) {
  u64_256 a=*(const u64_256*)ap,b=*(const u64_256*)bp;
  *(u64_256*)op=(a >> 3) >> 4;
}
__attribute__((noinline)) void packed_lshr_zero_64_256(const void*ap,const void*bp,void*op) {
  u64_256 a=*(const u64_256*)ap,b=*(const u64_256*)bp;
  *(u64_256*)op=(a >> 63) >> 1;
}
#endif
#ifndef MASK_KERNELS_ONLY
typedef void(*kernel)(const void*,const void*,void*);
struct contract {kernel fn;unsigned bits,width,kind;};
static struct contract contracts[]={
 {packed_shl_chain_16_128,16,128,0},
 {packed_shl_zero_16_128,16,128,1},
 {packed_lshr_chain_16_128,16,128,2},
 {packed_lshr_zero_16_128,16,128,3},
 {packed_ashr_chain_16_128,16,128,4},
 {packed_ashr_saturate_16_128,16,128,5},
 {packed_mask_direct_16_128,16,128,6},
 {packed_mask_reextend_16_128,16,128,7},
 {packed_mask_tree_16_128,16,128,8},
 {packed_nonmask_control_16_128,16,128,9},
 {packed_shared_control_16_128,16,128,10},
 {packed_shl_chain_32_128,32,128,0},
 {packed_shl_zero_32_128,32,128,1},
 {packed_lshr_chain_32_128,32,128,2},
 {packed_lshr_zero_32_128,32,128,3},
 {packed_ashr_chain_32_128,32,128,4},
 {packed_ashr_saturate_32_128,32,128,5},
 {packed_mask_direct_32_128,32,128,6},
 {packed_mask_reextend_32_128,32,128,7},
 {packed_mask_tree_32_128,32,128,8},
 {packed_nonmask_control_32_128,32,128,9},
 {packed_shared_control_32_128,32,128,10},
 {packed_shl_chain_64_128,64,128,0},
 {packed_shl_zero_64_128,64,128,1},
 {packed_lshr_chain_64_128,64,128,2},
 {packed_lshr_zero_64_128,64,128,3},
#ifndef ONLY128
 {packed_shl_chain_16_256,16,256,0},
 {packed_shl_zero_16_256,16,256,1},
 {packed_lshr_chain_16_256,16,256,2},
 {packed_lshr_zero_16_256,16,256,3},
 {packed_ashr_chain_16_256,16,256,4},
 {packed_ashr_saturate_16_256,16,256,5},
 {packed_mask_direct_16_256,16,256,6},
 {packed_mask_reextend_16_256,16,256,7},
 {packed_mask_tree_16_256,16,256,8},
 {packed_nonmask_control_16_256,16,256,9},
 {packed_shared_control_16_256,16,256,10},
 {packed_shl_chain_32_256,32,256,0},
 {packed_shl_zero_32_256,32,256,1},
 {packed_lshr_chain_32_256,32,256,2},
 {packed_lshr_zero_32_256,32,256,3},
 {packed_ashr_chain_32_256,32,256,4},
 {packed_ashr_saturate_32_256,32,256,5},
 {packed_mask_direct_32_256,32,256,6},
 {packed_mask_reextend_32_256,32,256,7},
 {packed_mask_tree_32_256,32,256,8},
 {packed_nonmask_control_32_256,32,256,9},
 {packed_shared_control_32_256,32,256,10},
 {packed_shl_chain_64_256,64,256,0},
 {packed_shl_zero_64_256,64,256,1},
 {packed_lshr_chain_64_256,64,256,2},
 {packed_lshr_zero_64_256,64,256,3},
#endif
};

static uint64_t arithmetic(uint64_t x,unsigned n,unsigned bits){uint64_t full=~(uint64_t)0>>(64-bits);return (x>>n)|((x&((uint64_t)1<<(bits-1)))?(full^(full>>n)):0);}
static uint64_t reference(uint64_t a,uint64_t b,unsigned bits,unsigned kind){
 uint64_t full=~(uint64_t)0>>(64-bits),m=(a<b)?full:0,p;
 switch(kind){
 case 0:return ((a<<3)&full)<<4&full;
 case 1:return ((a<<(bits-1))&full)<<1&full;
 case 2:return (a>>3)>>4;
 case 3:return (a>>(bits-1))>>1;
 case 4:return arithmetic(arithmetic(a,3,bits),4,bits);
 case 5:return arithmetic(arithmetic(a,bits-2,bits),4,bits);
 case 6:return arithmetic(m,7,bits);
 case 8:{uint64_t m1=((a^3)<(b|1))?full:0,m2=(((a+7)&full)<b)?full:0,m3=((a^128)<b)?full:0,m4=(((a-1)&full)<((b+1)&full))?full:0,m5=((a|5)<b)?full:0,m6=((a&~(uint64_t)1)<b)?full:0;m=((((((m|m1)&m2)^m3)&m4)|m5)&m6);break;}
 case 9:m=a;break;
 default:break;
 }
 p=(m<<7)&full;return arithmetic(p,7,bits)^(kind==10?p:0);
}
int main(void){
 uint64_t a[4] __attribute__((aligned(32))),b[4] __attribute__((aligned(32))),o[4] __attribute__((aligned(32))),seed=0x8123ab47,sum=0;
 unsigned n,k,i,checks=0;
 for(n=0;n<2048;n++){
  for(i=0;i<4;i++){seed=seed*6364136223846793005ULL+1442695040888963407ULL;a[i]=n<4?(n==0?0:n==1?~(uint64_t)0:n==2?0x8000800080008000ULL:0x7fff7fff7fff7fffULL):seed;seed=seed*6364136223846793005ULL+1442695040888963407ULL;b[i]=seed;}
  for(k=0;k<sizeof(contracts)/sizeof(contracts[0]);k++){
   struct contract c=contracts[k];unsigned bytes=c.bits/8,lanes=c.width/c.bits;
   c.fn(a,b,o);
   for(i=0;i<lanes;i++){uint64_t x=0,y=0,z=0,r;memcpy(&x,(char*)a+i*bytes,bytes);memcpy(&y,(char*)b+i*bytes,bytes);memcpy(&z,(char*)o+i*bytes,bytes);r=reference(x,y,c.bits,c.kind);if(z!=r){printf("mismatch %u %u %u %llx %llx\n",n,k,i,(unsigned long long)z,(unsigned long long)r);return 1;}sum=(sum^z)*0x100000001b3ULL;checks++;}
  }
 }
 printf("%llx %u\n",(unsigned long long)sum,checks);return 0;
}
#endif
