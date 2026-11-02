/* Explicit-SIMD throughput probe. Compile KIND=0..4 with the same ISA flags.
 * Every timed process independently checks all result lanes before reporting.
 * noinline keeps repeated writes observable across calls, without volatile
 * vector objects changing store lowering. No scalar overflow or FP UB. */
#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#ifndef KIND
#define KIND 0
#endif
#define N 1024
static int32_t input[N * 8] __attribute__((aligned(64)));
static double doubles[N * 8] __attribute__((aligned(64)));
static unsigned char output[N * 64] __attribute__((aligned(64)));
__attribute__((noinline)) static void kernel(void) {
    for (unsigned i=0;i<N;++i) {
#if KIND == 0
        __m128i v = _mm_loadu_si128((const __m128i_u *)(input+i*8));
        _mm512_storeu_si512(output+i*64, _mm512_zextsi128_si512(v));
#elif KIND == 1
        __m256i v = _mm256_loadu_si256((const __m256i_u *)(input+i*8));
        __m512d result = _mm512_cvtepi32_pd(v);
        __builtin_memcpy(output+i*64,&result,64);
#elif KIND == 2
        __m512d v; __builtin_memcpy(&v,doubles+i*8,64);
        __m256i result = _mm512_cvttpd_epi32(v);
        _mm256_storeu_si256((__m256i_u *)(output+i*32),result);
#elif KIND == 3
        __m256d v = _mm256_loadu_pd(doubles+i*4);
        _mm256_storeu_pd((double *)(output+i*32),_mm256_permute_pd(v,5));
#else
        __builtin_memcpy(output+i*64,doubles+i*8,64);
#endif
    }
}
int main(int argc, char **argv) {
    unsigned repetitions = argc > 1 ? (unsigned)atoi(argv[1]) : 1;
    if (!repetitions) return 2;
    for (unsigned i=0;i<N*8;++i) {
        input[i]=(int32_t)(i*17%65536)-32768;
        doubles[i]=(double)input[i]+0.75;
    }
    for (unsigned r=0;r<repetitions;++r) kernel();
    for (unsigned i=0;i<N;++i) {
#if KIND == 0
        for (unsigned j=0;j<64;++j) {
            unsigned char expected=j<16 ? ((unsigned char *)(input+i*8))[j] : 0;
            if (output[i*64+j]!=expected) return 3;
        }
#elif KIND == 1
        for (unsigned j=0;j<8;++j) {
            double v; __builtin_memcpy(&v,output+i*64+j*8,8);
            if (v!=(double)input[i*8+j]) return 3;
        }
#elif KIND == 2
        for (unsigned j=0;j<8;++j) {
            int32_t v; __builtin_memcpy(&v,output+i*32+j*4,4);
            if (v!=(int32_t)doubles[i*8+j]) return 3;
        }
#elif KIND == 3
        for (unsigned j=0;j<4;++j) {
            double v; __builtin_memcpy(&v,output+i*32+j*8,8);
            if (v!=doubles[i*4+(j^1)]) return 3;
        }
#else
        for (unsigned j=0;j<64;++j)
            if (output[i*64+j]!=((unsigned char *)(doubles+i*8))[j]) return 3;
#endif
    }
    uint64_t checksum=0;
    for (unsigned i=0;i<sizeof output;++i) checksum=checksum*33+output[i];
    printf("%llu\n",(unsigned long long)checksum);
    return 0;
}
