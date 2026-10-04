//! Property-based tests for `encode_cnt` (the NEON `CNT` instruction encoder).
//!
//! `CNT Vd.<T>, Vn.<T>` counts the number of set bits in each byte of `Vn`.
//! It belongs to the AArch64 "Advanced SIMD two-register miscellaneous" group.
//!
//! Encoding (ARMv8 ARM):
//!   `0 Q 0 01110 00 100000 0101 10 Rn Rd`
//!    31 30 29-24 23-22 21-16  15-12 11-10 9-5 4-0
//!
//! Architecturally CNT is defined **only** for `<T>` in {`.8b` (Q=0), `.16b` (Q=1)};
//! every other arrangement is UNDEFINED and must be rejected.
//!
//! Golden words cross-checked against `llvm-mc`/objdump:
//!   `cnt v0.8b, v0.8b`   => `0x0e205800`
//!   `cnt v0.16b, v0.16b` => `0x4e205800`
//!
//! Regression properties verify that both vector arrangements match and that
//! only the valid byte arrangements `.8b` and `.16b` are accepted.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_cnt;
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

/// Arrangements that are UNDEFINED for CNT (non-byte lanes).
fn invalid_arr_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![
        Just("4h"),
        Just("8h"),
        Just("2s"),
        Just("4s"),
        Just("1d"),
        Just("2d"),
    ]
}

fn word_of(res: Result<EncodeResult, String>) -> u32 {
    match res {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {other:?}"),
    }
}

/// Constant opcode portion of the CNT encoding with Q=Rn=Rd=0.
const CNT_BASE: u32 = 0x0E20_5800;

// --- properties -----------------------------------------------------------

proptest! {
    /// Reference oracle: for valid (rd, rn, arrangement) the encoded word
    /// equals the ARMv8 ARM reference encoding.
    #[test]
    fn prop_cnt_matches_arm_reference(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        is_16b in any::<bool>(),
    ) {
        let arr = if is_16b { "16b" } else { "8b" };
        let ops = vec![vreg_arr(rd, arr), vreg_arr(rn, arr)];
        let word = word_of(encode_cnt(&ops));

        let q: u32 = if is_16b { 1 } else { 0 };
        let expected = CNT_BASE | (q << 30) | (rn << 5) | rd;
        prop_assert_eq!(word, expected);
    }

    /// Field isolation: Rd occupies exactly bits [4:0], Rn exactly bits [9:5],
    /// and neither register number leaks into any other field.
    #[test]
    fn prop_cnt_fields_isolated(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
    ) {
        let base = word_of(encode_cnt(&[vreg_arr(0, "8b"), vreg_arr(0, "8b")]));

        let with_rd = word_of(encode_cnt(&[vreg_arr(rd, "8b"), vreg_arr(0, "8b")]));
        prop_assert_eq!(with_rd & 0xFFFF_FFE0, base & 0xFFFF_FFE0, "Rd leaked above bit 4");
        prop_assert_eq!(with_rd & 0x1F, rd, "Rd not in bits [4:0]");

        let with_rn = word_of(encode_cnt(&[vreg_arr(0, "8b"), vreg_arr(rn, "8b")]));
        prop_assert_eq!(with_rn & !0x3E0, base & !0x3E0, "Rn leaked outside bits [9:5]");
        prop_assert_eq!((with_rn >> 5) & 0x1F, rn, "Rn not in bits [9:5]");
    }

    /// Q bit (bit 30) is the sole difference between `.8b` and `.16b`; every
    /// other bit (the fixed opcode) is constant for all register operands.
    #[test]
    fn prop_cnt_q_and_fixed_bits(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
    ) {
        let w8 = word_of(encode_cnt(&[vreg_arr(rd, "8b"), vreg_arr(rn, "8b")]));
        let w16 = word_of(encode_cnt(&[vreg_arr(rd, "16b"), vreg_arr(rn, "16b")]));

        // .8b -> Q=0, .16b -> Q=1, and that is the only bit that differs.
        prop_assert_eq!(w16 & !0x4000_0000, w8);
        prop_assert_eq!(w8 & 0x4000_0000, 0);
        prop_assert_eq!(w16 & 0x4000_0000, 0x4000_0000);

        // Mask off Q|Rn|Rd; the remainder is the constant opcode, == CNT_BASE.
        let mask = !(0x4000_0000 | 0x3E0 | 0x1F);
        prop_assert_eq!(w8 & mask, CNT_BASE & mask);
        prop_assert_eq!(w16 & mask, CNT_BASE & mask);
    }

    /// Source and destination arrangements must agree; bare registers are not
    /// valid operands for CNT either.
    #[test]
    fn prop_cnt_rejects_source_arrangement_mismatch(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        src_arr in prop_oneof![
            Just("16b"),
            Just("4h"),
            Just("8h"),
            Just("2s"),
            Just("4s"),
            Just("1d"),
            Just("2d"),
        ],
    ) {
        let dest = vreg_arr(rd, "8b");
        let mismatched = vec![dest.clone(), vreg_arr(rn, src_arr)];
        prop_assert!(encode_cnt(&mismatched).is_err(),
            "CNT .8b destination with .{src_arr} source must be rejected");

        let bare = vec![dest, Operand::Reg(format!("v{rn}"))];
        prop_assert!(encode_cnt(&bare).is_err(), "bare vector register must be rejected");
    }

    /// Negative contract: CNT is defined only for `.8b` and `.16b`; every
    /// other destination arrangement must be rejected.
    #[test]
    fn prop_cnt_rejects_non_byte_arrangements(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in invalid_arr_strategy(),
    ) {
        let ops = vec![vreg_arr(rd, arr), vreg_arr(rn, arr)];
        prop_assert!(
            encode_cnt(&ops).is_err(),
            "CNT with dest arrangement .{} should be rejected (UNDEFINED for CNT), got {:?}",
            arr,
            encode_cnt(&ops)
        );

        // A bare register with no arrangement as destination is equally invalid.
        let bare = vec![Operand::Reg(format!("v{rd}")), vreg_arr(rn, "8b")];
        prop_assert!(
            encode_cnt(&bare).is_err(),
            "CNT with bare-Reg destination (no arrangement) should be rejected, got {:?}",
            encode_cnt(&bare)
        );
    }
}

// --- deterministic boundary / golden checks -------------------------------

#[test]
fn golden_cnt_matches_llvm_mc() {
    // cnt v0.8b, v0.8b   => 0x0e205800
    assert_eq!(
        word_of(encode_cnt(&[vreg_arr(0, "8b"), vreg_arr(0, "8b")])),
        0x0E20_5800
    );
    // cnt v5.16b, v7.16b => 0x4e20_58e5 (Q=1, Rn=7 in bits[9:5]=0xE0, Rd=5)
    assert_eq!(
        word_of(encode_cnt(&[vreg_arr(5, "16b"), vreg_arr(7, "16b")])),
        0x4E20_5800 | (7 << 5) | 5
    );
    assert_eq!(0x4E20_5800 | (7 << 5) | 5, 0x4E20_58E5);
}

#[test]
fn rejects_too_few_operands() {
    assert!(encode_cnt(&[]).is_err(), "0 operands must error");
    assert!(
        encode_cnt(&[vreg_arr(0, "8b")]).is_err(),
        "1 operand must error"
    );
    assert!(
        encode_cnt(&[vreg_arr(0, "8b"), vreg_arr(1, "8b"), vreg_arr(2, "8b")]).is_err(),
        "extra operands must error"
    );
    // Exactly two valid operands must succeed.
    assert!(encode_cnt(&[vreg_arr(0, "8b"), vreg_arr(1, "8b")]).is_ok());
}
