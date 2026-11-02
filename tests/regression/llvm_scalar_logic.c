/* Twenty-five total scalar contracts, each at 32 and 64 bits.
 * See engineering/llvm-missed/SCALAR-LOGIC.md for the fifty-report audit.
 * This is fifty native forms, not fifty newly solved LLVM reports. */
#include <stdint.h>
#include <stdio.h>

__attribute__((noinline)) uint32_t logic_xor_sum_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return (x | y) & ~(x & y);
}

__attribute__((noinline)) uint32_t reference_xor_sum_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return x ^ y;
}

__attribute__((noinline)) uint32_t logic_xor_products_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return (x & ~y) | (~x & y);
}

__attribute__((noinline)) uint32_t reference_xor_products_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return x ^ y;
}

__attribute__((noinline)) uint32_t logic_xor_shared_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return ((x & y) ^ y) | ((x & y) ^ x);
}

__attribute__((noinline)) uint32_t reference_xor_shared_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return x ^ y;
}

__attribute__((noinline)) uint32_t logic_eliminate_y_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return (x & y) | (x & ~y);
}

__attribute__((noinline)) uint32_t reference_eliminate_y_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return x;
}

__attribute__((noinline)) uint32_t logic_eliminate_y_dual_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return (x | y) & (x | ~y);
}

__attribute__((noinline)) uint32_t reference_eliminate_y_dual_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return x;
}

__attribute__((noinline)) uint32_t logic_consensus_join_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return (x | y) & (x | m);
}

__attribute__((noinline)) uint32_t reference_consensus_join_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return x | (y & m);
}

__attribute__((noinline)) uint32_t logic_consensus_meet_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return (x & y) | (x & m);
}

__attribute__((noinline)) uint32_t reference_consensus_meet_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return x & (y | m);
}

__attribute__((noinline)) uint32_t logic_negate_join_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return ~((x | y) & (x | m));
}

__attribute__((noinline)) uint32_t reference_negate_join_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return ~(x | (y & m));
}

__attribute__((noinline)) uint32_t logic_negate_meet_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return ~((x & y) | (x & m));
}

__attribute__((noinline)) uint32_t reference_negate_meet_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return ~(x & (y | m));
}

__attribute__((noinline)) uint32_t logic_masked_clear_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return (x & ~m) | ((x & m) & ~(y & m));
}

__attribute__((noinline)) uint32_t reference_masked_clear_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return x & ~(y & m);
}

__attribute__((noinline)) uint32_t logic_masked_or_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return (x & ~m) | ((x & m) | (y & m));
}

__attribute__((noinline)) uint32_t reference_masked_or_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return x | (y & m);
}

__attribute__((noinline)) uint32_t logic_masked_xor_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return (x & ~m) | ((x & m) ^ (y & m));
}

__attribute__((noinline)) uint32_t reference_masked_xor_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return x ^ (y & m);
}

__attribute__((noinline)) uint32_t logic_masked_and_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return (x & ~m) | ((x & m) & (y & m));
}

__attribute__((noinline)) uint32_t reference_masked_and_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return x & (~m | y);
}

__attribute__((noinline)) uint32_t logic_complement_add_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return ((x ^ (uint32_t)42) + (uint32_t)1) + y;
}

__attribute__((noinline)) uint32_t reference_complement_add_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return y - (x ^ (uint32_t)~(uint32_t)42);
}

__attribute__((noinline)) uint32_t logic_complement_sub_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return y - ((x ^ (uint32_t)42) + (uint32_t)1);
}

__attribute__((noinline)) uint32_t reference_complement_sub_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return y + (x ^ (uint32_t)~(uint32_t)42);
}

__attribute__((noinline)) uint32_t logic_complement_commute_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return y + ((x ^ (uint32_t)42) + (uint32_t)1);
}

__attribute__((noinline)) uint32_t reference_complement_commute_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return y - (x ^ (uint32_t)~(uint32_t)42);
}

__attribute__((noinline)) uint32_t logic_shift_left_bit_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return (((uint32_t)2 << (x & (32-1))) & (uint32_t)16) != 0;
}

__attribute__((noinline)) uint32_t reference_shift_left_bit_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return (x & (32-1)) == 3;
}

__attribute__((noinline)) uint32_t logic_shift_right_bit_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return (((uint32_t)32 >> (x & (32-1))) & (uint32_t)16) == 0;
}

__attribute__((noinline)) uint32_t reference_shift_right_bit_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return (x & (32-1)) != 1;
}

__attribute__((noinline)) uint32_t logic_shift_low_bits_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return (((uint32_t)2 << (x & (32-1))) & (uint32_t)15) != 0;
}

__attribute__((noinline)) uint32_t reference_shift_low_bits_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return (x & (32-1)) < 3;
}

__attribute__((noinline)) uint32_t logic_shift_left_absent_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return (((uint32_t)32 << (x & (32-1))) & (uint32_t)2) != 0;
}

__attribute__((noinline)) uint32_t reference_shift_left_absent_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return (uint32_t)0;
}

__attribute__((noinline)) uint32_t logic_shift_right_absent_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return (((uint32_t)2 >> (x & (32-1))) & (uint32_t)32) == 0;
}

__attribute__((noinline)) uint32_t reference_shift_right_absent_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return (uint32_t)1;
}

__attribute__((noinline)) uint32_t logic_compare_not_lt_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return (uint32_t)~x < (uint32_t)~y;
}

__attribute__((noinline)) uint32_t reference_compare_not_lt_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return y < x;
}

__attribute__((noinline)) uint32_t logic_compare_not_eq_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return (uint32_t)~x == (uint32_t)~y;
}

__attribute__((noinline)) uint32_t reference_compare_not_eq_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return y == x;
}

__attribute__((noinline)) uint32_t logic_xor_complement_and_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return (x ^ y) & (x ^ (uint32_t)~y);
}

__attribute__((noinline)) uint32_t reference_xor_complement_and_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return (uint32_t)0;
}

__attribute__((noinline)) uint32_t logic_xor_complement_or_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return (x ^ y) | (x ^ (uint32_t)~y);
}

__attribute__((noinline)) uint32_t reference_xor_complement_or_32(uint32_t x, uint32_t y, uint32_t m) {
    (void)x; (void)y; (void)m;
    return (uint32_t)~(uint32_t)0;
}

__attribute__((noinline)) uint64_t logic_xor_sum_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return (x | y) & ~(x & y);
}

__attribute__((noinline)) uint64_t reference_xor_sum_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return x ^ y;
}

__attribute__((noinline)) uint64_t logic_xor_products_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return (x & ~y) | (~x & y);
}

__attribute__((noinline)) uint64_t reference_xor_products_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return x ^ y;
}

__attribute__((noinline)) uint64_t logic_xor_shared_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return ((x & y) ^ y) | ((x & y) ^ x);
}

__attribute__((noinline)) uint64_t reference_xor_shared_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return x ^ y;
}

__attribute__((noinline)) uint64_t logic_eliminate_y_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return (x & y) | (x & ~y);
}

__attribute__((noinline)) uint64_t reference_eliminate_y_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return x;
}

__attribute__((noinline)) uint64_t logic_eliminate_y_dual_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return (x | y) & (x | ~y);
}

__attribute__((noinline)) uint64_t reference_eliminate_y_dual_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return x;
}

__attribute__((noinline)) uint64_t logic_consensus_join_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return (x | y) & (x | m);
}

__attribute__((noinline)) uint64_t reference_consensus_join_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return x | (y & m);
}

__attribute__((noinline)) uint64_t logic_consensus_meet_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return (x & y) | (x & m);
}

__attribute__((noinline)) uint64_t reference_consensus_meet_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return x & (y | m);
}

__attribute__((noinline)) uint64_t logic_negate_join_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return ~((x | y) & (x | m));
}

__attribute__((noinline)) uint64_t reference_negate_join_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return ~(x | (y & m));
}

__attribute__((noinline)) uint64_t logic_negate_meet_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return ~((x & y) | (x & m));
}

__attribute__((noinline)) uint64_t reference_negate_meet_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return ~(x & (y | m));
}

__attribute__((noinline)) uint64_t logic_masked_clear_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return (x & ~m) | ((x & m) & ~(y & m));
}

__attribute__((noinline)) uint64_t reference_masked_clear_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return x & ~(y & m);
}

__attribute__((noinline)) uint64_t logic_masked_or_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return (x & ~m) | ((x & m) | (y & m));
}

__attribute__((noinline)) uint64_t reference_masked_or_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return x | (y & m);
}

__attribute__((noinline)) uint64_t logic_masked_xor_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return (x & ~m) | ((x & m) ^ (y & m));
}

__attribute__((noinline)) uint64_t reference_masked_xor_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return x ^ (y & m);
}

__attribute__((noinline)) uint64_t logic_masked_and_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return (x & ~m) | ((x & m) & (y & m));
}

__attribute__((noinline)) uint64_t reference_masked_and_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return x & (~m | y);
}

__attribute__((noinline)) uint64_t logic_complement_add_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return ((x ^ (uint64_t)42) + (uint64_t)1) + y;
}

__attribute__((noinline)) uint64_t reference_complement_add_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return y - (x ^ (uint64_t)~(uint64_t)42);
}

__attribute__((noinline)) uint64_t logic_complement_sub_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return y - ((x ^ (uint64_t)42) + (uint64_t)1);
}

__attribute__((noinline)) uint64_t reference_complement_sub_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return y + (x ^ (uint64_t)~(uint64_t)42);
}

__attribute__((noinline)) uint64_t logic_complement_commute_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return y + ((x ^ (uint64_t)42) + (uint64_t)1);
}

__attribute__((noinline)) uint64_t reference_complement_commute_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return y - (x ^ (uint64_t)~(uint64_t)42);
}

__attribute__((noinline)) uint64_t logic_shift_left_bit_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return (((uint64_t)2 << (x & (64-1))) & (uint64_t)16) != 0;
}

__attribute__((noinline)) uint64_t reference_shift_left_bit_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return (x & (64-1)) == 3;
}

__attribute__((noinline)) uint64_t logic_shift_right_bit_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return (((uint64_t)32 >> (x & (64-1))) & (uint64_t)16) == 0;
}

__attribute__((noinline)) uint64_t reference_shift_right_bit_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return (x & (64-1)) != 1;
}

__attribute__((noinline)) uint64_t logic_shift_low_bits_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return (((uint64_t)2 << (x & (64-1))) & (uint64_t)15) != 0;
}

__attribute__((noinline)) uint64_t reference_shift_low_bits_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return (x & (64-1)) < 3;
}

__attribute__((noinline)) uint64_t logic_shift_left_absent_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return (((uint64_t)32 << (x & (64-1))) & (uint64_t)2) != 0;
}

__attribute__((noinline)) uint64_t reference_shift_left_absent_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return (uint64_t)0;
}

__attribute__((noinline)) uint64_t logic_shift_right_absent_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return (((uint64_t)2 >> (x & (64-1))) & (uint64_t)32) == 0;
}

__attribute__((noinline)) uint64_t reference_shift_right_absent_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return (uint64_t)1;
}

__attribute__((noinline)) uint64_t logic_compare_not_lt_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return (uint64_t)~x < (uint64_t)~y;
}

__attribute__((noinline)) uint64_t reference_compare_not_lt_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return y < x;
}

__attribute__((noinline)) uint64_t logic_compare_not_eq_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return (uint64_t)~x == (uint64_t)~y;
}

__attribute__((noinline)) uint64_t reference_compare_not_eq_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return y == x;
}

__attribute__((noinline)) uint64_t logic_xor_complement_and_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return (x ^ y) & (x ^ (uint64_t)~y);
}

__attribute__((noinline)) uint64_t reference_xor_complement_and_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return (uint64_t)0;
}

__attribute__((noinline)) uint64_t logic_xor_complement_or_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return (x ^ y) | (x ^ (uint64_t)~y);
}

__attribute__((noinline)) uint64_t reference_xor_complement_or_64(uint64_t x, uint64_t y, uint64_t m) {
    (void)x; (void)y; (void)m;
    return (uint64_t)~(uint64_t)0;
}

static uint64_t next(uint64_t *s) {
    *s ^= *s << 13; *s ^= *s >> 7; *s ^= *s << 17; return *s;
}
int main(void) {
    uint64_t seed = UINT64_C(0x243f6a8885a308d3), digest = 0;
    const uint64_t edges[] = {0, 1, 2, 31, 32, 63, 64, UINT32_MAX, UINT64_C(0x8000000000000000), UINT64_MAX};
    for (unsigned round = 0; round < 512; ++round) {
        uint64_t x = round < 10 ? edges[round] : next(&seed);
        uint64_t y = round < 10 ? edges[9-round] : next(&seed);
        uint64_t m = round < 10 ? edges[(round+3)%10] : next(&seed);
        if (logic_xor_sum_32(x,y,m) != reference_xor_sum_32(x,y,m)) {
            puts("mismatch: xor_sum_32"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_xor_sum_32(x,y,m);
        if (logic_xor_products_32(x,y,m) != reference_xor_products_32(x,y,m)) {
            puts("mismatch: xor_products_32"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_xor_products_32(x,y,m);
        if (logic_xor_shared_32(x,y,m) != reference_xor_shared_32(x,y,m)) {
            puts("mismatch: xor_shared_32"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_xor_shared_32(x,y,m);
        if (logic_eliminate_y_32(x,y,m) != reference_eliminate_y_32(x,y,m)) {
            puts("mismatch: eliminate_y_32"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_eliminate_y_32(x,y,m);
        if (logic_eliminate_y_dual_32(x,y,m) != reference_eliminate_y_dual_32(x,y,m)) {
            puts("mismatch: eliminate_y_dual_32"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_eliminate_y_dual_32(x,y,m);
        if (logic_consensus_join_32(x,y,m) != reference_consensus_join_32(x,y,m)) {
            puts("mismatch: consensus_join_32"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_consensus_join_32(x,y,m);
        if (logic_consensus_meet_32(x,y,m) != reference_consensus_meet_32(x,y,m)) {
            puts("mismatch: consensus_meet_32"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_consensus_meet_32(x,y,m);
        if (logic_negate_join_32(x,y,m) != reference_negate_join_32(x,y,m)) {
            puts("mismatch: negate_join_32"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_negate_join_32(x,y,m);
        if (logic_negate_meet_32(x,y,m) != reference_negate_meet_32(x,y,m)) {
            puts("mismatch: negate_meet_32"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_negate_meet_32(x,y,m);
        if (logic_masked_clear_32(x,y,m) != reference_masked_clear_32(x,y,m)) {
            puts("mismatch: masked_clear_32"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_masked_clear_32(x,y,m);
        if (logic_masked_or_32(x,y,m) != reference_masked_or_32(x,y,m)) {
            puts("mismatch: masked_or_32"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_masked_or_32(x,y,m);
        if (logic_masked_xor_32(x,y,m) != reference_masked_xor_32(x,y,m)) {
            puts("mismatch: masked_xor_32"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_masked_xor_32(x,y,m);
        if (logic_masked_and_32(x,y,m) != reference_masked_and_32(x,y,m)) {
            puts("mismatch: masked_and_32"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_masked_and_32(x,y,m);
        if (logic_complement_add_32(x,y,m) != reference_complement_add_32(x,y,m)) {
            puts("mismatch: complement_add_32"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_complement_add_32(x,y,m);
        if (logic_complement_sub_32(x,y,m) != reference_complement_sub_32(x,y,m)) {
            puts("mismatch: complement_sub_32"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_complement_sub_32(x,y,m);
        if (logic_complement_commute_32(x,y,m) != reference_complement_commute_32(x,y,m)) {
            puts("mismatch: complement_commute_32"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_complement_commute_32(x,y,m);
        if (logic_shift_left_bit_32(x,y,m) != reference_shift_left_bit_32(x,y,m)) {
            puts("mismatch: shift_left_bit_32"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_shift_left_bit_32(x,y,m);
        if (logic_shift_right_bit_32(x,y,m) != reference_shift_right_bit_32(x,y,m)) {
            puts("mismatch: shift_right_bit_32"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_shift_right_bit_32(x,y,m);
        if (logic_shift_low_bits_32(x,y,m) != reference_shift_low_bits_32(x,y,m)) {
            puts("mismatch: shift_low_bits_32"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_shift_low_bits_32(x,y,m);
        if (logic_shift_left_absent_32(x,y,m) != reference_shift_left_absent_32(x,y,m)) {
            puts("mismatch: shift_left_absent_32"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_shift_left_absent_32(x,y,m);
        if (logic_shift_right_absent_32(x,y,m) != reference_shift_right_absent_32(x,y,m)) {
            puts("mismatch: shift_right_absent_32"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_shift_right_absent_32(x,y,m);
        if (logic_compare_not_lt_32(x,y,m) != reference_compare_not_lt_32(x,y,m)) {
            puts("mismatch: compare_not_lt_32"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_compare_not_lt_32(x,y,m);
        if (logic_compare_not_eq_32(x,y,m) != reference_compare_not_eq_32(x,y,m)) {
            puts("mismatch: compare_not_eq_32"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_compare_not_eq_32(x,y,m);
        if (logic_xor_complement_and_32(x,y,m) != reference_xor_complement_and_32(x,y,m)) {
            puts("mismatch: xor_complement_and_32"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_xor_complement_and_32(x,y,m);
        if (logic_xor_complement_or_32(x,y,m) != reference_xor_complement_or_32(x,y,m)) {
            puts("mismatch: xor_complement_or_32"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_xor_complement_or_32(x,y,m);
        if (logic_xor_sum_64(x,y,m) != reference_xor_sum_64(x,y,m)) {
            puts("mismatch: xor_sum_64"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_xor_sum_64(x,y,m);
        if (logic_xor_products_64(x,y,m) != reference_xor_products_64(x,y,m)) {
            puts("mismatch: xor_products_64"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_xor_products_64(x,y,m);
        if (logic_xor_shared_64(x,y,m) != reference_xor_shared_64(x,y,m)) {
            puts("mismatch: xor_shared_64"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_xor_shared_64(x,y,m);
        if (logic_eliminate_y_64(x,y,m) != reference_eliminate_y_64(x,y,m)) {
            puts("mismatch: eliminate_y_64"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_eliminate_y_64(x,y,m);
        if (logic_eliminate_y_dual_64(x,y,m) != reference_eliminate_y_dual_64(x,y,m)) {
            puts("mismatch: eliminate_y_dual_64"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_eliminate_y_dual_64(x,y,m);
        if (logic_consensus_join_64(x,y,m) != reference_consensus_join_64(x,y,m)) {
            puts("mismatch: consensus_join_64"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_consensus_join_64(x,y,m);
        if (logic_consensus_meet_64(x,y,m) != reference_consensus_meet_64(x,y,m)) {
            puts("mismatch: consensus_meet_64"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_consensus_meet_64(x,y,m);
        if (logic_negate_join_64(x,y,m) != reference_negate_join_64(x,y,m)) {
            puts("mismatch: negate_join_64"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_negate_join_64(x,y,m);
        if (logic_negate_meet_64(x,y,m) != reference_negate_meet_64(x,y,m)) {
            puts("mismatch: negate_meet_64"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_negate_meet_64(x,y,m);
        if (logic_masked_clear_64(x,y,m) != reference_masked_clear_64(x,y,m)) {
            puts("mismatch: masked_clear_64"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_masked_clear_64(x,y,m);
        if (logic_masked_or_64(x,y,m) != reference_masked_or_64(x,y,m)) {
            puts("mismatch: masked_or_64"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_masked_or_64(x,y,m);
        if (logic_masked_xor_64(x,y,m) != reference_masked_xor_64(x,y,m)) {
            puts("mismatch: masked_xor_64"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_masked_xor_64(x,y,m);
        if (logic_masked_and_64(x,y,m) != reference_masked_and_64(x,y,m)) {
            puts("mismatch: masked_and_64"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_masked_and_64(x,y,m);
        if (logic_complement_add_64(x,y,m) != reference_complement_add_64(x,y,m)) {
            puts("mismatch: complement_add_64"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_complement_add_64(x,y,m);
        if (logic_complement_sub_64(x,y,m) != reference_complement_sub_64(x,y,m)) {
            puts("mismatch: complement_sub_64"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_complement_sub_64(x,y,m);
        if (logic_complement_commute_64(x,y,m) != reference_complement_commute_64(x,y,m)) {
            puts("mismatch: complement_commute_64"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_complement_commute_64(x,y,m);
        if (logic_shift_left_bit_64(x,y,m) != reference_shift_left_bit_64(x,y,m)) {
            puts("mismatch: shift_left_bit_64"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_shift_left_bit_64(x,y,m);
        if (logic_shift_right_bit_64(x,y,m) != reference_shift_right_bit_64(x,y,m)) {
            puts("mismatch: shift_right_bit_64"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_shift_right_bit_64(x,y,m);
        if (logic_shift_low_bits_64(x,y,m) != reference_shift_low_bits_64(x,y,m)) {
            puts("mismatch: shift_low_bits_64"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_shift_low_bits_64(x,y,m);
        if (logic_shift_left_absent_64(x,y,m) != reference_shift_left_absent_64(x,y,m)) {
            puts("mismatch: shift_left_absent_64"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_shift_left_absent_64(x,y,m);
        if (logic_shift_right_absent_64(x,y,m) != reference_shift_right_absent_64(x,y,m)) {
            puts("mismatch: shift_right_absent_64"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_shift_right_absent_64(x,y,m);
        if (logic_compare_not_lt_64(x,y,m) != reference_compare_not_lt_64(x,y,m)) {
            puts("mismatch: compare_not_lt_64"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_compare_not_lt_64(x,y,m);
        if (logic_compare_not_eq_64(x,y,m) != reference_compare_not_eq_64(x,y,m)) {
            puts("mismatch: compare_not_eq_64"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_compare_not_eq_64(x,y,m);
        if (logic_xor_complement_and_64(x,y,m) != reference_xor_complement_and_64(x,y,m)) {
            puts("mismatch: xor_complement_and_64"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_xor_complement_and_64(x,y,m);
        if (logic_xor_complement_or_64(x,y,m) != reference_xor_complement_or_64(x,y,m)) {
            puts("mismatch: xor_complement_or_64"); return 1;
        }
        digest = digest * UINT64_C(0x100000001b3) ^ logic_xor_complement_or_64(x,y,m);
    }
    printf("scalar logic: %016llx\n", (unsigned long long)digest);
    return 0;
}
