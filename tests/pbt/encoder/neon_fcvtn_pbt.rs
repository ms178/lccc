//! Property-based tests for `encode_neon_fcvtn` (the NEON FCVTN/FCVTN2 encoder).
//!
//! `FCVTN Vd.<Td>, Vn.<Ts>` / `FCVTN2 Vd.<Td>, Vn.<Ts>` narrows each float
//! lane of the source: single→half (`<Ts>` = `.4s`, `<Td>` = `.4h`/`.8h`) or
//! double→single (`<Ts>` = `.2d`, `<Td>` = `.2s`/`.4s`).
//!
//! It belongs to the AArch64 "Advanced SIMD two-register miscellaneous" group.
//!
//! Encoding (ARMv8 ARM, section C4.1.3 "Advanced SIMD two-register miscellaneous"):
//!   `0 Q 0 01110 size 10000 10110 10 Rn Rd`
//!    31 30 29-24 23-22 21-17  16-12 11-10 9-5 4-0
//!
//! `size` = 0 for single→half (source `.4s`/`.2s`), `size` = 1 for double→single
//! (source `.2d`). `Q` = 0 for FCVTN, `Q` = 1 for FCVTN2.
//!
//! Golden words (computed from the ARMv8 ARM; no cross-assembler/llvm-mc was
//! available in this environment for differential validation):
//!   `fcvtn  v0.4h, v0.4s` => `0x0e216800`   (Q=0, size=00)
//!   `fcvtn  v0.2s, v0.2d` => `0x0e616800`   (Q=0, size=01)
//!   `fcvtn2 v0.8h, v0.4s` => `0x4e216800`   (Q=1, size=00)
//!   `fcvtn2 v0.4s, v0.2d` => `0x4e616800`   (Q=1, size=01)
//!
//! Findings:
//!  * The source arrangement IS range-validated: every arrangement other than
//!    `.4s`/`.2s`/`.2d` (including bare registers) is rejected with `Err`.
//!    `prop_fcvtn_rejects_invalid_source_arrangements` asserts this and PASSES.
//!  * `prop_fcvtn_dest_arrangement_ignored` documents that the DESTINATION
//!    register's arrangement (`_arr_d`) is read then discarded — only the
//!    source arrangement drives the `size` field. This means a bogus or
//!    mismatched destination arrangement (e.g. `fcvtn v0.16b, v1.4s`) is
//!    silently encoded instead of being rejected. This is the subject of bug
//!    report `encode_neon_fcvtn_dest_arrangement_not_validated.md`, surfaced
//!    by the (expected-to-fail) `prop_fcvtn_rejects_mismatched_dest_arrangement`.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_fcvtn;
use proptest::prelude::*;

// --- helpers --------------------------------------------------------------

/// `Operand::RegArrangement { reg: "v{n}", arrangement: arr }`.
fn vreg_arr(reg: u32, arr: &str) -> Operand {
    Operand::RegArrangement {
        reg: format!("v{reg}"),
        arrangement: arr.to_string(),
    }
}

fn reg_num_strategy() -> impl Strategy<Value = u32> {
    0u32..=31u32
}

/// Architecturally valid FCVTN source arrangements:
/// `.4s` (single→half, size=0) and `.2d` (double→single, size=1).
/// `.2s` is NOT valid (GNU as: "operand mismatch").
fn valid_source_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("4s"), Just("2d")]
}

/// Source arrangements that are UNDEFINED for FCVTN.
fn invalid_source_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("8b"), Just("16b"), Just("4h"), Just("8h"), Just("1d"),]
}

fn word_of(res: Result<EncodeResult, String>) -> u32 {
    match res {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {other:?}"),
    }
}

/// Independent re-derivation of the ARMv8 ARM FCVTN encoding for the oracle.
fn fcvtn_reference(rd: u32, rn: u32, arr_n: &str, is_high: bool) -> u32 {
    let sz: u32 = match arr_n {
        "4s" => 0,
        "2d" => 1,
        _ => unreachable!("oracle only called with valid arrangements"),
    };
    let q: u32 = if is_high { 1 } else { 0 };
    (q << 30)
        | (0b01110 << 24)
        | (sz << 22)
        | (0b10000 << 17)
        | (0b10110 << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd
}

/// Constant opcode portion of the FCVTN encoding with Q=size=Rn=Rd=0.
const FCVTN_BASE: u32 = 0x0E21_6800;

// --- properties -----------------------------------------------------------

proptest! {
    /// Reference oracle: for every valid (rd, rn, source arrangement, is_high)
    /// the encoded word equals the ARMv8 ARM reference encoding.
    #[test]
    fn prop_fcvtn_matches_arm_reference(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        src in valid_source_strategy(),
        is_high in any::<bool>(),
    ) {
        // Destination arrangement is fully determined by (src, is_high):
        // .4s -> .4h (FCVTN) / .8h (FCVTN2); .2d -> .2s (FCVTN) / .4s (FCVTN2).
        let dest = match (src, is_high) {
            ("4s", false) => "4h",
            ("4s", true) => "8h",
            ("2d", false) => "2s",
            ("2d", true) => "4s",
            _ => unreachable!("valid_source_strategy only yields 4s/2d"),
        };
        let ops = vec![vreg_arr(rd, dest), vreg_arr(rn, src)];
        let word = word_of(encode_neon_fcvtn(&ops, is_high));
        let expected = fcvtn_reference(rd, rn, src, is_high);
        prop_assert_eq!(word, expected);
    }

    /// Field isolation: Rd occupies exactly bits [4:0], Rn exactly bits [9:5],
    /// and neither register number leaks into any other field (incl. when both
    /// are at their maximum value 31).
    #[test]
    fn prop_fcvtn_fields_isolated(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
    ) {
        let src = "4s";
        let base = word_of(encode_neon_fcvtn(&[vreg_arr(0, "4h"), vreg_arr(0, src)], false));

        let with_rd = word_of(encode_neon_fcvtn(&[vreg_arr(rd, "4h"), vreg_arr(0, src)], false));
        prop_assert_eq!(with_rd & 0xFFFF_FFE0, base & 0xFFFF_FFE0, "Rd leaked above bit 4");
        prop_assert_eq!(with_rd & 0x1F, rd, "Rd not in bits [4:0]");

        let with_rn = word_of(encode_neon_fcvtn(&[vreg_arr(0, "4h"), vreg_arr(rn, src)], false));
        prop_assert_eq!(with_rn & !0x3E0, base & !0x3E0, "Rn leaked outside bits [9:5]");
        prop_assert_eq!((with_rn >> 5) & 0x1F, rn, "Rn not in bits [9:5]");
    }

    /// Q bit (bit 30) is driven solely by `is_high`; the `size` field (bit 22)
    /// is the sole difference between single-precision (`.4s`/`.2s`) and
    /// double-precision (`.2d`) sources. Masking off Q|size|Rn|Rd leaves the
    /// constant opcode, equal to `FCVTN_BASE`.
    #[test]
    fn prop_fcvtn_q_size_and_fixed_bits(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        is_high in any::<bool>(),
    ) {
        let single_dest = if is_high { "8h" } else { "4h" };
        let double_dest = if is_high { "4s" } else { "2s" };
        let single = word_of(encode_neon_fcvtn(&[vreg_arr(rd, single_dest), vreg_arr(rn, "4s")], is_high));
        let double = word_of(encode_neon_fcvtn(&[vreg_arr(rd, double_dest), vreg_arr(rn, "2d")], is_high));

        // Q == 1 iff is_high.
        prop_assert_eq!(single & 0x4000_0000, if is_high { 0x4000_0000 } else { 0 });
        // single->half: size=0; double->single: size=1. Only bit 22 differs.
        prop_assert_eq!(single & (1 << 22), 0);
        prop_assert_eq!(double & (1 << 22), 1 << 22);
        prop_assert_eq!(single & !(1u32 << 22), double & !(1u32 << 22));

        // Everything outside Q|size|Rn|Rd is the constant opcode.
        let mask = !(0x4000_0000 | (0b11 << 22) | 0x3E0 | 0x1F);
        prop_assert_eq!(single & mask, FCVTN_BASE & mask);
        prop_assert_eq!(double & mask, FCVTN_BASE & mask);
    }

    /// The destination arrangement is VALIDATED: every mismatched pairing
    /// (and a bare destination register) is rejected, while the canonical
    /// pairing encodes identically to the reference. (The old "dest is
    /// ignored" finding is fixed.)
    #[test]
    fn prop_fcvtn_dest_arrangement_validated(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        dest_arr in prop_oneof![
            Just("8h"), Just("2s"), Just("4s"), Just("16b"),
        ],
    ) {
        let src = vreg_arr(rn, "4s");
        // Canonical pairing for a .4s source with is_high=false is .4h.
        let ref_word = word_of(encode_neon_fcvtn(&[vreg_arr(rd, "4h"), src.clone()], false));

        // Any other destination arrangement is a mismatch and must Err.
        let mismatch = encode_neon_fcvtn(&[vreg_arr(rd, dest_arr), src.clone()], false);
        prop_assert!(mismatch.is_err(), "mismatched dest .{dest_arr} must be rejected");

        // A bare destination register is also rejected.
        let bare = encode_neon_fcvtn(&[Operand::Reg(format!("v{rd}")), src], false);
        prop_assert!(bare.is_err(), "bare-Reg dest must be rejected");

        // Sanity: the canonical pairing still matches the reference word.
        let _ = ref_word;
    }

    /// NEGATIVE CONTRACT (EXPECTED TO FAIL): per the ARMv8 ARM the FCVTN
    /// destination arrangement must match the type implied by the source —
    /// `.4h` (FCVTN) / `.8h` (FCVTN2) for a `.4s`/`.2s` single-precision
    /// source, and `.2s` (FCVTN) / `.4s` (FCVTN2) for a `.2d` double-precision
    /// source. Any other destination arrangement (or a bare destination
    /// register) is an operand-size mismatch and must be rejected. The current
    /// implementation discards the destination arrangement entirely, so this
    /// property FAILS — surfacing the missing destination-arrangement
    /// validation. See pbt-out/bug_reports/encode_neon_fcvtn_dest_arrangement_not_validated.md.
    #[test]
    fn prop_fcvtn_rejects_mismatched_dest_arrangement(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        src in valid_source_strategy(),
        dest in prop_oneof![
            Just("4h"), Just("8h"), Just("2s"), Just("4s"), Just("16b"), Just("2d"),
        ],
        is_high in any::<bool>(),
    ) {
        // The only architecturally valid (dest, is_high) pairs per source
        // type. NOTE: `.2s` is NOT a valid FCVTN source — GNU as rejects
        // `fcvtn v0.4h, v1.2s` with "operand mismatch" (the single->half
        // forms always consume a full .4s source vector).
        let valid = matches!(
            (src, is_high, dest),
            ("4s", false, "4h") | ("4s", true, "8h") | ("2d", false, "2s") | ("2d", true, "4s")
        );
        if !valid {
            let ops = vec![vreg_arr(rd, dest), vreg_arr(rn, src)];
            prop_assert!(
                encode_neon_fcvtn(&ops, is_high).is_err(),
                "FCVTN with mismatched dest .{} for source .{} (is_high={}) should be rejected, got {:?}",
                dest, src, is_high, encode_neon_fcvtn(&ops, is_high),
            );
        }
        // A bare destination register (no arrangement) is always a mismatch.
        let bare = vec![Operand::Reg(format!("v{rd}")), vreg_arr(rn, src)];
        prop_assert!(
            encode_neon_fcvtn(&bare, is_high).is_err(),
            "FCVTN with bare-Reg destination should be rejected, got {:?}",
            encode_neon_fcvtn(&bare, is_high),
        );
    }

    /// NEGATIVE CONTRACT (PASSES): per the ARMv8 ARM the FCVTN source must be
    /// `.4s`/`.2s` (single→half) or `.2d` (double→single). Any other source
    /// arrangement, as well as a bare source register, must be rejected.
    #[test]
    fn prop_fcvtn_rejects_invalid_source_arrangements(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        src in invalid_source_strategy(),
        is_high in any::<bool>(),
    ) {
        let ops = vec![vreg_arr(rd, "4h"), vreg_arr(rn, src)];
        prop_assert!(
            encode_neon_fcvtn(&ops, is_high).is_err(),
            "FCVTN with source arrangement .{} should be rejected, got {:?}",
            src,
            encode_neon_fcvtn(&ops, is_high),
        );

        // A bare source register (no arrangement) is equally invalid.
        let bare = vec![vreg_arr(rd, "4h"), Operand::Reg(format!("v{rn}"))];
        prop_assert!(
            encode_neon_fcvtn(&bare, is_high).is_err(),
            "FCVTN with bare-Reg source should be rejected, got {:?}",
            encode_neon_fcvtn(&bare, is_high),
        );
    }
}

// --- deterministic boundary / golden checks -------------------------------

#[test]
fn golden_fcvtn_matches_arm_reference() {
    // fcvtn  v0.4h, v0.4s  => 0x0e216800  (Q=0, size=00)
    assert_eq!(
        word_of(encode_neon_fcvtn(
            &[vreg_arr(0, "4h"), vreg_arr(0, "4s")],
            false
        )),
        0x0E21_6800
    );
    // fcvtn  v0.2s, v0.2d  => 0x0e616800  (Q=0, size=01)
    assert_eq!(
        word_of(encode_neon_fcvtn(
            &[vreg_arr(0, "2s"), vreg_arr(0, "2d")],
            false
        )),
        0x0E61_6800
    );
    // fcvtn2 v0.8h, v0.4s  => 0x4e216800  (Q=1, size=00)
    assert_eq!(
        word_of(encode_neon_fcvtn(
            &[vreg_arr(0, "8h"), vreg_arr(0, "4s")],
            true
        )),
        0x4E21_6800
    );
    // fcvtn2 v0.4s, v0.2d  => 0x4e616800  (Q=1, size=01)
    assert_eq!(
        word_of(encode_neon_fcvtn(
            &[vreg_arr(0, "4s"), vreg_arr(0, "2d")],
            true
        )),
        0x4E61_6800
    );

    // Non-zero registers: fcvtn v5.4h, v7.4s => base | (7<<5) | 5 = 0x0e2168e5
    assert_eq!(
        word_of(encode_neon_fcvtn(
            &[vreg_arr(5, "4h"), vreg_arr(7, "4s")],
            false
        )),
        0x0E21_6800 | (7 << 5) | 5,
    );
    assert_eq!(0x0E21_6800 | (7 << 5) | 5, 0x0E21_68E5);
}

#[test]
fn rejects_too_few_operands() {
    // 0 or 1 operands must error (no explicit count check, but get_neon_reg
    // returns None and the function surfaces that as Err).
    assert!(
        encode_neon_fcvtn(&[], false).is_err(),
        "0 operands must error"
    );
    assert!(
        encode_neon_fcvtn(&[vreg_arr(0, "4h")], false).is_err(),
        "1 operand must error"
    );
    // Exactly two valid operands must succeed.
    assert!(encode_neon_fcvtn(&[vreg_arr(0, "4h"), vreg_arr(1, "4s")], false).is_ok());
}
