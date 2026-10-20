#include <stdint.h>
#include <stdio.h>
#define NOINLINE __attribute__((noinline))
NOINLINE unsigned d_add(unsigned x,unsigned y) {(void)x;(void)y;return ((x&255u)+(y&255u))&15u;}
NOINLINE unsigned d_sub(unsigned x,unsigned y) {(void)x;(void)y;return ((x&255u)-(y&255u))&15u;}
NOINLINE unsigned d_mul(unsigned x,unsigned y) {(void)x;(void)y;return ((x&255u)*(y&255u))&15u;}
NOINLINE unsigned d_mul_tz(unsigned x,unsigned y) {(void)x;(void)y;return ((x&15u)*1536u)&1536u;}
NOINLINE unsigned d_shl(unsigned x,unsigned y) {(void)x;(void)y;return ((x&255u)<<7)&1536u;}
NOINLINE unsigned d_lshr(unsigned x,unsigned y) {(void)x;(void)y;return ((x&0xff00u)>>8)&15u;}
NOINLINE unsigned d_ashr(unsigned x,unsigned y) {(void)x;(void)y;return ((int)(x&0xffffff80u)>>7)&15u;}
NOINLINE unsigned d_rotate(unsigned x,unsigned y) {(void)x;(void)y;return (((x&65535u)<<8)|((x&65535u)>>24))&0xf00u;}
NOINLINE unsigned d_not(unsigned x,unsigned y) {(void)x;(void)y;return (~(x&255u))&15u;}
NOINLINE unsigned d_neg(unsigned x,unsigned y) {(void)x;(void)y;return (-(x&255u))&15u;}
NOINLINE unsigned d_xor(unsigned x,unsigned y) {(void)x;(void)y;return ((x&255u)^(y&255u))&15u;}
NOINLINE unsigned d_or(unsigned x,unsigned y) {(void)x;(void)y;return ((x&255u)|(y&255u))&15u;}
NOINLINE unsigned d_and(unsigned x,unsigned y) {(void)x;(void)y;return ((x&255u)&(y&255u))&15u;}
NOINLINE unsigned d_bswap(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_bswap32(x&0xff000000u)&15u;}
NOINLINE unsigned d_rem_shl(unsigned x,unsigned y) {(void)x;(void)y;return ((x%48u)<<9)&1536u;}
NOINLINE unsigned d_rem_lshr(unsigned x,unsigned y) {(void)x;(void)y;return ((x%768u)>>4)&15u;}
NOINLINE unsigned d_rem_add(unsigned x,unsigned y) {(void)x;(void)y;return ((x%48u)+(y%80u))&15u;}
NOINLINE unsigned d_rem_mul(unsigned x,unsigned y) {(void)x;(void)y;return ((x%48u)*1536u)&1536u;}
NOINLINE unsigned tz_odd(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_ctz(x*3u);}
NOINLINE unsigned tz_variable_odd(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_ctz(x*(y|1u));}
NOINLINE unsigned tz_neg(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_ctz(-x);}
NOINLINE unsigned tz_isolate(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_ctz(x&-x);}
NOINLINE unsigned tz_or_neg(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_ctz(x|-x);}
NOINLINE unsigned tz_xor_shift(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_ctz(x^(x<<7));}
NOINLINE unsigned tz_or_shift(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_ctz(x|(x<<7));}
NOINLINE unsigned tz_add_shift(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_ctz(x+(x<<7));}
NOINLINE unsigned tz_sub_shift(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_ctz(x-(x<<7));}
NOINLINE unsigned lz_or_shift(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_clz(x|(x>>7));}
NOINLINE unsigned lz_xor_shift(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_clz(x^(x>>7));}
NOINLINE unsigned lz_andnot_shift(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_clz(x&~(x>>7));}
NOINLINE unsigned tz_andnot_shift(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_ctz(x&~(x<<7));}
NOINLINE unsigned pc_swap(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_popcount(__builtin_bswap32(x));}
NOINLINE unsigned pc_rotate(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_popcount((x<<7)|(x>>25));}
NOINLINE unsigned parity_add(unsigned x,unsigned y) {(void)x;(void)y;return (__builtin_popcount(x)+__builtin_popcount(y))&1u;}
NOINLINE unsigned parity_sub(unsigned x,unsigned y) {(void)x;(void)y;return (__builtin_popcount(x)-__builtin_popcount(y))&1u;}
NOINLINE unsigned parity_xor(unsigned x,unsigned y) {(void)x;(void)y;return (__builtin_popcount(x)^__builtin_popcount(y))&1u;}
NOINLINE unsigned parity_not(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_popcount(~x)&1u;}
NOINLINE unsigned parity_even_xor(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_popcount(x^0x5au)&1u;}
NOINLINE unsigned pc_partition(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_popcount(x&0x55555555u)+__builtin_popcount(x&0xaaaaaaaau);}
NOINLINE unsigned pc_symmetric_difference(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_popcount(x|y)-__builtin_popcount(x&y);}
NOINLINE unsigned pc_complement(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_popcount(x)+__builtin_popcount(~x);}
NOINLINE unsigned pc_variable_partition(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_popcount(x&y)+__builtin_popcount(x&~y);}
NOINLINE unsigned pc_and_xor(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_popcount(x&y)+__builtin_popcount(x^y);}
NOINLINE unsigned pc_union_minus_xor(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_popcount(x|y)-__builtin_popcount(x^y);}
NOINLINE unsigned pc_union_minus_left(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_popcount(x|y)-__builtin_popcount(x&~y);}
NOINLINE unsigned pc_union_minus_right(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_popcount(x|y)-__builtin_popcount(~x&y);}
NOINLINE unsigned pc_xor_minus_left(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_popcount(x^y)-__builtin_popcount(x&~y);}
NOINLINE unsigned pc_xor_minus_right(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_popcount(x^y)-__builtin_popcount(~x&y);}
NOINLINE unsigned pc_triangle_xor(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_popcount(x)+__builtin_popcount(y)-2*__builtin_popcount(x&y);}
NOINLINE unsigned pc_triangle_and(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_popcount(x)+__builtin_popcount(y)-__builtin_popcount(x^y);}
NOINLINE unsigned pc_triangle_or(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_popcount(x)+__builtin_popcount(y)+__builtin_popcount(x^y);}
NOINLINE unsigned pc_doubled_or(unsigned x,unsigned y) {(void)x;(void)y;return 2*__builtin_popcount(x|y)-__builtin_popcount(x)-__builtin_popcount(y);}
NOINLINE unsigned pc_demorgan_and(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_popcount(x&y)+__builtin_popcount(~x|~y);}
NOINLINE unsigned pc_demorgan_or(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_popcount(x|y)+__builtin_popcount(~x&~y);}
NOINLINE unsigned pc_complement_difference(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_popcount(~(x&y))-__builtin_popcount(~(x|y));}
NOINLINE unsigned pc_xnor_minus_and(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_popcount(~(x^y))-__builtin_popcount(x&y);}
NOINLINE unsigned pc_nor_plus_xor(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_popcount(~(x|y))+__builtin_popcount(x^y);}
NOINLINE unsigned pc_partition_four(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_popcount(x&y)+__builtin_popcount(x&~y)+__builtin_popcount(~x&y)+__builtin_popcount(~x&~y);}
NOINLINE unsigned pc_partition_three(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_popcount(x&y)+__builtin_popcount(x&~y)+__builtin_popcount(~x&y);}
NOINLINE unsigned pc_absorption(unsigned x,unsigned y) {(void)x;(void)y;return __builtin_popcount((x|y)&x)+__builtin_popcount(~x&y);}
int main(void) {unsigned x=0,y=1; static const unsigned edges[]={0,1,2,7,127,128,255,256,0x7fffffff,0x80000000,0xfffffffe,0xffffffff};
for(unsigned i=0;i<65548;++i) {x=i<12?edges[i]:x*1664525u+1013904223u;y=y*22695477u+1;
if(d_add(x,y)!=(unsigned)((x+y)&15u)) {printf("FAIL d_add %u %u\n",x,y);return 1;}
if(d_sub(x,y)!=(unsigned)((x-y)&15u)) {printf("FAIL d_sub %u %u\n",x,y);return 1;}
if(d_mul(x,y)!=(unsigned)((x*y)&15u)) {printf("FAIL d_mul %u %u\n",x,y);return 1;}
if(d_mul_tz(x,y)!=(unsigned)((x*1536u)&1536u)) {printf("FAIL d_mul_tz %u %u\n",x,y);return 1;}
if(d_shl(x,y)!=(unsigned)((x<<7)&1536u)) {printf("FAIL d_shl %u %u\n",x,y);return 1;}
if(d_lshr(x,y)!=(unsigned)((x>>8)&15u)) {printf("FAIL d_lshr %u %u\n",x,y);return 1;}
if(d_ashr(x,y)!=(unsigned)(((int)x>>7)&15u)) {printf("FAIL d_ashr %u %u\n",x,y);return 1;}
if(d_rotate(x,y)!=(unsigned)(((x<<8)|(x>>24))&0xf00u)) {printf("FAIL d_rotate %u %u\n",x,y);return 1;}
if(d_not(x,y)!=(unsigned)((~x)&15u)) {printf("FAIL d_not %u %u\n",x,y);return 1;}
if(d_neg(x,y)!=(unsigned)((-x)&15u)) {printf("FAIL d_neg %u %u\n",x,y);return 1;}
if(d_xor(x,y)!=(unsigned)((x^y)&15u)) {printf("FAIL d_xor %u %u\n",x,y);return 1;}
if(d_or(x,y)!=(unsigned)((x|y)&15u)) {printf("FAIL d_or %u %u\n",x,y);return 1;}
if(d_and(x,y)!=(unsigned)((x&y)&15u)) {printf("FAIL d_and %u %u\n",x,y);return 1;}
if(d_bswap(x,y)!=(unsigned)(__builtin_bswap32(x)&15u)) {printf("FAIL d_bswap %u %u\n",x,y);return 1;}
if(d_rem_shl(x,y)!=(unsigned)((x<<9)&1536u)) {printf("FAIL d_rem_shl %u %u\n",x,y);return 1;}
if(d_rem_lshr(x,y)!=(unsigned)((x>>4)&15u)) {printf("FAIL d_rem_lshr %u %u\n",x,y);return 1;}
if(d_rem_add(x,y)!=(unsigned)((x+y)&15u)) {printf("FAIL d_rem_add %u %u\n",x,y);return 1;}
if(d_rem_mul(x,y)!=(unsigned)((x*1536u)&1536u)) {printf("FAIL d_rem_mul %u %u\n",x,y);return 1;}
if(x!=0 && tz_odd(x,y)!=(unsigned)(__builtin_ctz(x))) {printf("FAIL tz_odd %u %u\n",x,y);return 1;}
if(x!=0 && tz_variable_odd(x,y)!=(unsigned)(__builtin_ctz(x))) {printf("FAIL tz_variable_odd %u %u\n",x,y);return 1;}
if(x!=0 && tz_neg(x,y)!=(unsigned)(__builtin_ctz(x))) {printf("FAIL tz_neg %u %u\n",x,y);return 1;}
if(x!=0 && tz_isolate(x,y)!=(unsigned)(__builtin_ctz(x))) {printf("FAIL tz_isolate %u %u\n",x,y);return 1;}
if(x!=0 && tz_or_neg(x,y)!=(unsigned)(__builtin_ctz(x))) {printf("FAIL tz_or_neg %u %u\n",x,y);return 1;}
if(x!=0 && tz_xor_shift(x,y)!=(unsigned)(__builtin_ctz(x))) {printf("FAIL tz_xor_shift %u %u\n",x,y);return 1;}
if(x!=0 && tz_or_shift(x,y)!=(unsigned)(__builtin_ctz(x))) {printf("FAIL tz_or_shift %u %u\n",x,y);return 1;}
if(x!=0 && tz_add_shift(x,y)!=(unsigned)(__builtin_ctz(x))) {printf("FAIL tz_add_shift %u %u\n",x,y);return 1;}
if(x!=0 && tz_sub_shift(x,y)!=(unsigned)(__builtin_ctz(x))) {printf("FAIL tz_sub_shift %u %u\n",x,y);return 1;}
if(x!=0 && lz_or_shift(x,y)!=(unsigned)(__builtin_clz(x))) {printf("FAIL lz_or_shift %u %u\n",x,y);return 1;}
if(x!=0 && lz_xor_shift(x,y)!=(unsigned)(__builtin_clz(x))) {printf("FAIL lz_xor_shift %u %u\n",x,y);return 1;}
if(x!=0 && lz_andnot_shift(x,y)!=(unsigned)(__builtin_clz(x))) {printf("FAIL lz_andnot_shift %u %u\n",x,y);return 1;}
if(x!=0 && tz_andnot_shift(x,y)!=(unsigned)(__builtin_ctz(x))) {printf("FAIL tz_andnot_shift %u %u\n",x,y);return 1;}
if(pc_swap(x,y)!=(unsigned)(__builtin_popcount(x))) {printf("FAIL pc_swap %u %u\n",x,y);return 1;}
if(pc_rotate(x,y)!=(unsigned)(__builtin_popcount(x))) {printf("FAIL pc_rotate %u %u\n",x,y);return 1;}
if(parity_add(x,y)!=(unsigned)(__builtin_popcount(x^y)&1u)) {printf("FAIL parity_add %u %u\n",x,y);return 1;}
if(parity_sub(x,y)!=(unsigned)(__builtin_popcount(x^y)&1u)) {printf("FAIL parity_sub %u %u\n",x,y);return 1;}
if(parity_xor(x,y)!=(unsigned)(__builtin_popcount(x^y)&1u)) {printf("FAIL parity_xor %u %u\n",x,y);return 1;}
if(parity_not(x,y)!=(unsigned)(__builtin_popcount(x)&1u)) {printf("FAIL parity_not %u %u\n",x,y);return 1;}
if(parity_even_xor(x,y)!=(unsigned)(__builtin_popcount(x)&1u)) {printf("FAIL parity_even_xor %u %u\n",x,y);return 1;}
if(pc_partition(x,y)!=(unsigned)(__builtin_popcount(x))) {printf("FAIL pc_partition %u %u\n",x,y);return 1;}
if(pc_symmetric_difference(x,y)!=(unsigned)(__builtin_popcount(x^y))) {printf("FAIL pc_symmetric_difference %u %u\n",x,y);return 1;}
if(pc_complement(x,y)!=(unsigned)(32u)) {printf("FAIL pc_complement %u %u\n",x,y);return 1;}
if(pc_variable_partition(x,y)!=(unsigned)(__builtin_popcount(x))) {printf("FAIL pc_variable_partition %u %u\n",x,y);return 1;}
if(pc_and_xor(x,y)!=(unsigned)(__builtin_popcount(x|y))) {printf("FAIL pc_and_xor %u %u\n",x,y);return 1;}
if(pc_union_minus_xor(x,y)!=(unsigned)(__builtin_popcount(x&y))) {printf("FAIL pc_union_minus_xor %u %u\n",x,y);return 1;}
if(pc_union_minus_left(x,y)!=(unsigned)(__builtin_popcount(y))) {printf("FAIL pc_union_minus_left %u %u\n",x,y);return 1;}
if(pc_union_minus_right(x,y)!=(unsigned)(__builtin_popcount(x))) {printf("FAIL pc_union_minus_right %u %u\n",x,y);return 1;}
if(pc_xor_minus_left(x,y)!=(unsigned)(__builtin_popcount(~x&y))) {printf("FAIL pc_xor_minus_left %u %u\n",x,y);return 1;}
if(pc_xor_minus_right(x,y)!=(unsigned)(__builtin_popcount(x&~y))) {printf("FAIL pc_xor_minus_right %u %u\n",x,y);return 1;}
if(pc_triangle_xor(x,y)!=(unsigned)(__builtin_popcount(x^y))) {printf("FAIL pc_triangle_xor %u %u\n",x,y);return 1;}
if(pc_triangle_and(x,y)!=(unsigned)(2u*__builtin_popcount(x&y))) {printf("FAIL pc_triangle_and %u %u\n",x,y);return 1;}
if(pc_triangle_or(x,y)!=(unsigned)(2u*__builtin_popcount(x|y))) {printf("FAIL pc_triangle_or %u %u\n",x,y);return 1;}
if(pc_doubled_or(x,y)!=(unsigned)(__builtin_popcount(x^y))) {printf("FAIL pc_doubled_or %u %u\n",x,y);return 1;}
if(pc_demorgan_and(x,y)!=(unsigned)(32u)) {printf("FAIL pc_demorgan_and %u %u\n",x,y);return 1;}
if(pc_demorgan_or(x,y)!=(unsigned)(32u)) {printf("FAIL pc_demorgan_or %u %u\n",x,y);return 1;}
if(pc_complement_difference(x,y)!=(unsigned)(__builtin_popcount(x^y))) {printf("FAIL pc_complement_difference %u %u\n",x,y);return 1;}
if(pc_xnor_minus_and(x,y)!=(unsigned)(__builtin_popcount(~(x|y)))) {printf("FAIL pc_xnor_minus_and %u %u\n",x,y);return 1;}
if(pc_nor_plus_xor(x,y)!=(unsigned)(__builtin_popcount(~(x&y)))) {printf("FAIL pc_nor_plus_xor %u %u\n",x,y);return 1;}
if(pc_partition_four(x,y)!=(unsigned)(32u)) {printf("FAIL pc_partition_four %u %u\n",x,y);return 1;}
if(pc_partition_three(x,y)!=(unsigned)(__builtin_popcount(x|y))) {printf("FAIL pc_partition_three %u %u\n",x,y);return 1;}
if(pc_absorption(x,y)!=(unsigned)(__builtin_popcount(x|y))) {printf("FAIL pc_absorption %u %u\n",x,y);return 1;}
}puts("PASS batch3 integer demand and cardinality");return 0;}
