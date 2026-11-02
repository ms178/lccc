/* Carry-based conditional increments: signed byte values, both polarities,
   every operand width, all byte patterns, and unsigned accumulator wrap. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#define DEFINE(T, tag) \
__attribute__((noinline)) uint32_t ne32_##tag(const T *p, unsigned n, uint32_t s) { \
    for (unsigned i=0;i<n;++i) { if(p[i]) ++s; } return s; } \
__attribute__((noinline)) uint32_t eq32_##tag(const T *p, unsigned n, uint32_t s) { \
    for (unsigned i=0;i<n;++i) { if(!p[i]) ++s; } return s; } \
__attribute__((noinline)) uint64_t ne64_##tag(const T *p, unsigned n, uint64_t s) { \
    for (unsigned i=0;i<n;++i) { if(p[i]) ++s; } return s; } \
__attribute__((noinline)) uint64_t eq64_##tag(const T *p, unsigned n, uint64_t s) { \
    for (unsigned i=0;i<n;++i) { if(!p[i]) ++s; } return s; }
DEFINE(int8_t, i8)
DEFINE(uint16_t, u16)
DEFINE(uint32_t, u32)
DEFINE(uint64_t, u64)
static int8_t b[257];
static uint16_t w[257];
static uint32_t d[257];
static uint64_t q[257];
static unsigned checks;
#define CHECK(T,tag,array) do { \
    for(unsigned n=0;n<=257;++n) { \
        unsigned zeros=(n!=0)+(n==257), ones=n-zeros; \
        uint32_t seeds32[]={0,1,0x7fffffffU,0x80000000U,UINT32_MAX}; \
        uint64_t seeds64[]={0,1,UINT64_C(0x7fffffffffffffff),UINT64_C(0x8000000000000000),UINT64_MAX}; \
        for(unsigned k=0;k<5;++k) { \
            if(ne32_##tag(array,n,seeds32[k])!=seeds32[k]+ones || \
               eq32_##tag(array,n,seeds32[k])!=seeds32[k]+zeros || \
               ne64_##tag(array,n,seeds64[k])!=seeds64[k]+ones || \
               eq64_##tag(array,n,seeds64[k])!=seeds64[k]+zeros) abort(); \
            checks+=4; \
        } \
    } \
} while(0)
int main(void) {
    for(unsigned i=0;i<257;++i) {
        unsigned x=i&255;
        b[i]=(int8_t)x;
        w[i]=(uint16_t)(x*257);
        d[i]=x*UINT32_C(0x01010101);
        q[i]=x*UINT64_C(0x0101010101010101);
    }
    CHECK(int8_t,i8,b);
    CHECK(uint16_t,u16,w);
    CHECK(uint32_t,u32,d);
    CHECK(uint64_t,u64,q);
    printf("conditional_count: %u checks PASS\n",checks);
    return 0;
}
