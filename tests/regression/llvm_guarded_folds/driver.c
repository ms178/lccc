/* UB-free guarded folds contracts; unsigned negation includes the sign boundary. */
#ifndef GF_FREESTANDING
#include <stdint.h>
#include <stdio.h>
#include <limits.h>
#include <sys/mman.h>
#include <unistd.h>
#else
typedef unsigned int uint32_t;
typedef int int32_t;
typedef long long int64_t;
typedef unsigned long long uint64_t;
#define INT_MIN (-2147483647-1)
#define INT_MAX 2147483647
#define UINT64_MAX (~(uint64_t)0)
int printf(const char *, ...);
#endif
#define NI __attribute__((noinline))
NI uint32_t gf_neg32(uint32_t x, uint32_t b) { b &= 1; return (x ^ -b) + b; }
NI uint64_t gf_neg64(uint64_t x, uint64_t b) { b &= 1; return (x ^ -b) + b; }
NI int32_t gf_signed32(int32_t x, unsigned b) { int32_t bit = (int32_t)(b & 1); return (x ^ -bit) + bit; }
NI int64_t gf_signed64(int64_t x, unsigned b) { int64_t bit = (int64_t)(b & 1); return (x ^ -bit) + bit; }
NI uint32_t gf_cmp32(uint32_t x, uint32_t y) { uint32_t b = x < y; return (x ^ -b) + b; }
NI uint64_t gf_cmp64(uint64_t x, uint64_t y) { uint64_t b = x < y; return (x ^ -b) + b; }
NI uint32_t gf_abs(int32_t x) { uint32_t b = x < 0, u = (uint32_t)x; return (u ^ -b) + b; }
NI unsigned gf_range(int x, unsigned y) { return (long long)x >= 0 && (long long)x < (int)(y & 15); }
NI unsigned gf_range_bad(int x, int y) { return x >= 0 && x < y; }
NI uint32_t gf_truthy32(uint32_t x, uint32_t b) { return (x ^ -b) + b; }
NI uint64_t gf_truthy64(uint64_t x, uint64_t b) { return (x ^ -b) + b; }
NI unsigned gf_shift(unsigned x, unsigned n, unsigned a, unsigned b) { return (x >> (n & 31)) == 0 ? a : b; }
NI unsigned gf_load22(const unsigned char *p) { return p[16] + ((unsigned)p[21] << 8); }
NI unsigned char gf_div8(unsigned char x) { return (unsigned char)(123 / x); }
NI unsigned gf_ch(unsigned x,unsigned y,unsigned z) { return (x & y) ^ (~x & z); }
extern unsigned *gf_names;
extern unsigned *gf_lock;
extern volatile unsigned gf_calls;
extern void gf_cleanup(unsigned *p);
NI unsigned gf_phi(unsigned *saved, unsigned *arg) {
    unsigned *p;
    unsigned ok;
    if (!arg) { gf_cleanup(saved); gf_lock = 0; ok = gf_names != 0; p = 0; }
    else { ok = 1; p = saved; }
    return ok && p != 0;
}
int main(void) {
    static const uint64_t us[] = {0,1,2,3,15,31,32,63,64,127,255,256,65535,2147483647,2147483648ULL,4294967295ULL,1ULL<<63,UINT64_MAX};
    static const int xs[] = {INT_MIN,INT_MIN+1,-256,-16,-1,0,1,2,14,15,16,255,INT_MAX};
    uint64_t sum = 0;
    for (unsigned i=0;i<sizeof(us)/sizeof(us[0]);i++) {
        for (unsigned j=0;j<sizeof(us)/sizeof(us[0]);j++) {
            uint64_t x=us[i], y=us[j];
            sum = sum*33 + gf_neg32((uint32_t)x,(uint32_t)y) + gf_neg64(x,y);
            sum = sum*33 + gf_cmp32((uint32_t)x,(uint32_t)y) + gf_cmp64(x,y);
            sum = sum*33 + gf_truthy32((uint32_t)x,(uint32_t)y) + gf_truthy64(x,y);
            sum = sum*33 + gf_shift((unsigned)x,(unsigned)y,17,29) + gf_ch((unsigned)x,(unsigned)y,0xabc123);
        }
    }
    for (unsigned i=0;i<sizeof(xs)/sizeof(xs[0]);i++) {
        sum=sum*33+gf_abs(xs[i]);
        for (unsigned j=0;j<sizeof(xs)/sizeof(xs[0]);j++)
            sum=sum*33+gf_range(xs[i],(unsigned)xs[j])+gf_range_bad(xs[i],xs[j]);
    }
    /* In particular MIN with b=0 is defined. The introduced IR NEG is a
       wrapping operation, not a new C/LLVM signed-no-wrap assertion. */
    for (unsigned i=0;i<sizeof(xs)/sizeof(xs[0]);i++) {
        if (gf_signed32(xs[i],0) != xs[i]) return 8;
        if (xs[i] != INT_MIN && (uint32_t)gf_signed32(xs[i],1) != 0u-(uint32_t)xs[i]) return 9;
    }
    static const int64_t wide[] = {(-9223372036854775807LL-1),-1,0,1,9223372036854775807LL};
    for(unsigned i=0;i<sizeof(wide)/sizeof(wide[0]);i++) {
        if(gf_signed64(wide[i],0) != wide[i]) return 10;
        if(i != 0 && (uint64_t)gf_signed64(wide[i],1) != 0ULL-(uint64_t)wide[i]) return 11;
    }
    unsigned local=7;
    for(unsigned i=0;i<8;i++) {
        gf_names=(i&1)?&local:0;
        gf_lock=&local;
        sum=sum*33+gf_phi((i&2)?&local:0,(i&4)?&local:0);
    }
    if(gf_calls != 4) return 2;
    for(unsigned x=1;x<256;x++) sum=sum*33+gf_div8((unsigned char)x);
    #ifndef GF_FREESTANDING
    long page=sysconf(_SC_PAGESIZE);
    if(page<22) return 3;
    unsigned char *p=mmap(0,(unsigned long)page*2,PROT_READ|PROT_WRITE,MAP_PRIVATE|MAP_ANONYMOUS,-1,0);
    if(p==MAP_FAILED) return 4;
    if(mprotect(p+page,(unsigned long)page,PROT_NONE)) return 5;
    unsigned char *last=p+page-22;
    #else
    unsigned char last[22];
    #endif
    for(unsigned i=0;i<22;i++) last[i]=(unsigned char)(i*17+3);
    sum=sum*33+gf_load22(last);
    #ifndef GF_FREESTANDING
    if(munmap(p,(unsigned long)page*2)) return 6;
    #endif
    printf("guarded-folds %016llx calls=%u\n",(unsigned long long)sum,gf_calls);
    return sum == 0xf850cc118b230ab2ULL ? 0 : 7;
}
