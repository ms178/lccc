//! Property-based tests for `encode_neon_three_diff_narrow` (the NEON
//! "three-different narrowing" encoder for ADDHN/RADDHN/SUBHN/RSUBHN and their
//! `2` upper-half variants).
//!
//! The instructions belong to the AArch64 "Advanced SIMD three different"
//! group and have the *narrowing* layout:
//!
//! ```text
//!   31  30  29  28-24  23-22  21  20-16  15-12  11-10  9-5  4-0
//!    0   Q   U  01110   size   1   Rm    opcode   00    Rn   Rd
//! ```
//!
//! For the narrowing family the source register is *wider* than the
//! destination. The encoder derives `size` from the **source** arrangement
//! (`arr_n`, operand 1): `.8h`→`00`, `.4s`→`01`, `.2d`→`10`. `Q` is set
//! **only** by `is_high` (the `2` variants). `u_bit` selects the rounding
//! family; `opcode` is `0b0100` (ADDHN/RADDHN) or `0b0110` (SUBHN/RSUBHN).
//!
//! ## Oracle
//! The golden words in `GOLDEN` were produced with `clang --target=aarch64 -c`
//! (LLVM's AArch64 assembler) and read back from the `.text` section — they
//! are fully independent of this crate. The independent reference encoder
//! `ref_encode` is assembled field-by-field from the documented ARMv8-A
//! layout, so a shared off-by-one in the implementation is still caught by the
//! LLVM-anchored golden check.
//!
//! ## Finding (documented by the `#[ignore]`d test
//! `narrow_rejects_out_of_range_opcode_and_u_bit`)
//! The docstring states `u_bit` is the 1-bit U field (bit 29) and `opcode` is a
//! 4-bit field (bits 15-12). The implementation ORs both straight into the
//! word with **no range check**, so `opcode >= 0x10` overflows into the Rm
//! field (bits 20-16) and `u_bit >= 2` overflows into the Q field (bit 30),
//! silently emitting a different instruction rather than `Err`. No spec cites
//! wrapping as intentional here. (The register fields are safe — `parse_reg_num`
//! already clamps to 0-31.) This is the same latent defect as the sibling
//! widening encoder `encode_neon_three_diff` (see
//! `NEON_THREE_DIFF_RANGE_BUG_REPORT.md`).

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_three_diff_narrow;
use proptest::prelude::*;

// --- helpers --------------------------------------------------------------

/// `Operand::RegArrangement { reg: "v{n}", arrangement }`.
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

/// 4-bit opcode (0..=0xF), matching the documented field width at bits [15:12].
fn opcode_strategy() -> impl Strategy<Value = u32> {
    0u32..16u32
}

/// Architecturally valid SOURCE arrangements for three-different narrowing:
/// `.8h`, `.4s`, `.2d` (the wide, lower-numbered element group).
fn src_arrangement_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("8h"), Just("4s"), Just("2d")]
}

fn is_high_strategy() -> impl Strategy<Value = bool> {
    prop_oneof![Just(true), Just(false)]
}

/// Independent reference encoder assembled field-by-field from the ARMv8-A
/// "Advanced SIMD three different" narrowing layout:
/// `0 Q U 01110 size 1 Rm opcode 00 Rn Rd`.
fn ref_encode(
    rd: u32,
    rn: u32,
    rm: u32,
    u_bit: u32,
    opcode: u32,
    is_high: bool,
    arr_n: &str,
) -> u32 {
    // size encodes the source (wide) element width for the narrow family.
    let size: u32 = match arr_n {
        "8h" => 0b00,
        "4s" => 0b01,
        "2d" => 0b10,
        _ => unreachable!("invalid arrangement in reference: {arr_n}"),
    };
    // Q is driven solely by is_high for the narrowing family.
    let q: u32 = if is_high { 1 } else { 0 };

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

/// `(rd, rn, rm, u_bit, opcode, is_high, src_arr, golden_word)`.
/// Each golden word was produced by `clang --target=aarch64 -c` on the
/// mnemonic whose encoding matches these parameters (addhn/subhn/raddhn/
/// rsubhn and their `2` variants), then read back from `.text`.
const GOLDEN: &[GoldenRow] = &[
    // addhn  v0.8b,  v1.8h,  v2.8h     -> 0x0E224020
    (0, 1, 2, 0, 0b0100, false, "8h", 0x0E224020),
    // addhn  v5.4h,  v6.4s,  v7.4s     -> 0x0E6740C5
    (5, 6, 7, 0, 0b0100, false, "4s", 0x0E6740C5),
    // subhn  v0.8b,  v1.8h,  v2.8h     -> 0x0E226020
    (0, 1, 2, 0, 0b0110, false, "8h", 0x0E226020),
    // raddhn v31.8b, v30.8h, v29.8h    -> 0x2E3D43DF
    (31, 30, 29, 1, 0b0100, false, "8h", 0x2E3D43DF),
    // rsubhn2 v17.4s, v18.2d, v19.2d   -> 0x6EB36251
    (17, 18, 19, 1, 0b0110, true, "2d", 0x6EB36251),
    // subhn2 v10.8h, v11.4s, v12.4s    -> 0x4E6C616A
    (10, 11, 12, 0, 0b0110, true, "4s", 0x4E6C616A),
    // addhn  v9.2s,  v10.2d, v11.2d    -> 0x0EAB4149
    (9, 10, 11, 0, 0b0100, false, "2d", 0x0EAB4149),
    // raddhn2 v3.16b, v4.8h, v5.8h     -> 0x6E254083
    (3, 4, 5, 1, 0b0100, true, "8h", 0x6E254083),
];

#[test]
fn narrow_matches_golden_table() {
    for &(rd, rn, rm, u_bit, opcode, is_high, src, golden) in GOLDEN {
        // Only the SOURCE (operand 1) arrangement drives encoding; give all
        // three the source arrangement so the operands are self-consistent.
        let operands = vec![va(rd, src), va(rn, src), va(rm, src)];
        let got = word_of(encode_neon_three_diff_narrow(
            &operands, u_bit, opcode, is_high,
        ));
        assert_eq!(
            got, golden,
            "narrow (rd={rd},rn={rn},rm={rm},u={u_bit},op=0x{opcode:x},is_high={is_high},\
             src={src}): got 0x{got:08X}, expected 0x{golden:08X}",
        );
    }
}

proptest! {
    /// Differential: for all valid inputs the implementation must agree with the
    /// independent field-by-field reference encoder.
    #[test]
    fn narrow_matches_reference_encoder(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        u_bit in u_bit_strategy(),
        opcode in opcode_strategy(),
        is_high in is_high_strategy(),
        src in src_arrangement_strategy(),
    ) {
        let operands = vec![va(rd, src), va(rn, src), va(rm, src)];
        let got = word_of(encode_neon_three_diff_narrow(&operands, u_bit, opcode, is_high));
        let want = ref_encode(rd, rn, rm, u_bit, opcode, is_high, src);
        prop_assert_eq!(got, want);
    }

    /// Fixed bits are constant regardless of inputs: bit 31 = 0,
    /// bits [28:24] = 0b01110, bit 21 = 1, bits [11:10] = 00.
    #[test]
    fn narrow_fixed_bits_are_constant(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        u_bit in u_bit_strategy(),
        opcode in opcode_strategy(),
        is_high in is_high_strategy(),
        src in src_arrangement_strategy(),
    ) {
        let operands = vec![va(rd, src), va(rn, src), va(rm, src)];
        let w = word_of(encode_neon_three_diff_narrow(&operands, u_bit, opcode, is_high));

        prop_assert_eq!(w >> 31, 0, "bit 31 must be 0");
        prop_assert_eq!((w >> 24) & 0b1_1111, 0b0_1110, "bits [28:24] must be 0b01110");
        prop_assert_eq!((w >> 21) & 1, 1, "bit 21 must be 1");
        prop_assert_eq!((w >> 10) & 0b11, 0b00, "bits [11:10] must be 00");
    }

    /// Field semantics: register fields round-trip exactly; `is_high` forces
    /// Q (bit 30) = 1; `u_bit` lands on bit 29; `opcode` occupies bits [15:12];
    /// `size` (bits [23:22]) tracks the source element width.
    #[test]
    fn narrow_field_semantics(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        u_bit in u_bit_strategy(),
        opcode in opcode_strategy(),
        is_high in is_high_strategy(),
        src in src_arrangement_strategy(),
    ) {
        let operands = vec![va(rd, src), va(rn, src), va(rm, src)];
        let w = word_of(encode_neon_three_diff_narrow(&operands, u_bit, opcode, is_high));

        prop_assert_eq!(w & 0x1F, rd, "Rd bits [4:0]");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn bits [9:5]");
        prop_assert_eq!((w >> 16) & 0x1F, rm, "Rm bits [20:16]");
        prop_assert_eq!((w >> 12) & 0xF, opcode, "opcode bits [15:12]");
        prop_assert_eq!((w >> 29) & 1, u_bit, "U bit 29");
        let want_q: u32 = if is_high { 1 } else { 0 };
        prop_assert_eq!((w >> 30) & 1, want_q, "Q bit 30 == is_high");
        let want_size: u32 = match src {
            "8h" => 0b00,
            "4s" => 0b01,
            "2d" => 0b10,
            _ => unreachable!(),
        };
        prop_assert_eq!((w >> 22) & 0b11, want_size, "size bits [23:22] from source width");
    }

    /// Finding: for the narrowing family Q is driven SOLELY by `is_high` — the
    /// source arrangement width does not influence it (unlike the widening
    /// sibling). Concretely, `.8h`/`.4s`/`.2d` with `is_high=false` all yield
    /// Q=0. This is correct per ARM (the `2` variant is what selects Q), and is
    /// asserted here to pin the contract.
    #[test]
    fn narrow_q_depends_only_on_is_high(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        src in src_arrangement_strategy(),
        is_high in is_high_strategy(),
    ) {
        let operands = vec![va(rd, src), va(rn, src), va(rm, src)];
        let w = word_of(encode_neon_three_diff_narrow(&operands, 0, 0b0100, is_high));
        let want_q: u32 = if is_high { 1 } else { 0 };
        prop_assert_eq!((w >> 30) & 1, want_q, "Q must equal is_high regardless of arrangement");
    }

    /// Negative contract (validated inputs): unsupported source arrangements
    /// and too-few operands are rejected with `Err`.
    #[test]
    fn narrow_rejects_invalid_inputs(
        bad_arr in "8b|16b|4h|2s|4s2|1d|2d2|8s|16s|3q|bogus|h|s|d|2s2",
    ) {
        // The SOURCE arrangement (operand 1) is what the encoder validates.
        let operands_ok = vec![va(0, "8h"), va(1, bad_arr.as_str()), va(2, "8h")];
        let res = encode_neon_three_diff_narrow(&operands_ok, 0, 0b0100, false);
        prop_assert!(
            res.is_err(),
            "expected Err for source arrangement {:?}, got {:?}",
            bad_arr, res
        );

        // Too few operands (0, 1, or 2).
        let res = encode_neon_three_diff_narrow(&[va(0, "8h")], 0, 0b0100, false);
        prop_assert!(res.is_err(), "expected Err for 1 operand, got {:?}", res);
        let res = encode_neon_three_diff_narrow(&[], 0, 0b0100, false);
        prop_assert!(res.is_err(), "expected Err for 0 operands, got {:?}", res);
    }

    /// FINDING: out-of-range `opcode` (>= 0x10) and `u_bit` (>= 2) silently
    /// overflow into adjacent fields instead of returning `Err`. Per the
    /// docstring these are a 4-bit opcode (bits 15-12) and a 1-bit U field
    /// (bit 29); no spec cites wrapping as intentional. Marked `#[ignore]` so
    /// the suite stays green; run with
    /// `cargo test narrow_rejects_out_of_range_opcode_and_u_bit -- --ignored`.
    #[test]
    #[ignore = "opcode/u_bit not range-validated; see module docs"]
    fn narrow_rejects_out_of_range_opcode_and_u_bit(
        opcode_oob in 16u32..256u32,
        u_bit_oob in 2u32..16u32,
    ) {
        let operands = vec![va(0, "8h"), va(1, "8h"), va(2, "8h")];
        let res = encode_neon_three_diff_narrow(&operands, 0, opcode_oob, false);
        prop_assert!(
            res.is_err(),
            "expected Err for out-of-range opcode 0x{:x}, got Ok(0x{:08X})",
            opcode_oob,
            word_of(res.clone())
        );

        let res = encode_neon_three_diff_narrow(&operands, u_bit_oob, 0b0100, false);
        prop_assert!(
            res.is_err(),
            "expected Err for out-of-range u_bit {}, got Ok(0x{:08X})",
            u_bit_oob,
            word_of(res.clone())
        );
    }
    /// FINDING (destination arrangement not validated): the encoder derives
    /// the entire word from the SOURCE arrangement + `is_high`; the destination
    /// register's arrangement (operand 0) is read for its number then discarded,
    /// and is **never** checked against the narrow type implied by the source.
    /// A mismatched dest arrangement (e.g. `.4s` paired with a `.8h` source, or
    /// a bare register with no arrangement at all) is silently accepted and
    /// produces the SAME word as the correct dest — `clang --target=aarch64`
    /// rejects these (`invalid operand for instruction`). No repo docstring/test
    /// states this permissiveness is intentional. Marked `#[ignore]`; run with
    /// `cargo test narrow_rejects_inconsistent_destination_arrangement -- --ignored`.
    #[test]
    #[ignore = "destination arrangement not validated; see Bug 2 / module docs"]
    fn narrow_rejects_inconsistent_destination_arrangement(
        bad_dest in prop_oneof![Just("4s"), Just("2d"), Just("8b"), Just("8h"), Just("4h"), Just("2s"), Just("16b"), Just("bogus")],
    ) {
        // Correct dest for a .8h source is .8b; everything else (including
        // unrelated arrangements and junk) should be rejected, but is not.
        let ops = vec![va(0, bad_dest), va(1, "8h"), va(2, "8h")];
        let res = encode_neon_three_diff_narrow(&ops, 0, 0b0100, false);
        prop_assert!(
            res.is_err(),
            "expected Err for inconsistent dest arrangement {:?} (source .8h), got {:?}",
            bad_dest, res
        );

        // Bare register with no arrangement as destination is equally invalid.
        let bare = vec![Operand::Reg("v0".into()), va(1, "8h"), va(2, "8h")];
        let res = encode_neon_three_diff_narrow(&bare, 0, 0b0100, false);
        prop_assert!(
            res.is_err(),
            "expected Err for bare-Reg destination (no arrangement), got {:?}",
            res
        );
    }
}

// --- deterministic boundary checks ---------------------------------------

#[test]
fn rejects_too_few_operands() {
    assert!(
        encode_neon_three_diff_narrow(&[], 0, 0b0100, false).is_err(),
        "0 operands must error"
    );
    assert!(
        encode_neon_three_diff_narrow(&[va(0, "8h")], 0, 0b0100, false).is_err(),
        "1 operand must error"
    );
    assert!(
        encode_neon_three_diff_narrow(&[va(0, "8h"), va(1, "8h")], 0, 0b0100, false).is_err(),
        "2 operands must error"
    );
    // Exactly three valid operands must succeed.
    assert!(
        encode_neon_three_diff_narrow(&[va(0, "8h"), va(1, "8h"), va(2, "8h")], 0, 0b0100, false)
            .is_ok()
    );
}
