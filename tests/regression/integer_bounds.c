/* Producer ranges, signed/truncated carriers and complete/holey switches.
 * No ctz/clz zero UB: the defined zero result is handled explicitly. */
#include <stdint.h>
#include <stdio.h>
#define NI __attribute__((noinline))
static NI long count_delta(uint64_t x, unsigned kind) {
    if (kind == 0) return x ? (__builtin_ctzll(x) >> 3) : 8;
    if (kind == 1) return x ? (__builtin_clzll(x) >> 3) : 8;
    return __builtin_popcountll(x) >> 3;
}
static NI long count_reference(uint64_t x, unsigned kind) {
    unsigned n = 0;
    if (kind == 0) { while (n < 64 && ((x >> n) & 1) == 0) ++n; }
    else if (kind == 1) { while (n < 64 && ((x >> (63 - n)) & 1) == 0) ++n; }
    else { while (x) { n += x & 1; x >>= 1; } }
    return n / 8;
}
static NI long signed_byte(uint64_t x) { return (int8_t)(x & 255) >> 3; }
static NI long signed_word(uint32_t x) { return (int32_t)(x & UINT32_C(0x8000000f)); }

#define CASES(B) \
    case 0+(B): return y+3; case 1+(B): return y^7; \
    case 2+(B): return y*5; case 3+(B): return y|11; \
    case 4+(B): return y&13; case 5+(B): return y>>1; \
    case 6+(B): return y<<2; case 7+(B): return ~y; \
    case 8+(B): return y+256; case 9+(B): return y*17; \
    case 10+(B): return y^(y>>3); case 11+(B): return y-(y>>1); \
    case 12+(B): return (y&255)+29; case 13+(B): return y|257; \
    case 14+(B): return y/7; case 15+(B): return y*39+1;
#define FALLBACK UINT64_C(0xabcdef0012345678)
static NI uint64_t dispatch16(uint32_t x, uint64_t y) {
    switch (x & 15) { CASES(0) default: return FALLBACK; }
}
static NI uint64_t dispatch32(uint32_t x, uint64_t y) {
    switch (x & 31) { CASES(0) default: return FALLBACK; }
}
static NI uint64_t dispatch_offset(uint32_t x, uint64_t y) {
    switch ((x & 31) + 1) { CASES(1) default: return FALLBACK; }
}
static NI uint64_t dispatch_holes(uint32_t x, uint64_t y) {
    switch (x & 15) {
    case 0: return y+3; case 1: return y^7; case 2: return y*5;
    case 4: return y&13; case 5: return y>>1; case 6: return y<<2;
    case 8: return y+256; case 9: return y*17;
    case 10: return y^(y>>3); case 11: return y-(y>>1);
    case 12: return (y&255)+29; case 13: return y|257;
    case 14: return y/7; case 15: return y*39+1;
    default: return FALLBACK;
    }
}
static NI uint64_t dispatch_reference(volatile unsigned x, uint64_t y) {
    if(x==0)return y+3; if(x==1)return y^7; if(x==2)return y*5;
    if(x==3)return y|11; if(x==4)return y&13; if(x==5)return y>>1;
    if(x==6)return y<<2; if(x==7)return ~y; if(x==8)return y+256;
    if(x==9)return y*17; if(x==10)return y^(y>>3); if(x==11)return y-(y>>1);
    if(x==12)return (y&255)+29; if(x==13)return y|257;
    if(x==14)return y/7; if(x==15)return y*39+1;
    return FALLBACK;
}
int main(void) {
    uint64_t seed=UINT64_C(0x98123456781234fe), checksum=0;
    for(unsigned i=0;i<1024;++i) {
        seed=seed*UINT64_C(6364136223846793005)+1;
        uint64_t x = i < 64 ? (UINT64_C(1)<<i) : seed;
        if(i==64)x=0;
        for(unsigned kind=0;kind<3;++kind) {
            long a=count_delta(x,kind),b=count_reference(x,kind);
            if(a!=b)return 1;
            checksum+=a;
        }
        int n=(int)(i&255); if(n>=128)n-=256;
        long expect=n>=0?n/8:-((-n+7)/8);
        if(signed_byte(i)!=expect)return 2;
        uint32_t w=(uint32_t)seed&UINT32_C(0x8000000f);
        long signed_expect=(long)(w&UINT32_C(0x7fffffff));
        if(w&UINT32_C(0x80000000))signed_expect=signed_expect-2147483647L-1;
        if(signed_word((uint32_t)seed)!=signed_expect)return 3;
        unsigned s=(unsigned)seed;
        uint64_t a=dispatch16(s,seed), b=dispatch32(s,seed), c=dispatch_offset(s,seed);
        uint64_t h=dispatch_holes(s,seed);
        if(a!=dispatch_reference(s&15,seed))return 4;
        if(b!=dispatch_reference(s&31,seed)||c!=b)return 5;
        uint64_t expected_h=((s&15)==3||(s&15)==7)?FALLBACK:dispatch_reference(s&15,seed);
        if(h!=expected_h)return 6;
        checksum^=a+b+c+h+(uint64_t)signed_expect;
    }
    printf("integer-bounds %016llx\n",(unsigned long long)checksum);
    return 0;
}
