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

#[path = "pbt/encoder/neon_ins_pbt.rs"]
mod neon_ins_pbt;

#[path = "pbt/encoder/neon_ins_shl_addv_pbt.rs"]
mod neon_ins_shl_addv_pbt;

#[path = "pbt/encoder/neon_ld1r_pbt.rs"]
mod neon_ld1r_pbt;

#[path = "pbt/encoder/neon_ld_st_dispatch_pbt.rs"]
mod neon_ld_st_dispatch_pbt;

#[path = "pbt/encoder/neon_ld_st_multi_pbt.rs"]
mod neon_ld_st_multi_pbt;

#[path = "pbt/encoder/neon_ld_st_single_pbt.rs"]
mod neon_ld_st_single_pbt;

#[path = "pbt/encoder/neon_ldnr_pbt.rs"]
mod neon_ldnr_pbt;

#[path = "pbt/encoder/neon_logical_pbt.rs"]
mod neon_logical_pbt;

#[path = "pbt/encoder/neon_mla_mls_fields_pbt.rs"]
mod neon_mla_mls_fields_pbt;

#[path = "pbt/encoder/neon_mla_pbt.rs"]
mod neon_mla_pbt;

#[path = "pbt/encoder/neon_mls_pbt.rs"]
mod neon_mls_pbt;

#[path = "pbt/encoder/neon_movi_16bit_split_pbt.rs"]
mod neon_movi_16bit_split_pbt;

#[path = "pbt/encoder/neon_movi_pbt.rs"]
mod neon_movi_pbt;

#[path = "pbt/encoder/neon_mul_pbt.rs"]
mod neon_mul_pbt;

#[path = "pbt/encoder/neon_mvni_pbt.rs"]
mod neon_mvni_pbt;

#[path = "pbt/encoder/neon_not_pbt.rs"]
mod neon_not_pbt;

#[path = "pbt/encoder/neon_pmul_pbt.rs"]
mod neon_pmul_pbt;

#[path = "pbt/encoder/neon_pmull_pbt.rs"]
mod neon_pmull_pbt;

#[path = "pbt/encoder/neon_qshrn_pbt.rs"]
mod neon_qshrn_pbt;

#[path = "pbt/encoder/neon_rbit_pbt.rs"]
mod neon_rbit_pbt;

#[path = "pbt/encoder/neon_rev64_pbt.rs"]
mod neon_rev64_pbt;

#[path = "pbt/encoder/neon_scalar_qshrn_pbt.rs"]
mod neon_scalar_qshrn_pbt;
