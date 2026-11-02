/* Focused outlined-helper benchmark, NOT a replacement for the 45-workload
 * corpus or the full zstd matrix. The 40-million-iteration workload is fixed.
 * Unsigned recurrences and low-byte BZHI semantics are defined for every input. */
#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
__attribute__((noinline)) static uint64_t mask64(uint64_t x, unsigned n) { return _bzhi_u64(x,n); }
__attribute__((noinline)) static uint32_t mask32(uint32_t x, unsigned n) { return _bzhi_u32(x,n); }
int main(int argc,char **argv) {
 uint64_t state=argc>1?strtoull(argv[1],0,10):1234567, sum=0;
 for(unsigned i=0;i<40000000u;i++) {
  state=state*UINT64_C(6364136223846793005)+1;
  unsigned n=(unsigned)(state>>32);
  sum+=mask64(state,n) ^ mask32((uint32_t)state,n);
 }
 printf("%llu\n",(unsigned long long)sum);
 return 0;
}
