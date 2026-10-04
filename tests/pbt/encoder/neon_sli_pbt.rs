//! Property-based tests for `encode_neon_sli` (NEON SLI: Shift Left and Insert).
//!
//! Encoding (ARMv8 ARM, "Advanced SIMD shift by immediate" — SLI):
//!   `0 Q 1 0 11110 immh:immb 010101 Rn Rd`
//!    31 30 29 28-23 22-19 18-16 15-10  9-5 4-0
//!
//! SLI is the *accumulating* twin of SHL: the two instructions share the
//! same opcode field `010101` at bits [15:10] and differ ONLY in the U bit
//! (bit 29): SHL has U=0, SLI has U=1.
//!
//! Element-size / shift decode (architectural, identical to SHL):
//!   `esize = 8 << HighestSetBit(immh)`
//!   `shift = UInt(immh:immb) - esize`   =>  immh:immb = esize + shift
//! Valid `shift` range is `[0, esize-1]`.
//!
//! Golden words (hand-computed; SLI == SHL word with bit 29 set; cross-checkable
//! against `llvm-mc`/objdump):
//!   `sli v5.4s,  v7.4s,  #3` => 0x6F2354E5   (Q=1, U=1, 32-bit, immh:immb=35)
//!   `sli v0.16b, v0.16b, #5` => 0x6F0D5400   (Q=1, U=1,  8-bit, immh:immb=13)
//!   `sli v2.2d,  v3.2d,  #5` => 0x6F455462   (Q=1, U=1, 64-bit, immh:immb=69)
//!   `sli v3.2s,  v9.2s,  #1` => 0x2F215523   (Q=0, U=1, 32-bit, immh:immb=33)
//!
//! FINDING: `prop_sli_rejects_out_of_range_shift` is a NEGATIVE-CONTRACT
//! property. Per the ARM ARM, `shift` must lie in `[0, esize-1]`; anything
//! outside (negative, or `>= esize`) is UNDEFINED and must be rejected. The
//! implementation performs **no** range check: `immh_immb = (esize + shift) &
//! mask` is just masked, so an out-of-range shift either yields `immh == 0`
//! (a *reserved* encoding) or silently re-encodes as a *different* element
//! size. This property is `#[ignore]`'d so default `cargo test` stays green;
//! run with `cargo test -- --ignored` to witness the bug. See BUG_REPORT.md.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::{encode_neon_shl, encode_neon_sli};
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
    /// Reference oracle: the encoded word for four hand-computed cases
    /// (covering Q=0/Q=1, and 8/32/64-bit elements) matches the ARMv8 ARM
    /// golden words exactly.
    #[test]
    fn prop_sli_golden_words(which in 0u32..4u32) {
        let (arr, rd, rn, shift, golden): (&str, u32, u32, u32, u32) = match which {
            0 => ("4s",  5, 7, 3, 0x6F2354E5), // sli v5.4s,  v7.4s,  #3
            1 => ("16b", 0, 0, 5, 0x6F0D5400), // sli v0.16b, v0.16b, #5
            2 => ("2d",  2, 3, 5, 0x6F455462), // sli v2.2d,  v3.2d,  #5
            _ => ("2s",  3, 9, 1, 0x2F215523), // sli v3.2s,  v9.2s,  #1
        };
        let ops = vec![vreg_arr(rd, arr), vreg_arr(rn, arr), Operand::Imm(shift as i64)];
        let word = word_of(encode_neon_sli(&ops));
        prop_assert_eq!(word, golden, "golden mismatch for case {}", which);
    }

    /// Architectural decode oracle: extract immh:immb from the word and recover
    /// (esize, shift) via the ARM decode rule — independent of the encoder's
    /// forward expression `immh_immb = esize + shift`.
    #[test]
    fn prop_sli_immh_immb_decodes_correctly(
        (arr, _q, esize) in arr_strategy(),
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        shift in 0u32..64u32, // filtered below to the valid [0, esize-1] window
    ) {
        // Only test in-range shifts here; out-of-range is the negative contract.
        prop_assume!(shift < esize);

        let ops = vec![vreg_arr(rd, arr), vreg_arr(rn, arr), Operand::Imm(shift as i64)];
        let word = word_of(encode_neon_sli(&ops));

        let immh = (word >> 19) & 0xF;
        let immb = (word >> 16) & 0x7;
        let immhb = (immh << 3) | immb;

        prop_assert!(immh != 0, "immh must be non-zero (got reserved encoding) for .{}", arr);
        let decoded_esize = 8u32 << highest_set_bit(immh);
        prop_assert_eq!(decoded_esize, esize, "decoded element size mismatch for .{}", arr);
        prop_assert_eq!(immhb - decoded_esize, shift, "decoded shift mismatch for .{}", arr);
    }

    /// Bit-field placement & isolation: the fixed fields are pinned and the
    /// variable fields occupy exactly their lanes with no leakage. Also checks
    /// that the narrow↔wide counterpart of the same element size differs only
    /// in the Q bit (bit 30).
    #[test]
    fn prop_sli_field_placement(
        (arr, q, esize) in arr_strategy(),
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        shift in 0u32..64u32,
    ) {
        prop_assume!(shift < esize);

        let ops = vec![vreg_arr(rd, arr), vreg_arr(rn, arr), Operand::Imm(shift as i64)];
        let word = word_of(encode_neon_sli(&ops));

        // Bit 31 must be clear.
        prop_assert_eq!(word >> 31, 0, "bit 31 must be 0");
        // U (bit 29) is fixed to 1 for SLI.
        prop_assert_eq!((word >> 29) & 1, 1, "U bit must be 1 for SLI");
        // Fixed field bits [28:23] == 0b011110.
        prop_assert_eq!((word >> 23) & 0x3F, 0b011110, "bits [28:23] != 011110");
        // Fixed 6-bit opcode at bits [15:10] == 0b010101.
        prop_assert_eq!((word >> 10) & 0x3F, 0b010101, "bits [15:10] != 010101");
        // Q at bit 30.
        prop_assert_eq!((word >> 30) & 1, q, "Q bit mismatch");
        // Rd in [4:0], Rn in [9:5].
        prop_assert_eq!(word & 0x1F, rd, "Rd not in bits [4:0]");
        prop_assert_eq!((word >> 5) & 0x1F, rn, "Rn not in bits [9:5]");

        // No leakage: varying only Rd leaves every bit above [4:0] untouched,
        // and varying only Rn leaves every bit outside [9:5] untouched.
        let w_rd = word_of(encode_neon_sli(
            &[vreg_arr(rd, arr), vreg_arr(0, arr), Operand::Imm(shift as i64)]));
        let base00 = word_of(encode_neon_sli(
            &[vreg_arr(0, arr), vreg_arr(0, arr), Operand::Imm(shift as i64)]));
        prop_assert_eq!(w_rd & !0x1F, base00 & !0x1F, "Rd leaked beyond bits [4:0]");
        let word_rn = word_of(encode_neon_sli(
            &[vreg_arr(0, arr), vreg_arr(rn, arr), Operand::Imm(shift as i64)]));
        prop_assert_eq!(word_rn & !0x3E0, base00 & !0x3E0, "Rn leaked beyond bits [9:5]");
    }

    /// Differential oracle: SLI and SHL share opcode `010101` and differ ONLY
    /// in the U bit (bit 29). For identical (arr, rd, rn, shift) within the
    /// valid range, `sli_word ^ shl_word` must equal exactly `1 << 29`.
    #[test]
    fn prop_sli_differs_from_shl_only_in_u_bit(
        (arr, _q, esize) in arr_strategy(),
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        shift in 0u32..64u32,
    ) {
        prop_assume!(shift < esize);

        let ops = vec![vreg_arr(rd, arr), vreg_arr(rn, arr), Operand::Imm(shift as i64)];
        let sli_word = word_of(encode_neon_sli(&ops));
        let shl_word = word_of(encode_neon_shl(&ops));

        // SHL must have U=0, SLI must have U=1.
        prop_assert_eq!(shl_word & 0x2000_0000, 0, "SHL should have U=0");
        prop_assert_eq!(sli_word & 0x2000_0000, 0x2000_0000, "SLI should have U=1");
        // The XOR isolates exactly the differing bit.
        prop_assert_eq!(sli_word ^ shl_word, 1u32 << 29, "SLI/SHL differ in more than the U bit");
    }

    /// Q bit is the SOLE difference between a narrow arrangement and its wide
    /// counterpart (same esize): .8b↔.16b, .4h↔.8h, .2s↔.4s. For identical
    /// (rd, rn, shift) only bit 30 should differ.
    #[test]
    fn prop_sli_q_bit_is_sole_narrow_wide_difference(
        pair in prop_oneof![
            Just(("8b", "16b")),
            Just(("4h", "8h")),
            Just(("2s", "4s")),
        ],
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        shift in 0u32..8u32, // valid for the smallest esize (8-bit) in every pair
    ) {
        let (narrow, wide) = pair;
        let ops_n = vec![vreg_arr(rd, narrow), vreg_arr(rn, narrow), Operand::Imm(shift as i64)];
        let ops_w = vec![vreg_arr(rd, wide), vreg_arr(rn, wide), Operand::Imm(shift as i64)];
        let wn = word_of(encode_neon_sli(&ops_n));
        let ww = word_of(encode_neon_sli(&ops_w));

        prop_assert_eq!(wn & 0x4000_0000, 0, "narrow form should have Q=0");
        prop_assert_eq!(ww & 0x4000_0000, 0x4000_0000, "wide form should have Q=1");
        // Everything except bit 30 is identical.
        prop_assert_eq!(ww & !0x4000_0000, wn, "narrow/wide differ in more than the Q bit");
    }

    /// NEGATIVE CONTRACT (BUG WITNESS): per the ARM ARM the `shift` immediate
    /// for SLI must satisfy `0 <= shift <= esize-1`. Inputs outside that window
    /// are UNDEFINED and the encoder must return `Err`. The implementation
    /// performs no such check — it masks `esize + shift`, so out-of-range
    /// shifts are silently accepted and produce reserved/mis-decoded encodings.
    /// This property is `#[ignore]`'d so default `cargo test` stays green; run
    /// with `cargo test -- --ignored` to witness the failure.
    #[test]
    #[ignore = "documented bug: SLI accepts out-of-range shift immediates"]
    fn prop_sli_rejects_out_of_range_shift(
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

        let res = encode_neon_sli(&ops);
        prop_assert!(
            res.is_err(),
            "shift {} on .{} (esize {}) is out of range [0,{}] and must be rejected, but got {:?}",
            shift, arr, esize, esize.saturating_sub(1), res
        );
    }
}
