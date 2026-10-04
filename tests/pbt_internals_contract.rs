//! Contract test for the `lccc::pbt_internals` re-export surface.
//!
//! The integration-test crates reach the encoder through this module, so it
//! is effectively a public API for the test suite even though it is
//! `#[doc(hidden)]` and documented as unstable. That contract deserves a
//! test of its own: if an item is renamed, moved or narrowed without the
//! test suites being updated, the failure should say "the pbt_internals
//! contract changed", not surface as a scattered compile error in nine
//! integration binaries.
//!
//! The maintenance rule this pins: any change to the re-export surface is
//! deliberate and lands together with the suites that consume it (the
//! alternative -- a per-symbol allow-list -- was rejected because the
//! composition `pub use ...::*` plus a growing test campaign would churn
//! the list on every new suite for zero additional safety; see the doc
//! comment on `src/lib.rs::pbt_internals`).

use lccc::pbt_internals::*;

/// The core operand/entry-point surface the encoder suites are compiled
/// against. Every symbol here is referenced by name from at least one PBT
/// suite; a rename that keeps the suites compiling but changes semantics
/// (e.g. a different `EncodeResult` shape) is still caught by the suites
/// themselves -- this test catches removal/moves.
#[test]
fn pbt_internals_contract_surface() {
    // Operand constructors and the two result shapes.
    let _ = Operand::Reg("x0".to_string());
    let _ = Operand::Imm(1);
    let _ = EncodeResult::Word(0);

    // One representative entry point from every encoder family the suites
    // exercise; the full list is the `pub use` itself. The type aliases are
    // the contract: each assignment only compiles while the item keeps that
    // exact signature.
    type R = Result<EncodeResult, String>;
    let _: fn(&[Operand], bool, bool) -> R = encode_add_sub;
    let _: fn(&[Operand], u32) -> R = encode_logical;
    let _: fn(&[Operand]) -> R = encode_mov;
    let _: fn(&[Operand]) -> R = encode_mrs;
    let _: fn(&[Operand], bool, Option<u32>) -> R = encode_ldxr_stxr;
    let _: fn(&str, &[Operand]) -> R = encode_cas;
    let _: fn(&str, &[Operand]) -> R = encode_ldop;
    let _: fn(&[Operand], u32, u32) -> R = encode_neon_float_elem;
    let _: fn(&[Operand], u32) -> R = encode_neon_float_elem_scalar;
    let _: fn(&[Operand]) -> R = encode_prfm;

    // The shared operand-reading helpers.
    type GprPair = Result<(u32, bool), String>;
    let _: fn(&[Operand], usize) -> GprPair = get_gpr_strict;
    let _: fn(&[Operand], usize) -> Result<u32, String> = get_gpr_strict_x;
    let _: fn(&[Operand], usize) -> Result<u32, String> = get_gpr_strict_w;
    let _: fn(&str) -> bool = is_gp_reg;
    type GpTriple = Result<(u32, bool, u32), String>;
    let _: fn(&[Operand], &str) -> GpTriple = get_gp_reg_pair;
    let _: fn(&str) -> Option<u32> = parse_reg_num;
}

/// The class predicate and the parser must agree on what a register
/// spelling means (the `is_gp_reg`/`parse_reg_num` drift was how
/// `fmov d0,lr` was rejected while `mov x0,lr` assembled -- pinned here so
/// the contract test also covers the semantic coupling).
#[test]
fn pbt_internals_class_predicate_matches_parser() {
    for name in ["x0", "x30", "w0", "w30", "lr", "sp", "wsp", "xzr", "wzr"] {
        assert_eq!(
            is_gp_reg(name),
            parse_reg_num(name).is_some(),
            "`{name}` must classify the same way both helpers answer it"
        );
    }
    for name in ["d0", "s1", "h2", "q3", "v4", "b5", "x31", "x007", ""] {
        assert!(
            !is_gp_reg(name),
            "`{name}` is not a general-purpose register"
        );
    }
}
