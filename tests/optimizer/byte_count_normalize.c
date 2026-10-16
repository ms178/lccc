#include <stdint.h>
#include <stdio.h>
#include <limits.h>
#include <stdlib.h>
#include <string.h>

#define EACH_N(M,S) M(S,0) M(S,1) M(S,31) M(S,32) M(S,33) M(S,63) M(S,64) M(S,65) M(S,97)
#define EACH_CASE(M) EACH_N(M,0) EACH_N(M,2) EACH_N(M,31)
#define DEFINE(S,N) \
__attribute__((noinline)) static unsigned count_##S##_##N(const unsigned char *p, unsigned c) { \
    for (long i=S; i<=S+N-1; ++i) if (p[i]) ++c; return c; \
} \
__attribute__((noinline)) static unsigned long long zero_##S##_##N(const unsigned char *p, unsigned long long c) { \
    for (unsigned long i=S; i<(unsigned long)(S+N); ++i) if (!p[i]) ++c; return c; \
}
EACH_CASE(DEFINE)
DEFINE(2,4097)

/* Runtime bounds and an address expression deliberately outside the adapter's
   language provide an independent scalar count for the native assertions. */
__attribute__((noinline)) static unsigned reference(const unsigned char *p, long start, long n) {
    unsigned result=0;
    for (long j=0; j<n; ++j) result += p[start+j] != 0;
    return result;
}
__attribute__((noinline)) static unsigned long long escaping_iv(const unsigned char *p) {
    long i; unsigned count=0;
    for (i=2; i<=98; ++i) if (p[i]) ++count;
    return ((unsigned long long)i << 32) | count;
}
__attribute__((noinline)) static unsigned negative_start(const unsigned char *p) {
    unsigned count=0;
    for (long i=-1; i<64; ++i) if (p[i+1]) ++count;
    return count;
}

int main(void) {
    unsigned char bytes[160];
    unsigned long long digest=0;
    for (unsigned mode=0; mode<8; ++mode) {
        for (unsigned i=0; i<160; ++i)
            bytes[i] = mode==0 ? 0 : mode==1 ? 255 : i%5==0 ? 0 : (unsigned char)(i*73+mode*19);
        unsigned seed = mode&1 ? UINT_MAX-3 : 7;
        unsigned long long wide_seed = mode&1 ? ULLONG_MAX-3 : 7;
#define CHECK(S,N) { \
        unsigned n=reference(bytes,S,N); \
        unsigned size=(S+N) ? (S+N) : 1; \
        unsigned char *tight=malloc(size); if (!tight) return 90; \
        memcpy(tight,bytes,size); \
        unsigned a=count_##S##_##N(tight,seed); \
        unsigned long long b=zero_##S##_##N(tight,wide_seed); \
        free(tight); \
        if (a != seed+n || b != wide_seed+(N-n)) return 1; \
        digest=(digest*UINT64_C(6364136223846793005)) ^ a ^ (b<<1); \
    }
        EACH_CASE(CHECK)
        if (escaping_iv(bytes) != (UINT64_C(99)<<32 | reference(bytes,2,97))) return 2;
        if (negative_start(bytes) != reference(bytes,0,65)) return 3;
    }
    unsigned char *large=malloc(4099); if (!large) return 90;
    for (unsigned i=0; i<4099; ++i) large[i]=(i%5==0) ? 0 : (unsigned char)(i*73+7);
    unsigned n=reference(large,2,4097);
    if (count_2_4097(large,UINT_MAX-3) != UINT_MAX-3+n) return 4;
    if (zero_2_4097(large,ULLONG_MAX-3) != ULLONG_MAX-3+(4097-n)) return 5;
    digest ^= (unsigned long long)n << 32;
    free(large);
    printf("byte-count boundaries: %016llx\n",digest);
    return 0;
}
