//! Property-based tests for `encode_neon_shift_left_imm`.
//!
//! Encodes the AArch64 "Advanced SIMD shift by immediate" class used by the
//! *left* shifts: `SHL` (u=0, opcode=01010), `SQSHL` (u=0, opcode=01110) and
//! `UQSHL` (u=1, opcode=01110).
//!
//! Encoding (ARMv8 ARM, "Advanced SIMD shift by immediate"):
//!   `0 Q U 0 11110 immh:immb opcode 1 Rn Rd`
//!    31 30 29 28-23 22-19 18-16 15-11 10 9-5 4-0
//!
//! Element size / shift decode (architectural, for the *left* shifts):
//!   `esize  = 8 << HighestSetBit(immh)`
//!   `shift  = UInt(immh:immb) - esize`     =>  immh:immb = esize + shift
//!
//! Valid `shift` range for these instructions is `[0, esize-1]`.
//!
//! Golden words (hand-computed, cross-checkable against `llvm-mc`/objdump):
//!   `shl   v5.4s,  v7.4s,  #3`  (u=0,opc=0b01010) => 0x4F2354E5
//!   `sqshl v0.16b, v0.16b, #5`  (u=0,opc=0b01110) => 0x4F0D7400
//!   `uqshl v3.2s,  v9.2s,  #1`  (u=1,opc=0b01110) => 0x2F217523
//!
//! NOTE on findings:
//!  * `prop_shift_left_imm_rejects_out_of_range_shift` is a NEGATIVE-CONTRACT
//!    property. Per the ARM ARM, `shift` must lie in `[0, esize-1]`; anything
//!    outside (negative, or `>= esize`) is UNDEFINED and must be rejected.
//!    The current implementation does **no** range check: `immhb = esize +
//!    shift` is just masked, so a negative shift produces `immh=0000` (a
//!    *reserved* encoding) and an over-large shift silently re-encodes as a
//!    *different* element size. This property is EXPECTED TO FAIL, surfacing
//!    the missing validation. See BUG_REPORT.md.
//!  * The match arm also binds `_immh_base`, which is computed but never used
//!    (immh is derived entirely from `(esize+shift)>>3`). Dead value, noted.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_shift_left_imm;
use proptest::prelude::*;

// --- helpers --------------------------------------------------------------

fn vreg_arr(reg: u32, arr: &str) -> Operand {
    Operand::RegArrangement {
        reg: format!("v{reg}"),
        arrangement: arr.to_string(),
    }
}

fn word_of(res: Result<EncodeResult, String>) -> u32 {
    match res {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {other:?}"),
    }
}

fn reg_num_strategy() -> impl Strategy<Value = u32> {
    0u32..=31u32
}

/// All arrangements accepted by the encoder, with their (Q, esize).
fn arr_strategy() -> impl Strategy<Value = (&'static str, u32, u32)> {
    prop_oneof![
        Just(("8b", 0u32, 8u32)),
        Just(("16b", 1u32, 8u32)),
        Just(("4h", 0u32, 16u32)),
        Just(("8h", 1u32, 16u32)),
        Just(("2s", 0u32, 32u32)),
        Just(("4s", 1u32, 32u32)),
        Just(("2d", 1u32, 64u32)),
    ]
}

/// Architectural decode: `esize = 8 << HighestSetBit(immh)`.
fn highest_set_bit(x: u32) -> u32 {
    31 - x.leading_zeros()
}

// --- properties -----------------------------------------------------------

proptest! {
    /// Reference oracle: the encoded word for a few hand-computed cases
    /// (covering both U values, two opcodes, and 8b/4s/16b/2s elements)
    /// matches the ARMv8 ARM golden words exactly.
    #[test]
    fn prop_shift_left_imm_golden_words(which in 0u32..3u32) {
        let (arr, u, opcode, rd, rn, shift, golden): (&str, u32, u32, u32, u32, u32, u32) = match which {
            0 => ("4s",  0, 0b01010, 5, 7, 3, 0x4F2354E5), // shl   v5.4s,  v7.4s,  #3
            1 => ("16b", 0, 0b01110, 0, 0, 5, 0x4F0D7400), // sqshl v0.16b, v0.16b, #5
            _ => ("2s",  1, 0b01110, 3, 9, 1, 0x2F217523), // uqshl v3.2s,  v9.2s,  #1
        };
        let ops = vec![vreg_arr(rd, arr), vreg_arr(rn, arr), Operand::Imm(shift as i64)];
        let word = word_of(encode_neon_shift_left_imm(&ops, u, opcode));
        prop_assert_eq!(word, golden, "golden mismatch for case {}", which);
    }

    /// Architectural decode oracle: pull immh:immb back out of the word and
    /// recover (esize, shift) via the ARM decode rule — independent of the
    /// encoder's forward expression.
    #[test]
    fn prop_shift_left_imm_immh_immb_decodes_correctly(
        (arr, _q, esize) in arr_strategy(),
        u in 0u32..=1u32,
        opcode in 0u32..=0x1Fu32,
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        shift in 0u32..64u32, // filtered below to the valid [0, esize-1] window
    ) {
        // Only test in-range shifts here; out-of-range is the negative contract.
        prop_assume!(shift < esize);

        let ops = vec![vreg_arr(rd, arr), vreg_arr(rn, arr), Operand::Imm(shift as i64)];
        let word = word_of(encode_neon_shift_left_imm(&ops, u, opcode));

        let immh = (word >> 19) & 0xF;
        let immb = (word >> 16) & 0x7;
        let immhb = (immh << 3) | immb;

        prop_assert!(immh != 0, "immh must be non-zero (got reserved encoding) for .{}", arr);
        let decoded_esize = 8u32 << highest_set_bit(immh);
        prop_assert_eq!(decoded_esize, esize, "decoded element size mismatch for .{}", arr);
        prop_assert_eq!(immhb - decoded_esize, shift, "decoded shift mismatch for .{}", arr);
    }

    /// Bit-field placement & isolation: the fixed fields are pinned and the
    /// variable fields occupy exactly their lanes with no leakage.
    #[test]
    fn prop_shift_left_imm_field_placement(
        (arr, q, esize) in arr_strategy(),
        u in 0u32..=1u32,
        opcode in 0u32..=0x1Fu32,
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        shift in 0u32..64u32,
    ) {
        prop_assume!(shift < esize);

        let ops = vec![vreg_arr(rd, arr), vreg_arr(rn, arr), Operand::Imm(shift as i64)];
        let word = word_of(encode_neon_shift_left_imm(&ops, u, opcode));

        // Bit 31 must be clear.
        prop_assert_eq!(word >> 31, 0, "bit 31 must be 0");
        // Fixed opcode field bits [28:23] == 0b011110.
        prop_assert_eq!((word >> 23) & 0x3F, 0b011110, "bits [28:23] != 011110");
        // Fixed bit 10 == 1.
        prop_assert_eq!((word >> 10) & 1, 1, "bit 10 must be 1");
        // U at bit 29, Q at bit 30.
        prop_assert_eq!((word >> 29) & 1, u, "U bit mismatch");
        prop_assert_eq!((word >> 30) & 1, q, "Q bit mismatch");
        // 5-bit opcode occupies exactly bits [15:11].
        prop_assert_eq!((word >> 11) & 0x1F, opcode, "opcode not in bits [15:11]");
        // Rd in [4:0], Rn in [9:5].
        prop_assert_eq!(word & 0x1F, rd, "Rd not in bits [4:0]");
        prop_assert_eq!((word >> 5) & 0x1F, rn, "Rn not in bits [9:5]");

        // No leakage: varying only Rd leaves every bit above [4:0] untouched,
        // and varying only Rn leaves every bit outside [9:5] untouched.
        let w_rd = word_of(encode_neon_shift_left_imm(
            &[vreg_arr(rd, arr), vreg_arr(0, arr), Operand::Imm(shift as i64)], u, opcode));
        let base00 = word_of(encode_neon_shift_left_imm(
            &[vreg_arr(0, arr), vreg_arr(0, arr), Operand::Imm(shift as i64)], u, opcode));
        prop_assert_eq!(w_rd & !0x1F, base00 & !0x1F, "Rd leaked beyond bits [4:0]");
        let word_rn = word_of(encode_neon_shift_left_imm(
            &[vreg_arr(0, arr), vreg_arr(rn, arr), Operand::Imm(shift as i64)], u, opcode));
        prop_assert_eq!(word_rn & !0x3E0, base00 & !0x3E0, "Rn leaked beyond bits [9:5]");
    }

    /// Q bit is the SOLE difference between a narrow arrangement and its wide
    /// counterpart (same esize): .8b↔.16b, .4h↔.8h, .2s↔.4s. For identical
    /// (u, opcode, rd, rn, shift) only bit 30 should differ.
    #[test]
    fn prop_shift_left_imm_q_bit_is_sole_narrow_wide_difference(
        pair in prop_oneof![
            Just(("8b", "16b")),
            Just(("4h", "8h")),
            Just(("2s", "4s")),
        ],
        u in 0u32..=1u32,
        opcode in 0u32..=0x1Fu32,
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        shift in 0u32..8u32, // valid for the smallest esize (8-bit) in every pair
    ) {
        let (narrow, wide) = pair;
        let ops_n = vec![vreg_arr(rd, narrow), vreg_arr(rn, narrow), Operand::Imm(shift as i64)];
        let ops_w = vec![vreg_arr(rd, wide), vreg_arr(rn, wide), Operand::Imm(shift as i64)];
        let wn = word_of(encode_neon_shift_left_imm(&ops_n, u, opcode));
        let ww = word_of(encode_neon_shift_left_imm(&ops_w, u, opcode));

        prop_assert_eq!(wn & 0x4000_0000, 0, "narrow form should have Q=0");
        prop_assert_eq!(ww & 0x4000_0000, 0x4000_0000, "wide form should have Q=1");
        // Everything except bit 30 is identical.
        prop_assert_eq!(ww & !0x4000_0000, wn, "narrow/wide differ in more than the Q bit");
    }

    /// NEGATIVE CONTRACT (EXPECTED TO FAIL): per the ARM ARM the `shift`
    /// immediate for these left-shift instructions must satisfy
    /// `0 <= shift <= esize-1`. Inputs outside that window are UNDEFINED and
    /// the encoder must return `Err`. The implementation performs no such
    /// check, so this property fails — surfacing the missing validation.
    #[test]
    #[ignore = "documented bug: shift-left immediate accepts out-of-range shifts"]
    fn prop_shift_left_imm_rejects_out_of_range_shift(
        (arr, _q, esize) in arr_strategy(),
        too_big in proptest::sample::select(vec![0u32, 1, 2, 4, 8, 16, 31, 32, 63, 64, 100, 255]),
        is_negative in any::<bool>(),
    ) {
        // Build a shift that is out of range: either >= esize or negative.
        let shift: i64 = if is_negative {
            -((too_big % 64) as i64 + 1) // negative shift
        } else {
            // Guarantee >= esize by adding esize to a non-negative base.
            (esize as i64) + (too_big as i64)
        };

        let ops = vec![
            vreg_arr(0, arr),
            vreg_arr(0, arr),
            Operand::Imm(shift),
        ];

        let res = encode_neon_shift_left_imm(&ops, 0, 0b01110);
        prop_assert!(
            res.is_err(),
            "shift {} on .{} (esize {}) is out of range [0,{}] and must be rejected, but got {:?}",
            shift, arr, esize, esize.saturating_sub(1), res
        );
    }
}
