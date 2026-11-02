/* Packed counts saturate; dynamic scalar counts use the XMM-count form. */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>
static volatile int counts[] = {-512,-256,-1,0,1,15,16,17,31,32,33,63,64,65,255,256,257,511,512,2147483647,(-2147483647-1)};
static unsigned hash(const void *p, unsigned n) { const unsigned char *b=p; unsigned h=2166136261u; for(unsigned i=0;i<n;++i) h=(h^b[i])*16777619u; return h; }
__attribute__((noinline)) static __m128i shift_128_0(__m128i a, int n) { return _mm_slli_epi16(a,n); }
__attribute__((noinline)) static __m128i shift_128_1(__m128i a, int n) { return _mm_srli_epi16(a,n); }
__attribute__((noinline)) static __m128i shift_128_2(__m128i a, int n) { return _mm_srai_epi16(a,n); }
__attribute__((noinline)) static __m128i shift_128_3(__m128i a, int n) { return _mm_slli_epi32(a,n); }
__attribute__((noinline)) static __m128i shift_128_4(__m128i a, int n) { return _mm_srli_epi32(a,n); }
__attribute__((noinline)) static __m128i shift_128_5(__m128i a, int n) { return _mm_srai_epi32(a,n); }
__attribute__((noinline)) static __m128i shift_128_6(__m128i a, int n) { return _mm_slli_epi64(a,n); }
__attribute__((noinline)) static __m128i shift_128_7(__m128i a, int n) { return _mm_srli_epi64(a,n); }
__attribute__((noinline)) static __m256i shift_256_0(__m256i a, int n) { return _mm256_slli_epi16(a,n); }
__attribute__((noinline)) static __m256i shift_256_1(__m256i a, int n) { return _mm256_srli_epi16(a,n); }
__attribute__((noinline)) static __m256i shift_256_2(__m256i a, int n) { return _mm256_srai_epi16(a,n); }
__attribute__((noinline)) static __m256i shift_256_3(__m256i a, int n) { return _mm256_slli_epi32(a,n); }
__attribute__((noinline)) static __m256i shift_256_4(__m256i a, int n) { return _mm256_srli_epi32(a,n); }
__attribute__((noinline)) static __m256i shift_256_5(__m256i a, int n) { return _mm256_srai_epi32(a,n); }
__attribute__((noinline)) static __m256i shift_256_6(__m256i a, int n) { return _mm256_slli_epi64(a,n); }
__attribute__((noinline)) static __m256i shift_256_7(__m256i a, int n) { return _mm256_srli_epi64(a,n); }
int main(void) {
  unsigned char bytes[32]; for(unsigned i=0;i<32;++i) bytes[i]=(unsigned char)(i*11+37);
  __m128i a; __m256i b; memcpy(&a,bytes,16); memcpy(&b,bytes,32);
  for(unsigned j=0;j<sizeof(counts)/sizeof(counts[0]);++j) { int n=counts[j]; printf("%d:",n);
    { __m128i r=shift_128_0(a,n); printf("%08x",hash(&r,sizeof r)); }
    { __m128i r=shift_128_1(a,n); printf("%08x",hash(&r,sizeof r)); }
    { __m128i r=shift_128_2(a,n); printf("%08x",hash(&r,sizeof r)); }
    { __m128i r=shift_128_3(a,n); printf("%08x",hash(&r,sizeof r)); }
    { __m128i r=shift_128_4(a,n); printf("%08x",hash(&r,sizeof r)); }
    { __m128i r=shift_128_5(a,n); printf("%08x",hash(&r,sizeof r)); }
    { __m128i r=shift_128_6(a,n); printf("%08x",hash(&r,sizeof r)); }
    { __m128i r=shift_128_7(a,n); printf("%08x",hash(&r,sizeof r)); }
    { __m256i r=shift_256_0(b,n); printf("%08x",hash(&r,sizeof r)); }
    { __m256i r=shift_256_1(b,n); printf("%08x",hash(&r,sizeof r)); }
    { __m256i r=shift_256_2(b,n); printf("%08x",hash(&r,sizeof r)); }
    { __m256i r=shift_256_3(b,n); printf("%08x",hash(&r,sizeof r)); }
    { __m256i r=shift_256_4(b,n); printf("%08x",hash(&r,sizeof r)); }
    { __m256i r=shift_256_5(b,n); printf("%08x",hash(&r,sizeof r)); }
    { __m256i r=shift_256_6(b,n); printf("%08x",hash(&r,sizeof r)); }
    { __m256i r=shift_256_7(b,n); printf("%08x",hash(&r,sizeof r)); }
    puts("");
  }
  printf("constant:");
  { __m128i r=_mm_slli_epi16(a,-256); printf("%08x",hash(&r,sizeof r)); }
  { __m128i r=_mm_slli_epi16(a,256); printf("%08x",hash(&r,sizeof r)); }
  { __m128i r=_mm_slli_epi16(a,512); printf("%08x",hash(&r,sizeof r)); }
  { __m128i r=_mm_srli_epi16(a,-256); printf("%08x",hash(&r,sizeof r)); }
  { __m128i r=_mm_srli_epi16(a,256); printf("%08x",hash(&r,sizeof r)); }
  { __m128i r=_mm_srli_epi16(a,512); printf("%08x",hash(&r,sizeof r)); }
  { __m128i r=_mm_srai_epi16(a,-256); printf("%08x",hash(&r,sizeof r)); }
  { __m128i r=_mm_srai_epi16(a,256); printf("%08x",hash(&r,sizeof r)); }
  { __m128i r=_mm_srai_epi16(a,512); printf("%08x",hash(&r,sizeof r)); }
  { __m128i r=_mm_slli_epi32(a,-256); printf("%08x",hash(&r,sizeof r)); }
  { __m128i r=_mm_slli_epi32(a,256); printf("%08x",hash(&r,sizeof r)); }
  { __m128i r=_mm_slli_epi32(a,512); printf("%08x",hash(&r,sizeof r)); }
  { __m128i r=_mm_srli_epi32(a,-256); printf("%08x",hash(&r,sizeof r)); }
  { __m128i r=_mm_srli_epi32(a,256); printf("%08x",hash(&r,sizeof r)); }
  { __m128i r=_mm_srli_epi32(a,512); printf("%08x",hash(&r,sizeof r)); }
  { __m128i r=_mm_srai_epi32(a,-256); printf("%08x",hash(&r,sizeof r)); }
  { __m128i r=_mm_srai_epi32(a,256); printf("%08x",hash(&r,sizeof r)); }
  { __m128i r=_mm_srai_epi32(a,512); printf("%08x",hash(&r,sizeof r)); }
  { __m128i r=_mm_slli_epi64(a,-256); printf("%08x",hash(&r,sizeof r)); }
  { __m128i r=_mm_slli_epi64(a,256); printf("%08x",hash(&r,sizeof r)); }
  { __m128i r=_mm_slli_epi64(a,512); printf("%08x",hash(&r,sizeof r)); }
  { __m128i r=_mm_srli_epi64(a,-256); printf("%08x",hash(&r,sizeof r)); }
  { __m128i r=_mm_srli_epi64(a,256); printf("%08x",hash(&r,sizeof r)); }
  { __m128i r=_mm_srli_epi64(a,512); printf("%08x",hash(&r,sizeof r)); }
  { __m256i r=_mm256_slli_epi16(b,-256); printf("%08x",hash(&r,sizeof r)); }
  { __m256i r=_mm256_slli_epi16(b,256); printf("%08x",hash(&r,sizeof r)); }
  { __m256i r=_mm256_slli_epi16(b,512); printf("%08x",hash(&r,sizeof r)); }
  { __m256i r=_mm256_srli_epi16(b,-256); printf("%08x",hash(&r,sizeof r)); }
  { __m256i r=_mm256_srli_epi16(b,256); printf("%08x",hash(&r,sizeof r)); }
  { __m256i r=_mm256_srli_epi16(b,512); printf("%08x",hash(&r,sizeof r)); }
  { __m256i r=_mm256_srai_epi16(b,-256); printf("%08x",hash(&r,sizeof r)); }
  { __m256i r=_mm256_srai_epi16(b,256); printf("%08x",hash(&r,sizeof r)); }
  { __m256i r=_mm256_srai_epi16(b,512); printf("%08x",hash(&r,sizeof r)); }
  { __m256i r=_mm256_slli_epi32(b,-256); printf("%08x",hash(&r,sizeof r)); }
  { __m256i r=_mm256_slli_epi32(b,256); printf("%08x",hash(&r,sizeof r)); }
  { __m256i r=_mm256_slli_epi32(b,512); printf("%08x",hash(&r,sizeof r)); }
  { __m256i r=_mm256_srli_epi32(b,-256); printf("%08x",hash(&r,sizeof r)); }
  { __m256i r=_mm256_srli_epi32(b,256); printf("%08x",hash(&r,sizeof r)); }
  { __m256i r=_mm256_srli_epi32(b,512); printf("%08x",hash(&r,sizeof r)); }
  { __m256i r=_mm256_srai_epi32(b,-256); printf("%08x",hash(&r,sizeof r)); }
  { __m256i r=_mm256_srai_epi32(b,256); printf("%08x",hash(&r,sizeof r)); }
  { __m256i r=_mm256_srai_epi32(b,512); printf("%08x",hash(&r,sizeof r)); }
  { __m256i r=_mm256_slli_epi64(b,-256); printf("%08x",hash(&r,sizeof r)); }
  { __m256i r=_mm256_slli_epi64(b,256); printf("%08x",hash(&r,sizeof r)); }
  { __m256i r=_mm256_slli_epi64(b,512); printf("%08x",hash(&r,sizeof r)); }
  { __m256i r=_mm256_srli_epi64(b,-256); printf("%08x",hash(&r,sizeof r)); }
  { __m256i r=_mm256_srli_epi64(b,256); printf("%08x",hash(&r,sizeof r)); }
  { __m256i r=_mm256_srli_epi64(b,512); printf("%08x",hash(&r,sizeof r)); }
  puts(""); return 0;
}
