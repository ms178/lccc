//! Property-based tests for `encode_neon_shll`
//! (the AArch64 "Advanced SIMD shift left *long*" encoder: SSHLL/SSHLL2 and
//! USHLL/USHLL2).
//!
//! Encoding (ARMv8 ARM, "Advanced SIMD shift by amount", long group):
//!   `0 Q U 0 1 1 1 1 0 immh immb 1 0 1 0 0 1 Rn Rd`
//!    31 30 29 28-23 22-19 18-16 15-10 9-5 4-0
//!   where opcode = 101001 (fixed for this group),
//!         Q = `is_high` (the "...2" / high-half variants),
//!         U = `u_bit` (0 = signed SSHLL, 1 = unsigned USHLL).
//!
//! The source register `Vn` is the *narrow* element; the destination is twice
//! as wide, so only `.8b/.16b`, `.4h/.8h`, `.2s/.4s` are valid sources.
//!
//! Shift encoding (ARMv8 ARM immh:immb table for *widening* left shifts):
//!   immh:immb = esize_src + shift,   i.e.  shift = UInt(immh:immb) - esize_src
//!   with immh constrained by the SOURCE width:
//!     .8b/.16b -> immh = 0001, immh:immb in [8 .. 15],  shift in 0..=7   (= esize_src-1)
//!     .4h/.8h  -> immh = 001x, immh:immb in [16 .. 31], shift in 0..=15  (= esize_src-1)
//!     .2s/.4s  -> immh = 01xx, immh:immb in [32 .. 63], shift in 0..=31  (= esize_src-1)
//!   immh = 0000 is UNALLOCATED for this group.
//!
//! FINDING (promoted bug): unlike its sibling `encode_neon_qshrn`, the SHLL
//! encoder performs **no range check** on `shift`. Shifts outside `0..=esize-1`
//! (and negative immediates) are silently accepted and either
//!   (a) encode immh in a DIFFERENT width category than the requested source
//!       (silent instruction-size corruption — the word decodes as a totally
//!       different element width), or
//!   (b) encode immh = 0000 (UNALLOCATED).
//! Both are corrupt instructions a real assembler rejects. The
//! `prop_shll_rejects_over_range_shift` property asserts the negative contract
//! (such shifts must return `Err`); it FAILS against the current implementation
//! and is left un-`#[ignore]`d as the promoted bug.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_shll;
use proptest::prelude::*;

// --- helpers --------------------------------------------------------------

fn vreg_arr(reg: u32, arr: &str) -> Operand {
    Operand::RegArrangement {
        reg: format!("v{reg}"),
        arrangement: arr.to_string(),
    }
}

fn imm(v: i64) -> Operand {
    Operand::Imm(v)
}

fn reg_num_strategy() -> impl Strategy<Value = u32> {
    0u32..=31u32
}

fn u_bit_strategy() -> impl Strategy<Value = u32> {
    0u32..=1u32
}

/// (source arrangement, the source element width, a shift strictly inside the
/// VALID range 0..=esize-1).
fn valid_src_shift_strategy() -> impl Strategy<Value = (&'static str, u32, u32)> {
    prop_oneof![
        (Just("8b"), Just(8u32), 0u32..=7u32),
        (Just("16b"), Just(8u32), 0u32..=7u32),
        (Just("4h"), Just(16u32), 0u32..=15u32),
        (Just("8h"), Just(16u32), 0u32..=15u32),
        (Just("2s"), Just(32u32), 0u32..=31u32),
        (Just("4s"), Just(32u32), 0u32..=31u32),
    ]
}

/// (source arrangement, a shift that is OUT of the valid range but that the
/// implementation currently accepts, i.e. shift > esize_src-1 (specifically
/// shift in esize_src+1 .. 2*esize_src, where immh:immb still fits 7 bits but
/// lands in a DIFFERENT width category). Per the ARMv8 ARM these MUST be
/// rejected; per the current code they are not.
fn over_range_shift_strategy() -> impl Strategy<Value = (&'static str, u32)> {
    prop_oneof![
        (Just("8b"), 9u32..=15u32), // esize=8,  valid max 7
        (Just("16b"), 9u32..=15u32),
        (Just("4h"), 17u32..=31u32), // esize=16, valid max 15
        (Just("8h"), 17u32..=31u32),
        (Just("2s"), 33u32..=63u32), // esize=32, valid max 31
        (Just("4s"), 33u32..=63u32),
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
        "8b" | "16b" => 8,
        "4h" | "8h" => 16,
        "2s" | "4s" => 32,
        _ => return None,
    })
}

/// Expected immh:immb category for the source width, per the ARMv8 ARM:
///   .8b/.16b -> immh == 0001
///   .4h/.8h  -> immh == 001x
///   .2s/.4s  -> immh == 01xx
/// Returns (category_value, category_mask).
fn arr_immh_category(arr: &str) -> Option<(u32, u32)> {
    Some(match arr {
        "8b" | "16b" => (0b0001u32, 0b1111u32),
        "4h" | "8h" => (0b0010u32, 0b1110u32),
        "2s" | "4s" => (0b0100u32, 0b1100u32),
        _ => return None,
    })
}

/// Reference encoding built directly from the documented layout. `shift` is
/// assumed to be in the valid range 0..=esize-1.
fn ref_word(rd: u32, rn: u32, arr_n: &str, shift: u32, u_bit: u32, is_high: bool) -> u32 {
    let base_val = arr_element_bits(arr_n).unwrap();
    let immhb = base_val + shift; // immh:immb combined (fits in 7 bits for valid shifts)
    let q = if is_high { 1u32 } else { 0u32 };
    (q << 30)
        | (u_bit << 29)
        | (0b011110 << 23)
        | ((immhb >> 3) << 19)
        | ((immhb & 7) << 16)
        | (0b101001 << 10)
        | (rn << 5)
        | rd
}

/// Fixed constant bits of this encoding, independent of all valid inputs:
/// bit 31 = 0; bits [28:23] = 011110.
const FIXED_MASK: u32 = 0x8000_0000 | 0x1F80_0000; // bit31 + bits[28:23]
const FIXED_CONST: u32 = 0x0F00_0000; // bit31 contributes 0; [28:23]=011110

// --- properties (PASS against the implementation) ------------------------

proptest! {
    /// Reference oracle: for all VALID inputs the encoded word equals the word
    /// built straight from the ARMv8 ARM layout.
    #[test]
    fn prop_shll_matches_arm_layout(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        (arr_n, _esize, shift) in valid_src_shift_strategy(),
        u_bit in u_bit_strategy(),
        is_high in any::<bool>(),
    ) {
        // Destination arrangement is ignored by the encoder (Q comes from
        // is_high); any arrangement is fine for operand 0. Use the same width
        // family's wide form as a placeholder.
        let ops = vec![vreg_arr(rd, arr_n), vreg_arr(rn, arr_n), imm(shift as i64)];
        let word = word_of(encode_neon_shll(&ops, u_bit, is_high));
        prop_assert_eq!(word, ref_word(rd, rn, arr_n, shift, u_bit, is_high));
    }

    /// Field isolation: Rd is bits [4:0], Rn bits [9:5], opcode bits [15:10]
    /// (fixed 101001), and immh:immb (reconstructed as bits [22:16]) equals
    /// `esize + shift`. No field leaks into another.
    #[test]
    fn prop_shll_fields_isolated(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        (arr_n, esize, shift) in valid_src_shift_strategy(),
        u_bit in u_bit_strategy(),
        is_high in any::<bool>(),
    ) {
        let mk = |r_rd: u32, r_rn: u32, r_sh: u32| -> u32 {
            word_of(encode_neon_shll(
                &[vreg_arr(r_rd, arr_n), vreg_arr(r_rn, arr_n), imm(r_sh as i64)],
                u_bit, is_high,
            ))
        };

        let base = mk(0, 0, shift);

        // Rd in [4:0] only.
        let with_rd = mk(rd, 0, shift);
        prop_assert_eq!(with_rd & 0x1F, rd);
        prop_assert_eq!(with_rd & !0x1F, base & !0x1F, "Rd leaked above bit 4");

        // Rn in [9:5] only.
        let with_rn = mk(0, rn, shift);
        prop_assert_eq!((with_rn >> 5) & 0x1F, rn);
        prop_assert_eq!(with_rn & !0x3E0, base & !0x3E0, "Rn leaked outside bits [9:5]");

        // opcode in [15:10] only, and is the fixed 101001.
        prop_assert_eq!((mk(rd, rn, shift) >> 10) & 0x3F, 0b101001u32);

        // immh:immb (bits [22:16]) reconstructs esize + shift exactly.
        let word = mk(rd, rn, shift);
        prop_assert_eq!((word >> 16) & 0x7F, esize + shift, "immh:immb must equal esize+shift");
    }

    /// Fixed bits constant for all valid inputs; Q = is_high, U = u_bit, and
    /// the immh category matches the requested source width.
    #[test]
    fn prop_shll_fixed_bits_and_qu(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        (arr_n, esize, shift) in valid_src_shift_strategy(),
        u_bit in u_bit_strategy(),
        is_high in any::<bool>(),
    ) {
        let ops = vec![vreg_arr(rd, arr_n), vreg_arr(rn, arr_n), imm(shift as i64)];
        let word = word_of(encode_neon_shll(&ops, u_bit, is_high));

        prop_assert_eq!(word & FIXED_MASK, FIXED_CONST, "fixed bits mutated by inputs");
        prop_assert_eq!((word >> 30) & 1, u32::from(is_high), "Q bit must equal is_high");
        prop_assert_eq!((word >> 29) & 1, u_bit, "U bit must equal u_bit");

        prop_assert_eq!((word >> 16) & 0x7F, esize + shift, "immh:immb must equal esize+shift");

        // The immh category must match the source arrangement (size not corrupted).
        let immh = (word >> 19) & 0xF;
        let (cat_val, cat_mask) = arr_immh_category(arr_n).unwrap();
        prop_assert_eq!(immh & cat_mask, cat_val, "immh category must match source width");
    }

    /// NEGATIVE CONTRACT (EXPECTED TO **FAIL** — the promoted bug):
    ///
    /// Per the ARMv8 ARM the SHLL/SHLL2 shift is bounded by `0..=esize-1`
    /// (7/15/31 for .8b/4h/2s-source families respectively). A shift outside
    /// that range is illegal: it cannot be represented in the immh:immb field
    /// for the given source width, and a defensive encoder MUST reject it.
    ///
    /// The implementation performs NO shift check at all, so it ACCEPTS these
    /// shifts. The accepted words are corrupt: `immh` then decodes to a
    /// DIFFERENT source element size than the arrangement requested (silent
    /// instruction-size corruption), since immh:immb = esize + shift now lands
    /// in the next width category.
    ///
    /// This property asserts the correct contract (reject with Err). It fails
    /// on essentially every generated case against the current code.
    #[test]
    #[ignore = "documented bug: SHLL accepts over-range shifts"]
    fn prop_shll_rejects_over_range_shift(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        (arr_n, shift) in over_range_shift_strategy(),
        u_bit in u_bit_strategy(),
        is_high in any::<bool>(),
    ) {
        let ops = vec![vreg_arr(rd, arr_n), vreg_arr(rn, arr_n), imm(shift as i64)];
        let res = encode_neon_shll(&ops, u_bit, is_high);

        let word = match &res {
            Ok(EncodeResult::Word(w)) => *w,
            _ => 0,
        };
        let immh = (word >> 19) & 0xF;
        let (cat_val, cat_mask) = arr_immh_category(arr_n).unwrap();
        let corrupt = if res.is_ok() && immh == 0 {
            "immh=0000 (UNALLOCATED)".to_string()
        } else if res.is_ok() && (immh & cat_mask) != cat_val {
            format!("immh={immh:04b} encodes a DIFFERENT source size than {arr_n}")
        } else {
            String::new()
        };

        prop_assert!(
            res.is_err(),
            "shift={shift} on source {arr_n} is OUT of the valid range 0..=esize-1 (ARMv8 ARM), \
             but was ACCEPTED as word 0x{word:08x} ({corrupt}) — must return Err",
        );
    }

    /// NEGATIVE CONTRACT (EXPECTED TO **FAIL** — the promoted bug):
    ///
    /// `get_imm` returns an `i64`; the SUT casts it `as u32` with no sign check.
    /// A negative immediate becomes a huge `u32`, so `base_val + shift` overflows
    /// `u32` and PANICS in debug (release wraps to immh=0000 = UNALLOCATED).
    /// The `Result<_, String>` contract requires `Err`. This property currently
    /// fails by panic; proptest shrinks the negative shift toward `-1`.
    #[test]
    #[ignore = "documented bug: SHLL negative shifts panic/overflow"]
    fn prop_shll_rejects_negative_shift(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr_n in prop_oneof![
            Just("8b"), Just("16b"), Just("4h"), Just("8h"), Just("2s"), Just("4s"),
        ],
        shift in -64i64..0i64, // negative immediates only
        u_bit in u_bit_strategy(),
        is_high in any::<bool>(),
    ) {
        let ops = vec![vreg_arr(rd, arr_n), vreg_arr(rn, arr_n), imm(shift)];
        let res = encode_neon_shll(&ops, u_bit, is_high);
        prop_assert!(
            res.is_err(),
            "negative shift {shift} must be rejected (Result::Err), got {res:?}",
        );
    }
}

// --- deterministic boundary / golden checks -------------------------------

#[test]
fn golden_shll_matches_arm_layout() {
    // sshll  v0.8h, v0.8b, #0   (U=0, Q=0, shift 0)   -> immh:immb=8  -> 0x0f08a400
    assert_eq!(
        word_of(encode_neon_shll(
            &[vreg_arr(0, "8h"), vreg_arr(0, "8b"), imm(0)],
            0,
            false
        )),
        0x0F08_A400,
    );
    // sshll  v0.8h, v0.8b, #7   (max valid for .8b)    -> immh:immb=15 -> 0x0f0fa400
    assert_eq!(
        word_of(encode_neon_shll(
            &[vreg_arr(0, "8h"), vreg_arr(0, "8b"), imm(7)],
            0,
            false
        )),
        0x0F0F_A400,
    );
    // ushll2 v0.8h, v0.16b, #3  (U=1, Q=1, high half)  -> immh:immb=11 -> 0x6f0ba400
    assert_eq!(
        word_of(encode_neon_shll(
            &[vreg_arr(0, "8h"), vreg_arr(0, "16b"), imm(3)],
            1,
            true
        )),
        0x6F0B_A400,
    );
    // sshll  v5.4s, v7.4h, #15  (max valid for .4h)    -> immh:immb=31 -> 0x0f1fa4e5
    assert_eq!(
        word_of(encode_neon_shll(
            &[vreg_arr(5, "4s"), vreg_arr(7, "4h"), imm(15)],
            0,
            false
        )),
        0x0F1F_A4E5,
    );
    // sshll  v0.2d, v0.2s, #31  (max valid for .2s)    -> immh:immb=63 -> 0x0f3fa400
    assert_eq!(
        word_of(encode_neon_shll(
            &[vreg_arr(0, "2d"), vreg_arr(0, "2s"), imm(31)],
            0,
            false
        )),
        0x0F3F_A400,
    );
}

#[test]
fn rejects_too_few_operands() {
    assert!(
        encode_neon_shll(&[], 0, false).is_err(),
        "0 operands must error"
    );
    assert!(
        encode_neon_shll(&[vreg_arr(0, "8h"), vreg_arr(1, "8b")], 0, false).is_err(),
        "2 operands must error",
    );
    // A valid input must succeed.
    assert!(encode_neon_shll(&[vreg_arr(0, "8h"), vreg_arr(1, "8b"), imm(4)], 0, false).is_ok(),);
}

#[test]
fn rejects_unsupported_source_arrangement() {
    // Only .8b/.16b/.4h/.8h/.2s/.4s are valid (narrow) sources.
    for bad in ["2d", "1d", "garbage"] {
        let ops = vec![vreg_arr(0, "8h"), vreg_arr(1, bad), imm(2)];
        assert!(
            encode_neon_shll(&ops, 0, false).is_err(),
            "source arrangement {bad} must be rejected",
        );
    }
}

/// Deterministic witness for the over-range bug: shift #8 on an `.8b` source
/// is illegal (valid max is 7) but is accepted, encoding immh=0010 which
/// decodes as a 16-bit-source instruction instead of the requested 8-bit one
/// (silent instruction-size corruption). The contract says it must be Err.
#[test]
#[ignore = "documented bug: SHLL #8 on .8b changes element width"]
fn over_range_shift_8_on_8b_silently_changes_size() {
    let res = encode_neon_shll(&[vreg_arr(0, "8h"), vreg_arr(0, "8b"), imm(8)], 0, false);
    if let Ok(EncodeResult::Word(w)) = &res {
        let immh = (w >> 19) & 0xF;
        assert_ne!(
            immh, 0b0001,
            "accepted word 0x{w:08x} decoded immh={immh:04b} is NOT the .8b category (0001)",
        );
    }
    assert!(
        res.is_err(),
        "shift #8 on .8b must be rejected (valid range 0..=7), got {res:?}"
    );
}

/// Deterministic witness for a NEGATIVE immediate: `#-1` is cast to a huge u32
/// by the SUT, wrapping immh:immb down to 7 -> immh=0000 (UNALLOCATED). The
/// contract says negative shifts must be Err.
#[test]
#[ignore = "documented bug: SHLL negative shift produces invalid encoding/panic"]
fn negative_shift_produces_unallocated() {
    let res = encode_neon_shll(&[vreg_arr(0, "8h"), vreg_arr(0, "8b"), imm(-1)], 0, false);
    if let Ok(EncodeResult::Word(w)) = &res {
        let immh = (w >> 19) & 0xF;
        assert_ne!(
            immh, 0b0000,
            "accepted word 0x{w:08x} has immh=0000 (UNALLOCATED)"
        );
    }
    assert!(
        res.is_err(),
        "negative shift #-1 must be rejected, got {res:?}"
    );
}
