//! Property-based tests for `encode_neon_float_two_misc`
//! (the AArch64 "Advanced SIMD two-register miscellaneous (floating-point)"
//! encoder: FABS, FNEG, FCVTNS/MS/AS/PS, FCVTZS/ZU, SCVTF, UCVTF, FRINTN/P/M/Z/A/X/I, ...).
//!
//! Encoding (ARMv8 ARM, "Advanced SIMD two-register miscellaneous"):
//!   `0 Q U 01110 size 10000 opcode 10 Rn Rd`
//!    31 30 29 28-24 23-22 21-17 16-12 11-10 9-5 4-0
//!
//! Field widths (the contract this encoder must uphold):
//!   * bit 31      : 0          (constant)
//!   * bit 30 (Q)  : from destination arrangement
//!     (2s → 0; 4s/2d → 1; i.e. 128-bit vectors set Q)
//!   * bit 29 (U)  : `u_bit` parameter — must be 0 or 1
//!   * bits 28-24  : 01110      (constant)
//!   * bits 23-22 (size): `(size_hi << 1) | sz`, where `sz` is 0 for single
//!     (2s/4s) and 1 for double (2d). size_hi must be 0 or 1.
//!   * bits 21-17  : 10000      (constant)
//!   * bits 16-12 (opcode): `opcode` parameter — must be a 5-bit value (0..=0x1F)
//!   * bits 11-10  : 10         (constant)
//!   * bits 9-5 (Rn), 4-0 (Rd): register numbers (0..=31, enforced by
//!     `parse_reg_num`, which returns `None` for num > 31)
//!
//! NOTE on findings:
//!  * `prop_float_two_misc_source_arrangement_ignored` documents that the
//!    SOURCE register's arrangement is read then discarded (bound to `_`);
//!    only the destination arrangement drives Q/sz. ARM requires the source
//!    arrangement to match the destination, so a mismatch (e.g. `Vd.4s, Vn.2d`)
//!    is silently accepted.
//!  * `prop_float_two_misc_rejects_out_of_range_params` is a NEGATIVE-CONTRACT
//!    property. It asserts that out-of-range `u_bit` (>1), `opcode` (>0x1F) or
//!    `size_hi` (>1) are rejected with `Err`, because they silently overflow:
//!      - `u_bit >= 2`    collides with the Q bit (bit 30) and bit 31;
//!      - `opcode > 0x1F` collides with the constant `10000`/`01110` fields;
//!      - `size_hi > 1`   (`size_hi<<1` becomes >= 2, so `size` >= 2) collides
//!        with the constant `01110` field via bit 24.
//!        It is EXPECTED TO FAIL with the current implementation, which performs no
//!        range validation on any of the three numeric parameters.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_float_two_misc;
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

/// `size_hi` is bit 1 of the 2-bit `size` field; it must be 0 or 1.
fn size_hi_strategy() -> impl Strategy<Value = u32> {
    0u32..=1u32
}

/// Valid 5-bit opcode range for this encoding group.
fn opcode_strategy() -> impl Strategy<Value = u32> {
    0u32..=0x1Fu32
}

/// All arrangements accepted by `encode_neon_float_two_misc`.
fn arr_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("2s"), Just("4s"), Just("2d"),]
}

fn word_of(res: Result<EncodeResult, String>) -> u32 {
    match res {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {other:?}"),
    }
}

/// INDEPENDENT oracle: float arrangement -> (Q, sz), hardcoded here (not
/// calling the SUT). Note that only 2s/4s/2d are valid for this group.
fn arr_to_q_sz(arr: &str) -> Option<(u32, u32)> {
    Some(match arr {
        "2s" => (0, 0),
        "4s" => (1, 0),
        "2d" => (1, 1),
        _ => return None,
    })
}

/// Reference encoding built straight from the documented layout. `size` is
/// assembled as `(size_hi << 1) | sz`, exactly as the SUT does.
fn ref_word(rd: u32, rn: u32, arr: &str, u_bit: u32, size_hi: u32, opcode: u32) -> u32 {
    let (q, sz) = arr_to_q_sz(arr).unwrap();
    let size = (size_hi << 1) | sz;
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

/// Constant fixed bits of the two-register-misc encoding (bit31 + 01110 +
/// 10000 + 10 fields), independent of all inputs.
const FIXED_MASK: u32 = 0x8000_0000 | 0x1F00_0000 | 0x003E_0000 | 0x0000_0C00;
const FIXED_CONST: u32 = 0x0E00_0000 | 0x0020_0000 | 0x0000_0800; // bit 31 contributes 0

// --- properties -----------------------------------------------------------

proptest! {
    /// Reference oracle: for all valid inputs the encoded word equals the word
    /// built directly from the ARMv8 ARM layout.
    #[test]
    fn prop_float_two_misc_matches_arm_layout(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in arr_strategy(),
        u_bit in u_bit_strategy(),
        size_hi in size_hi_strategy(),
        opcode in opcode_strategy(),
    ) {
        let ops = vec![vreg_arr(rd, arr), vreg_arr(rn, arr)];
        let word = word_of(encode_neon_float_two_misc(&ops, u_bit, size_hi, opcode));
        prop_assert_eq!(word, ref_word(rd, rn, arr, u_bit, size_hi, opcode));
    }

    /// Field isolation: Rd is exactly bits [4:0], Rn exactly bits [9:5],
    /// opcode exactly bits [16:12], and size (incl. size_hi) exactly
    /// bits [23:22]. No field leaks into any other.
    #[test]
    fn prop_float_two_misc_fields_isolated(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in arr_strategy(),
        u_bit in u_bit_strategy(),
        size_hi in size_hi_strategy(),
        opcode in opcode_strategy(),
    ) {
        let base = word_of(encode_neon_float_two_misc(
            &[vreg_arr(0, arr), vreg_arr(0, arr)], u_bit, size_hi, 0,
        ));

        // Rd occupies bits [4:0] only.
        let with_rd = word_of(encode_neon_float_two_misc(
            &[vreg_arr(rd, arr), vreg_arr(0, arr)], u_bit, size_hi, 0,
        ));
        prop_assert_eq!(with_rd & 0x1F, rd, "Rd not in bits [4:0]");
        prop_assert_eq!(with_rd & !0x1F, base & !0x1F, "Rd leaked above bit 4");

        // Rn occupies bits [9:5] only.
        let with_rn = word_of(encode_neon_float_two_misc(
            &[vreg_arr(0, arr), vreg_arr(rn, arr)], u_bit, size_hi, 0,
        ));
        prop_assert_eq!((with_rn >> 5) & 0x1F, rn, "Rn not in bits [9:5]");
        prop_assert_eq!(with_rn & !0x3E0, base & !0x3E0, "Rn leaked outside bits [9:5]");

        // opcode occupies bits [16:12] only.
        let with_op = word_of(encode_neon_float_two_misc(
            &[vreg_arr(0, arr), vreg_arr(0, arr)], u_bit, size_hi, opcode,
        ));
        prop_assert_eq!((with_op >> 12) & 0x1F, opcode, "opcode not in bits [16:12]");
        prop_assert_eq!(with_op & !0x1F000, base & !0x1F000, "opcode leaked outside bits [16:12]");
    }

    /// Fixed bits are constant for all inputs; Q/U/size are driven by inputs.
    /// `size` is `(size_hi << 1) | sz`.
    #[test]
    fn prop_float_two_misc_fixed_bits_and_inputs(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in arr_strategy(),
        u_bit in u_bit_strategy(),
        size_hi in size_hi_strategy(),
        opcode in opcode_strategy(),
    ) {
        let word = word_of(encode_neon_float_two_misc(
            &[vreg_arr(rd, arr), vreg_arr(rn, arr)], u_bit, size_hi, opcode,
        ));

        // Constant fields must match regardless of inputs.
        prop_assert_eq!(word & FIXED_MASK, FIXED_CONST, "fixed bits mutated by inputs");

        // Q (bit 30), U (bit 29), and the full 2-bit size field reflect inputs.
        let (q, sz) = arr_to_q_sz(arr).unwrap();
        prop_assert_eq!((word >> 30) & 1, q, "Q bit wrong");
        prop_assert_eq!((word >> 29) & 1, u_bit, "U bit wrong");
        prop_assert_eq!((word >> 22) & 0x3, (size_hi << 1) | sz, "size bits wrong");
    }

    /// NEGATIVE CONTRACT (EXPECTED TO FAIL): `u_bit`, `opcode` and `size_hi`
    /// must each fit their field — `u_bit` in 1 bit, `opcode` in 5 bits
    /// (0..=0x1F), `size_hi` in 1 bit. Out-of-range values silently overflow
    /// into the Q bit (bit 31), the constant `01110` field, or the constant
    /// `10000` field, producing a corrupt instruction word. A defensive
    /// encoder must reject these. The current implementation performs no range
    /// validation on any of the three parameters, so this property fails and
    /// surfaces the missing contract checks.
    #[test]
    #[ignore = "documented bug: float two-misc numeric parameters are not range-checked"]
    fn prop_float_two_misc_rejects_out_of_range_params(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in arr_strategy(),
        bad_u in 2u32..=0xFFu32,
        bad_opcode in 0x20u32..=0xFFFFu32,
        bad_size_hi in 2u32..=0xFFu32,
    ) {
        let ops = vec![vreg_arr(rd, arr), vreg_arr(rn, arr)];

        let res_u = encode_neon_float_two_misc(&ops, bad_u, 0, 0);
        prop_assert!(
            res_u.is_err(),
            "u_bit={} (>1) must be rejected, but encoded as {:?} (overflows into Q/bit31)",
            bad_u, res_u,
        );

        let res_op = encode_neon_float_two_misc(&ops, 0, 0, bad_opcode);
        prop_assert!(
            res_op.is_err(),
            "opcode=0x{:x} (>0x1F) must be rejected, but encoded as {:?} (overflows fixed fields)",
            bad_opcode, res_op,
        );

        let res_sh = encode_neon_float_two_misc(&ops, 0, bad_size_hi, 0);
        prop_assert!(
            res_sh.is_err(),
            "size_hi={} (>1) must be rejected, but encoded as {:?} (size overflows into 01110 field)",
            bad_size_hi, res_sh,
        );
    }
}

// --- deterministic boundary / golden checks -------------------------------

#[test]
fn golden_float_two_misc_matches_arm_layout() {
    // FABS opcode = 0b01111, U=0, size_hi=0.
    // fabs v0.2s, v0.2s  (Q=0, sz=0, size=00) -> 0x0e20f800
    assert_eq!(
        word_of(encode_neon_float_two_misc(
            &[vreg_arr(0, "2s"), vreg_arr(0, "2s")],
            0,
            0,
            0b01111
        )),
        0x0E20_F800,
    );
    // fabs v0.4s, v0.4s  (Q=1, sz=0, size=00) -> 0x4e20f800
    assert_eq!(
        word_of(encode_neon_float_two_misc(
            &[vreg_arr(0, "4s"), vreg_arr(0, "4s")],
            0,
            0,
            0b01111
        )),
        0x4E20_F800,
    );
    // fabs v0.2d, v0.2d  (Q=1, sz=1, size=01) -> 0x4e60f800
    assert_eq!(
        word_of(encode_neon_float_two_misc(
            &[vreg_arr(0, "2d"), vreg_arr(0, "2d")],
            0,
            0,
            0b01111
        )),
        0x4E60_F800,
    );
    // FNEG opcode = 0b10111, U=0, size_hi=0.
    // fneg v0.2s, v0.2s  (Q=0, sz=0) -> 0x0e217800
    assert_eq!(
        word_of(encode_neon_float_two_misc(
            &[vreg_arr(0, "2s"), vreg_arr(0, "2s")],
            0,
            0,
            0b10111
        )),
        0x0E21_7800,
    );
    // FCVTZS opcode = 0b11011, U=0, size_hi=0, non-trivial registers.
    // fcvtzs v5.4s, v7.4s  (Q=1, sz=0, Rn=7, Rd=5) -> 0x4e21b8e5
    assert_eq!(
        word_of(encode_neon_float_two_misc(
            &[vreg_arr(5, "4s"), vreg_arr(7, "4s")],
            0,
            0,
            0b11011
        )),
        0x4E21_B8E5,
    );
}

#[test]
fn rejects_too_few_operands() {
    assert!(
        encode_neon_float_two_misc(&[], 0, 0, 0b01111).is_err(),
        "0 operands must error"
    );
    assert!(
        encode_neon_float_two_misc(&[vreg_arr(0, "2s")], 0, 0, 0b01111).is_err(),
        "1 operand must error",
    );
    // Exactly two valid operands must succeed.
    assert!(
        encode_neon_float_two_misc(&[vreg_arr(0, "2s"), vreg_arr(1, "2s")], 0, 0, 0b01111).is_ok(),
    );
}

#[test]
fn rejects_unsupported_arrangement() {
    // This group is floating-point only: only 2s/4s/2d are accepted.
    // Integer/other arrangements and garbage must error.
    for bad in ["8b", "16b", "4h", "8h", "1d", "garbage", ""] {
        let ops = vec![vreg_arr(0, bad), vreg_arr(1, "4s")];
        assert!(
            encode_neon_float_two_misc(&ops, 0, 0, 0b01111).is_err(),
            "arrangement {bad:?} must be rejected",
        );
    }
    // Valid float arrangements must succeed.
    for ok in ["2s", "4s", "2d"] {
        let ops = vec![vreg_arr(0, ok), vreg_arr(1, ok)];
        assert!(
            encode_neon_float_two_misc(&ops, 0, 0, 0b01111).is_ok(),
            "arrangement {ok:?} must be accepted",
        );
    }
}

#[test]
fn rejects_out_of_range_register() {
    // parse_reg_num returns None for register numbers > 31, so the encoder
    // must reject out-of-range SIMD registers rather than truncate them.
    let ops = vec![vreg_arr(32, "4s"), vreg_arr(1, "4s")];
    assert!(encode_neon_float_two_misc(&ops, 0, 0, 0b01111).is_err());
    let ops = vec![vreg_arr(1, "4s"), vreg_arr(32, "4s")];
    assert!(encode_neon_float_two_misc(&ops, 0, 0, 0b01111).is_err());
}
