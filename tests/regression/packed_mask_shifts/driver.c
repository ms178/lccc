/* Compiled separately with GCC by bench_packed_mask_shifts.py, without LTO.
 * These are call/memory-inclusive, dependent SSE2 maps, not whole workloads.
 * Constant-latency SIMD operations execute even if repeated maps converge.
 */
#include <stdint.h>
#include <stdio.h>
#ifndef CASE
#define CASE 0
#endif
#ifndef ITERATIONS
#define ITERATIONS 268435456ULL
#endif
extern void packed_shl_chain_16_128(const void*,const void*,void*);
extern void packed_ashr_chain_32_128(const void*,const void*,void*);
extern void packed_mask_reextend_16_128(const void*,const void*,void*);
extern void packed_mask_tree_32_128(const void*,const void*,void*);
extern void packed_shared_control_32_128(const void*,const void*,void*);
int main(void) {
 void (*functions[])(const void*,const void*,void*)={
  packed_shl_chain_16_128,packed_ashr_chain_32_128,packed_mask_reextend_16_128,
  packed_mask_tree_32_128,packed_shared_control_32_128};
 uint32_t a[4] __attribute__((aligned(16)))={0x12345678,0xffffffff,0x80000000,0x7fffffff};
 uint32_t b[4] __attribute__((aligned(16)))={0x95f165f1,0x73937771,0x2acdeb31,0x13373331};
 uint64_t i;
 for(i=0;i<ITERATIONS;i++) functions[CASE](a,b,a);
 printf("%08x %08x %08x %08x\n",a[0],a[1],a[2],a[3]);
 return 0;
}
