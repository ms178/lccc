// FLAGS: -mavx512f
/* Full-width copies may touch exactly 64 bytes, never the following page. */
#include <immintrin.h>
#include <sys/mman.h>
#include <unistd.h>
#include <stdio.h>
#include <string.h>
__attribute__((noinline)) static void copy64(void *dst, const void *src) {
    __builtin_memcpy(dst, src, 64);
}
static volatile __m512i observed;
__attribute__((noinline)) static void copy64_wide(void *dst, const void *src) {
    observed = _mm512_zextsi128_si512(_mm_loadu_si128((const __m128i_u *)src));
    __builtin_memcpy(dst, src, 64);
}
int main(void) {
    long n = sysconf(_SC_PAGESIZE);
    if (n < 256) return 2;
    unsigned char *a = mmap(0,(unsigned long)n*2,PROT_READ|PROT_WRITE,MAP_PRIVATE|MAP_ANONYMOUS,-1,0);
    unsigned char *b = mmap(0,(unsigned long)n*2,PROT_READ|PROT_WRITE,MAP_PRIVATE|MAP_ANONYMOUS,-1,0);
    if (a == MAP_FAILED || b == MAP_FAILED) return 2;
    if (mprotect(a+n,n,PROT_NONE) || mprotect(b+n,n,PROT_NONE)) return 2;
    for (unsigned variant=0; variant<2; ++variant) {
    for (unsigned offset=0; offset<64; ++offset) {
        unsigned char *src=a+n-64-offset, *dst=b+n-64-offset;
        memset(dst-1,0x55,65);
        if (offset) dst[64]=0x55;
        for (unsigned j=0;j<64;++j) src[j]=(unsigned char)(j*7+offset);
        if (variant) copy64_wide(dst,src); else copy64(dst,src);
        if (dst[-1]!=0x55) return 3;
        if (offset && dst[64]!=0x55) return 3;
        for (unsigned j=0;j<64;++j) if (src[j]!=dst[j]) return 3;
    }
    }
    munmap(a,n*2); munmap(b,n*2);
    puts("128 exact-width aligned/unaligned copies OK");
    return 0;
}
