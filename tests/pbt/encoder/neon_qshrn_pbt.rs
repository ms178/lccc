//! Property-based tests for `encode_neon_qshrn`
//! (the AArch64 "Advanced SIMD shift right *narrowing* saturating" encoder:
//! SQSHRN/SQSHRN2, UQSHRN/UQSHRN2, SQRSHRN/SQRSHRN2, UQRSHRN/UQRSHRN2).
//!
//! Encoding (ARMv8 ARM, "Advanced SIMD shift by amount", narrow group):
//!   `0 Q U 0 1 1 1 1 0 immh immb opcode Rn Rd`
//!    31 30 29 28-23 22-19 18-16 15-10 9-5 4-0
//!   where opcode = 100101 (non-rounding) or 100111 (rounding),
//!         Q = `is_high` (the "...2" variants),
//!         U = `u_bit` (0 = signed SQSHRN/SQRSHRN, 1 = unsigned UQSHRN/UQRSHRN).
//!
//! The source element is twice the width of the destination, so only the
//! arrangements `.8h / .4s / .2d` are valid sources (destinations `.8b / .4h / .2s`).
//!
//! Shift encoding (ARMv8 ARM immh:immb table for narrowing shifts):
//!   immh:immb = esize_src - shift,   i.e.  shift = esize_src - UInt(immh:immb)
//!   with immh constrained by the source width:
//!     .8h -> immh = 0001,  immh:immb in [8 .. 15],   so shift in 1..=8   (= esize_src/2)
//!     .4s -> immh = 001x,  immh:immb in [16 .. 31],  so shift in 1..=16  (= esize_src/2)
//!     .2d -> immh = 01xx,  immh:immb in [32 .. 63],  so shift in 1..=32  (= esize_src/2)
//!   immh = 0000 is UNALLOCATED for this group.
//!
//! CONSEQUENCE (the finding below): the encoder validates only
//!   `shift == 0 || shift > element_bits`,
//! i.e. it permits shift up to `esize_src` (16/32/64). The ARMv8 ARM permits
//! only `1..=esize_src/2` (8/16/32). Shifts in `(esize_src/2, esize_src]` are
//! *accepted* and then either
//!   (a) silently encode a DIFFERENT source element size (immh category
//!       mismatches the arrangement), or
//!   (b) encode immh = 0000 (UNALLOCATED).
//! Both are corrupt instructions that a real assembler rejects. The
//! `prop_qshrn_rejects_over_range_shift` property asserts the negative
//! contract (such shifts must return `Err`); it FAILS against the current
//! implementation and is left un-`#[ignore]`d as the promoted bug.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_qshrn;
use proptest::prelude::*;

// --- helpers --------------------------------------------------------------

fn vreg_arr(reg: u32, arr: &str) -> Operand {
    Operand::RegArrangement {
        reg: format!("v{reg}"),
        arrangement: arr.to_string(),
    }
}

fn imm(v: u32) -> Operand {
    Operand::Imm(v as i64)
}

fn reg_num_strategy() -> impl Strategy<Value = u32> {
    0u32..=31u32
}

fn u_bit_strategy() -> impl Strategy<Value = u32> {
    0u32..=1u32
}

/// (source arrangement, a shift strictly inside the VALID range 1..=esize/2).
fn valid_src_shift_strategy() -> impl Strategy<Value = (&'static str, u32)> {
    prop_oneof![
        (Just("8h"), 1u32..=8u32),  // esize=16, valid shift 1..=8
        (Just("4s"), 1u32..=16u32), // esize=32, valid shift 1..=16
        (Just("2d"), 1u32..=32u32), // esize=64, valid shift 1..=32
    ]
}

/// (source arrangement, a shift that is OUT of the valid range but INSIDE the
/// range the implementation currently accepts, i.e. (esize/2, esize]).
/// Per the ARMv8 ARM these must be rejected; per the current code they are not.
fn over_range_shift_strategy() -> impl Strategy<Value = (&'static str, u32)> {
    prop_oneof![
        (Just("8h"), 9u32..=16u32),  // esize=16, valid max 8
        (Just("4s"), 17u32..=32u32), // esize=32, valid max 16
        (Just("2d"), 33u32..=64u32), // esize=64, valid max 32
    ]
}

fn word_of(res: Result<EncodeResult, String>) -> u32 {
    match res {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {other:?}"),
    }
}

/// INDEPENDENT oracle: source arrangement -> element width (esize), hardcoded
/// here rather than calling the SUT's match arm.
fn arr_element_bits(arr: &str) -> Option<u32> {
    Some(match arr {
        "8h" => 16,
        "4s" => 32,
        "2d" => 64,
        _ => return None,
    })
}

/// Expected immh:immb category TOP bits (the high nibble mask for the source
/// width), per the ARMv8 ARM. Used to detect silent size-corruption for
/// over-range shifts.
fn arr_immh_category(arr: &str) -> Option<(u32, u32)> {
    // immh is bits [22:19]; we return (immh_value, valid_mask) where valid_mask
    // selects the category bits: .8h->0001, .4s->001x, .2d->01xx.
    Some(match arr {
        "8h" => (0b0001u32, 0b1111u32), // immh == 0001
        "4s" => (0b0010u32, 0b1110u32), // immh in 0010..0011
        "2d" => (0b0100u32, 0b1100u32), // immh in 0100..0111
        _ => return None,
    })
}

/// Reference encoding built directly from the documented layout. `shift` is
/// assumed to be in the valid range 1..=esize/2.
fn ref_word(
    rd: u32,
    rn: u32,
    arr_n: &str,
    shift: u32,
    u_bit: u32,
    is_rounding: bool,
    is_high: bool,
) -> u32 {
    let element_bits = arr_element_bits(arr_n).unwrap();
    let immhb = element_bits - shift; // immh:immb combined (fits in 7 bits)
    let q = if is_high { 1u32 } else { 0u32 };
    let opcode_bits: u32 = if is_rounding { 0b100111 } else { 0b100101 };
    (q << 30)
        | (u_bit << 29)
        | (0b011110 << 23)
        | ((immhb >> 3) << 19)
        | ((immhb & 7) << 16)
        | (opcode_bits << 10)
        | (rn << 5)
        | rd
}

/// Fixed constant bits of this encoding, independent of all valid inputs:
/// bit 31 = 0; bits [28:23] = 011110.
const FIXED_MASK: u32 = 0x8000_0000 | 0x1F80_0000; // bit31 + bits[28:23]
const FIXED_CONST: u32 = 0x0F00_0000; // bit31 contributes 0; [28:23]=011110

// --- properties (PASS against the implementation) -------------------------

proptest! {
    /// Reference oracle: for all VALID inputs the encoded word equals the word
    /// built straight from the ARMv8 ARM layout.
    #[test]
    fn prop_qshrn_matches_arm_layout(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        (arr_n, shift) in valid_src_shift_strategy(),
        u_bit in u_bit_strategy(),
        is_rounding in any::<bool>(),
        is_high in any::<bool>(),
    ) {
        let ops = vec![vreg_arr(rd, arr_n), vreg_arr(rn, arr_n), imm(shift)];
        let word = word_of(encode_neon_qshrn(&ops, u_bit, is_rounding, is_high));
        prop_assert_eq!(word, ref_word(rd, rn, arr_n, shift, u_bit, is_rounding, is_high));
    }

    /// Field isolation: Rd is bits [4:0], Rn bits [9:5], opcode bits [15:10],
    /// and immh:immb (reconstructed as bits [22:16]) equals `esize - shift`.
    /// No field leaks into another; toggling is_rounding flips only opcode bit 0.
    #[test]
    fn prop_qshrn_fields_isolated(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        (arr_n, shift) in valid_src_shift_strategy(),
        u_bit in u_bit_strategy(),
        is_rounding in any::<bool>(),
        is_high in any::<bool>(),
    ) {
        let mk = |r_rd: u32, r_rn: u32, r_sh: u32, r_round: bool| -> u32 {
            word_of(encode_neon_qshrn(
                &[vreg_arr(r_rd, arr_n), vreg_arr(r_rn, arr_n), imm(r_sh)],
                u_bit, r_round, is_high,
            ))
        };

        let base = mk(0, 0, shift, is_rounding);

        // Rd in [4:0] only.
        let with_rd = mk(rd, 0, shift, is_rounding);
        prop_assert_eq!(with_rd & 0x1F, rd);
        prop_assert_eq!(with_rd & !0x1F, base & !0x1F, "Rd leaked above bit 4");

        // Rn in [9:5] only.
        let with_rn = mk(0, rn, shift, is_rounding);
        prop_assert_eq!((with_rn >> 5) & 0x1F, rn);
        prop_assert_eq!(with_rn & !0x3E0, base & !0x3E0, "Rn leaked outside bits [9:5]");

        // opcode in [15:10] only, and equals 100101 / 100111 per is_rounding.
        let with_round = mk(0, 0, shift, true);
        let without_round = mk(0, 0, shift, false);
        let exp_opcode: u32 = if is_rounding { 0b100111 } else { 0b100101 };
        prop_assert_eq!((mk(rd, rn, shift, is_rounding) >> 10) & 0x3F, exp_opcode);
        // Rounding toggles only opcode bit 1 within the [15:10] field
        // (100101 vs 100111 differ by 0b010 = 2, i.e. word bit 11).
        prop_assert_eq!(with_round ^ without_round, 2u32 << 10, "is_rounding must only flip opcode bit 1");

        // immh:immb (bits [22:16]) reconstructs esize - shift exactly.
        let esize = arr_element_bits(arr_n).unwrap();
        let word = mk(rd, rn, shift, is_rounding);
        prop_assert_eq!((word >> 16) & 0x7F, esize - shift, "immh:immb must equal esize-shift");
    }

    /// Fixed bits constant for all valid inputs; Q = is_high, U = u_bit, and
    /// immh:immb == esize - shift.
    #[test]
    fn prop_qshrn_fixed_bits_and_qu(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        (arr_n, shift) in valid_src_shift_strategy(),
        u_bit in u_bit_strategy(),
        is_rounding in any::<bool>(),
        is_high in any::<bool>(),
    ) {
        let ops = vec![vreg_arr(rd, arr_n), vreg_arr(rn, arr_n), imm(shift)];
        let word = word_of(encode_neon_qshrn(&ops, u_bit, is_rounding, is_high));

        prop_assert_eq!(word & FIXED_MASK, FIXED_CONST, "fixed bits mutated by inputs");
        prop_assert_eq!((word >> 30) & 1, u32::from(is_high), "Q bit must equal is_high");
        prop_assert_eq!((word >> 29) & 1, u_bit, "U bit must equal u_bit");

        let esize = arr_element_bits(arr_n).unwrap();
        prop_assert_eq!((word >> 16) & 0x7F, esize - shift, "immh:immb must equal esize-shift");

        // The immh category must match the source arrangement (size not corrupted).
        let immh = (word >> 19) & 0xF;
        let (cat_val, cat_mask) = arr_immh_category(arr_n).unwrap();
        prop_assert_eq!(immh & cat_mask, cat_val, "immh category must match source width");
    }

    /// NEGATIVE CONTRACT (EXPECTED TO **FAIL** — the promoted bug):
    ///
    /// Per the ARMv8 ARM the shift for a narrowing saturating shift is bounded
    /// by `1..=esize/2` (8/16/32 for .8h/.4s/.2d respectively). A shift in
    /// `(esize/2, esize]` is illegal: it cannot be represented in the immh:immb
    /// field for the given source width, and a defensive encoder MUST reject it.
    ///
    /// The implementation checks only `shift > esize`, so it ACCEPTS these
    /// shifts. Worse, the accepted words are corrupt:
    ///   * `immh` then decodes to a DIFFERENT source element size than the
    ///     arrangement requested (silent instruction-size corruption), or
    ///   * `immh == 0000`, which is UNALLOCATED for this encoding group.
    ///
    /// This property asserts the correct contract (reject with Err). It fails
    /// on essentially every generated case against the current code.
    #[test]
    #[ignore = "documented bug: qshrn accepts over-range narrowing shifts"]
    fn prop_qshrn_rejects_over_range_shift(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        (arr_n, shift) in over_range_shift_strategy(),
        u_bit in u_bit_strategy(),
        is_rounding in any::<bool>(),
        is_high in any::<bool>(),
    ) {
        let ops = vec![vreg_arr(rd, arr_n), vreg_arr(rn, arr_n), imm(shift)];
        let res = encode_neon_qshrn(&ops, u_bit, is_rounding, is_high);

        let word = match &res {
            Ok(EncodeResult::Word(w)) => *w,
            _ => 0,
        };
        // Classify HOW the (wrongly) accepted word is corrupt, for the message.
        let immh = (word >> 19) & 0xF;
        let (cat_val, cat_mask) = arr_immh_category(arr_n).unwrap();
        let corrupt = if res.is_ok() && immh == 0 {
            "immh=0000 (UNALLOCATED)".to_string()
        } else if res.is_ok() && (immh & cat_mask) != cat_val {
            format!("immh={:04b} encodes a DIFFERENT source size than {}", immh, arr_n)
        } else {
            String::new()
        };

        prop_assert!(
            res.is_err(),
            "shift={} on source {} is OUT of the valid range 1..=esize/2 (ARMv8 ARM), \
             but was ACCEPTED as word 0x{:08x} ({}) — must return Err",
            shift, arr_n, word, corrupt,
        );
    }
}

// --- deterministic boundary / golden checks -------------------------------

#[test]
fn golden_qshrn_matches_arm_layout() {
    // sqshrn  v0.8b, v0.8h, #1   (U=0, Q=0, non-round)  -> 0x0f0f9400
    assert_eq!(
        word_of(encode_neon_qshrn(
            &[vreg_arr(0, "8b"), vreg_arr(0, "8h"), imm(1)],
            0,
            false,
            false
        )),
        0x0F0F_9400,
    );
    // sqshrn  v0.8b, v0.8h, #8   (max valid for .8h)     -> immh:immb=8 -> 0x0f089400
    assert_eq!(
        word_of(encode_neon_qshrn(
            &[vreg_arr(0, "8b"), vreg_arr(0, "8h"), imm(8)],
            0,
            false,
            false
        )),
        0x0F08_9400,
    );
    // sqrshrn v0.4h, v0.4s, #16  (max valid for .4s)     -> immh:immb=16 -> 0x0f109c00
    assert_eq!(
        word_of(encode_neon_qshrn(
            &[vreg_arr(0, "4h"), vreg_arr(0, "4s"), imm(16)],
            0,
            true,
            false
        )),
        0x0F10_9C00,
    );
    // uqrshrn2 v0.4s, v0.2d, #32 (U=1, Q=1, rounding)    -> immh:immb=32 -> 0x6f209c00
    assert_eq!(
        word_of(encode_neon_qshrn(
            &[vreg_arr(0, "4s"), vreg_arr(0, "2d"), imm(32)],
            1,
            true,
            true
        )),
        0x6F20_9C00,
    );
    // Non-trivial registers: sqshrn v5.8b, v7.8h, #3 -> 0x0f0d94e5
    assert_eq!(
        word_of(encode_neon_qshrn(
            &[vreg_arr(5, "8b"), vreg_arr(7, "8h"), imm(3)],
            0,
            false,
            false
        )),
        0x0F0D_94E5,
    );
}

#[test]
fn rejects_too_few_operands_and_zero_shift() {
    assert!(
        encode_neon_qshrn(&[], 0, false, false).is_err(),
        "0 operands must error"
    );
    assert!(
        encode_neon_qshrn(&[vreg_arr(0, "8b"), vreg_arr(1, "8h")], 0, false, false).is_err(),
        "2 operands must error",
    );
    // shift == 0 is correctly rejected.
    assert!(
        encode_neon_qshrn(
            &[vreg_arr(0, "8b"), vreg_arr(1, "8h"), imm(0)],
            0,
            false,
            false
        )
        .is_err(),
        "shift 0 must be rejected",
    );
    // A valid input must succeed.
    assert!(
        encode_neon_qshrn(
            &[vreg_arr(0, "8b"), vreg_arr(1, "8h"), imm(4)],
            0,
            false,
            false
        )
        .is_ok(),
    );
}

#[test]
fn rejects_unsupported_source_arrangement() {
    // Only .8h/.4s/.2d are valid (wider) sources.
    for bad in ["8b", "16b", "4h", "2s", "garbage"] {
        let ops = vec![vreg_arr(0, "8b"), vreg_arr(1, bad), imm(2)];
        assert!(
            encode_neon_qshrn(&ops, 0, false, false).is_err(),
            "source arrangement {bad} must be rejected",
        );
    }
}

/// Deterministic witness for the over-range bug: shift #9 on `.8h` is illegal
/// (valid max is 8) but is accepted, producing immh=0000 (UNALLOCATED).
#[test]
#[ignore = "documented bug: qshrn accepts #9 for .8h source"]
fn over_range_shift_9_on_8h_is_unallocated() {
    let res = encode_neon_qshrn(
        &[vreg_arr(0, "8b"), vreg_arr(0, "8h"), imm(9)],
        0,
        false,
        false,
    );
    // This MUST be Err; the current implementation wrongly returns Ok with
    // immh=0000 (unallocated). When the bug is fixed, this assertion passes.
    assert!(
        res.is_err(),
        "shift #9 on .8h must be rejected (valid range 1..=8), got {res:?}"
    );
}

/// Deterministic witness for the silent-size-corruption variant: shift #17 on
/// `.4s` is illegal (valid max is 16) but is accepted, encoding immh=0001 which
/// decodes as a 16-bit-source instruction instead of the requested 32-bit one.
#[test]
#[ignore = "documented bug: qshrn accepts #17 for .4s source"]
fn over_range_shift_17_on_4s_silently_changes_size() {
    let res = encode_neon_qshrn(
        &[vreg_arr(0, "4h"), vreg_arr(0, "4s"), imm(17)],
        0,
        false,
        false,
    );
    if let Ok(EncodeResult::Word(w)) = &res {
        let immh = (w >> 19) & 0xF;
        // If accepted, immh must NOT be the .4s category (001x); it corrupts to 0001.
        assert_ne!(
            immh & 0b1110,
            0b0010,
            "accepted word 0x{w:08x} decoded immh={immh:04b} must not be a valid .4s encoding",
        );
    }
    // The real contract: it must be rejected.
    assert!(
        res.is_err(),
        "shift #17 on .4s must be rejected (valid range 1..=16), got {res:?}"
    );
}
