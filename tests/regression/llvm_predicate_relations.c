#include <stdint.h>
#include <stdio.h>
/* Total unsigned contracts; all shifts are fixed and in range. */
__attribute__((noinline)) uint32_t predicate_nested_same_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return c ? (c ? x : y) : y;
}
__attribute__((noinline)) uint32_t reference_nested_same_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return c ? x : y;
}
__attribute__((noinline)) uint32_t predicate_nested_inverse_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return c ? (!c ? x : y) : y;
}
__attribute__((noinline)) uint32_t reference_nested_inverse_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return y;
}
__attribute__((noinline)) uint32_t predicate_nested_false_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return c ? x : (c ? y : x);
}
__attribute__((noinline)) uint32_t reference_nested_false_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return x;
}
__attribute__((noinline)) uint32_t predicate_nested_both_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return c ? (c ? x : y) : (c ? x : y);
}
__attribute__((noinline)) uint32_t reference_nested_both_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return c ? x : y;
}
__attribute__((noinline)) uint32_t predicate_swapped_add_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (c ? x : y) + (c ? y : x);
}
__attribute__((noinline)) uint32_t reference_swapped_add_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return x + y;
}
__attribute__((noinline)) uint32_t predicate_swapped_mul_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (c ? x : y) * (c ? y : x);
}
__attribute__((noinline)) uint32_t reference_swapped_mul_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return x * y;
}
__attribute__((noinline)) uint32_t predicate_swapped_and_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (c ? x : y) & (c ? y : x);
}
__attribute__((noinline)) uint32_t reference_swapped_and_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return x & y;
}
__attribute__((noinline)) uint32_t predicate_swapped_or_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (c ? x : y) | (c ? y : x);
}
__attribute__((noinline)) uint32_t reference_swapped_or_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return x | y;
}
__attribute__((noinline)) uint32_t predicate_swapped_xor_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (c ? x : y) ^ (c ? y : x);
}
__attribute__((noinline)) uint32_t reference_swapped_xor_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return x ^ y;
}
__attribute__((noinline)) uint32_t predicate_swapped_equal_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (c ? x : y) == (c ? y : x);
}
__attribute__((noinline)) uint32_t reference_swapped_equal_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return x == y;
}
__attribute__((noinline)) uint32_t predicate_swapped_unequal_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (c ? x : y) != (c ? y : x);
}
__attribute__((noinline)) uint32_t reference_swapped_unequal_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return x != y;
}
__attribute__((noinline)) uint32_t predicate_inverse_add_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (c ? x : y) + (!c ? x : y);
}
__attribute__((noinline)) uint32_t reference_inverse_add_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return x + y;
}
__attribute__((noinline)) uint32_t predicate_constant_equal_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (c ? (uint32_t)7 : (uint32_t)4) == (c ? (uint32_t)7 : (uint32_t)5);
}
__attribute__((noinline)) uint32_t reference_constant_equal_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return c;
}
__attribute__((noinline)) uint32_t predicate_constant_unequal_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (c ? (uint32_t)7 : (uint32_t)4) != (c ? (uint32_t)7 : (uint32_t)5);
}
__attribute__((noinline)) uint32_t reference_constant_unequal_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return !c;
}
__attribute__((noinline)) uint32_t predicate_constant_less_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (c ? (uint32_t)7 : (uint32_t)4) < (c ? (uint32_t)6 : (uint32_t)5);
}
__attribute__((noinline)) uint32_t reference_constant_less_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return !c;
}
__attribute__((noinline)) uint32_t predicate_constant_greater_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (c ? (uint32_t)7 : (uint32_t)4) > (c ? (uint32_t)6 : (uint32_t)5);
}
__attribute__((noinline)) uint32_t reference_constant_greater_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return c;
}
__attribute__((noinline)) uint32_t predicate_high_mask_equal_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (x | (uint32_t)~(uint32_t)15) == x;
}
__attribute__((noinline)) uint32_t reference_high_mask_equal_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return x >= (uint32_t)~(uint32_t)15;
}
__attribute__((noinline)) uint32_t predicate_high_mask_unequal_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (x | (uint32_t)~(uint32_t)15) != x;
}
__attribute__((noinline)) uint32_t reference_high_mask_unequal_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return x < (uint32_t)~(uint32_t)15;
}
__attribute__((noinline)) uint32_t predicate_low_mask_equal_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (x & (uint32_t)15) == x;
}
__attribute__((noinline)) uint32_t reference_low_mask_equal_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return x <= (uint32_t)15;
}
__attribute__((noinline)) uint32_t predicate_low_mask_unequal_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (x & (uint32_t)15) != x;
}
__attribute__((noinline)) uint32_t reference_low_mask_unequal_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return x > (uint32_t)15;
}
__attribute__((noinline)) uint32_t predicate_bounded_complement_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (uint32_t)15 - (x & (uint32_t)15);
}
__attribute__((noinline)) uint32_t reference_bounded_complement_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (x & (uint32_t)15) ^ (uint32_t)15;
}
__attribute__((noinline)) uint32_t predicate_topbit_choice_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (x >> (32-1)) ? (x >> (32-1)) : (x >> (32-1)) + y;
}
__attribute__((noinline)) uint32_t reference_topbit_choice_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (x >> (32-1)) ? (uint32_t)1 : y;
}
__attribute__((noinline)) uint32_t predicate_range_arm_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return x < (uint32_t)16 ? x & (uint32_t)15 : y;
}
__attribute__((noinline)) uint32_t reference_range_arm_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return x < (uint32_t)16 ? x : y;
}
__attribute__((noinline)) uint32_t predicate_single_bit_fact_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (x & (uint32_t)1) ? ((x & (uint32_t)3) == 0) : y;
}
__attribute__((noinline)) uint32_t reference_single_bit_fact_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (x & (uint32_t)1) ? (uint32_t)0 : y;
}
__attribute__((noinline)) uint32_t predicate_inverse_nested_mul_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (c ? x : y) * (!c ? x : y);
}
__attribute__((noinline)) uint32_t reference_inverse_nested_mul_32(uint32_t x, uint32_t y, uint32_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return x * y;
}
__attribute__((noinline)) uint64_t predicate_nested_same_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return c ? (c ? x : y) : y;
}
__attribute__((noinline)) uint64_t reference_nested_same_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return c ? x : y;
}
__attribute__((noinline)) uint64_t predicate_nested_inverse_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return c ? (!c ? x : y) : y;
}
__attribute__((noinline)) uint64_t reference_nested_inverse_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return y;
}
__attribute__((noinline)) uint64_t predicate_nested_false_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return c ? x : (c ? y : x);
}
__attribute__((noinline)) uint64_t reference_nested_false_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return x;
}
__attribute__((noinline)) uint64_t predicate_nested_both_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return c ? (c ? x : y) : (c ? x : y);
}
__attribute__((noinline)) uint64_t reference_nested_both_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return c ? x : y;
}
__attribute__((noinline)) uint64_t predicate_swapped_add_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (c ? x : y) + (c ? y : x);
}
__attribute__((noinline)) uint64_t reference_swapped_add_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return x + y;
}
__attribute__((noinline)) uint64_t predicate_swapped_mul_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (c ? x : y) * (c ? y : x);
}
__attribute__((noinline)) uint64_t reference_swapped_mul_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return x * y;
}
__attribute__((noinline)) uint64_t predicate_swapped_and_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (c ? x : y) & (c ? y : x);
}
__attribute__((noinline)) uint64_t reference_swapped_and_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return x & y;
}
__attribute__((noinline)) uint64_t predicate_swapped_or_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (c ? x : y) | (c ? y : x);
}
__attribute__((noinline)) uint64_t reference_swapped_or_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return x | y;
}
__attribute__((noinline)) uint64_t predicate_swapped_xor_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (c ? x : y) ^ (c ? y : x);
}
__attribute__((noinline)) uint64_t reference_swapped_xor_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return x ^ y;
}
__attribute__((noinline)) uint64_t predicate_swapped_equal_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (c ? x : y) == (c ? y : x);
}
__attribute__((noinline)) uint64_t reference_swapped_equal_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return x == y;
}
__attribute__((noinline)) uint64_t predicate_swapped_unequal_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (c ? x : y) != (c ? y : x);
}
__attribute__((noinline)) uint64_t reference_swapped_unequal_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return x != y;
}
__attribute__((noinline)) uint64_t predicate_inverse_add_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (c ? x : y) + (!c ? x : y);
}
__attribute__((noinline)) uint64_t reference_inverse_add_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return x + y;
}
__attribute__((noinline)) uint64_t predicate_constant_equal_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (c ? (uint64_t)7 : (uint64_t)4) == (c ? (uint64_t)7 : (uint64_t)5);
}
__attribute__((noinline)) uint64_t reference_constant_equal_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return c;
}
__attribute__((noinline)) uint64_t predicate_constant_unequal_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (c ? (uint64_t)7 : (uint64_t)4) != (c ? (uint64_t)7 : (uint64_t)5);
}
__attribute__((noinline)) uint64_t reference_constant_unequal_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return !c;
}
__attribute__((noinline)) uint64_t predicate_constant_less_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (c ? (uint64_t)7 : (uint64_t)4) < (c ? (uint64_t)6 : (uint64_t)5);
}
__attribute__((noinline)) uint64_t reference_constant_less_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return !c;
}
__attribute__((noinline)) uint64_t predicate_constant_greater_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (c ? (uint64_t)7 : (uint64_t)4) > (c ? (uint64_t)6 : (uint64_t)5);
}
__attribute__((noinline)) uint64_t reference_constant_greater_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return c;
}
__attribute__((noinline)) uint64_t predicate_high_mask_equal_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (x | (uint64_t)~(uint64_t)15) == x;
}
__attribute__((noinline)) uint64_t reference_high_mask_equal_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return x >= (uint64_t)~(uint64_t)15;
}
__attribute__((noinline)) uint64_t predicate_high_mask_unequal_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (x | (uint64_t)~(uint64_t)15) != x;
}
__attribute__((noinline)) uint64_t reference_high_mask_unequal_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return x < (uint64_t)~(uint64_t)15;
}
__attribute__((noinline)) uint64_t predicate_low_mask_equal_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (x & (uint64_t)15) == x;
}
__attribute__((noinline)) uint64_t reference_low_mask_equal_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return x <= (uint64_t)15;
}
__attribute__((noinline)) uint64_t predicate_low_mask_unequal_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (x & (uint64_t)15) != x;
}
__attribute__((noinline)) uint64_t reference_low_mask_unequal_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return x > (uint64_t)15;
}
__attribute__((noinline)) uint64_t predicate_bounded_complement_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (uint64_t)15 - (x & (uint64_t)15);
}
__attribute__((noinline)) uint64_t reference_bounded_complement_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (x & (uint64_t)15) ^ (uint64_t)15;
}
__attribute__((noinline)) uint64_t predicate_topbit_choice_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (x >> (64-1)) ? (x >> (64-1)) : (x >> (64-1)) + y;
}
__attribute__((noinline)) uint64_t reference_topbit_choice_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (x >> (64-1)) ? (uint64_t)1 : y;
}
__attribute__((noinline)) uint64_t predicate_range_arm_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return x < (uint64_t)16 ? x & (uint64_t)15 : y;
}
__attribute__((noinline)) uint64_t reference_range_arm_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return x < (uint64_t)16 ? x : y;
}
__attribute__((noinline)) uint64_t predicate_single_bit_fact_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (x & (uint64_t)1) ? ((x & (uint64_t)3) == 0) : y;
}
__attribute__((noinline)) uint64_t reference_single_bit_fact_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (x & (uint64_t)1) ? (uint64_t)0 : y;
}
__attribute__((noinline)) uint64_t predicate_inverse_nested_mul_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return (c ? x : y) * (!c ? x : y);
}
__attribute__((noinline)) uint64_t reference_inverse_nested_mul_64(uint64_t x, uint64_t y, uint64_t z) {
    unsigned c = z != 0; (void)c; (void)x; (void)y;
    return x * y;
}
int main(void) {
    uint64_t seed = UINT64_C(0x243f6a8885a308d3), digest = 0;
    const uint64_t edges[] = {0,1,2,15,16,17,127,128,255,256,UINT32_MAX,UINT64_MAX,UINT64_C(0x8000000000000000)};
    for (unsigned round = 0; round < 1024; ++round) {
        seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17;
        uint64_t x = round < 13 ? edges[round] : seed;
        uint64_t y = edges[round % 13] ^ (seed >> 1), z = (round & 1) ? seed : 0;
        if (predicate_nested_same_32(x,y,z) != reference_nested_same_32(x,y,z)) { puts("mismatch: nested_same/32"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_nested_same_32(x,y,z);
        if (predicate_nested_inverse_32(x,y,z) != reference_nested_inverse_32(x,y,z)) { puts("mismatch: nested_inverse/32"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_nested_inverse_32(x,y,z);
        if (predicate_nested_false_32(x,y,z) != reference_nested_false_32(x,y,z)) { puts("mismatch: nested_false/32"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_nested_false_32(x,y,z);
        if (predicate_nested_both_32(x,y,z) != reference_nested_both_32(x,y,z)) { puts("mismatch: nested_both/32"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_nested_both_32(x,y,z);
        if (predicate_swapped_add_32(x,y,z) != reference_swapped_add_32(x,y,z)) { puts("mismatch: swapped_add/32"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_swapped_add_32(x,y,z);
        if (predicate_swapped_mul_32(x,y,z) != reference_swapped_mul_32(x,y,z)) { puts("mismatch: swapped_mul/32"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_swapped_mul_32(x,y,z);
        if (predicate_swapped_and_32(x,y,z) != reference_swapped_and_32(x,y,z)) { puts("mismatch: swapped_and/32"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_swapped_and_32(x,y,z);
        if (predicate_swapped_or_32(x,y,z) != reference_swapped_or_32(x,y,z)) { puts("mismatch: swapped_or/32"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_swapped_or_32(x,y,z);
        if (predicate_swapped_xor_32(x,y,z) != reference_swapped_xor_32(x,y,z)) { puts("mismatch: swapped_xor/32"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_swapped_xor_32(x,y,z);
        if (predicate_swapped_equal_32(x,y,z) != reference_swapped_equal_32(x,y,z)) { puts("mismatch: swapped_equal/32"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_swapped_equal_32(x,y,z);
        if (predicate_swapped_unequal_32(x,y,z) != reference_swapped_unequal_32(x,y,z)) { puts("mismatch: swapped_unequal/32"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_swapped_unequal_32(x,y,z);
        if (predicate_inverse_add_32(x,y,z) != reference_inverse_add_32(x,y,z)) { puts("mismatch: inverse_add/32"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_inverse_add_32(x,y,z);
        if (predicate_constant_equal_32(x,y,z) != reference_constant_equal_32(x,y,z)) { puts("mismatch: constant_equal/32"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_constant_equal_32(x,y,z);
        if (predicate_constant_unequal_32(x,y,z) != reference_constant_unequal_32(x,y,z)) { puts("mismatch: constant_unequal/32"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_constant_unequal_32(x,y,z);
        if (predicate_constant_less_32(x,y,z) != reference_constant_less_32(x,y,z)) { puts("mismatch: constant_less/32"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_constant_less_32(x,y,z);
        if (predicate_constant_greater_32(x,y,z) != reference_constant_greater_32(x,y,z)) { puts("mismatch: constant_greater/32"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_constant_greater_32(x,y,z);
        if (predicate_high_mask_equal_32(x,y,z) != reference_high_mask_equal_32(x,y,z)) { puts("mismatch: high_mask_equal/32"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_high_mask_equal_32(x,y,z);
        if (predicate_high_mask_unequal_32(x,y,z) != reference_high_mask_unequal_32(x,y,z)) { puts("mismatch: high_mask_unequal/32"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_high_mask_unequal_32(x,y,z);
        if (predicate_low_mask_equal_32(x,y,z) != reference_low_mask_equal_32(x,y,z)) { puts("mismatch: low_mask_equal/32"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_low_mask_equal_32(x,y,z);
        if (predicate_low_mask_unequal_32(x,y,z) != reference_low_mask_unequal_32(x,y,z)) { puts("mismatch: low_mask_unequal/32"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_low_mask_unequal_32(x,y,z);
        if (predicate_bounded_complement_32(x,y,z) != reference_bounded_complement_32(x,y,z)) { puts("mismatch: bounded_complement/32"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_bounded_complement_32(x,y,z);
        if (predicate_topbit_choice_32(x,y,z) != reference_topbit_choice_32(x,y,z)) { puts("mismatch: topbit_choice/32"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_topbit_choice_32(x,y,z);
        if (predicate_range_arm_32(x,y,z) != reference_range_arm_32(x,y,z)) { puts("mismatch: range_arm/32"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_range_arm_32(x,y,z);
        if (predicate_single_bit_fact_32(x,y,z) != reference_single_bit_fact_32(x,y,z)) { puts("mismatch: single_bit_fact/32"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_single_bit_fact_32(x,y,z);
        if (predicate_inverse_nested_mul_32(x,y,z) != reference_inverse_nested_mul_32(x,y,z)) { puts("mismatch: inverse_nested_mul/32"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_inverse_nested_mul_32(x,y,z);
        if (predicate_nested_same_64(x,y,z) != reference_nested_same_64(x,y,z)) { puts("mismatch: nested_same/64"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_nested_same_64(x,y,z);
        if (predicate_nested_inverse_64(x,y,z) != reference_nested_inverse_64(x,y,z)) { puts("mismatch: nested_inverse/64"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_nested_inverse_64(x,y,z);
        if (predicate_nested_false_64(x,y,z) != reference_nested_false_64(x,y,z)) { puts("mismatch: nested_false/64"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_nested_false_64(x,y,z);
        if (predicate_nested_both_64(x,y,z) != reference_nested_both_64(x,y,z)) { puts("mismatch: nested_both/64"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_nested_both_64(x,y,z);
        if (predicate_swapped_add_64(x,y,z) != reference_swapped_add_64(x,y,z)) { puts("mismatch: swapped_add/64"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_swapped_add_64(x,y,z);
        if (predicate_swapped_mul_64(x,y,z) != reference_swapped_mul_64(x,y,z)) { puts("mismatch: swapped_mul/64"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_swapped_mul_64(x,y,z);
        if (predicate_swapped_and_64(x,y,z) != reference_swapped_and_64(x,y,z)) { puts("mismatch: swapped_and/64"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_swapped_and_64(x,y,z);
        if (predicate_swapped_or_64(x,y,z) != reference_swapped_or_64(x,y,z)) { puts("mismatch: swapped_or/64"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_swapped_or_64(x,y,z);
        if (predicate_swapped_xor_64(x,y,z) != reference_swapped_xor_64(x,y,z)) { puts("mismatch: swapped_xor/64"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_swapped_xor_64(x,y,z);
        if (predicate_swapped_equal_64(x,y,z) != reference_swapped_equal_64(x,y,z)) { puts("mismatch: swapped_equal/64"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_swapped_equal_64(x,y,z);
        if (predicate_swapped_unequal_64(x,y,z) != reference_swapped_unequal_64(x,y,z)) { puts("mismatch: swapped_unequal/64"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_swapped_unequal_64(x,y,z);
        if (predicate_inverse_add_64(x,y,z) != reference_inverse_add_64(x,y,z)) { puts("mismatch: inverse_add/64"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_inverse_add_64(x,y,z);
        if (predicate_constant_equal_64(x,y,z) != reference_constant_equal_64(x,y,z)) { puts("mismatch: constant_equal/64"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_constant_equal_64(x,y,z);
        if (predicate_constant_unequal_64(x,y,z) != reference_constant_unequal_64(x,y,z)) { puts("mismatch: constant_unequal/64"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_constant_unequal_64(x,y,z);
        if (predicate_constant_less_64(x,y,z) != reference_constant_less_64(x,y,z)) { puts("mismatch: constant_less/64"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_constant_less_64(x,y,z);
        if (predicate_constant_greater_64(x,y,z) != reference_constant_greater_64(x,y,z)) { puts("mismatch: constant_greater/64"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_constant_greater_64(x,y,z);
        if (predicate_high_mask_equal_64(x,y,z) != reference_high_mask_equal_64(x,y,z)) { puts("mismatch: high_mask_equal/64"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_high_mask_equal_64(x,y,z);
        if (predicate_high_mask_unequal_64(x,y,z) != reference_high_mask_unequal_64(x,y,z)) { puts("mismatch: high_mask_unequal/64"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_high_mask_unequal_64(x,y,z);
        if (predicate_low_mask_equal_64(x,y,z) != reference_low_mask_equal_64(x,y,z)) { puts("mismatch: low_mask_equal/64"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_low_mask_equal_64(x,y,z);
        if (predicate_low_mask_unequal_64(x,y,z) != reference_low_mask_unequal_64(x,y,z)) { puts("mismatch: low_mask_unequal/64"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_low_mask_unequal_64(x,y,z);
        if (predicate_bounded_complement_64(x,y,z) != reference_bounded_complement_64(x,y,z)) { puts("mismatch: bounded_complement/64"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_bounded_complement_64(x,y,z);
        if (predicate_topbit_choice_64(x,y,z) != reference_topbit_choice_64(x,y,z)) { puts("mismatch: topbit_choice/64"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_topbit_choice_64(x,y,z);
        if (predicate_range_arm_64(x,y,z) != reference_range_arm_64(x,y,z)) { puts("mismatch: range_arm/64"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_range_arm_64(x,y,z);
        if (predicate_single_bit_fact_64(x,y,z) != reference_single_bit_fact_64(x,y,z)) { puts("mismatch: single_bit_fact/64"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_single_bit_fact_64(x,y,z);
        if (predicate_inverse_nested_mul_64(x,y,z) != reference_inverse_nested_mul_64(x,y,z)) { puts("mismatch: inverse_nested_mul/64"); return 1; }
        digest = digest * UINT64_C(0x100000001b3) ^ predicate_inverse_nested_mul_64(x,y,z);
    }
    printf("%llu\n", (unsigned long long)digest);
    return 0;
}
