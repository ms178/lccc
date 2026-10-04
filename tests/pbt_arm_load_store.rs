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

#[path = "pbt/encoder/load_store_ldar_ldxr_class_pbt.rs"]
mod load_store_ldar_ldxr_class_pbt;

#[path = "pbt/encoder/load_store_ldaxr_stlxr_pbt.rs"]
mod load_store_ldaxr_stlxr_pbt;

#[path = "pbt/encoder/load_store_ldop_pbt.rs"]
mod load_store_ldop_pbt;

#[path = "pbt/encoder/load_store_ldp_stp_pbt.rs"]
mod load_store_ldp_stp_pbt;

#[path = "pbt/encoder/load_store_ldr_str_pbt.rs"]
mod load_store_ldr_str_pbt;

#[path = "pbt/encoder/load_store_ldst_stop_pbt.rs"]
mod load_store_ldst_stop_pbt;

#[path = "pbt/encoder/load_store_pbt.rs"]
mod load_store_pbt;

#[path = "pbt/encoder/load_store_prfm_prop_tests.rs"]
mod load_store_prfm_prop_tests;
