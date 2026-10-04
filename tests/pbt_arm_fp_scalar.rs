//! Property-based test suite (transplanted from the
//! thanhtoantnt/claudes-c-compiler pbt-tests campaign, adapted to the lccc
//! tree; see `src/lib.rs::pbt_internals` for why these live as integration
//! test binaries). Bug-witness properties that fail against current main
//! carry `#[ignore = "bug witness: ..."]` — run them with `--ignored` after
//! fixing the corresponding defect to re-lock the regression.

#[path = "pbt/encoder/fp_scalar_bank_precision_pbt.rs"]
mod fp_scalar_bank_precision_pbt;

#[path = "pbt/encoder/fp_scalar_fcvt_rounding_pbt.rs"]
mod fp_scalar_fcvt_rounding_pbt;

#[path = "pbt/encoder/fp_scalar_fmadd_fmsub_pbt.rs"]
mod fp_scalar_fmadd_fmsub_pbt;

#[path = "pbt/encoder/fp_scalar_fmov_pbt.rs"]
mod fp_scalar_fmov_pbt;

#[path = "pbt/encoder/fp_scalar_fneg_fabs_fsqrt_fcvt_pbt.rs"]
mod fp_scalar_fneg_fabs_fsqrt_fcvt_pbt;

#[path = "pbt/encoder/fp_scalar_int_to_float_pbt.rs"]
mod fp_scalar_int_to_float_pbt;
