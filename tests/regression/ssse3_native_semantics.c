/* Hardware operations are checked against overflow-free scalar definitions.
 * In particular INT_MIN negation wraps, horizontal subtract wraps, and
 * PMULHRSW(-32768,-32768) yields 0x8000, not saturated 0x7fff. */
#include <tmmintrin.h>
#include <smmintrin.h>
#include <stdint.h>
typedef union { __m128i v; uint32_t u[4]; int32_t d[4];
    uint16_t uw[8]; int16_t w[8]; uint8_t ub[16]; int8_t b[16]; } lanes;
static __m128i sign_wrapper(__m128i a,__m128i b) { return _mm_sign_epi32(a,b); }
static __m128i multiply_wrapper(__m128i a,__m128i b) { return _mm_mullo_epi32(a,b); }
static __m128i (*volatile indirect_sign)(__m128i,__m128i)=sign_wrapper;
static __m128i (*volatile indirect_multiply)(__m128i,__m128i)=multiply_wrapper;
static uint32_t seed=0x7319ab42u;
static uint32_t next(void) { seed^=seed<<13; seed^=seed>>17; seed^=seed<<5; return seed; }
static unsigned sat(int v) { return (uint16_t)(v>32767?32767:v< -32768?-32768:v); }
__attribute__((noinline)) static __m128i apply(int op, __m128i a, __m128i b) {
    switch(op) {
    case 0: return _mm_hsub_epi16(a,b);
    case 1: return _mm_hsub_epi32(a,b);
    case 2: return _mm_hadds_epi16(a,b);
    case 3: return _mm_hsubs_epi16(a,b);
    case 4: return _mm_sign_epi8(a,b);
    case 5: return _mm_sign_epi16(a,b);
    case 6: return _mm_sign_epi32(a,b);
    case 7: return _mm_mulhrs_epi16(a,b);
    default:return _mm_mullo_epi32(a,b);
    }
}
int main(void) {
    for(int round=0;round<256;round++) {
        lanes a,b,r;
        for(int i=0;i<4;i++) { a.u[i]=next(); b.u[i]=next(); }
        if(round==0) for(int i=0;i<4;i++) { a.u[i]=0x80008000u; b.u[i]=0x80008000u; }
        if(round==1) for(int i=0;i<4;i++) { a.u[i]=0x80000000u; b.u[i]=0xffffffffu; }
        if(round==2) for(int i=0;i<4;i++) { a.u[i]=0x7fff7fffu; b.u[i]=0x80008000u; }
        if(round==3) for(int i=0;i<4;i++) b.u[i]=0;
        if(round==4) for(int i=0;i<4;i++) { a.u[i]=0x80007fffu; b.u[i]=0x7fff8000u; }
        if(round==5) for(int i=0;i<4;i++) { a.u[i]=0x00400040u; b.u[i]=0x01000100u; }
        if(round==6) for(int i=0;i<4;i++) { a.u[i]=0x00400040u; b.u[i]=0xff00ff00u; }
        if(round==7) for(int i=0;i<4;i++) { a.u[i]=0x00010001u; b.u[i]=0x3fff3fffu; }
        if(round==8) for(int i=0;i<4;i++) { a.u[i]=0x00010001u; b.u[i]=0x40014001u; }
        r.v=indirect_sign(a.v,b.v);
        for(int i=0;i<4;i++) if(r.u[i]!=(b.d[i]<0?0u-a.u[i]:b.d[i]==0?0u:a.u[i])) return 21;
        r.v=indirect_multiply(a.v,b.v);
        for(int i=0;i<4;i++) if(r.u[i]!=a.u[i]*b.u[i]) return 22;
#define CHECK_ALIGN(N) do { \
    r.v=_mm_alignr_epi8(a.v,b.v,N); \
    for(int j=0;j<16;j++) { \
        unsigned k=(unsigned)j+(N); \
        unsigned expected=k<16?b.ub[k]:k<32?a.ub[k-16]:0; \
        if(r.ub[j]!=expected) return 23; \
    } \
} while(0)
        CHECK_ALIGN(0); CHECK_ALIGN(1); CHECK_ALIGN(15); CHECK_ALIGN(16);
        CHECK_ALIGN(17); CHECK_ALIGN(31); CHECK_ALIGN(32); CHECK_ALIGN(255);
#undef CHECK_ALIGN
        for(int op=0;op<9;op++) {
            r.v=apply(op,a.v,b.v);
            if(op==0 || op==2 || op==3) {
                for(int i=0;i<4;i++) {
                    unsigned ea,eb;
                    if(op==0) { ea=(uint16_t)(a.uw[2*i]-a.uw[2*i+1]); eb=(uint16_t)(b.uw[2*i]-b.uw[2*i+1]); }
                    else if(op==2) { ea=sat((int)a.w[2*i]+a.w[2*i+1]); eb=sat((int)b.w[2*i]+b.w[2*i+1]); }
                    else { ea=sat((int)a.w[2*i]-a.w[2*i+1]); eb=sat((int)b.w[2*i]-b.w[2*i+1]); }
                    if(r.uw[i]!=ea || r.uw[i+4]!=eb) return 10+op;
                }
            } else if(op==1) {
                for(int i=0;i<2;i++) if(r.u[i]!=a.u[2*i]-a.u[2*i+1] || r.u[i+2]!=b.u[2*i]-b.u[2*i+1]) return 11;
            } else if(op==4) {
                for(int i=0;i<16;i++) if(r.ub[i]!=(uint8_t)(b.b[i]<0?-a.b[i]:b.b[i]==0?0:a.b[i])) return 14;
            } else if(op==5) {
                for(int i=0;i<8;i++) if(r.uw[i]!=(uint16_t)(b.w[i]<0?-a.w[i]:b.w[i]==0?0:a.w[i])) return 15;
            } else if(op==6) {
                for(int i=0;i<4;i++) if(r.u[i]!=(b.d[i]<0?0u-a.u[i]:b.d[i]==0?0u:a.u[i])) return 16;
            } else if(op==7) {
                for(int i=0;i<8;i++) {
                    int p=(int)a.w[i]*(int)b.w[i]+16384;
                    int expected=p>=0?p/32768:-((32767-p)/32768);
                    if(r.uw[i]!=(uint16_t)expected) return 17;
                }
            } else for(int i=0;i<4;i++) if(r.u[i]!=a.u[i]*b.u[i]) return 18;
        }
    }
    return 0;
}
