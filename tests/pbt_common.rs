//! Property-based test suite (transplanted from the
//! thanhtoantnt/claudes-c-compiler pbt-tests campaign, adapted to the lccc
//! tree; see `src/lib.rs::pbt_internals` for why these live as integration
//! test binaries). Bug-witness properties that fail against current main
//! carry `#[ignore = "bug witness: ..."]` — run them with `--ignored` after
//! fixing the corresponding defect to re-lock the regression.
#[path = "pbt/common/const_arith_binop_prop_tests.rs"]
mod const_arith_binop_prop_tests;
#[path = "pbt/common/const_eval_pbt.rs"]
mod const_eval_pbt;
#[path = "pbt/common/long_double_pbt.rs"]
mod long_double_pbt;
