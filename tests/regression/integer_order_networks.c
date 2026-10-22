#ifdef INTEGER_ORDER_ASSEMBLY
/* Assembly-only cross-target check: no host libc/sysroot dependencies. */
typedef unsigned int uint32_t;
typedef unsigned long long uint64_t;
typedef int int32_t;
typedef long long int64_t;
#define UINT32_C(n) n##U
#define UINT64_C(n) n##ULL
_Static_assert(sizeof(uint32_t)==4 && sizeof(uint64_t)==8, "integer carriers");
#else
#include <stdint.h>
#include <stdio.h>
#endif
static __attribute__((always_inline)) inline uint32_t reverse32(uint32_t x) {
    x=((x>>1)&UINT32_C(0x55555555))|((x&UINT32_C(0x55555555))<<1);
    x=((x>>2)&UINT32_C(0x33333333))|((x&UINT32_C(0x33333333))<<2);
    x=((x>>4)&UINT32_C(0x0f0f0f0f))|((x&UINT32_C(0x0f0f0f0f))<<4);
    x=((x>>8)&UINT32_C(0x00ff00ff))|((x&UINT32_C(0x00ff00ff))<<8);
    return (x>>16)|(x<<16);
}
#define NI __attribute__((noinline))
NI uint32_t reverse_left(uint32_t x) {return reverse32(reverse32(x)<<1);}
NI uint32_t reverse_right(uint32_t x) {return reverse32(reverse32(x)>>1);}
NI uint32_t reverse_not(uint32_t x) {return reverse32(~(reverse32(x)>>1));}
NI uint32_t reverse_or(uint32_t x) {uint32_t r=reverse32(x); return reverse32(r|(r>>1));}
NI uint32_t reverse_and(uint32_t x) {uint32_t r=reverse32(x); return reverse32(r&(r>>1));}
NI uint32_t reverse_andnot(uint32_t x) {uint32_t r=reverse32(x); return reverse32(r&~(r>>1));}
#define EXTREMA(T,S) \
NI T sentinel_min_##S(T x,T y) {T m=x<y?x:y; return x==(T)~(T)0?y:m;} \
NI unsigned min_le_##S(T x,T y) {T m=x<y?x:y; return m<=x;} \
NI unsigned max_lt_##S(T x,T y) {T m=x>y?x:y; return m<x;}
EXTREMA(uint32_t,u32)
EXTREMA(uint64_t,u64)
NI unsigned signed_min_le(int32_t x,int32_t y) {int32_t m=x<y?x:y; return m<=x;}
NI unsigned signed_max_lt(int64_t x,int64_t y) {int64_t m=x>y?x:y; return m<x;}
#ifndef INTEGER_ORDER_ASSEMBLY
int main(void) {
    uint64_t seed=UINT64_C(0x243f6a8885a308d3),digest=0;
    for(unsigned i=0;i<4096;++i) {
        seed^=seed<<13;seed^=seed>>7;seed^=seed<<17;
        uint32_t x=(uint32_t)seed, y=(uint32_t)(seed>>32);
        if(i<32) x=UINT32_C(1)<<i;
        uint32_t got[6]={reverse_left(x),reverse_right(x),reverse_not(x),reverse_or(x),reverse_and(x),reverse_andnot(x)};
        uint32_t ref[6]={x>>1,x<<1,~(x<<1),x|(x<<1),x&(x<<1),x&~(x<<1)};
        for(unsigned k=0;k<6;++k) {if(got[k]!=ref[k]) {puts("reverse mismatch");return 1;} digest=digest*UINT64_C(0x100000001b3)^got[k];}
        uint64_t a=(i&1)?seed:UINT64_MAX,b=(i&2)?~seed:0;
        if(sentinel_min_u32(x,y)!=(x<y?x:y) || sentinel_min_u64(a,b)!=(a<b?a:b) || !min_le_u32(x,y) || !min_le_u64(a,b) || max_lt_u32(x,y) || max_lt_u64(a,b) || !signed_min_le((int32_t)x,(int32_t)y) || signed_max_lt((int64_t)a,(int64_t)b)) {puts("extremum mismatch");return 1;}
        digest=digest*UINT64_C(0x100000001b3)^sentinel_min_u64(a,b);
    }
    printf("%llu\n",(unsigned long long)digest);return 0;
}

#endif
