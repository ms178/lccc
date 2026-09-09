#include <stdio.h>
#include <string.h>
typedef unsigned long long u64;
static inline u64 read64(const void *p){ u64 v; __builtin_memcpy(&v,p,8); return v; }
u64 seed=123;
int main(void){
  u64 a[8], b[8]; u64 s=0;
  for(int i=0;i<8;i++){ a[i]=i*0x100000001b3ULL+seed; b[i]=a[i]^(i<<12); }
  for(int i=0;i<8;i++) s += read64(&a[i]) ^ read64(&b[i]);
  // XOR the two, should equal sum of (i<<12)
  printf("%llu exact=%llu\n", s, (seed^seed)+(0ULL));
  u64 expect=0; for(int i=0;i<8;i++) expect += (u64)(i<<12);
  printf("expect=%llu %s\n", expect, s==expect?"OK":"FAIL");
  printf("memcpy-back=%llu\n", read64(&a[3]));
  return 0;
}
