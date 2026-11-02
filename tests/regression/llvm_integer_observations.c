#include <stdint.h>
#include <stdio.h>
#define NOINLINE __attribute__((noinline))
NOINLINE uint32_t odd_mul(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return (c ? x*3u : x)==0; }
NOINLINE uint32_t or_neg(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return (c ? (x | -x) : x)==0; }
NOINLINE uint32_t isolate_low(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return (c ? (x & -x) : x)==0; }
NOINLINE uint32_t or_shl(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return (c ? (x | (x<<7)) : x)==0; }
NOINLINE uint32_t or_lshr(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return (c ? (x | (x>>7)) : x)==0; }
NOINLINE uint32_t xor_shl(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return (c ? (x ^ (x<<7)) : x)==0; }
NOINLINE uint32_t xor_lshr(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return (c ? (x ^ (x>>7)) : x)==0; }
NOINLINE uint32_t add_shl(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return (c ? (x + (x<<7)) : x)==0; }
NOINLINE uint32_t sub_shl(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return (c ? (x - (x<<7)) : x)==0; }
NOINLINE uint32_t rsub_shl(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return (c ? ((x<<7) - x) : x)==0; }
NOINLINE uint32_t swap_zero(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return (c ? __builtin_bswap32(x) : x)==0; }
NOINLINE uint32_t rotate_zero(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return (c ? ((x<<7)|(x>>25)) : x)==0; }
NOINLINE uint32_t pop_zero(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return (c ? (unsigned)__builtin_popcount(x) : x)==0; }
NOINLINE uint32_t add_xor(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return (c ? x+5u : x^5u)==5; }
NOINLINE uint32_t rsub_xor(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return (c ? 5u-x : x^5u)==5; }
NOINLINE uint32_t sub_xor(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return (c ? x-5u : x^(-5u))==(-5u); }
NOINLINE uint32_t mul_eq(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return x*3u==21; }
NOINLINE uint32_t pop_full(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return __builtin_popcount(x)==32; }
NOINLINE uint32_t clz_zero(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return __builtin_clz(x)==0; }
NOINLINE uint32_t ctz_zero(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return __builtin_ctz(x)==0; }
NOINLINE uint32_t lshr_zero(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return (x>>7)==0; }
NOINLINE uint32_t lshr_full(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return (x>>7)==(0xffffffffu>>7); }
NOINLINE uint32_t ashr_zero(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return ((int)x>>7)==0; }
NOINLINE uint32_t ashr_full(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return ((int)x>>7)==-1; }
NOINLINE uint32_t shl_zero(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return (x<<7)==0; }
NOINLINE uint32_t or_mask(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return (x|0x5a5au)==0x5a5au; }
NOINLINE uint32_t not_mask(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return ((~x)&0x5a5au)==0; }
NOINLINE uint32_t div_zero(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return (x/37u)==0; }
NOINLINE uint32_t pop_le(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return (unsigned)__builtin_popcount(x)<=x; }
NOINLINE uint32_t cancel_add(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return x+c==y+c; }
NOINLINE uint32_t cancel_sub(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return x-c==y-c; }
NOINLINE uint32_t cancel_rsub(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return c-x==c-y; }
NOINLINE uint32_t cancel_xor(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return (x^c)==(y^c); }
NOINLINE uint32_t cancel_neg(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return -x==-y; }
NOINLINE uint32_t cancel_not(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return ~x==~y; }
NOINLINE uint32_t cancel_swap(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return __builtin_bswap32(x)==__builtin_bswap32(y); }
NOINLINE uint32_t cancel_rotate(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return ((x<<7)|(x>>25))==((y<<7)|(y>>25)); }
NOINLINE uint32_t swap_and(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return __builtin_bswap32(__builtin_bswap32(x)&__builtin_bswap32(y)); }
NOINLINE uint32_t swap_or(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return __builtin_bswap32(__builtin_bswap32(x)|__builtin_bswap32(y)); }
NOINLINE uint32_t swap_xor(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return __builtin_bswap32(__builtin_bswap32(x)^__builtin_bswap32(y)); }
NOINLINE uint32_t swap_not(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return __builtin_bswap32(~__builtin_bswap32(x)); }
NOINLINE uint32_t swap_shl(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return __builtin_bswap32(__builtin_bswap32(x)<<8); }
NOINLINE uint32_t swap_lshr(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return __builtin_bswap32(__builtin_bswap32(x)>>8); }
NOINLINE uint32_t swap_select(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return __builtin_bswap32(c ? __builtin_bswap32(x):__builtin_bswap32(y)); }
NOINLINE uint32_t urem_low(uint32_t x,uint32_t y,uint32_t c) { (void)x; (void)y; (void)c; return (x%48u)&15u; }
NOINLINE unsigned observed_batch(unsigned x, volatile unsigned *c) { return (*c ? x*3u:x)==0; }
static volatile unsigned evaluations;
NOINLINE unsigned effect(void) { return ++evaluations; }
NOINLINE unsigned effect_batch(unsigned x) { return (effect() ? x*3u:x)==0; }
NOINLINE unsigned shared_batch(unsigned x,unsigned c) { unsigned a=__builtin_bswap32(x); return a+(unsigned)((c?a:x)==0); }
NOINLINE unsigned even_batch(unsigned x,unsigned c) {return (c?x*6u:x)==0;}
NOINLINE unsigned zero_shift_batch(unsigned x,unsigned c) {return (c?(x^(x<<0)):x)==0;}
int main(void) {
    uint32_t x=0,y=0;
    static const uint32_t edges[]={0,1,2,7,21,31,32,37,127,128,255,256,0x7fffffff,0x80000000,0xffffff80,0xffffffff};
    for(unsigned i=0;i<65552;++i) {
        x=i<16?edges[i]:x*1664525u+1013904223u;
        y=(i&1)?x:x*22695477u+1;
        for(unsigned c=0;c<4;++c) {
            if(odd_mul(x,y,c)!=(uint32_t)(x==0)) { printf("FAIL odd_mul %u %u %u\n",x,y,c); return 1; }
            if(or_neg(x,y,c)!=(uint32_t)(x==0)) { printf("FAIL or_neg %u %u %u\n",x,y,c); return 1; }
            if(isolate_low(x,y,c)!=(uint32_t)(x==0)) { printf("FAIL isolate_low %u %u %u\n",x,y,c); return 1; }
            if(or_shl(x,y,c)!=(uint32_t)(x==0)) { printf("FAIL or_shl %u %u %u\n",x,y,c); return 1; }
            if(or_lshr(x,y,c)!=(uint32_t)(x==0)) { printf("FAIL or_lshr %u %u %u\n",x,y,c); return 1; }
            if(xor_shl(x,y,c)!=(uint32_t)(x==0)) { printf("FAIL xor_shl %u %u %u\n",x,y,c); return 1; }
            if(xor_lshr(x,y,c)!=(uint32_t)(x==0)) { printf("FAIL xor_lshr %u %u %u\n",x,y,c); return 1; }
            if(add_shl(x,y,c)!=(uint32_t)(x==0)) { printf("FAIL add_shl %u %u %u\n",x,y,c); return 1; }
            if(sub_shl(x,y,c)!=(uint32_t)(x==0)) { printf("FAIL sub_shl %u %u %u\n",x,y,c); return 1; }
            if(rsub_shl(x,y,c)!=(uint32_t)(x==0)) { printf("FAIL rsub_shl %u %u %u\n",x,y,c); return 1; }
            if(swap_zero(x,y,c)!=(uint32_t)(x==0)) { printf("FAIL swap_zero %u %u %u\n",x,y,c); return 1; }
            if(rotate_zero(x,y,c)!=(uint32_t)(x==0)) { printf("FAIL rotate_zero %u %u %u\n",x,y,c); return 1; }
            if(pop_zero(x,y,c)!=(uint32_t)(x==0)) { printf("FAIL pop_zero %u %u %u\n",x,y,c); return 1; }
            if(add_xor(x,y,c)!=(uint32_t)(x==0)) { printf("FAIL add_xor %u %u %u\n",x,y,c); return 1; }
            if(rsub_xor(x,y,c)!=(uint32_t)(x==0)) { printf("FAIL rsub_xor %u %u %u\n",x,y,c); return 1; }
            if(sub_xor(x,y,c)!=(uint32_t)(x==0)) { printf("FAIL sub_xor %u %u %u\n",x,y,c); return 1; }
            if(mul_eq(x,y,c)!=(uint32_t)(x==7u)) { printf("FAIL mul_eq %u %u %u\n",x,y,c); return 1; }
            if(pop_full(x,y,c)!=(uint32_t)(x==UINT32_MAX)) { printf("FAIL pop_full %u %u %u\n",x,y,c); return 1; }
            if(x!=0 && clz_zero(x,y,c)!=(uint32_t)((x & 0x80000000u)!=0)) { printf("FAIL clz_zero %u %u %u\n",x,y,c); return 1; }
            if(x!=0 && ctz_zero(x,y,c)!=(uint32_t)((x & 1u)!=0)) { printf("FAIL ctz_zero %u %u %u\n",x,y,c); return 1; }
            if(lshr_zero(x,y,c)!=(uint32_t)(x<128u)) { printf("FAIL lshr_zero %u %u %u\n",x,y,c); return 1; }
            if(lshr_full(x,y,c)!=(uint32_t)(x>=0xffffff80u)) { printf("FAIL lshr_full %u %u %u\n",x,y,c); return 1; }
            if(ashr_zero(x,y,c)!=(uint32_t)(x<128u)) { printf("FAIL ashr_zero %u %u %u\n",x,y,c); return 1; }
            if(ashr_full(x,y,c)!=(uint32_t)(x>=0xffffff80u)) { printf("FAIL ashr_full %u %u %u\n",x,y,c); return 1; }
            if(shl_zero(x,y,c)!=(uint32_t)((x & 0x1ffffffu)==0)) { printf("FAIL shl_zero %u %u %u\n",x,y,c); return 1; }
            if(or_mask(x,y,c)!=(uint32_t)((x & ~0x5a5au)==0)) { printf("FAIL or_mask %u %u %u\n",x,y,c); return 1; }
            if(not_mask(x,y,c)!=(uint32_t)((x & 0x5a5au)==0x5a5au)) { printf("FAIL not_mask %u %u %u\n",x,y,c); return 1; }
            if(div_zero(x,y,c)!=(uint32_t)(x<37u)) { printf("FAIL div_zero %u %u %u\n",x,y,c); return 1; }
            if(pop_le(x,y,c)!=(uint32_t)(1u)) { printf("FAIL pop_le %u %u %u\n",x,y,c); return 1; }
            if(cancel_add(x,y,c)!=(uint32_t)(x==y)) { printf("FAIL cancel_add %u %u %u\n",x,y,c); return 1; }
            if(cancel_sub(x,y,c)!=(uint32_t)(x==y)) { printf("FAIL cancel_sub %u %u %u\n",x,y,c); return 1; }
            if(cancel_rsub(x,y,c)!=(uint32_t)(x==y)) { printf("FAIL cancel_rsub %u %u %u\n",x,y,c); return 1; }
            if(cancel_xor(x,y,c)!=(uint32_t)(x==y)) { printf("FAIL cancel_xor %u %u %u\n",x,y,c); return 1; }
            if(cancel_neg(x,y,c)!=(uint32_t)(x==y)) { printf("FAIL cancel_neg %u %u %u\n",x,y,c); return 1; }
            if(cancel_not(x,y,c)!=(uint32_t)(x==y)) { printf("FAIL cancel_not %u %u %u\n",x,y,c); return 1; }
            if(cancel_swap(x,y,c)!=(uint32_t)(x==y)) { printf("FAIL cancel_swap %u %u %u\n",x,y,c); return 1; }
            if(cancel_rotate(x,y,c)!=(uint32_t)(x==y)) { printf("FAIL cancel_rotate %u %u %u\n",x,y,c); return 1; }
            if(swap_and(x,y,c)!=(uint32_t)(x&y)) { printf("FAIL swap_and %u %u %u\n",x,y,c); return 1; }
            if(swap_or(x,y,c)!=(uint32_t)(x|y)) { printf("FAIL swap_or %u %u %u\n",x,y,c); return 1; }
            if(swap_xor(x,y,c)!=(uint32_t)(x^y)) { printf("FAIL swap_xor %u %u %u\n",x,y,c); return 1; }
            if(swap_not(x,y,c)!=(uint32_t)(~x)) { printf("FAIL swap_not %u %u %u\n",x,y,c); return 1; }
            if(swap_shl(x,y,c)!=(uint32_t)(x>>8)) { printf("FAIL swap_shl %u %u %u\n",x,y,c); return 1; }
            if(swap_lshr(x,y,c)!=(uint32_t)(x<<8)) { printf("FAIL swap_lshr %u %u %u\n",x,y,c); return 1; }
            if(swap_select(x,y,c)!=(uint32_t)(c?x:y)) { printf("FAIL swap_select %u %u %u\n",x,y,c); return 1; }
            if(urem_low(x,y,c)!=(uint32_t)(x&15u)) { printf("FAIL urem_low %u %u %u\n",x,y,c); return 1; }
            if(even_batch(x,c)!=(unsigned)(x==0 || (c && x==0x80000000u))) return 5;
            if(zero_shift_batch(x,c)!=(unsigned)(c || x==0)) return 6;
            volatile unsigned condition=c;
            if(observed_batch(x,&condition)!=(x==0)) return 2;
            unsigned old=evaluations;
            if(effect_batch(x)!=(x==0) || evaluations!=old+1) return 3;
            if(shared_batch(x,c)!=__builtin_bswap32(x)+(unsigned)(x==0)) return 4;
        }
    }
    puts("PASS 45 integer witnesses, volatile/call effects and shared producers");
    return 0;
}
