//! Property-based tests for `encode_neon_two_misc`
//! (the AArch64 "Advanced SIMD two-register miscellaneous" integer encoder:
//! ABS, NEG, CLS, CLZ, SUQADD, USQADD, SADDLP, UADDLP, SADALP, UADALP, NOT, ...).
//!
//! Encoding (ARMv8 ARM, "Advanced SIMD two-register miscellaneous"):
//!   `0 Q U 01110 size 10000 opcode 10 Rn Rd`
//!    31 30 29 28-24 23-22 21-17 16-12 11-10 9-5 4-0
//!
//! Field widths (the contract this encoder must uphold):
//!   * bit 31     : 0          (constant)
//!   * bit 30 (Q) : from destination arrangement (0=64-bit, 1=128-bit vector)
//!   * bit 29 (U) : `u_bit` parameter — must be 0 or 1
//!   * bits 28-24 : 01110      (constant)
//!   * bits 23-22 (size): from destination arrangement (2 bits)
//!   * bits 21-17 : 10000      (constant)
//!   * bits 16-12 (opcode): `opcode` parameter — must be a 5-bit value (0..=0x1F)
//!   * bits 11-10 : 10         (constant)
//!   * bits 9-5 (Rn), 4-0 (Rd): register numbers (0..=31, guaranteed by parse_reg_num)
//!
//! Golden words cross-checked against the ARMv8 ARM layout:
//!   `abs   v0.16b, v0.16b` => 0x4e20b800   (U=0, opcode=01011, Q=1, size=00)
//!   `abs   v0.8b,  v0.8b`  => 0x0e20b800   (Q=0, size=00)
//!   `abs   v0.4s,  v0.4s`  => 0x4ea0b800   (Q=1, size=10)
//!   `suqadd v0.16b, v0.16b`=> 0x6e200800   (U=1, opcode=00000)
//!   `cls   v5.4s,  v7.4s`  => 0x4ea018e5   (U=0, opcode=00001, Rn=7, Rd=5)
//!
//! NOTE on findings:
//!  * `prop_two_misc_source_arrangement_ignored` documents that the SOURCE
//!    register's arrangement is read then discarded (bound to `_`); only the
//!    destination arrangement drives Q/size. ARM requires dest/src to match,
//!    so a mismatch (e.g. `Vd.4s, Vn.2d`) is silently accepted.
//!  * `prop_two_misc_rejects_out_of_range_u_bit_and_opcode` is a NEGATIVE-
//!    CONTRACT property: it asserts that out-of-range `u_bit` (>1) or `opcode`
//!    (>0x1F, 5 bits) are rejected with `Err`, because they silently overflow
//!    into the Q bit / the fixed `01110` / `10000` fields and produce a
//!    corrupt instruction. It is EXPECTED TO FAIL with the current
//!    implementation, which performs no range validation on either parameter.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_two_misc;
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

/// All arrangements accepted by `neon_arr_to_q_size`.
fn arr_strategy() -> impl Strategy<Value = &'static str> {
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

/// INDEPENDENT oracle: arrangement -> (Q, size), hardcoded here (not calling
/// `neon_arr_to_q_size`) so the reference does not share code with the SUT.
fn arr_to_q_size(arr: &str) -> Option<(u32, u32)> {
    Some(match arr {
        "8b" => (0, 0b00),
        "16b" => (1, 0b00),
        "4h" => (0, 0b01),
        "8h" => (1, 0b01),
        "2s" => (0, 0b10),
        "4s" => (1, 0b10),
        "1d" => (0, 0b11),
        "2d" => (1, 0b11),
        _ => return None,
    })
}

/// Reference encoding built straight from the documented layout.
fn ref_word(rd: u32, rn: u32, arr: &str, u_bit: u32, opcode: u32) -> u32 {
    let (q, size) = arr_to_q_size(arr).unwrap();
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
    fn prop_two_misc_matches_arm_layout(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in arr_strategy(),
        u_bit in u_bit_strategy(),
        opcode in opcode_strategy(),
    ) {
        let ops = vec![vreg_arr(rd, arr), vreg_arr(rn, arr)];
        let word = word_of(encode_neon_two_misc(&ops, u_bit, opcode));
        prop_assert_eq!(word, ref_word(rd, rn, arr, u_bit, opcode));
    }

    /// Field isolation: Rd is exactly bits [4:0], Rn exactly bits [9:5],
    /// opcode exactly bits [16:12], and size exactly bits [23:22]. No field
    /// leaks into any other.
    #[test]
    fn prop_two_misc_fields_isolated(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in arr_strategy(),
        u_bit in u_bit_strategy(),
        opcode in opcode_strategy(),
    ) {
        let base = word_of(encode_neon_two_misc(&[vreg_arr(0, arr), vreg_arr(0, arr)], u_bit, 0));

        // Rd occupies bits [4:0] only.
        let with_rd = word_of(encode_neon_two_misc(&[vreg_arr(rd, arr), vreg_arr(0, arr)], u_bit, 0));
        prop_assert_eq!(with_rd & 0x1F, rd, "Rd not in bits [4:0]");
        prop_assert_eq!(with_rd & !0x1F, base & !0x1F, "Rd leaked above bit 4");

        // Rn occupies bits [9:5] only.
        let with_rn = word_of(encode_neon_two_misc(&[vreg_arr(0, arr), vreg_arr(rn, arr)], u_bit, 0));
        prop_assert_eq!((with_rn >> 5) & 0x1F, rn, "Rn not in bits [9:5]");
        prop_assert_eq!(with_rn & !0x3E0, base & !0x3E0, "Rn leaked outside bits [9:5]");

        // opcode occupies bits [16:12] only.
        let with_op = word_of(encode_neon_two_misc(&[vreg_arr(0, arr), vreg_arr(0, arr)], u_bit, opcode));
        prop_assert_eq!((with_op >> 12) & 0x1F, opcode, "opcode not in bits [16:12]");
        prop_assert_eq!(with_op & !0x1F000, base & !0x1F000, "opcode leaked outside bits [16:12]");
    }

    /// Fixed bits are constant for all inputs; Q/U are driven by inputs.
    #[test]
    fn prop_two_misc_fixed_bits_and_qu_size(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in arr_strategy(),
        u_bit in u_bit_strategy(),
        opcode in opcode_strategy(),
    ) {
        let word = word_of(encode_neon_two_misc(&[vreg_arr(rd, arr), vreg_arr(rn, arr)], u_bit, opcode));

        // Constant fields must match regardless of inputs.
        prop_assert_eq!(word & FIXED_MASK, FIXED_CONST, "fixed bits mutated by inputs");

        // Q (bit 30) and U (bit 29) reflect arrangement / u_bit parameter.
        let (q, size) = arr_to_q_size(arr).unwrap();
        prop_assert_eq!((word >> 30) & 1, q, "Q bit wrong");
        prop_assert_eq!((word >> 29) & 1, u_bit, "U bit wrong");
        prop_assert_eq!((word >> 22) & 0x3, size, "size bits wrong");
    }

    /// Documents finding: the SOURCE register's arrangement is read then
    /// discarded (bound to `_`). Varying it — even using a bare `Operand::Reg`
    /// with no arrangement — leaves the encoded word identical. ARM requires
    /// dest and source arrangements to match, so mismatches are silently
    /// accepted rather than rejected.
    #[test]
    fn prop_two_misc_source_arrangement_ignored(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        dest_arr in arr_strategy(),
        src_arr in arr_strategy(),
        u_bit in u_bit_strategy(),
        opcode in opcode_strategy(),
    ) {
        let dest = vreg_arr(rd, dest_arr);
        let ref_word = word_of(encode_neon_two_misc(
            &[dest.clone(), vreg_arr(rn, dest_arr)], u_bit, opcode,
        ));

        // Any source arrangement (even one differing from dest) encodes the same.
        let varied = word_of(encode_neon_two_misc(
            &[dest.clone(), vreg_arr(rn, src_arr)], u_bit, opcode,
        ));
        prop_assert_eq!(varied, ref_word, "source arrangement unexpectedly changed encoding");

        // A bare register (no arrangement) as source must also encode identically.
        let bare = word_of(encode_neon_two_misc(
            &[dest, Operand::Reg(format!("v{rn}"))], u_bit, opcode,
        ));
        prop_assert_eq!(bare, ref_word, "bare-Reg source changed encoding");
    }

    /// NEGATIVE CONTRACT (EXPECTED TO FAIL): `u_bit` must be 0 or 1 and
    /// `opcode` must fit in 5 bits (0..=0x1F). Out-of-range values silently
    /// overflow — `u_bit >= 2` collides with the Q bit (and bit 31), and
    /// `opcode > 0x1F` collides with the constant `01110`/`10000` fields —
    /// producing a corrupt instruction word. A defensive encoder must reject
    /// these. The current implementation performs no range validation, so this
    /// property fails and surfaces the missing contract check.
    #[test]
    fn prop_two_misc_rejects_out_of_range_u_bit_and_opcode(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in arr_strategy(),
        bad_u in 2u32..=0xFFu32,
        bad_opcode in 0x20u32..=0xFFFFu32,
    ) {
        let ops = vec![vreg_arr(rd, arr), vreg_arr(rn, arr)];

        let res_u = encode_neon_two_misc(&ops, bad_u, 0);
        prop_assert!(
            res_u.is_err(),
            "u_bit={} (>1) must be rejected, but encoded as {:?} (overflows into Q/bit31)",
            bad_u, res_u,
        );

        let res_op = encode_neon_two_misc(&ops, 0, bad_opcode);
        prop_assert!(
            res_op.is_err(),
            "opcode=0x{:x} (>0x1F) must be rejected, but encoded as {:?} (overflows fixed fields)",
            bad_opcode, res_op,
        );
    }
}

// --- deterministic boundary / golden checks -------------------------------

#[test]
fn golden_two_misc_matches_arm_layout() {
    // abs v0.16b, v0.16b  (U=0, opcode=01011, Q=1, size=00) -> 0x4e20b800
    assert_eq!(
        word_of(encode_neon_two_misc(
            &[vreg_arr(0, "16b"), vreg_arr(0, "16b")],
            0,
            0b01011
        )),
        0x4E20_B800,
    );
    // abs v0.8b, v0.8b   (Q=0, size=00) -> 0x0e20b800
    assert_eq!(
        word_of(encode_neon_two_misc(
            &[vreg_arr(0, "8b"), vreg_arr(0, "8b")],
            0,
            0b01011
        )),
        0x0E20_B800,
    );
    // abs v0.4s, v0.4s   (Q=1, size=10) -> 0x4ea0b800
    assert_eq!(
        word_of(encode_neon_two_misc(
            &[vreg_arr(0, "4s"), vreg_arr(0, "4s")],
            0,
            0b01011
        )),
        0x4EA0_B800,
    );
    // suqadd v0.16b, v0.16b (U=1, opcode=00000, Q=1, size=00) -> 0x6e200800
    assert_eq!(
        word_of(encode_neon_two_misc(
            &[vreg_arr(0, "16b"), vreg_arr(0, "16b")],
            1,
            0b00000
        )),
        0x6E20_0800,
    );
    // cls v5.4s, v7.4s   (U=0, opcode=00001, Q=1, size=10, Rn=7, Rd=5) -> 0x4ea018e5
    assert_eq!(
        word_of(encode_neon_two_misc(
            &[vreg_arr(5, "4s"), vreg_arr(7, "4s")],
            0,
            0b00001
        )),
        0x4EA0_18E5,
    );
}

#[test]
fn rejects_too_few_operands() {
    assert!(
        encode_neon_two_misc(&[], 0, 0b01011).is_err(),
        "0 operands must error"
    );
    assert!(
        encode_neon_two_misc(&[vreg_arr(0, "8b")], 0, 0b01011).is_err(),
        "1 operand must error",
    );
    // Exactly two valid operands must succeed.
    assert!(encode_neon_two_misc(&[vreg_arr(0, "8b"), vreg_arr(1, "8b")], 0, 0b01011).is_ok(),);
}

#[test]
fn rejects_unsupported_arrangement() {
    // An arrangement neon_arr_to_q_size does not recognise must error.
    let ops = vec![vreg_arr(0, "garbage"), vreg_arr(1, "8b")];
    assert!(encode_neon_two_misc(&ops, 0, 0b01011).is_err());
}
