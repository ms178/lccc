//! Property-based test suite (transplanted from the
//! thanhtoantnt/claudes-c-compiler pbt-tests campaign, adapted to the lccc
//! tree; see `src/lib.rs::pbt_internals` for why these live as integration
//! test binaries). Bug-witness properties that fail against current main
//! carry `#[ignore = "bug witness: ..."]` — run them with `--ignored` after
//! fixing the corresponding defect to re-lock the regression.
//!
//! Licence: SPDX-License-Identifier: CC0-1.0 OR MIT OR Apache-2.0 -- lccc's
//! licence set (see the LICENSE* files at the repo root). The transplant is
//! intra-lineage (lccc is itself a fork of thanhtoantnt/claudes-c-compiler),
//! so provenance doubles as a grant.

#[path = "pbt/encoder/neon_scalar_three_same_pbt.rs"]
mod neon_scalar_three_same_pbt;

#[path = "pbt/encoder/neon_scalar_two_misc_pbt.rs"]
mod neon_scalar_two_misc_pbt;

#[path = "pbt/encoder/neon_shift_imm_pbt.rs"]
mod neon_shift_imm_pbt;

#[path = "pbt/encoder/neon_shift_left_imm_pbt.rs"]
mod neon_shift_left_imm_pbt;

#[path = "pbt/encoder/neon_shll_pbt.rs"]
mod neon_shll_pbt;

#[path = "pbt/encoder/neon_shrn_pbt.rs"]
mod neon_shrn_pbt;

#[path = "pbt/encoder/neon_sli_pbt.rs"]
mod neon_sli_pbt;

#[path = "pbt/encoder/neon_sqshrun_pbt.rs"]
mod neon_sqshrun_pbt;

#[path = "pbt/encoder/neon_sri_pbt.rs"]
mod neon_sri_pbt;

#[path = "pbt/encoder/neon_sshr_pbt.rs"]
mod neon_sshr_pbt;

#[path = "pbt/encoder/neon_tbl_pbt.rs"]
mod neon_tbl_pbt;

#[path = "pbt/encoder/neon_tbx_pbt.rs"]
mod neon_tbx_pbt;

#[path = "pbt/encoder/neon_three_diff_narrow_pbt.rs"]
mod neon_three_diff_narrow_pbt;

#[path = "pbt/encoder/neon_three_diff_pbt.rs"]
mod neon_three_diff_pbt;

#[path = "pbt/encoder/neon_two_misc_narrow_pbt.rs"]
mod neon_two_misc_narrow_pbt;

#[path = "pbt/encoder/neon_two_misc_pbt.rs"]
mod neon_two_misc_pbt;

#[path = "pbt/encoder/neon_umov_pbt.rs"]
mod neon_umov_pbt;

#[path = "pbt/encoder/neon_ushr_pbt.rs"]
mod neon_ushr_pbt;

#[path = "pbt/encoder/neon_xtl_pbt.rs"]
mod neon_xtl_pbt;

#[path = "pbt/encoder/neon_zip_uzp_pbt.rs"]
mod neon_zip_uzp_pbt;
