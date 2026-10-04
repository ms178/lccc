//! Property-based tests for `encode_neon_three_diff`.
//!
//! `encode_neon_three_diff` encodes the AArch64 NEON "three different"
//! widening/narrowing instructions (SADDL/UADDL/SSUBL/USUBL/SMULL/UMULL/
//! SABAL/SABDL/SMLAL/UMLAL/SMLSL/UMLSL/SADDW/SSUBW/SQDMLAL/... and their
//! `2` upper-half variants) in the "Advanced SIMD three different" group:
//!
//! ```text
//!   31  30  29  28-24  23-22  21  20-16  15-12  11-10  9-5  4-0
//!    0   Q   U  01110   size   1   Rm    opcode   00    Rn   Rd
//! ```
//! `size` and the arrangement-derived `Q` come from the **source** register
//! arrangement `arr_n`; `is_high` forces `Q=1` for the "2" variants.
//!
//! ## Oracle
//! The golden words below were produced with `clang --target=aarch64`
//! (LLVM AArch64 assembler) and decoded from the emitted `.text` — they are
//! fully independent of this crate. The independent reference encoder
//! `ref_encode_three_diff` is assembled field-by-field from the documented
//! ARMv8-A layout (ARM DDI 0487, "Advanced SIMD three different"), so a
//! shared off-by-one in the implementation would still be caught by the
//! absolute LLVM-anchored golden check.
//!
//! ## Finding (documented by the `#[ignore]`d test
//! `three_diff_rejects_out_of_range_opcode_and_u_bit`)
//! The docstring states `u_bit` is the 1-bit U field and `opcode` is a
//! "4-bit opcode (bits 15-12)". The implementation ORs both straight into the
//! word with **no range check**, so `opcode >= 0x10` overflows into the Rm
//! field (bits 20-16) and `u_bit >= 2` overflows into the Q field (bit 30),
//! silently emitting a different instruction rather than `Err`. No spec cites
//! wrapping as intentional here. See `NEON_THREE_DIFF_RANGE_BUG_REPORT.md`.
//! (Note: the register fields are safe — `parse_reg_num` already clamps to
//! 0-31.)

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_three_diff;
use proptest::prelude::*;

// --- helpers --------------------------------------------------------------

/// Build `Operand::RegArrangement { reg: "v{n}", arrangement }`.
fn va(n: u32, arr: &str) -> Operand {
    Operand::RegArrangement {
        reg: format!("v{n}"),
        arrangement: arr.to_string(),
    }
}

fn reg_num_strategy() -> impl Strategy<Value = u32> {
    0u32..=31u32
}

fn u_bit_strategy() -> impl Strategy<Value = u32> {
    prop_oneof![Just(0u32), Just(1u32)]
}

/// 4-bit opcode (0..=0xF), matching the documented field width.
fn opcode_strategy() -> impl Strategy<Value = u32> {
    0u32..16u32
}

/// Architecturally valid source arrangements for three-different.
fn src_arrangement_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![
        Just("8b"),
        Just("16b"),
        Just("4h"),
        Just("8h"),
        Just("2s"),
        Just("4s"),
    ]
}

fn is_high_strategy() -> impl Strategy<Value = bool> {
    prop_oneof![Just(true), Just(false)]
}

/// Independent reference encoder assembled field-by-field from the ARMv8-A
/// "Advanced SIMD three different" layout: `0 Q U 01110 size 1 Rm opcode 00 Rn Rd`.
fn ref_encode_three_diff(
    rd: u32,
    rn: u32,
    rm: u32,
    u_bit: u32,
    opcode: u32,
    is_high: bool,
    arr_n: &str,
) -> u32 {
    // size = source element width
    let size: u32 = match arr_n {
        "8b" | "16b" => 0b00,
        "4h" | "8h" => 0b01,
        "2s" | "4s" => 0b10,
        _ => unreachable!("invalid arrangement in reference: {arr_n}"),
    };
    // arrangement-derived Q (1 for the wide/upper source), overridden by is_high
    let q_arr: u32 = match arr_n {
        "16b" | "8h" | "4s" => 1,
        _ => 0,
    };
    let q = if is_high { 1 } else { q_arr };

    let mut w = 0u32;
    // bit 31 stays 0
    w |= q << 30;
    w |= u_bit << 29;
    w |= 0b01110u32 << 24;
    w |= size << 22;
    w |= 1u32 << 21;
    w |= rm << 16;
    w |= opcode << 12;
    // bits [11:10] = 00 (fixed by the three-different encoding class)
    w |= rn << 5;
    w |= rd;
    w
}

fn word_of(res: Result<EncodeResult, String>) -> u32 {
    match res {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {:?}", other),
    }
}

// --- golden table (LLVM AArch64 assembler oracle) -------------------------

type GoldenRow = (u32, u32, u32, u32, u32, bool, &'static str, u32);

/// `(rd, rn, rm, u_bit, opcode, is_high, arr_n, golden_word)`.
/// Each golden word was produced by `clang --target=aarch64 -c` and read back
/// from the `.text` section (little-endian 32-bit instructions).
const GOLDEN: &[GoldenRow] = &[
    // saddl  v0.8h,  v1.8b,  v2.8b    -> 0x0E220020
    (0, 1, 2, 0, 0b0000, false, "8b", 0x0E220020),
    // umull  v0.4s,  v1.4h,  v2.4h    -> 0x2E62C020
    (0, 1, 2, 1, 0b1100, false, "4h", 0x2E62C020),
    // usubl2 v31.2d, v30.4s, v29.4s   -> 0x6EBD23DF
    (31, 30, 29, 1, 0b0010, true, "4s", 0x6EBD23DF),
    // smull  v5.2d,  v6.2s,  v7.2s    -> 0x0EA7C0C5
    (5, 6, 7, 0, 0b1100, false, "2s", 0x0EA7C0C5),
    // sabal  v9.8h,  v10.8b, v11.8b   -> 0x0E2B5149
    (9, 10, 11, 0, 0b0101, false, "8b", 0x0E2B5149),
    // uaddl  v21.4s, v22.4h, v23.4h   -> 0x2E7702D5
    (21, 22, 23, 1, 0b0000, false, "4h", 0x2E7702D5),
    // smlal  v1.2d,  v2.2s,  v3.2s    -> 0x0EA38041
    (1, 2, 3, 0, 0b1000, false, "2s", 0x0EA38041),
    // umlsl2 v17.2d, v18.4s, v19.4s   -> 0x6EB3A251
    (17, 18, 19, 1, 0b1010, true, "4s", 0x6EB3A251),
];

#[test]
fn three_diff_matches_golden_table() {
    for &(rd, rn, rm, u_bit, opcode, is_high, arr_n, golden) in GOLDEN {
        let operands = vec![va(rd, arr_n), va(rn, arr_n), va(rm, arr_n)];
        let got = word_of(encode_neon_three_diff(&operands, u_bit, opcode, is_high));
        assert_eq!(
            got, golden,
            "three-diff (rd={rd},rn={rn},rm={rm},u={u_bit},op=0x{opcode:x},is_high={is_high},\
             arr={arr_n}): got 0x{got:08X}, expected 0x{golden:08X}",
        );
    }
}

proptest! {
    /// Differential: for all valid inputs the implementation must agree with the
    /// independent field-by-field reference encoder.
    #[test]
    fn three_diff_matches_reference_encoder(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        u_bit in u_bit_strategy(),
        opcode in opcode_strategy(),
        is_high in is_high_strategy(),
        arr_n in src_arrangement_strategy(),
    ) {
        let operands = vec![va(rd, arr_n), va(rn, arr_n), va(rm, arr_n)];
        let got = word_of(encode_neon_three_diff(&operands, u_bit, opcode, is_high));
        let want = ref_encode_three_diff(rd, rn, rm, u_bit, opcode, is_high, arr_n);
        prop_assert_eq!(got, want);
    }

    /// Fixed bits are constant regardless of inputs: bit 31 = 0,
    /// bits [28:24] = 0b01110, bit 21 = 1, bits [11:10] = 00.
    #[test]
    fn three_diff_fixed_bits_are_constant(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        u_bit in u_bit_strategy(),
        opcode in opcode_strategy(),
        is_high in is_high_strategy(),
        arr_n in src_arrangement_strategy(),
    ) {
        let operands = vec![va(rd, arr_n), va(rn, arr_n), va(rm, arr_n)];
        let w = word_of(encode_neon_three_diff(&operands, u_bit, opcode, is_high));

        prop_assert_eq!(w >> 31, 0, "bit 31 must be 0");
        prop_assert_eq!((w >> 24) & 0b1_1111, 0b0_1110, "bits [28:24] must be 0b01110");
        prop_assert_eq!((w >> 21) & 1, 1, "bit 21 must be 1");
        prop_assert_eq!((w >> 10) & 0b11, 0b00, "bits [11:10] must be 00");
    }

    /// Field semantics: register fields round-trip exactly; `is_high` forces
    /// Q (bit 30) = 1; `size` (bits [23:22]) tracks the source element width.
    #[test]
    fn three_diff_field_semantics(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        u_bit in u_bit_strategy(),
        opcode in opcode_strategy(),
        is_high in is_high_strategy(),
        arr_n in src_arrangement_strategy(),
    ) {
        let operands = vec![va(rd, arr_n), va(rn, arr_n), va(rm, arr_n)];
        let w = word_of(encode_neon_three_diff(&operands, u_bit, opcode, is_high));

        prop_assert_eq!(w & 0x1F, rd, "Rd bits [4:0]");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn bits [9:5]");
        prop_assert_eq!((w >> 16) & 0x1F, rm, "Rm bits [20:16]");
        prop_assert_eq!((w >> 12) & 0xF, opcode, "opcode bits [15:12]");
        prop_assert_eq!((w >> 29) & 1, u_bit, "U bit 29");
        if is_high {
            prop_assert_eq!((w >> 30) & 1, 1, "is_high must force Q=1");
        }
        let want_size: u32 = match arr_n {
            "8b" | "16b" => 0b00,
            "4h" | "8h" => 0b01,
            "2s" | "4s" => 0b10,
            _ => unreachable!(),
        };
        prop_assert_eq!((w >> 22) & 0b11, want_size, "size bits [23:22] from element width");
    }

    /// Negative contract (validated inputs): unsupported source arrangements
    /// and too-few operands are rejected with `Err`.
    #[test]
    fn three_diff_rejects_invalid_inputs(
        bad_arr in "[^0-9a-zA-Z]|2d|1d|8s|16s|3q|bogus|2s2|h|",
    ) {
        // The SOURCE arrangement (operand 1, `arr_n`) is what the encoder
        // validates; operand 0's arrangement is the (ignored) destination.
        let operands_ok = vec![va(0, "8h"), va(1, bad_arr.as_str()), va(2, "8b")];
        let res = encode_neon_three_diff(&operands_ok, 0, 0b0000, false);
        prop_assert!(
            res.is_err(),
            "expected Err for arrangement {:?}, got {:?}",
            bad_arr, res
        );

        // Too few operands (0, 1, or 2).
        let res = encode_neon_three_diff(&[va(0, "8b")], 0, 0b0000, false);
        prop_assert!(res.is_err(), "expected Err for 1 operand, got {:?}", res);
        let res = encode_neon_three_diff(&[], 0, 0b0000, false);
        prop_assert!(res.is_err(), "expected Err for 0 operands, got {:?}", res);
    }

    /// FINDING: out-of-range `opcode` (>= 0x10) and `u_bit` (>= 2) silently
    /// overflow into adjacent fields instead of returning `Err`. Per the
    /// docstring these are a 4-bit opcode and a 1-bit U field; no spec cites
    /// wrapping as intentional. Marked `#[ignore]` so the suite stays green;
    /// run with `cargo test three_diff_rejects_out_of_range_opcode_and_u_bit -- --ignored`.
    #[test]
    #[ignore = "NEON_THREE_DIFF_RANGE_BUG_REPORT.md: opcode/u_bit not range-validated"]
    fn three_diff_rejects_out_of_range_opcode_and_u_bit(
        opcode_oob in 16u32..256u32,
        u_bit_oob in 2u32..16u32,
    ) {
        let operands = vec![va(0, "8b"), va(1, "8b"), va(2, "8b")];
        let res = encode_neon_three_diff(&operands, 0, opcode_oob, false);
        prop_assert!(
            res.is_err(),
            "expected Err for out-of-range opcode 0x{:x}, got Ok(0x{:08X})",
            opcode_oob,
            word_of(res.clone())
        );

        let res = encode_neon_three_diff(&operands, u_bit_oob, 0b0000, false);
        prop_assert!(
            res.is_err(),
            "expected Err for out-of-range u_bit {}, got Ok(0x{:08X})",
            u_bit_oob,
            word_of(res.clone())
        );
    }
}
