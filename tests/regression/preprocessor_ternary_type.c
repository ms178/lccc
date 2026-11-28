/* C11 6.10.1p4 and 6.5.15p5: #if uses intmax_t/uintmax_t, and
   ?: converts BOTH arms to their common type before selecting a value.
   Inspired by sametyilmaztemel/claudes-c-compiler 5d3f86f8de.
   Keep this separate from C-expression folding: preprocessing chooses code. */
#if (1 ? -1 : 0U) < 0
#error selected_signed_arm_must_convert_to_unsigned
#endif
#if (0 ? 0U : -1) < 0
#error unsigned_true_arm_also_affects_false_arm
#endif
#if (0 ? 1U : -1) / 2 != 9223372036854775807
#error unsigned_division_after_conditional
#endif
#if (1 ? -1 : 0xffffffffffffffff) < 0
#error unsuffixed_unsigned_literal_affects_result
#endif
#if (1 ? (1 ? -1 : 0) : 0U) < 0
#error nested_common_type_must_propagate
#endif
#if (1 ? -1 : (0 ? 0 : 1U)) < 0
#error inactive_nested_unsigned_arm_affects_result
#endif
#if (1U ? -1 : 0) >= 0
#error condition_type_must_not_affect_result_type
#endif
#if (0 ? 0 : -1) >= 0
#error two_signed_arms_stay_signed
#endif
#if (1 ? -1 : (0U == 0)) >= 0
#error comparison_result_is_signed
#endif
#if (1 ? -1 : !0U) >= 0
#error logical_result_is_signed
#endif
#if (1 ? -1 : (0U, 0)) >= 0
#error comma_result_has_right_operand_type
#endif
#if (1 ? -1 : (0, 0U)) < 0
#error comma_right_operand_can_make_result_unsigned
#endif
/* The middle operand is an expression, not a conditional-expression. */
#if (1 ? 0, 7 : 9) != 7
#error comma_in_middle_operand
#endif
#if (0 ? 0, 7U : -1) < 0
#error inactive_middle_comma_preserves_result_type
#endif
#if (1 ? 1U, -1 : 0) >= 0
#error middle_comma_discards_left_operand_type
#endif
/* Inactive arms still supply their type, but must not emit -Wundef diagnostics. */
#if (1 ? -1 : (UNDEFINED_TERNARY_ARM + 0U)) < 0
#error inactive_undefined_arm_has_unsigned_type
#endif
#if (0 ? UNDEFINED_TERNARY_ARM : 1) != 1
#error inactive_undefined_arm_value
#endif
#if 1 ? 0 ? 0 : 1 : 0
#define LCCC_PP_TERNARY_RIGHT_ASSOCIATIVE 1
#else
#error conditional_operator_associativity
#endif
int main(void) { return 0; }
