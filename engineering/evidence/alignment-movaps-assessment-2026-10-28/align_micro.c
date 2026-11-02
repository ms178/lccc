#include <stdint.h>
#include <stdio.h>
#include <time.h>
#include <immintrin.h>
__attribute__((aligned(64))) static volatile __m128i buf[4096];
__attribute__((noinline))
uint64_t run_qa(int iters){
    __m128i acc=_mm_setzero_si128();
    struct timespec a,b; clock_gettime(CLOCK_MONOTONIC,&a);
    for(int i=0;i<iters;i++){
        for(int j=0;j<4096;j++){ __m128i v=_mm_load_si128((__m128i*)&buf[j]); acc=_mm_add_epi32(acc,v); _mm_store_si128((__m128i*)&buf[j],v); }
    }
    clock_gettime(CLOCK_MONOTONIC,&b);
    buf[0]=acc;
    return (b.tv_sec-a.tv_sec)*1000000000ull+(b.tv_nsec-a.tv_nsec);
}
__attribute__((noinline))
uint64_t run_qu(int iters){
    __m128i acc=_mm_setzero_si128();
    struct timespec a,b; clock_gettime(CLOCK_MONOTONIC,&a);
    for(int i=0;i<iters;i++){
        for(int j=0;j<4096;j++){ __m128i v=_mm_loadu_si128((__m128i*)&buf[j]); acc=_mm_add_epi32(acc,v); _mm_storeu_si128((__m128i*)&buf[j],v); }
    }
    clock_gettime(CLOCK_MONOTONIC,&b);
    buf[1]=acc;
    return (b.tv_sec-a.tv_sec)*1000000000ull+(b.tv_nsec-a.tv_nsec);
}
int main(){
    for(int w=0;w<3;w++){run_qa(20);run_qu(20);}
    uint64_t amin=~0ull,umin=~0ull;
    for(int r=0;r<9;r++){uint64_t a=run_qa(50);if(a<amin)amin=a;uint64_t u=run_qu(50);if(u<umin)umin=u;}
    printf("aligned(movdqa) min=%llu ns  unaligned(movdqu) min=%llu ns  ratio u/a=%.4f\n",
        (unsigned long long)amin,(unsigned long long)umin,(double)umin/amin);
    return 0;
}
