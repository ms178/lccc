/* Fixed-trip maps: wraparound and every untouched tail, including SLP's
 * tiny-loop path. Keep each trip visible to the optimizer. */
#include <stdint.h>
#include <stdio.h>
#define F(N) \
__attribute__((noinline)) void small##N(uint64_t *restrict d, \
 const uint64_t *restrict a, const uint64_t *restrict b) { \
 for (unsigned i=0; i<N; ++i) d[i]=a[i]-b[i]; }
F(3) F(4) F(5) F(6) F(7) F(8) F(9) F(10) F(11) F(12) F(13) F(14) F(15) F(16)
int main(void) {
 uint64_t a[20], b[20], d[20];
 void (*fs[])(uint64_t *,const uint64_t *,const uint64_t *) = {
 small3,small4,small5,small6,small7,small8,small9,small10,small11,
 small12,small13,small14,small15,small16};
 for (unsigned t=0;t<14;++t) {
  for (unsigned i=0;i<20;++i) { a[i]=UINT64_MAX-i*17; b[i]=i*31+99; d[i]=123; }
  fs[t](d,a,b);
  for (unsigned i=0;i<20;++i) {
   volatile uint64_t x=a[i], y=b[i];
   uint64_t want=i<t+3 ? x-y : 123;
   if (d[i]!=want) { printf("FAIL trip=%u lane=%u\n",t+3,i); return 1; }
  }
 }
 puts("OK map_i64_small_trip");
 return 0;
}
