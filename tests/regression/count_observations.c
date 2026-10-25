/* Defined scalar count observations; references use bit loops, not builtins. */
#include <stdint.h>
#include <stdio.h>
volatile uint32_t count_observation_reads;
#define NOINLINE __attribute__((noinline))
#define FOR_WORD(T, S, PC, CLZ, CTZ, W, SIGNED) \
NOINLINE int co_cap##S(T x, T y) { (void)y; return x<256u ? PC(x)<=8 : 1; } \
NOINLINE int co_inverse##S(T x, T y) { (void)y; return x>=256u ? 1 : PC(x)<=8; } \
NOINLINE int co_swapped##S(T x, T y) { (void)y; return 256u>x ? PC(x)<=8 : 1; } \
NOINLINE int co_leading##S(T x, T y) { (void)y; return x<256u ? CLZ(x|1)>=(W-8) : 1; } \
NOINLINE int co_trailing##S(T x, T y) { (void)y; return (x&255u)==0 ? CTZ(x|256u)>=8 : 1; } \
NOINLINE int co_nonzero##S(T x, T y) { (void)y; return PC(x)==1 ? x!=0 : 0; } \
NOINLINE int co_zero##S(T x, T y) { (void)y; return PC(x)==0 ? x==0 : 0; } \
NOINLINE int co_full##S(T x, T y) { (void)y; return PC(x)==W; } \
NOINLINE int co_total_leading##S(T x, T y) { (void)y; int n=x ? CLZ(x):W; return n==W; } \
NOINLINE int co_total_trailing##S(T x, T y) { (void)y; int n=x ? CTZ(x):W; return n==W; } \
NOINLINE int co_shared##S(T x, T y) { (void)y; int n=PC(x); return n+(x<256u ? n<=8 : 1); } \
NOINLINE int co_volatile##S(T x, T y) { (void)y; ++count_observation_reads; return x<256u ? PC(x)<=8 : 1; } \
NOINLINE int co_near_cap##S(T x, T y) { (void)y; return x<512u ? PC(x)<=8 : 1; } \
NOINLINE int co_near_signed##S(T x, T y) { (void)y; return (SIGNED)x<256 ? PC(x)<=8 : 1; } \
NOINLINE int co_near_other##S(T x, T y) { return x<256u ? PC(y)<=8 : 1; } \
NOINLINE int co_near_leading##S(T x, T y) { (void)y; return x<256u ? CLZ(x|1)>(W-8) : 1; } \
NOINLINE int co_near_nonzero##S(T x, T y) { return PC(x)==1 ? y!=0 : 0; } \
NOINLINE int co_join_div##S(T x, T y) { if(y&1u) { if(PC(x)!=1)return 0; } if(!x)return -1; return (int)((y&16383u)/x); } \
NOINLINE T co_div##S(T x, T y) { if(PC(x)!=1)return 0; if(!x)return (T)-1; return y/x; } \
NOINLINE T co_rem##S(T x, T y) { if(PC(x)!=1)return 0; if(!x)return (T)-1; return y%x; } \
NOINLINE int co_pow2_cfg##S(T x, T y) { (void)y; if(!x)return 0; if(x&(x-1u))return 0; return 1; } \
NOINLINE int co_ctz_parity##S(T x, T y) { (void)y; return (x ? CTZ(x):W)&1; } \
NOINLINE int co_clz_parity##S(T x, T y) { (void)y; return (x ? CLZ(x):W)&1; } \
NOINLINE int co_odd##S(T x, T y) { (void)y; T m=x*3u; return m ? CTZ(m):0; } \
NOINLINE int co_exact_control##S(T x, T y) { (void)y; return x ? ((x>>CTZ(x))!=0):0; }
FOR_WORD(uint32_t,32,__builtin_popcount,__builtin_clz,__builtin_ctz,32,int32_t)
FOR_WORD(uint64_t,64,__builtin_popcountll,__builtin_clzll,__builtin_ctzll,64,int64_t)
NOINLINE int co_split_zero32(uint64_t x, uint64_t y) {
    (void)y;
    if(!x) return 32; /* deliberately NOT the wide total-count sentinel 64 */
    uint32_t low=(uint32_t)x;
    return low ? __builtin_ctz(low) : 32+__builtin_ctz((uint32_t)(x>>32));
}
#ifndef COUNT_OBSERVATIONS_KERNEL_ONLY
static int pc(uint64_t x) { int n=0; while(x) {n+=(int)(x&1u);x>>=1;} return n; }
static int tz(uint64_t x) { int n=0; while(x && !(x&1u)) {++n;x>>=1;} return n; }
static int lz(uint64_t x, int w) { int n=0;uint64_t bit=(uint64_t)1<<(w-1);while(bit && !(x&bit)) {++n;bit>>=1;}return n; }
static uint64_t hash=1469598103934665603ull;
#define CHECK(F,X,Y,E) do { int got=F(X,Y),want=(E); if(got!=want) { \
    printf("FAIL %s x=%016llx y=%016llx got=%d want=%d\n",#F,(unsigned long long)(X),(unsigned long long)(Y),got,want);return 1;} \
    hash=(hash^(uint32_t)got)*1099511628211ull; } while(0)
#define CHECK_WORD(T,S,W,SIGNED) do { \
    T a=(T)x,b=(T)y;int n=pc(a); \
    CHECK(co_cap##S,a,b,1);CHECK(co_inverse##S,a,b,1);CHECK(co_swapped##S,a,b,1); \
    CHECK(co_leading##S,a,b,1);CHECK(co_trailing##S,a,b,1); \
    CHECK(co_nonzero##S,a,b,n==1);CHECK(co_zero##S,a,b,a==0);CHECK(co_shared##S,a,b,n+1); \
    CHECK(co_volatile##S,a,b,1);CHECK(co_full##S,a,b,a==(T)-1); \
    CHECK(co_total_leading##S,a,b,a==0);CHECK(co_total_trailing##S,a,b,a==0); \
    CHECK(co_near_cap##S,a,b,a<512u ? n<=8:1); \
    CHECK(co_near_signed##S,a,b,(SIGNED)a<256 ? n<=8:1); \
    CHECK(co_near_other##S,a,b,a<256u ? pc(b)<=8:1); \
    CHECK(co_near_leading##S,a,b,a<256u ? lz((uint64_t)a|1,W)>(W-8):1); \
    CHECK(co_near_nonzero##S,a,b,n==1 ? b!=0:0); \
    CHECK(co_join_div##S,a,b,((b&1u) && n!=1) ? 0 : (!a ? -1:(int)((b&16383u)/a))); \
    CHECK(co_div##S,a,(T)12345,n==1 ? (int)((T)12345/a):0); \
    CHECK(co_rem##S,a,(T)12345,n==1 ? (int)((T)12345%a):0); \
    CHECK(co_pow2_cfg##S,a,b,n==1); \
    CHECK(co_ctz_parity##S,a,b,(a ? tz(a):W)&1); \
    CHECK(co_clz_parity##S,a,b,lz(a,W)&1); \
    CHECK(co_odd##S,a,b,a ? tz((T)(a*3u)):0); \
    CHECK(co_exact_control##S,a,b,a!=0); \
} while(0)
static int check(uint64_t x,uint64_t y) {
    uint32_t before=count_observation_reads;
    CHECK_WORD(uint32_t,32,32,int32_t);CHECK_WORD(uint64_t,64,64,int64_t);
    CHECK(co_split_zero32,x,y,x ? tz(x):32);
    return count_observation_reads!=before+2u;
}
int main(void) {
    uint64_t state=0x9e3779b97f4a7c15ull;
    for(uint32_t x=0;x<65536u;++x) if(check(x,x ? ~(uint64_t)x:2))return 1;
    for(int b=0;b<64;++b) {
        uint64_t x=(uint64_t)1<<b;
        if(check(x,x-1)||check(x-1,x)||check(~x,x)||check(x+1,0))return 1;
    }
    for(int i=0;i<4096;++i) {
        state^=state<<13;state^=state>>7;state^=state<<17;
        if(check(state,state*0xd1342543de82ef95ull))return 1;
    }
    printf("%016llx %u\n",(unsigned long long)hash,count_observation_reads);
    return 0;
}
#endif
