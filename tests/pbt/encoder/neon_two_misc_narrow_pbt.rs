//! Property-based tests for `encode_neon_two_misc_narrow`
//! (the AArch64 "Advanced SIMD two-register miscellaneous *narrowing*" encoder:
//! XTN, XTN2, UQXTN/UQXTN2, SQXTN/SQXTN2, SQXTUN/SQXTUN2).
//!
//! Encoding (ARMv8 ARM, "Advanced SIMD two-register miscellaneous", narrow group):
//!   `0 Q U 01110 size 10000 opcode 10 Rn Rd`
//!    31 30 29 28-24 23-22 21-17 16-12 11-10 9-5 4-0
//!
//! CRUCIAL DIFFERENCE from the non-narrow `encode_neon_two_misc`:
//!   * bit 30 (Q) is driven by the `is_high` parameter (XTN2/UQXTN2/...),
//!     NOT by any arrangement specifier.
//!   * bits 23-22 (size) come from the SOURCE (2nd) operand arrangement:
//!     .8h -> 00, .4s -> 01, .2d -> 10  (the wider source)
//!   * The DESTINATION (1st) operand's arrangement is read then DISCARDED
//!     (bound to `_arr_d`); only its register number is used.
//!
//! Field widths (the contract this encoder must uphold for VALID inputs):
//!   * bit 31     : 0          (constant — only holds while `u_bit <= 1`)
//!   * bit 30 (Q) : `is_high`
//!   * bit 29 (U) : `u_bit` parameter — must be 0 or 1
//!   * bits 28-24 : 01110      (constant)
//!   * bits 23-22 (size): from source arrangement
//!   * bits 21-17 : 10000      (constant)
//!   * bits 16-12 (opcode): `opcode` parameter — must be a 5-bit value (0..=0x1F)
//!   * bits 11-10 : 10         (constant)
//!   * bits 9-5 (Rn), 4-0 (Rd): register numbers (0..=31, guaranteed by parse_reg_num)
//!
//! Golden words cross-checked against the ARMv8 ARM layout:
//!   `xtn  v0.8b,  v0.8h` => 0x0e212800   (U=0, opcode=10010, Q=0, size=00)
//!   `xtn2 v0.16b, v0.8h` => 0x4e212800   (Q=1)
//!   `xtn  v5.4h,  v7.4s` => 0x0e6128e5   (Q=0, size=01, Rn=7, Rd=5)
//!   `uqxtn v0.8b, v0.8h` => 0x2e212800   (U=1)
//!
//! NOTE on findings:
//!  * `prop_narrow_dest_arrangement_ignored_and_q_from_is_high` documents that
//!    the DESTINATION arrangement is discarded — any arrangement (even a bare
//!    `Operand::Reg` with none, or a width-mismatched one) is silently
//!    accepted. ARM constrains dest width from the source size, but the encoder
//!    does not check.
//!  * `prop_narrow_rejects_out_of_range_u_bit_and_opcode` is a NEGATIVE-CONTRACT
//!    property: it asserts that out-of-range `u_bit` (>1) or `opcode` (>0x1F)
//!    are rejected with `Err`, because they silently overflow into the Q bit /
//!    bit 31 / the fixed `01110`/`10000` fields and produce a corrupt
//!    instruction. It is EXPECTED TO FAIL with the current implementation,
//!    which performs no range validation on either parameter.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_two_misc_narrow;
use proptest::prelude::*;

// --- helpers --------------------------------------------------------------

fn vreg_arr(reg: u32, arr: &str) -> Operand {
    Operand::RegArrangement {
        reg: format!("v{reg}"),
        arrangement: arr.to_string(),
    }
}

fn reg_num_strategy() -> impl Strategy<Value = u32> {
    0u32..=31u32
}

fn u_bit_strategy() -> impl Strategy<Value = u32> {
    0u32..=1u32
}

/// Valid 5-bit opcode range for this encoding group.
fn opcode_strategy() -> impl Strategy<Value = u32> {
    0u32..=0x1Fu32
}

/// Only the SOURCE arrangements accepted by this narrow encoder.
fn src_arr_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("8h"), Just("4s"), Just("2d")]
}

/// Arbitrary arrangements — used to demonstrate the destination arrangement is
/// ignored. Includes widths that are semantically wrong for a narrowing op.
fn any_arr_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![
        Just("8b"),
        Just("16b"),
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

/// INDEPENDENT oracle: SOURCE arrangement -> size, hardcoded here (not calling
/// the SUT's match arm) so the reference does not share code with the encoder.
fn arr_n_to_size(arr: &str) -> Option<u32> {
    Some(match arr {
        "8h" => 0b00,
        "4s" => 0b01,
        "2d" => 0b10,
        _ => return None,
    })
}

/// Reference encoding built straight from the documented layout. Q comes from
/// `is_high`, size from the SOURCE arrangement.
fn ref_word(rd: u32, rn: u32, arr_n: &str, u_bit: u32, opcode: u32, is_high: bool) -> u32 {
    let q = if is_high { 1u32 } else { 0u32 };
    let size = arr_n_to_size(arr_n).unwrap();
    (q << 30)
        | (u_bit << 29)
        | (0b01110 << 24)
        | (size << 22)
        | (0b10000 << 17)
        | (opcode << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd
}

/// Constant fixed bits of this encoding (bit31 + 01110 + 10000 + 10 fields),
/// independent of all VALID inputs.
const FIXED_MASK: u32 = 0x8000_0000 | 0x1F00_0000 | 0x003E_0000 | 0x0000_0C00;
const FIXED_CONST: u32 = 0x0E00_0000 | 0x0020_0000 | 0x0000_0800; // bit 31 contributes 0

// --- properties -----------------------------------------------------------

proptest! {
    /// Reference oracle: for all valid inputs the encoded word equals the word
    /// built directly from the ARMv8 ARM layout (Q from is_high, size from src).
    #[test]
    fn prop_narrow_matches_arm_layout(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr_n in src_arr_strategy(),
        u_bit in u_bit_strategy(),
        opcode in opcode_strategy(),
        is_high in any::<bool>(),
    ) {
        let ops = vec![vreg_arr(rd, arr_n), vreg_arr(rn, arr_n)];
        let word = word_of(encode_neon_two_misc_narrow(&ops, u_bit, opcode, is_high));
        prop_assert_eq!(word, ref_word(rd, rn, arr_n, u_bit, opcode, is_high));
    }

    /// Field isolation: Rd is exactly bits [4:0], Rn exactly bits [9:5],
    /// opcode exactly bits [16:12], and size exactly bits [23:22]. No field
    /// leaks into any other. Q (bit 30) must be the `is_high` bit and must not
    /// be perturbed by the register numbers.
    #[test]
    fn prop_narrow_fields_isolated(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr_n in src_arr_strategy(),
        u_bit in u_bit_strategy(),
        opcode in opcode_strategy(),
        is_high in any::<bool>(),
    ) {
        let base = word_of(encode_neon_two_misc_narrow(
            &[vreg_arr(0, arr_n), vreg_arr(0, arr_n)], u_bit, 0, is_high,
        ));

        // Rd occupies bits [4:0] only.
        let with_rd = word_of(encode_neon_two_misc_narrow(
            &[vreg_arr(rd, arr_n), vreg_arr(0, arr_n)], u_bit, 0, is_high,
        ));
        prop_assert_eq!(with_rd & 0x1F, rd, "Rd not in bits [4:0]");
        prop_assert_eq!(with_rd & !0x1F, base & !0x1F, "Rd leaked above bit 4");

        // Rn occupies bits [9:5] only.
        let with_rn = word_of(encode_neon_two_misc_narrow(
            &[vreg_arr(0, arr_n), vreg_arr(rn, arr_n)], u_bit, 0, is_high,
        ));
        prop_assert_eq!((with_rn >> 5) & 0x1F, rn, "Rn not in bits [9:5]");
        prop_assert_eq!(with_rn & !0x3E0, base & !0x3E0, "Rn leaked outside bits [9:5]");

        // opcode occupies bits [16:12] only.
        let with_op = word_of(encode_neon_two_misc_narrow(
            &[vreg_arr(0, arr_n), vreg_arr(0, arr_n)], u_bit, opcode, is_high,
        ));
        prop_assert_eq!((with_op >> 12) & 0x1F, opcode, "opcode not in bits [16:12]");
        prop_assert_eq!(with_op & !0x1F000, base & !0x1F000, "opcode leaked outside bits [16:12]");
    }

    /// Fixed bits are constant for all valid inputs; Q is exactly `is_high`,
    /// U is exactly `u_bit`, and size is derived from the SOURCE arrangement.
    #[test]
    fn prop_narrow_fixed_bits_and_qu_size(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr_n in src_arr_strategy(),
        u_bit in u_bit_strategy(),
        opcode in opcode_strategy(),
        is_high in any::<bool>(),
    ) {
        let word = word_of(encode_neon_two_misc_narrow(
            &[vreg_arr(rd, arr_n), vreg_arr(rn, arr_n)], u_bit, opcode, is_high,
        ));

        // Constant fields must match regardless of inputs.
        prop_assert_eq!(word & FIXED_MASK, FIXED_CONST, "fixed bits mutated by inputs");

        // Q (bit 30) is `is_high`, U (bit 29) is `u_bit`, size (bits 23:22) is from src.
        let q = if is_high { 1u32 } else { 0u32 };
        let size = arr_n_to_size(arr_n).unwrap();
        prop_assert_eq!((word >> 30) & 1, q, "Q bit must equal is_high");
        prop_assert_eq!((word >> 29) & 1, u_bit, "U bit wrong");
        prop_assert_eq!((word >> 22) & 0x3, size, "size bits wrong (must come from source arr)");
    }

    /// FINDING: the DESTINATION register's arrangement is read then discarded
    /// (bound to `_arr_d`), and Q comes from `is_high`, NOT from the dest
    /// arrangement. Varying the dest arrangement — even to a bare `Operand::Reg`
    /// with no arrangement, or to a width that mismatches the source — leaves
    /// the encoded word identical. ARM constrains dest width from the source
    /// size, so mismatches are silently accepted rather than rejected.
    /// A bare register as the SOURCE, however, has no arrangement and is correctly rejected.
    #[test]
    fn prop_narrow_dest_arrangement_ignored_and_q_from_is_high(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr_n in src_arr_strategy(),
        dest_arr in any_arr_strategy(),
        u_bit in u_bit_strategy(),
        opcode in opcode_strategy(),
        is_high in any::<bool>(),
    ) {
        let dest = vreg_arr(rd, dest_arr);
        let ref_w = word_of(encode_neon_two_misc_narrow(
            &[dest.clone(), vreg_arr(rn, arr_n)], u_bit, opcode, is_high,
        ));

        // Any destination arrangement (even one differing from source) encodes the same.
        let varied = word_of(encode_neon_two_misc_narrow(
            &[vreg_arr(rd, if dest_arr == "8b" { "2d" } else { "8b" }), vreg_arr(rn, arr_n)],
            u_bit, opcode, is_high,
        ));
        prop_assert_eq!(varied, ref_w, "dest arrangement unexpectedly changed encoding");

        // A bare destination register (no arrangement) must also encode identically.
        let bare = word_of(encode_neon_two_misc_narrow(
            &[Operand::Reg(format!("v{rd}")), vreg_arr(rn, arr_n)], u_bit, opcode, is_high,
        ));
        prop_assert_eq!(bare, ref_w, "bare-Reg dest changed encoding");

        // Q must equal is_high regardless of dest arrangement; toggling is_high flips Q.
        let w_lo = word_of(encode_neon_two_misc_narrow(&[dest, vreg_arr(rn, arr_n)], u_bit, opcode, false));
        let w_hi = word_of(encode_neon_two_misc_narrow(&[vreg_arr(rd, dest_arr), vreg_arr(rn, arr_n)], u_bit, opcode, true));
        prop_assert_eq!(w_lo & (1 << 30), 0, "is_high=false must clear Q");
        prop_assert_eq!(w_hi & (1 << 30), 1 << 30, "is_high=true must set Q");

        // Conversely, a bare SOURCE register (no arrangement) cannot yield a size -> must error.
        let bare_src = encode_neon_two_misc_narrow(&[vreg_arr(rd, arr_n), Operand::Reg(format!("v{rn}"))], u_bit, opcode, is_high);
        prop_assert!(bare_src.is_err(), "bare-Reg source must be rejected (no arrangement to derive size)");
    }

    /// NEGATIVE CONTRACT (EXPECTED TO FAIL): `u_bit` must be 0 or 1 and
    /// `opcode` must fit in 5 bits (0..=0x1F). Out-of-range values silently
    /// overflow — `u_bit == 2` collides with the Q bit (bit 30), `u_bit >= 4`
    /// collides with bit 31 (the required `0`), and `opcode > 0x1F` collides
    /// with the constant `10000`/`01110` fields — producing a corrupt
    /// instruction word. A defensive encoder must reject these. The current
    /// implementation performs no range validation, so this property fails and
    /// surfaces the missing contract check.
    #[test]
    #[ignore = "documented bug: out-of-range u_bit/opcode are not rejected"]
    fn prop_narrow_rejects_out_of_range_u_bit_and_opcode(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr_n in src_arr_strategy(),
        bad_u in 2u32..=0xFFu32,
        bad_opcode in 0x20u32..=0xFFFFu32,
        is_high in any::<bool>(),
    ) {
        let ops = vec![vreg_arr(rd, arr_n), vreg_arr(rn, arr_n)];

        let res_u = encode_neon_two_misc_narrow(&ops, bad_u, 0, is_high);
        prop_assert!(
            res_u.is_err(),
            "u_bit={} (>1) must be rejected, but encoded as {:?} (overflows Q/bit31)",
            bad_u, res_u,
        );

        let res_op = encode_neon_two_misc_narrow(&ops, 0, bad_opcode, is_high);
        prop_assert!(
            res_op.is_err(),
            "opcode=0x{:x} (>0x1F) must be rejected, but encoded as {:?} (overflows fixed fields)",
            bad_opcode, res_op,
        );
    }
}

// --- deterministic boundary / golden checks -------------------------------

#[test]
fn golden_narrow_matches_arm_layout() {
    // xtn v0.8b, v0.8h  (U=0, opcode=10010, Q=0, size=00) -> 0x0e212800
    assert_eq!(
        word_of(encode_neon_two_misc_narrow(
            &[vreg_arr(0, "8b"), vreg_arr(0, "8h")],
            0,
            0b10010,
            false
        )),
        0x0E21_2800,
    );
    // xtn2 v0.16b, v0.8h  (Q=1) -> 0x4e212800
    assert_eq!(
        word_of(encode_neon_two_misc_narrow(
            &[vreg_arr(0, "16b"), vreg_arr(0, "8h")],
            0,
            0b10010,
            true
        )),
        0x4E21_2800,
    );
    // xtn v5.4h, v7.4s  (Q=0, size=01, Rn=7, Rd=5) -> 0x0e6128e5
    assert_eq!(
        word_of(encode_neon_two_misc_narrow(
            &[vreg_arr(5, "4h"), vreg_arr(7, "4s")],
            0,
            0b10010,
            false
        )),
        0x0E61_28E5,
    );
    // uqxtn v0.8b, v0.8h  (U=1, opcode=10010, Q=0, size=00) -> 0x2e212800
    assert_eq!(
        word_of(encode_neon_two_misc_narrow(
            &[vreg_arr(0, "8b"), vreg_arr(0, "8h")],
            1,
            0b10010,
            false
        )),
        0x2E21_2800,
    );
    // xtn v0.2s, v0.2d  (Q=0, size=10) -> 0x0ea12800
    assert_eq!(
        word_of(encode_neon_two_misc_narrow(
            &[vreg_arr(0, "2s"), vreg_arr(0, "2d")],
            0,
            0b10010,
            false
        )),
        0x0EA1_2800,
    );
}

#[test]
fn rejects_too_few_operands() {
    assert!(
        encode_neon_two_misc_narrow(&[], 0, 0b10010, false).is_err(),
        "0 operands must error"
    );
    assert!(
        encode_neon_two_misc_narrow(&[vreg_arr(0, "8b")], 0, 0b10010, false).is_err(),
        "1 operand must error",
    );
    // Exactly two valid operands must succeed.
    assert!(
        encode_neon_two_misc_narrow(&[vreg_arr(0, "8b"), vreg_arr(1, "8h")], 0, 0b10010, false)
            .is_ok(),
    );
}

#[test]
fn rejects_unsupported_source_arrangement() {
    // A source arrangement this narrow encoder does not recognise must error.
    // (8b/16b/4h/2s/4s... only 8h/4s/2d are valid wider sources.)
    let ops = vec![vreg_arr(0, "8b"), vreg_arr(1, "8b")];
    assert!(encode_neon_two_misc_narrow(&ops, 0, 0b10010, false).is_err());
    let ops2 = vec![vreg_arr(0, "8b"), vreg_arr(1, "garbage")];
    assert!(encode_neon_two_misc_narrow(&ops2, 0, 0b10010, false).is_err());
}
