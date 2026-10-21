#include <stdio.h>
#define NOINLINE __attribute__((noinline))
/* Wide companions of the JSON contracts; count result type remains C int. */
NOINLINE unsigned long long d_add(unsigned long long x,unsigned long long y) {(void)x;(void)y;return ((x&255u)+(y&255u))&15u;}
NOINLINE unsigned long long d_sub(unsigned long long x,unsigned long long y) {(void)x;(void)y;return ((x&255u)-(y&255u))&15u;}
NOINLINE unsigned long long d_mul(unsigned long long x,unsigned long long y) {(void)x;(void)y;return ((x&255u)*(y&255u))&15u;}
NOINLINE unsigned long long d_mul_tz(unsigned long long x,unsigned long long y) {(void)x;(void)y;return ((x&15u)*1536u)&1536u;}
NOINLINE unsigned long long d_shl(unsigned long long x,unsigned long long y) {(void)x;(void)y;return ((x&255u)<<7)&1536u;}
NOINLINE unsigned long long d_lshr(unsigned long long x,unsigned long long y) {(void)x;(void)y;return ((x&0xff00u)>>8)&15u;}
NOINLINE unsigned long long d_ashr(unsigned long long x,unsigned long long y) {(void)x;(void)y;return ((long long)(x&(~127ull))>>7)&15u;}
NOINLINE unsigned long long d_rotate(unsigned long long x,unsigned long long y) {(void)x;(void)y;return (((x&65535u)<<8)|((x&65535u)>>56))&0xf00u;}
NOINLINE unsigned long long d_not(unsigned long long x,unsigned long long y) {(void)x;(void)y;return (~(x&255u))&15u;}
NOINLINE unsigned long long d_neg(unsigned long long x,unsigned long long y) {(void)x;(void)y;return (-(x&255u))&15u;}
NOINLINE unsigned long long d_xor(unsigned long long x,unsigned long long y) {(void)x;(void)y;return ((x&255u)^(y&255u))&15u;}
NOINLINE unsigned long long d_or(unsigned long long x,unsigned long long y) {(void)x;(void)y;return ((x&255u)|(y&255u))&15u;}
NOINLINE unsigned long long d_and(unsigned long long x,unsigned long long y) {(void)x;(void)y;return ((x&255u)&(y&255u))&15u;}
NOINLINE unsigned long long d_bswap(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_bswap64(x&0xff00000000000000ull)&15u;}
NOINLINE unsigned long long d_rem_shl(unsigned long long x,unsigned long long y) {(void)x;(void)y;return ((x%48u)<<9)&1536u;}
NOINLINE unsigned long long d_rem_lshr(unsigned long long x,unsigned long long y) {(void)x;(void)y;return ((x%768u)>>4)&15u;}
NOINLINE unsigned long long d_rem_add(unsigned long long x,unsigned long long y) {(void)x;(void)y;return ((x%48u)+(y%80u))&15u;}
NOINLINE unsigned long long d_rem_mul(unsigned long long x,unsigned long long y) {(void)x;(void)y;return ((x%48u)*1536u)&1536u;}
NOINLINE unsigned long long tz_odd(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_ctzll(x*3u);}
NOINLINE unsigned long long tz_variable_odd(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_ctzll(x*(y|1u));}
NOINLINE unsigned long long tz_neg(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_ctzll(-x);}
NOINLINE unsigned long long tz_isolate(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_ctzll(x&-x);}
NOINLINE unsigned long long tz_or_neg(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_ctzll(x|-x);}
NOINLINE unsigned long long tz_xor_shift(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_ctzll(x^(x<<7));}
NOINLINE unsigned long long tz_or_shift(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_ctzll(x|(x<<7));}
NOINLINE unsigned long long tz_add_shift(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_ctzll(x+(x<<7));}
NOINLINE unsigned long long tz_sub_shift(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_ctzll(x-(x<<7));}
NOINLINE unsigned long long lz_or_shift(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_clzll(x|(x>>7));}
NOINLINE unsigned long long lz_xor_shift(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_clzll(x^(x>>7));}
NOINLINE unsigned long long lz_andnot_shift(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_clzll(x&~(x>>7));}
NOINLINE unsigned long long tz_andnot_shift(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_ctzll(x&~(x<<7));}
NOINLINE unsigned long long pc_swap(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_popcountll(__builtin_bswap64(x));}
NOINLINE unsigned long long pc_rotate(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_popcountll((x<<7)|(x>>57));}
NOINLINE unsigned long long parity_add(unsigned long long x,unsigned long long y) {(void)x;(void)y;return (__builtin_popcountll(x)+__builtin_popcountll(y))&1u;}
NOINLINE unsigned long long parity_sub(unsigned long long x,unsigned long long y) {(void)x;(void)y;return (__builtin_popcountll(x)-__builtin_popcountll(y))&1u;}
NOINLINE unsigned long long parity_xor(unsigned long long x,unsigned long long y) {(void)x;(void)y;return (__builtin_popcountll(x)^__builtin_popcountll(y))&1u;}
NOINLINE unsigned long long parity_not(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_popcountll(~x)&1u;}
NOINLINE unsigned long long parity_even_xor(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_popcountll(x^0x5au)&1u;}
NOINLINE unsigned long long pc_partition(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_popcountll(x&0x55555555u)+__builtin_popcountll(x&0xffffffffaaaaaaaaull);}
NOINLINE unsigned long long pc_symmetric_difference(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_popcountll(x|y)-__builtin_popcountll(x&y);}
NOINLINE unsigned long long pc_complement(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_popcountll(x)+__builtin_popcountll(~x);}
NOINLINE unsigned long long pc_variable_partition(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_popcountll(x&y)+__builtin_popcountll(x&~y);}
NOINLINE unsigned long long pc_and_xor(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_popcountll(x&y)+__builtin_popcountll(x^y);}
NOINLINE unsigned long long pc_union_minus_xor(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_popcountll(x|y)-__builtin_popcountll(x^y);}
NOINLINE unsigned long long pc_union_minus_left(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_popcountll(x|y)-__builtin_popcountll(x&~y);}
NOINLINE unsigned long long pc_union_minus_right(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_popcountll(x|y)-__builtin_popcountll(~x&y);}
NOINLINE unsigned long long pc_xor_minus_left(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_popcountll(x^y)-__builtin_popcountll(x&~y);}
NOINLINE unsigned long long pc_xor_minus_right(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_popcountll(x^y)-__builtin_popcountll(~x&y);}
NOINLINE unsigned long long pc_triangle_xor(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_popcountll(x)+__builtin_popcountll(y)-2*__builtin_popcountll(x&y);}
NOINLINE unsigned long long pc_triangle_and(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_popcountll(x)+__builtin_popcountll(y)-__builtin_popcountll(x^y);}
NOINLINE unsigned long long pc_triangle_or(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_popcountll(x)+__builtin_popcountll(y)+__builtin_popcountll(x^y);}
NOINLINE unsigned long long pc_doubled_or(unsigned long long x,unsigned long long y) {(void)x;(void)y;return 2*__builtin_popcountll(x|y)-__builtin_popcountll(x)-__builtin_popcountll(y);}
NOINLINE unsigned long long pc_demorgan_and(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_popcountll(x&y)+__builtin_popcountll(~x|~y);}
NOINLINE unsigned long long pc_demorgan_or(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_popcountll(x|y)+__builtin_popcountll(~x&~y);}
NOINLINE unsigned long long pc_complement_difference(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_popcountll(~(x&y))-__builtin_popcountll(~(x|y));}
NOINLINE unsigned long long pc_xnor_minus_and(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_popcountll(~(x^y))-__builtin_popcountll(x&y);}
NOINLINE unsigned long long pc_nor_plus_xor(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_popcountll(~(x|y))+__builtin_popcountll(x^y);}
NOINLINE unsigned long long pc_partition_four(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_popcountll(x&y)+__builtin_popcountll(x&~y)+__builtin_popcountll(~x&y)+__builtin_popcountll(~x&~y);}
NOINLINE unsigned long long pc_partition_three(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_popcountll(x&y)+__builtin_popcountll(x&~y)+__builtin_popcountll(~x&y);}
NOINLINE unsigned long long pc_absorption(unsigned long long x,unsigned long long y) {(void)x;(void)y;return __builtin_popcountll((x|y)&x)+__builtin_popcountll(~x&y);}
int main(void) {unsigned long long x=0,y=1; static const unsigned long long edges[]={0,1,2,7,127,128,255,256,0x7fffffffffffffffull,0x8000000000000000ull,0xfffffffffffffffeull,0xffffffffffffffffull};
for(unsigned i=0;i<65548;++i) {x=i<12?edges[i]:x*1664525u+1013904223u;y=y*22695477u+1;
if(d_add(x,y)!=(unsigned long long)((x+y)&15u)) {printf("FAIL wide d_add %llu %llu\n",x,y);return 1;}
if(d_sub(x,y)!=(unsigned long long)((x-y)&15u)) {printf("FAIL wide d_sub %llu %llu\n",x,y);return 1;}
if(d_mul(x,y)!=(unsigned long long)((x*y)&15u)) {printf("FAIL wide d_mul %llu %llu\n",x,y);return 1;}
if(d_mul_tz(x,y)!=(unsigned long long)((x*1536u)&1536u)) {printf("FAIL wide d_mul_tz %llu %llu\n",x,y);return 1;}
if(d_shl(x,y)!=(unsigned long long)((x<<7)&1536u)) {printf("FAIL wide d_shl %llu %llu\n",x,y);return 1;}
if(d_lshr(x,y)!=(unsigned long long)((x>>8)&15u)) {printf("FAIL wide d_lshr %llu %llu\n",x,y);return 1;}
if(d_ashr(x,y)!=(unsigned long long)(((long long)x>>7)&15u)) {printf("FAIL wide d_ashr %llu %llu\n",x,y);return 1;}
if(d_rotate(x,y)!=(unsigned long long)(((x<<8)|(x>>56))&0xf00u)) {printf("FAIL wide d_rotate %llu %llu\n",x,y);return 1;}
if(d_not(x,y)!=(unsigned long long)((~x)&15u)) {printf("FAIL wide d_not %llu %llu\n",x,y);return 1;}
if(d_neg(x,y)!=(unsigned long long)((-x)&15u)) {printf("FAIL wide d_neg %llu %llu\n",x,y);return 1;}
if(d_xor(x,y)!=(unsigned long long)((x^y)&15u)) {printf("FAIL wide d_xor %llu %llu\n",x,y);return 1;}
if(d_or(x,y)!=(unsigned long long)((x|y)&15u)) {printf("FAIL wide d_or %llu %llu\n",x,y);return 1;}
if(d_and(x,y)!=(unsigned long long)((x&y)&15u)) {printf("FAIL wide d_and %llu %llu\n",x,y);return 1;}
if(d_bswap(x,y)!=(unsigned long long)(__builtin_bswap64(x)&15u)) {printf("FAIL wide d_bswap %llu %llu\n",x,y);return 1;}
if(d_rem_shl(x,y)!=(unsigned long long)((x<<9)&1536u)) {printf("FAIL wide d_rem_shl %llu %llu\n",x,y);return 1;}
if(d_rem_lshr(x,y)!=(unsigned long long)((x>>4)&15u)) {printf("FAIL wide d_rem_lshr %llu %llu\n",x,y);return 1;}
if(d_rem_add(x,y)!=(unsigned long long)((x+y)&15u)) {printf("FAIL wide d_rem_add %llu %llu\n",x,y);return 1;}
if(d_rem_mul(x,y)!=(unsigned long long)((x*1536u)&1536u)) {printf("FAIL wide d_rem_mul %llu %llu\n",x,y);return 1;}
if(x!=0 && tz_odd(x,y)!=(unsigned long long)(__builtin_ctzll(x))) {printf("FAIL wide tz_odd %llu %llu\n",x,y);return 1;}
if(x!=0 && tz_variable_odd(x,y)!=(unsigned long long)(__builtin_ctzll(x))) {printf("FAIL wide tz_variable_odd %llu %llu\n",x,y);return 1;}
if(x!=0 && tz_neg(x,y)!=(unsigned long long)(__builtin_ctzll(x))) {printf("FAIL wide tz_neg %llu %llu\n",x,y);return 1;}
if(x!=0 && tz_isolate(x,y)!=(unsigned long long)(__builtin_ctzll(x))) {printf("FAIL wide tz_isolate %llu %llu\n",x,y);return 1;}
if(x!=0 && tz_or_neg(x,y)!=(unsigned long long)(__builtin_ctzll(x))) {printf("FAIL wide tz_or_neg %llu %llu\n",x,y);return 1;}
if(x!=0 && tz_xor_shift(x,y)!=(unsigned long long)(__builtin_ctzll(x))) {printf("FAIL wide tz_xor_shift %llu %llu\n",x,y);return 1;}
if(x!=0 && tz_or_shift(x,y)!=(unsigned long long)(__builtin_ctzll(x))) {printf("FAIL wide tz_or_shift %llu %llu\n",x,y);return 1;}
if(x!=0 && tz_add_shift(x,y)!=(unsigned long long)(__builtin_ctzll(x))) {printf("FAIL wide tz_add_shift %llu %llu\n",x,y);return 1;}
if(x!=0 && tz_sub_shift(x,y)!=(unsigned long long)(__builtin_ctzll(x))) {printf("FAIL wide tz_sub_shift %llu %llu\n",x,y);return 1;}
if(x!=0 && lz_or_shift(x,y)!=(unsigned long long)(__builtin_clzll(x))) {printf("FAIL wide lz_or_shift %llu %llu\n",x,y);return 1;}
if(x!=0 && lz_xor_shift(x,y)!=(unsigned long long)(__builtin_clzll(x))) {printf("FAIL wide lz_xor_shift %llu %llu\n",x,y);return 1;}
if(x!=0 && lz_andnot_shift(x,y)!=(unsigned long long)(__builtin_clzll(x))) {printf("FAIL wide lz_andnot_shift %llu %llu\n",x,y);return 1;}
if(x!=0 && tz_andnot_shift(x,y)!=(unsigned long long)(__builtin_ctzll(x))) {printf("FAIL wide tz_andnot_shift %llu %llu\n",x,y);return 1;}
if(pc_swap(x,y)!=(unsigned long long)(__builtin_popcountll(x))) {printf("FAIL wide pc_swap %llu %llu\n",x,y);return 1;}
if(pc_rotate(x,y)!=(unsigned long long)(__builtin_popcountll(x))) {printf("FAIL wide pc_rotate %llu %llu\n",x,y);return 1;}
if(parity_add(x,y)!=(unsigned long long)(__builtin_popcountll(x^y)&1u)) {printf("FAIL wide parity_add %llu %llu\n",x,y);return 1;}
if(parity_sub(x,y)!=(unsigned long long)(__builtin_popcountll(x^y)&1u)) {printf("FAIL wide parity_sub %llu %llu\n",x,y);return 1;}
if(parity_xor(x,y)!=(unsigned long long)(__builtin_popcountll(x^y)&1u)) {printf("FAIL wide parity_xor %llu %llu\n",x,y);return 1;}
if(parity_not(x,y)!=(unsigned long long)(__builtin_popcountll(x)&1u)) {printf("FAIL wide parity_not %llu %llu\n",x,y);return 1;}
if(parity_even_xor(x,y)!=(unsigned long long)(__builtin_popcountll(x)&1u)) {printf("FAIL wide parity_even_xor %llu %llu\n",x,y);return 1;}
if(pc_partition(x,y)!=(unsigned long long)(__builtin_popcountll(x))) {printf("FAIL wide pc_partition %llu %llu\n",x,y);return 1;}
if(pc_symmetric_difference(x,y)!=(unsigned long long)(__builtin_popcountll(x^y))) {printf("FAIL wide pc_symmetric_difference %llu %llu\n",x,y);return 1;}
if(pc_complement(x,y)!=(unsigned long long)(64u)) {printf("FAIL wide pc_complement %llu %llu\n",x,y);return 1;}
if(pc_variable_partition(x,y)!=(unsigned long long)(__builtin_popcountll(x))) {printf("FAIL wide pc_variable_partition %llu %llu\n",x,y);return 1;}
if(pc_and_xor(x,y)!=(unsigned long long)(__builtin_popcountll(x|y))) {printf("FAIL wide pc_and_xor %llu %llu\n",x,y);return 1;}
if(pc_union_minus_xor(x,y)!=(unsigned long long)(__builtin_popcountll(x&y))) {printf("FAIL wide pc_union_minus_xor %llu %llu\n",x,y);return 1;}
if(pc_union_minus_left(x,y)!=(unsigned long long)(__builtin_popcountll(y))) {printf("FAIL wide pc_union_minus_left %llu %llu\n",x,y);return 1;}
if(pc_union_minus_right(x,y)!=(unsigned long long)(__builtin_popcountll(x))) {printf("FAIL wide pc_union_minus_right %llu %llu\n",x,y);return 1;}
if(pc_xor_minus_left(x,y)!=(unsigned long long)(__builtin_popcountll(~x&y))) {printf("FAIL wide pc_xor_minus_left %llu %llu\n",x,y);return 1;}
if(pc_xor_minus_right(x,y)!=(unsigned long long)(__builtin_popcountll(x&~y))) {printf("FAIL wide pc_xor_minus_right %llu %llu\n",x,y);return 1;}
if(pc_triangle_xor(x,y)!=(unsigned long long)(__builtin_popcountll(x^y))) {printf("FAIL wide pc_triangle_xor %llu %llu\n",x,y);return 1;}
if(pc_triangle_and(x,y)!=(unsigned long long)(2u*__builtin_popcountll(x&y))) {printf("FAIL wide pc_triangle_and %llu %llu\n",x,y);return 1;}
if(pc_triangle_or(x,y)!=(unsigned long long)(2u*__builtin_popcountll(x|y))) {printf("FAIL wide pc_triangle_or %llu %llu\n",x,y);return 1;}
if(pc_doubled_or(x,y)!=(unsigned long long)(__builtin_popcountll(x^y))) {printf("FAIL wide pc_doubled_or %llu %llu\n",x,y);return 1;}
if(pc_demorgan_and(x,y)!=(unsigned long long)(64u)) {printf("FAIL wide pc_demorgan_and %llu %llu\n",x,y);return 1;}
if(pc_demorgan_or(x,y)!=(unsigned long long)(64u)) {printf("FAIL wide pc_demorgan_or %llu %llu\n",x,y);return 1;}
if(pc_complement_difference(x,y)!=(unsigned long long)(__builtin_popcountll(x^y))) {printf("FAIL wide pc_complement_difference %llu %llu\n",x,y);return 1;}
if(pc_xnor_minus_and(x,y)!=(unsigned long long)(__builtin_popcountll(~(x|y)))) {printf("FAIL wide pc_xnor_minus_and %llu %llu\n",x,y);return 1;}
if(pc_nor_plus_xor(x,y)!=(unsigned long long)(__builtin_popcountll(~(x&y)))) {printf("FAIL wide pc_nor_plus_xor %llu %llu\n",x,y);return 1;}
if(pc_partition_four(x,y)!=(unsigned long long)(64u)) {printf("FAIL wide pc_partition_four %llu %llu\n",x,y);return 1;}
if(pc_partition_three(x,y)!=(unsigned long long)(__builtin_popcountll(x|y))) {printf("FAIL wide pc_partition_three %llu %llu\n",x,y);return 1;}
if(pc_absorption(x,y)!=(unsigned long long)(__builtin_popcountll(x|y))) {printf("FAIL wide pc_absorption %llu %llu\n",x,y);return 1;}
}puts("PASS bit-demand wide demand and cardinality");return 0;}
