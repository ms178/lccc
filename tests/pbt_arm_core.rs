//! Property-based test suite (transplanted from the
//! thanhtoantnt/claudes-c-compiler pbt-tests campaign, adapted to the lccc
//! tree; see `src/lib.rs::pbt_internals` for why these live as integration
//! test binaries). Bug-witness properties that fail against current main
//! carry `#[ignore = "bug witness: ..."]` — run them with `--ignored` after
//! fixing the corresponding defect to re-lock the regression.

#[path = "pbt/encoder/bitfield_cls_clz_crc_rev_pbt.rs"]
mod bitfield_cls_clz_crc_rev_pbt;

#[path = "pbt/encoder/compare_branch_cond_pbt.rs"]
mod compare_branch_cond_pbt;

#[path = "pbt/encoder/compare_branch_condselect_regclass_pbt.rs"]
mod compare_branch_condselect_regclass_pbt;

#[path = "pbt/encoder/compare_branch_regclass_pbt.rs"]
mod compare_branch_regclass_pbt;

#[path = "pbt/encoder/data_processing_adc_sbc_neg_negs_pbt.rs"]
mod data_processing_adc_sbc_neg_negs_pbt;

#[path = "pbt/encoder/data_processing_addsub_div_bitmask_pbt.rs"]
mod data_processing_addsub_div_bitmask_pbt;

#[path = "pbt/encoder/data_processing_div_fpsimd_sp_pbt.rs"]
mod data_processing_div_fpsimd_sp_pbt;

#[path = "pbt/encoder/data_processing_extend_pbt.rs"]
mod data_processing_extend_pbt;

#[path = "pbt/encoder/data_processing_logical_not_pbt.rs"]
mod data_processing_logical_not_pbt;

#[path = "pbt/encoder/data_processing_mneg_smaddl_smulh_umulh_pbt.rs"]
mod data_processing_mneg_smaddl_smulh_umulh_pbt;

#[path = "pbt/encoder/data_processing_mov_dispatch_pbt.rs"]
mod data_processing_mov_dispatch_pbt;

#[path = "pbt/encoder/data_processing_mov_wide_imm_pbt.rs"]
mod data_processing_mov_wide_imm_pbt;

#[path = "pbt/encoder/data_processing_mul_madd_msub_umaddl_umull_pbt.rs"]
mod data_processing_mul_madd_msub_umaddl_umull_pbt;

#[path = "pbt/encoder/data_processing_shift_pbt.rs"]
mod data_processing_shift_pbt;

#[path = "pbt/encoder/data_processing_smulh_umulh_uxtb_uxth_pbt.rs"]
mod data_processing_smulh_umulh_uxtb_uxth_pbt;

#[path = "pbt/encoder/data_processing_smull_smaddl_smulh_mneg_fpsimd_sp_pbt.rs"]
mod data_processing_smull_smaddl_smulh_mneg_fpsimd_sp_pbt;

#[path = "pbt/encoder/data_processing_sxtb_sxth_sxtw_fpsimd_pbt.rs"]
mod data_processing_sxtb_sxth_sxtw_fpsimd_pbt;

#[path = "pbt/encoder/div_tst_cbz_regclass_pbt.rs"]
mod div_tst_cbz_regclass_pbt;

#[path = "pbt/encoder/encode_logical_pbt.rs"]
mod encode_logical_pbt;
