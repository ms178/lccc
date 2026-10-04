//! Property-based tests focused on **condition-code handling** across the
//! conditional-select / conditional-branch encoders defined in
//! `compare_branch.rs`.
//!
//! Targets:
//!   * `encode_cond_branch` (B.cond)
//!   * `encode_cinc`, `encode_cinv`, `encode_cneg` (-> CSINC/CSINV/CSNEG with Rm==Rn)
//!   * `encode_cset`, `encode_csetm` (-> CSINC/CSINV with Rm==Rn==XZR)
//!
//! Two intertwined behaviors are exercised:
//!
//! 1. **`encode_cond_branch` uses the condition VERBATIM** (it does NOT invert)
//!    and accepts all 16 condition codes — AL (14) and NV (15) included.
//!    Verified against `clang --target=aarch64`: `b.eq`, `b.al`, `b.nv` all
//!    assemble cleanly.
//!
//! 2. **The conditional-SELECT aliases INVERT the condition** (`inv_cond = cond ^ 1`)
//!    before placing it in the cond field [15:12]. Because inversion swaps
//!    AL <-> NV, and because the ARM ARM reserves cond==AL/NV for the
//!    conditional-select group, those aliases MUST reject AL/NV. Verified
//!    against clang: `cset/csetm/cinc/cinv/cneg ...,al|nv` fail with
//!    "condition codes AL and NV are invalid for this instruction". The base
//!    forms `csinc/csinv/csneg ...,al|nv` ARE accepted (clang exit 0), so an
//!    alias differs from its base form exactly on the AL/NV inputs.
//!
//! The SUT currently accepts AL/NV for the aliases. The negative-contract
//! property asserting the corrected contract is therefore marked `#[ignore]`
//! as a **bug witness** so the default `cargo test` run stays green. Run it
//! explicitly with `cargo test --lib compare_branch_cond_pbt -- --ignored`.

use lccc::pbt_internals::Operand;
use lccc::pbt_internals::*; // encode_cinc/cinv/cneg/cset/csetm/cond_branch + csinc/csinv/csneg + EncodeResult
use proptest::prelude::*;

// ---- Condition tables mirroring `encode_cond` (encoder/mod.rs:345) ----

/// All spellings with their 4-bit code. `cs`/`hs` and `cc`/`lo` are aliases.
const COND_TABLE: &[(&str, u32)] = &[
    ("eq", 0),
    ("ne", 1),
    ("cs", 2),
    ("hs", 2),
    ("cc", 3),
    ("lo", 3),
    ("mi", 4),
    ("pl", 5),
    ("vs", 6),
    ("vc", 7),
    ("hi", 8),
    ("ls", 9),
    ("ge", 10),
    ("lt", 11),
    ("gt", 12),
    ("le", 13),
    ("al", 14),
    ("nv", 15),
];

/// Exactly one name per distinct 4-bit value 0..=15 (no aliases). Indices 0..14
/// are the architecturally-valid condition values for the conditional-select
/// group; indices 14..16 are the reserved AL/NV values.
const COND_VALUES: &[(u32, &str)] = &[
    (0, "eq"),
    (1, "ne"),
    (2, "cs"),
    (3, "cc"),
    (4, "mi"),
    (5, "pl"),
    (6, "vs"),
    (7, "vc"),
    (8, "hi"),
    (9, "ls"),
    (10, "ge"),
    (11, "lt"),
    (12, "gt"),
    (13, "le"),
    (14, "al"),
    (15, "nv"),
];

/// Inversion (`v ^ 1`) swaps these complementary pairs. The first 8 pairs lie
/// entirely within the valid range 0..=13; the final pair is the reserved
/// AL <-> NV swap that the aliases must reject.
const COND_PAIRS: &[(&str, &str)] = &[
    ("eq", "ne"),
    ("cs", "cc"),
    ("hs", "lo"),
    ("mi", "pl"),
    ("vs", "vc"),
    ("hi", "ls"),
    ("ge", "lt"),
    ("gt", "le"),
    ("al", "nv"),
];

/// Canonical name for a 4-bit condition value (inverse of `encode_cond`).
fn name_of_value(v: u32) -> &'static str {
    COND_VALUES
        .iter()
        .find(|(cv, _)| *cv == v)
        .map(|(_, n)| *n)
        .unwrap_or("nv")
}

/// 4-bit code for a condition name (mirrors `encode_cond`).
fn value_of(name: &str) -> u32 {
    COND_TABLE
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, v)| *v)
        .unwrap_or(0)
}

fn o_reg(name: &str) -> Operand {
    Operand::Reg(name.to_string())
}

fn o_cond(name: &str) -> Operand {
    Operand::Cond(name.to_string())
}

/// Extract the encoded instruction word from either `EncodeResult` variant.
fn word_of(r: Result<EncodeResult, String>) -> u32 {
    match r {
        Ok(EncodeResult::Word(w)) => w,
        Ok(EncodeResult::WordWithReloc { word, .. }) => word,
        other => panic!("expected Word/WordWithReloc, got {:?}", other),
    }
}

/// Encode any of the 5 condition-**inverting** aliases.
///   func 0 -> cinc   (Rd, Rn, cond)
///   func 1 -> cinv   (Rd, Rn, cond)
///   func 2 -> cneg   (Rd, Rn, cond)
///   func 3 -> cset   (Rd, cond)        Rn is ignored
///   func 4 -> csetm  (Rd, cond)        Rn is ignored
/// `sf` (and thus the encoded register width) is derived ONLY from Rd, which
/// matches all five encoders (they read `is_64` from operand 0 alone).
fn alias_word(
    func: usize,
    rd: u32,
    is_64: bool,
    rn: u32,
    cond: &str,
) -> Result<EncodeResult, String> {
    let rd_name = if is_64 {
        format!("x{}", rd)
    } else {
        format!("w{}", rd)
    };
    // Encoders enforce sf consistency across operands; the alias forms must
    // pair registers of the same width.
    let rn_name = if is_64 {
        format!("x{}", rn)
    } else {
        format!("w{}", rn)
    };
    match func {
        0 => encode_cinc(&[o_reg(&rd_name), o_reg(&rn_name), o_cond(cond)]),
        1 => encode_cinv(&[o_reg(&rd_name), o_reg(&rn_name), o_cond(cond)]),
        2 => encode_cneg(&[o_reg(&rd_name), o_reg(&rn_name), o_cond(cond)]),
        3 => encode_cset(&[o_reg(&rd_name), o_cond(cond)]),
        4 => encode_csetm(&[o_reg(&rd_name), o_cond(cond)]),
        _ => unreachable!(),
    }
}

proptest! {
    // =============================================================
    // Group A — encode_cond_branch uses the condition VERBATIM and
    // accepts ALL 16 codes (AL=14, NV=15 included). Reference oracle,
    // verified against `clang --target=aarch64` (b.eq/b.al/b.nv assemble).
    // =============================================================
    #[test]
    fn prop_cond_branch_uses_condition_verbatim_all_sixteen(
        idx in 0usize..COND_TABLE.len(),
        sym in "[a-z][a-z0-9_]{0,7}",
    ) {
        let (cond_name, cond_val) = COND_TABLE[idx];
        let res = encode_cond_branch(cond_name, &[Operand::Symbol(sym)]);
        prop_assert!(res.is_ok(), "B.{} must encode (got {:?})", cond_name, res);
        let word = word_of(res);
        // cond occupies the low nibble [3:0], UN-inverted (== table value).
        prop_assert_eq!(word & 0xF, cond_val);
        // ... and is therefore NOT the inverted value (inversion is the aliases' job).
        prop_assert_ne!(word & 0xF, cond_val ^ 1);
    }

    // =============================================================
    // Group B — the 5 conditional-SELECT aliases INVERT the condition
    // (`inv_cond = cond ^ 1`). For every alias and every architecturally-valid
    // condition (values 0..=13, i.e. excluding the reserved AL/NV), the emitted
    // cond field [15:12] equals the input value XOR 1.
    // =============================================================
    #[test]
    fn prop_aliases_invert_condition(
        func in 0usize..5usize,
        rd in 0u32..=30u32,
        is_64 in any::<bool>(),
        rn in 0u32..=30u32,
        valid_idx in 0usize..14usize, // COND_VALUES[0..14] => values 0..=13
    ) {
        let (cond_val, cond_name) = COND_VALUES[valid_idx];
        let word = word_of(alias_word(func, rd, is_64, rn, cond_name));
        prop_assert_eq!((word >> 12) & 0xF, cond_val ^ 1);
    }

    // =============================================================
    // Group C — inversion is an INVOLUTION: inverting the input condition
    // inverts the emitted cond field. For every alias and every valid cond c:
    //   field(alias, invert(c)) == field(alias, c) ^ 1 == value(c).
    // =============================================================
    #[test]
    fn prop_inversion_is_involution(
        func in 0usize..5usize,
        rd in 0u32..=30u32,
        is_64 in any::<bool>(),
        rn in 0u32..=30u32,
        valid_idx in 0usize..14usize,
    ) {
        let (cond_val, cond_name) = COND_VALUES[valid_idx];
        // value(c) ^ 1 stays within 0..=13 for c in 0..=13, so this is a valid name.
        let inv_name = name_of_value(cond_val ^ 1);
        let field_c = (word_of(alias_word(func, rd, is_64, rn, cond_name)) >> 12) & 0xF;
        let field_inv = (word_of(alias_word(func, rd, is_64, rn, inv_name)) >> 12) & 0xF;
        prop_assert_eq!(field_inv, field_c ^ 1);
        // Double inversion returns to the original value.
        prop_assert_eq!(field_inv, cond_val);
    }

    // =============================================================
    // Group C' — inversion swaps exactly the complementary pairs. For every
    // pair (a, b) differing only in bit 0 and within the valid range:
    //   field(alias, a) == value(b)  and  field(alias, b) == value(a).
    // =============================================================
    #[test]
    fn prop_inversion_swaps_complementary_pairs(
        func in 0usize..5usize,
        rd in 0u32..=30u32,
        pair_idx in 0usize..8usize, // 8 pairs within 0..=13 (excludes al<->nv)
    ) {
        let (a, b) = COND_PAIRS[pair_idx];
        let (va, vb) = (value_of(a), value_of(b));
        let field_a = (word_of(alias_word(func, rd, true, rd, a)) >> 12) & 0xF;
        let field_b = (word_of(alias_word(func, rd, true, rd, b)) >> 12) & 0xF;
        prop_assert_eq!(field_a, vb);
        prop_assert_eq!(field_b, va);
    }

    // =============================================================
    // Group D — differential: each alias equals its base conditional-select
    // form with the INVERTED condition (and Rm==Rn, or Rm==Rn==XZR for
    // cset/csetm). Holds for every valid condition. This is the defining
    // semantics of the alias (CSINC/CSINV/CSNEG base form, never inverted).
    // =============================================================
    #[test]
    fn prop_alias_equals_base_form_inverted(
        func in 0usize..5usize,
        rd in 0u32..=30u32,
        is_64 in any::<bool>(),
        rn in 0u32..=30u32,
        valid_idx in 0usize..14usize,
    ) {
        let (cond_val, cond_name) = COND_VALUES[valid_idx];
        let inv_name = name_of_value(cond_val ^ 1);
        let rd_name = if is_64 { format!("x{}", rd) } else { format!("w{}", rd) };
        // sf consistency is enforced across operands; pair widths throughout.
        let rn_name = if is_64 { format!("x{}", rn) } else { format!("w{}", rn) };
        let zr_name = if is_64 { "xzr" } else { "wzr" };

        let alias_w = word_of(alias_word(func, rd, is_64, rn, cond_name));
        let base = match func {
            0 => encode_csinc(&[o_reg(&rd_name), o_reg(&rn_name), o_reg(&rn_name), o_cond(inv_name)]),
            1 => encode_csinv(&[o_reg(&rd_name), o_reg(&rn_name), o_reg(&rn_name), o_cond(inv_name)]),
            2 => encode_csneg(&[o_reg(&rd_name), o_reg(&rn_name), o_reg(&rn_name), o_cond(inv_name)]),
            3 => encode_csinc(&[o_reg(&rd_name), o_reg(zr_name), o_reg(zr_name), o_cond(inv_name)]),
            4 => encode_csinv(&[o_reg(&rd_name), o_reg(zr_name), o_reg(zr_name), o_cond(inv_name)]),
            _ => unreachable!(),
        };
        prop_assert_eq!(alias_w, word_of(base));
    }

    // =============================================================
    // Group E — BUG WITNESS (ignored). AL/NV are reserved for the
    // conditional-select group. clang rejects `cset/csetm/cinc/cinv/cneg
    // ...,al|nv` with "condition codes AL and NV are invalid for this
    // instruction". Because the aliases invert, an AL/NV input yields a cond
    // field of NV/AL — a reserved word. The SUT currently accepts these; this
    // property asserts the corrected Err contract.
    //
    // Run explicitly:
    //   cargo test --lib compare_branch_cond_pbt -- --ignored prop_aliases_reject_al_nv
    // (Filed for cset/csetm/cinc: gh issues 29/31/22; cinv/cneg share the
    //  same unvalidated-condition root cause.)
    // =============================================================
    #[test]
    #[ignore = "documented bug: aliases accept reserved AL/NV conditions (gh #22 #29 #31)"]
    fn prop_aliases_reject_al_nv(
        func in 0usize..5usize,
        rd in 0u32..=30u32,
        cond_idx in 14usize..16usize, // COND_VALUES[14]=al, [15]=nv
    ) {
        let (_v, cond_name) = COND_VALUES[cond_idx];
        let res = alias_word(func, rd, true, rd, cond_name);
        prop_assert!(
            res.is_err(),
            "alias {} must reject reserved condition '{}' (got {:?})",
            func, cond_name, res
        );
    }

    // =============================================================
    // Group F — contrast: B.cond is NOT subject to the AL/NV restriction.
    // encode_cond_branch accepts AL and NV (clang assembles b.al / b.nv) and
    // places them verbatim in the cond field. This locks the asymmetry: the
    // reserved-ness is specific to the conditional-SELECT group, not to
    // condition codes in general.
    // =============================================================
    #[test]
    fn prop_cond_branch_accepts_al_nv(
        cond_idx in 14usize..16usize,
        sym in "[a-z][a-z0-9_]{0,7}",
    ) {
        let (cond_val, cond_name) = COND_VALUES[cond_idx];
        let res = encode_cond_branch(cond_name, &[Operand::Symbol(sym)]);
        prop_assert!(res.is_ok(), "B.{} must be accepted (got {:?})", cond_name, res);
        prop_assert_eq!(word_of(res) & 0xF, cond_val);
    }
}
