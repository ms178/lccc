/* Defined count composition; sentinels keep legacy builtins nonzero. */
#include <stdint.h>
#include <stdio.h>
#define NI __attribute__((noinline))
NI unsigned count_min_trailing(unsigned x, unsigned y) {
    unsigned a=__builtin_ctz(x|0x80000000u), b=__builtin_ctz(y|0x80000000u);
    return a<b?a:b;
}
NI unsigned count_min_leading(unsigned x, unsigned y) {
    unsigned a=__builtin_clz(x|1u), b=__builtin_clz(y|1u);
    return a>b?b:a;
}
NI unsigned count_cap_trailing(unsigned x) {
    unsigned a=__builtin_ctz(x|0x80000000u); return a<6?a:6;
}
NI unsigned count_cap_leading(unsigned x) {
    unsigned a=__builtin_clz(x|1u); return a<6?a:6;
}
NI unsigned count_mixed(unsigned x,unsigned y) {
    unsigned a=__builtin_ctz(x|0x80000000u), b=__builtin_clz(y|1u); return a<b?a:b;
}
NI unsigned count_max(unsigned x,unsigned y) {
    unsigned a=__builtin_ctz(x|0x80000000u), b=__builtin_ctz(y|0x80000000u); return a>b?a:b;
}
NI unsigned count_shared(unsigned x,unsigned y) {
    unsigned a=__builtin_ctz(x|0x80000000u), b=__builtin_ctz(y|0x80000000u); return (a<b?a:b)+a+b;
}
/* Reviewed rotate/popcount/bit-length forms remain controls, not new folds. */
NI uint8_t rotate_byte_select(uint8_t x) { uint8_t lo=(uint8_t)(x<<2), hi=(uint8_t)(x>>6); return x<128?(uint8_t)(lo|hi):lo; }
NI unsigned popcount_halves(uint64_t x) { return (unsigned)__builtin_popcount((unsigned)x)+(unsigned)__builtin_popcount((unsigned)(x>>32)); }
NI uint64_t rotate_word(uint64_t x,unsigned n) { n&=63;return (x<<n)|(x>>((-n)&63)); }
NI unsigned byte_bit_length(unsigned x) { return (uint8_t)((__builtin_clz(x|1u)^31)+1); }

#ifndef WORD_COUNTS_BENCH
static unsigned trailing(unsigned x) { unsigned n=0; while((x&1u)==0){x>>=1;n++;}return n; }
static unsigned leading(unsigned x) { unsigned n=0; while((x&0x80000000u)==0){x<<=1;n++;}return n; }
int main(void) {
    uint32_t state=1; uint64_t sum=0;
    for(unsigned i=0;i<20000;i++) {
        state=state*1664525u+1013904223u;
        unsigned x=i<32?(i?1u<<i:0):state;
        unsigned y=state^(state>>13), a=trailing(x|0x80000000u),b=trailing(y|0x80000000u);
        unsigned l=leading(x|1u),m=leading(y|1u);
        if(count_min_trailing(x,y)!=(a<b?a:b)||count_min_leading(x,y)!=(l<m?l:m))return 1;
        if(count_cap_trailing(x)!=(a<6?a:6)||count_cap_leading(x)!=(l<6?l:6))return 2;
        if(count_mixed(x,y)!=(a<m?a:m)||count_max(x,y)!=(a>b?a:b)||count_shared(x,y)!=(a<b?a:b)+a+b)return 3;
        uint64_t wide=((uint64_t)y<<32)|x; unsigned bits=0;for(uint64_t z=wide;z;z>>=1)bits+=(unsigned)(z&1);
        unsigned shift=i&63; uint64_t rotated=wide;for(unsigned j=0;j<shift;j++)rotated=(rotated<<1)|(rotated>>63);
        uint8_t byte=(uint8_t)x;uint8_t rotated_byte=(uint8_t)((byte<<2)|(byte>>6));
        if(popcount_halves(wide)!=bits||rotate_word(wide,shift)!=rotated||byte_bit_length(x)!=32-leading(x|1u))return 4;
        if(rotate_byte_select(byte)!=(byte<128?rotated_byte:(uint8_t)(byte<<2)))return 5;
        sum=sum*33+count_min_trailing(x,y)+count_cap_leading(x);
    }
    printf("word-counts %016llx\n",(unsigned long long)sum);return 0;
}

#endif /* WORD_COUNTS_BENCH */
