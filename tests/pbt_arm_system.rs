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

#[path = "pbt/encoder/system_barriers_hints_pbt.rs"]
mod system_barriers_hints_pbt;

#[path = "pbt/encoder/system_msr_mrs_pbt.rs"]
mod system_msr_mrs_pbt;

#[path = "pbt/encoder/system_sys_at_dc_ic_tlbi_pbt.rs"]
mod system_sys_at_dc_ic_tlbi_pbt;
