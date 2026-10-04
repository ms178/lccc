//! Property-based test suite (transplanted from the
//! thanhtoantnt/claudes-c-compiler pbt-tests campaign, adapted to the lccc
//! tree; see `src/lib.rs::pbt_internals` for why these live as integration
//! test binaries). Bug-witness properties that fail against current main
//! carry `#[ignore = "bug witness: ..."]` — run them with `--ignored` after
//! fixing the corresponding defect to re-lock the regression.

#[path = "pbt/encoder/neon_across_long_pbt.rs"]
mod neon_across_long_pbt;

#[path = "pbt/encoder/neon_across_pbt.rs"]
mod neon_across_pbt;

#[path = "pbt/encoder/neon_add_sub_pbt.rs"]
mod neon_add_sub_pbt;

#[path = "pbt/encoder/neon_addv_pbt.rs"]
mod neon_addv_pbt;

#[path = "pbt/encoder/neon_bic_pbt.rs"]
mod neon_bic_pbt;

#[path = "pbt/encoder/neon_bitwise_insert_pbt.rs"]
mod neon_bitwise_insert_pbt;

#[path = "pbt/encoder/neon_bsl_pbt.rs"]
mod neon_bsl_pbt;

#[path = "pbt/encoder/neon_cmp_zero_pbt.rs"]
mod neon_cmp_zero_pbt;

#[path = "pbt/encoder/neon_cnt_pbt.rs"]
mod neon_cnt_pbt;

#[path = "pbt/encoder/neon_dup_pbt.rs"]
mod neon_dup_pbt;

#[path = "pbt/encoder/neon_elem_long_pbt.rs"]
mod neon_elem_long_pbt;

#[path = "pbt/encoder/neon_elem_pbt.rs"]
mod neon_elem_pbt;

#[path = "pbt/encoder/neon_eor3_pbt.rs"]
mod neon_eor3_pbt;

#[path = "pbt/encoder/neon_eor_rev_pbt.rs"]
mod neon_eor_rev_pbt;

#[path = "pbt/encoder/neon_ext_pbt.rs"]
mod neon_ext_pbt;

#[path = "pbt/encoder/neon_faddp_pbt.rs"]
mod neon_faddp_pbt;

#[path = "pbt/encoder/neon_fcvtl_pbt.rs"]
mod neon_fcvtl_pbt;

#[path = "pbt/encoder/neon_fcvtn_pbt.rs"]
mod neon_fcvtn_pbt;

#[path = "pbt/encoder/neon_float_cmp_zero_pbt.rs"]
mod neon_float_cmp_zero_pbt;

#[path = "pbt/encoder/neon_float_elem_pbt.rs"]
mod neon_float_elem_pbt;

#[path = "pbt/encoder/neon_float_three_same_pbt.rs"]
mod neon_float_three_same_pbt;

#[path = "pbt/encoder/neon_float_two_misc_pbt.rs"]
mod neon_float_two_misc_pbt;
