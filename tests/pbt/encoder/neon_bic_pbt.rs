//! Property-based tests for `encode_neon_bic`.
//!
//! `encode_neon_bic` encodes the AArch64 NEON `BIC` (Bitwise Bit Clear, vector)
//! instruction — `BIC Vd.T, Vn.T, Vm.T` — in the "Advanced SIMD three same"
//! logical group:
//!
//! ```text
//!   31 30 29 28-24 23-22 21 20-16 15-11 10 9-5 4-0
//!    0  Q  U  01110  size  1   Rm  opcode  1  Rn  Rd
//! ```
//! with **U=0**, **size=01**, **opcode=00011**, and **bit10=1**. Per the ARMv8-A
//! ARM (ARM DDI 0487, "Advanced SIMD three same", BIC row) `BIC` is the
//! `(U=0, size=01)` slot of the logical group and is defined **only for
//! `.8b` (Q=0) and `.16b` (Q=1)** — byte lanes, like the rest of the group
//! (AND/BIC/ORR/ORN/EOR/BSL/BIT/BIF). Every other arrangement is UNALLOCATED.
//!
//! ## Oracle
//! The golden words below were produced and independently verified against
//! LLVM's AArch64 assembler (`clang --target=aarch64` + objdump):
//!
//! ```text
//!   bic v0.16b, v1.16b, v2.16b   -> 0x4E621C20
//!   bic v0.8b,  v1.8b,  v2.8b    -> 0x0E621C20
//!   bic v5.16b, v6.16b, v7.16b   -> 0x4E671CC5
//!   bic v31.8b, v30.8b, v29.8b   -> 0x0E7D1FDF
//!   bic v10.16b,v11.16b,v12.16b  -> 0x4E6C1D6A
//! ```
//! The reference encoder below is assembled field-by-field from the documented
//! layout and is structurally independent of the crate implementation (it splits
//! `opcode5<<11 | 1<<10`, whereas the impl ORs `0b000111<<10`), so a shared
//! off-by-one would still be caught by the absolute golden check.
//!
//! ## Finding (documented by the `#[ignore]`d test `bic_rejects_non_byte`)
//! `BIC` is architecturally defined only for `.8b`/`.16b` (size=01 is a fixed
//! field; the group is byte-only). LLVM rejects `.4h/.8h/.2s/.4s/.1d/.2d` with
//! "invalid operand for instruction", but `encode_neon_bic` accepts any
//! arrangement: it maps `arr != "16b"` to `Q=0` and emits a valid-looking
//! byte-BIC word, silently corrupting the instruction. See
//! `BIC_NONBYTE_BUG_REPORT.md`.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_bic;
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

/// Arrangements that are architecturally VALID for BIC (byte lanes only).
fn valid_arrangement_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("8b"), Just("16b")]
}

/// Independent reference encoder assembled field-by-field from the ARM layout:
///   0 Q U 01110 size 1 Rm opcode5 1 Rn Rd   with U=0, size=01, opcode5=00011.
fn ref_encode_bic(rd: u32, rn: u32, rm: u32, arr: &str) -> u32 {
    let q: u32 = if arr == "16b" { 1 } else { 0 };
    let mut w = 0u32;
    w |= q << 30; // bit 31 stays 0
    // U (bit 29) = 0 for BIC
    w |= 0b01110u32 << 24; // bits 28-24
    w |= 0b01u32 << 22; // size [23:22] = 01
    w |= 1u32 << 21; // bit 21
    w |= rm << 16; // bits 20-16
    w |= 0b00011u32 << 11; // opcode5 bits 15-11
    w |= 1u32 << 10; // fixed '1'
    w |= rn << 5; // bits 9-5
    w |= rd; // bits 4-0
    w
}

fn word_of(res: Result<EncodeResult, String>) -> u32 {
    match res {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {other:?}"),
    }
}

// --- golden table (absolute oracle, cross-checked vs LLVM) ----------------

const GOLDEN: &[(u32, u32, u32, &str, u32)] = &[
    // (Rd, Rn, Rm, arrangement, expected_word)
    (0, 1, 2, "16b", 0x4E621C20),    // bic v0.16b, v1.16b, v2.16b
    (0, 1, 2, "8b", 0x0E621C20),     // bic v0.8b,  v1.8b,  v2.8b   (Q=0)
    (5, 6, 7, "16b", 0x4E671CC5),    // bic v5.16b, v6.16b, v7.16b
    (31, 30, 29, "8b", 0x0E7D1FDF),  // bic v31.8b, v30.8b, v29.8b
    (10, 11, 12, "16b", 0x4E6C1D6A), // bic v10.16b,v11.16b,v12.16b
];

#[test]
fn bic_matches_golden_table() {
    for &(rd, rn, rm, arr, expected) in GOLDEN {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let got = word_of(encode_neon_bic(&ops));
        assert_eq!(
            got, expected,
            "bic v{rd}.{arr}, v{rn}.{arr}, v{rm}.{arr}: got 0x{got:08X}, want 0x{expected:08X}",
        );
        assert_eq!(
            ref_encode_bic(rd, rn, rm, arr),
            expected,
            "reference encoder drift"
        );
    }
}

// --- arity / operand-shape contracts --------------------------------------

/// BIC requires three register operands. Fewer must yield `Err` (matches LLVM's
/// "too few operands for instruction").
#[test]
fn bic_rejects_too_few_operands() {
    let one = vec![va(0, "16b")];
    let two = vec![va(0, "16b"), va(1, "16b")];
    assert!(encode_neon_bic(&one).is_err(), "1 operand must error");
    assert!(encode_neon_bic(&two).is_err(), "2 operands must error");
}

/// A non-register operand in a register slot must yield `Err`, not panic or
/// silently encode.
#[test]
fn bic_rejects_non_register_operand() {
    let ops = vec![va(0, "16b"), va(1, "16b"), Operand::Imm(2)];
    assert!(
        encode_neon_bic(&ops).is_err(),
        "immediate in Rm slot must error"
    );
}

// --- properties -----------------------------------------------------------

proptest! {
    // === Oracle: reference encoder (differential) =========================
    // For every valid (.8b/.16b) arrangement and register triple, the
    // implementation must equal the independently-assembled reference word.
    #[test]
    fn matches_reference_encoder(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let got = word_of(encode_neon_bic(&ops));
        let want = ref_encode_bic(rd, rn, rm, arr);
        prop_assert_eq!(got, want);
    }

    // === Field placement: Rd/Rn/Rm round-trip + Q mapping ================
    // The five-bit register fields must round-trip exactly (no silent
    // truncation of in-range register numbers), Q must equal (arr == "16b"),
    // and size (bits 23-22) must be 01 for every valid BIC (it is a fixed
    // field, independent of arrangement).
    #[test]
    fn fields_round_trip_and_map_arrangement(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let w = word_of(encode_neon_bic(&ops));
        let q = if arr == "16b" { 1u32 } else { 0u32 };

        prop_assert_eq!(w & 0x1F, rd, "Rd field");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field");
        prop_assert_eq!((w >> 16) & 0x1F, rm, "Rm field");
        prop_assert_eq!((w >> 30) & 0x1, q, "Q bit (1 only for .16b)");
        prop_assert_eq!((w >> 22) & 0x3, 0b01u32, "size bits must be 01 (BIC fixed field)");
    }

    // === Fixed-bits invariant =============================================
    // The architecturally-constant bits of the BIC encoding never change for
    // any valid input: bit31=0, U(bit29)=0, bits28-24=01110, bit21=1,
    // opcode5(bits15-11)=00011, bit10=1.
    #[test]
    fn fixed_bits_are_constant(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let w = word_of(encode_neon_bic(&ops));

        prop_assert_eq!((w >> 31) & 1, 0, "bit 31 must be 0");
        prop_assert_eq!((w >> 29) & 1, 0, "U bit must be 0 for BIC");
        prop_assert_eq!((w >> 24) & 0x1F, 0b01110, "bits 28-24");
        prop_assert_eq!((w >> 21) & 1, 1, "bit 21 must be 1");
        prop_assert_eq!((w >> 11) & 0x1F, 0b00011, "opcode5 bits 15-11");
        prop_assert_eq!((w >> 10) & 1, 1, "bit 10 must be 1");
    }

    // === Determinism ======================================================
    // Re-encoding the same operand triple is idempotent (referential
    // transparency / no hidden state).
    #[test]
    fn encoding_is_deterministic(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let w1 = word_of(encode_neon_bic(&ops));
        let w2 = word_of(encode_neon_bic(&ops));
        prop_assert_eq!(w1, w2);
    }
}

// --- documented finding: non-byte arrangements not rejected ---------------

// `BIC` is only defined for `.8b`/`.16b` (byte lanes; size=01 is a fixed
// field of the group). The ARMv8-A ARM "Advanced SIMD three same" table marks
// every other arrangement UNALLOCATED for BIC, and LLVM rejects them with
// "invalid operand for instruction". The encoder should return `Err`.
//
// This is a *property* that currently FAILS (the impl maps any
// `arr != "16b"` to Q=0 and emits a valid-looking byte-BIC word). It is
// `#[ignore]`d so the default `cargo test` stays green; the failing,
// shrunk counterexample it produces documents the SUT violation. Run with
// `cargo test -- --ignored bic_rejects_non_byte` to reproduce. See
// `pbt-out/bug_reports/encode_neon_bic_non_byte_arrangement.md`.
proptest! {
    #[test]
    #[ignore]
    fn bic_rejects_non_byte(
        arr in prop_oneof![
            Just("4h"), Just("8h"), Just("2s"), Just("4s"), Just("1d"), Just("2d"),
        ],
    ) {
        let ops = vec![va(0, arr), va(1, arr), va(2, arr)];
        let res = encode_neon_bic(&ops);
        prop_assert!(
            res.is_err(),
            "BIC does not support .{arr} (only .8b/.16b are allocated); \
             expected Err but got {:?}",
            res.as_ref().map(|e| match e {
                EncodeResult::Word(w) => format!("Ok(0x{w:08X})"),
                _ => format!("{e:?}"),
            }),
        );
    }
}
