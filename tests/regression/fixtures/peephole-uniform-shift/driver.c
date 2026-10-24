#include <stdint.h>
#include <stdio.h>
#include <string.h>
typedef uint32_t D4 __attribute__((vector_size(16)));
typedef uint32_t D8 __attribute__((vector_size(32)));
typedef uint64_t Q2 __attribute__((vector_size(16)));
typedef uint64_t Q4 __attribute__((vector_size(32)));
#ifdef AVX512_MODEL
typedef uint32_t D16 __attribute__((vector_size(64)));
typedef uint64_t Q8 __attribute__((vector_size(64)));
#endif
static uint64_t random_bits = UINT64_C(0xc65ae19d983b172f);
static uint64_t next(void) {
    random_bits ^= random_bits << 13;
    random_bits ^= random_bits >> 7;
    random_bits ^= random_bits << 17;
    return random_bits;
}
/* Integer-only model: no C out-of-range shifts or signed-right-shift rule. */
static uint64_t reference(uint64_t x, uint64_t c, unsigned bits, int op) {
    uint64_t mask = bits == 32 ? UINT32_MAX : UINT64_MAX;
    x &= mask;
    uint64_t fill = op == 2 && (x >> (bits-1)) ? mask : 0;
    if (c >= bits) return fill;
    if (!c) return x;
    if (!op) return (x << c) & mask;
    return (x >> c) | (fill ^ (fill >> c));
}
static const uint64_t counts[] = {
    0,1,2,15,16,31,32,33,63,64,65,127,128,255,256,
    UINT32_MAX,UINT64_C(0x100000000),UINT64_C(0x100000001),
    UINT64_C(0x8000000000000000),UINT64_MAX
};
#define DECL(name, type) \
    extern type original_##name(type,uint64_t); \
    extern type optimized_##name(type,uint64_t)
DECL(d4sl,D4); DECL(d4sr,D4); DECL(d4sa,D4);
DECL(d8sl,D8); DECL(d8sr,D8); DECL(d8sa,D8);
DECL(q2sl,Q2); DECL(q2sr,Q2); DECL(q4sl,Q4); DECL(q4sr,Q4);
#ifdef AVX512_MODEL
DECL(d16sl,D16); DECL(d16sr,D16); DECL(d16sa,D16);
DECL(q8sl,Q8); DECL(q8sr,Q8);
#endif
#define RUN(name,type,lanes,bits,op) do { \
    for(unsigned k=0;k<16384;k++) { \
        uint64_t c = k < sizeof(counts)/sizeof(counts[0]) ? counts[k] : ((k&1) ? next() : next() % (bits+2)); \
        type a; for(unsigned j=0;j<lanes;j++) a[j]=next(); \
        type old=original_##name(a,c), opt=optimized_##name(a,c); \
        uint64_t effective = bits == 32 ? (uint32_t)c : c; \
        for(unsigned j=0;j<lanes;j++) { \
            uint64_t want=reference(a[j],effective,bits,op); \
            if(old[j]!=want || opt[j]!=want) { \
                fprintf(stderr,"%s sample=%u lane=%u count=%llu\n",#name,k,j,(unsigned long long)c); \
                return 1; \
            } \
        } \
    } \
} while(0)
int main(void) {
    RUN(d4sl,D4,4,32,0); RUN(d4sr,D4,4,32,1); RUN(d4sa,D4,4,32,2);
    RUN(d8sl,D8,8,32,0); RUN(d8sr,D8,8,32,1); RUN(d8sa,D8,8,32,2);
    RUN(q2sl,Q2,2,64,0); RUN(q2sr,Q2,2,64,1);
    RUN(q4sl,Q4,4,64,0); RUN(q4sr,Q4,4,64,1);
#ifdef AVX512_MODEL
    RUN(d16sl,D16,16,32,0); RUN(d16sr,D16,16,32,1); RUN(d16sa,D16,16,32,2);
    RUN(q8sl,Q8,8,64,0); RUN(q8sr,Q8,8,64,1);
    puts("245760 exact uniform-shift comparisons");
#else
    puts("163840 exact uniform-shift comparisons");
#endif
    return 0;
}
