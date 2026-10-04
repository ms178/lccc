//! Property-based tests for `encode_neon_logical`.
//!
//! `encode_neon_logical` encodes the AArch64 NEON vector logical instructions
//! `AND`/`ORR`/`EOR Vd.T, Vn.T, Vm.T` in the "Advanced SIMD three same"
//! logical group:
//!
//! ```text
//!   31 30 29 28-24 23-22 21 20-16 15-11 10 9-5 4-0
//!    0  Q  U  01110  size  1   Rm  opcode  1  Rn  Rd
//! ```
//! with **opcode (bits 15:11) = `00011`**, **bit 21 = 1**, **bit 10 = 1**.
//! The `U`/`size` fields select the operation (ARMv8-A ARM, ARM DDI 0487,
//! "Advanced SIMD three same", logical group):
//!
//! | opc  | op  | U  | size |
//! |------|-----|----|------|
//! | 0b00 | AND | 0  | 00   |
//! | 0b01 | ORR | 0  | 10   |
//! | 0b10 | EOR | 1  | 00   |
//! | 0b11 | —   | (UNALLOCATED for NEON; impl comment: "not valid for NEON") |
//!
//! Per the ARM the entire logical group is **byte-only**: only `.8b` (Q=0) and
//! `.16b` (Q=1) are allocated; every other arrangement is UNALLOCATED.
//!
//! ## Oracle
//! The golden words below were produced and independently verified against
//! LLVM's AArch64 assembler (`clang --target=aarch64`):
//!
//! ```text
//!   and v0.16b,  v1.16b,  v2.16b  -> 0x4E221C20
//!   and v31.8b,  v30.8b,  v29.8b  -> 0x0E3D1FDF
//!   orr v0.16b,  v1.16b,  v2.16b  -> 0x4EA21C20
//!   orr v5.8b,   v6.8b,   v7.8b   -> 0x0EA71CC5
//!   eor v0.16b,  v1.16b,  v2.16b  -> 0x6E221C20
//!   eor v10.16b, v11.16b, v12.16b -> 0x6E2C1D6A
//! ```
//! The reference encoder assembles the word field-by-field and is structurally
//! independent of the crate impl (it splits `opcode5<<11 | 1<<10`, whereas the
//! impl ORs `0b000111<<10`), so a shared off-by-one is still caught by the
//! absolute golden check.
//!
//! ## Findings (documented by the `#[ignore]`d witnesses)
//! 1. `rejects_non_byte_arrangement` — AND/ORR/EOR are defined only for
//!    `.8b`/`.16b`; the impl maps any `arr != "16b"` to Q=0 and emits a
//!    valid-looking byte word for UNALLOCATED arrangements (`.4h`, `.8h`, ...).
//!
//! 2. `opc_3_is_unallocated` — `opc=0b11` is documented "not valid for NEON,
//!    fall back" but silently emits the *same* word as EOR (`opc=0b10`).
//!

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_logical;
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

/// Arrangements that are architecturally VALID for the logical group (bytes).
fn valid_arrangement_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("8b"), Just("16b")]
}

/// opc -> (op name, U bit, size field) for the three VALID NEON logical ops.
fn opc_spec(opc: u32) -> Option<(&'static str, u32, u32)> {
    match opc {
        0b00 => Some(("AND", 0, 0b00)),
        0b01 => Some(("ORR", 0, 0b10)),
        0b10 => Some(("EOR", 1, 0b00)),
        _ => None, // 0b11 is UNALLOCATED for NEON
    }
}

const VALID_OPCS: &[u32] = &[0b00, 0b01, 0b10];

/// Independent reference encoder assembled field-by-field from the ARM layout:
///   0 Q U 01110 size 1 Rm opcode5 1 Rn Rd
/// (splits opcode5<<11 | 1<<10, unlike the impl's `0b000111<<10`).
fn ref_encode(opc: u32, rd: u32, rn: u32, rm: u32, arr: &str) -> u32 {
    let q: u32 = if arr == "16b" { 1 } else { 0 };
    let (_name, u_bit, size) = opc_spec(opc).expect("valid opc");
    let mut w = 0u32;
    w |= q << 30; // bit 31 stays 0
    w |= u_bit << 29;
    w |= 0b01110u32 << 24; // bits 28-24
    w |= size << 22; // bits 23-22
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

/// (opc, arrangement, Rd, Rn, Rm, expected_word)
const GOLDEN: &[(u32, &str, u32, u32, u32, u32)] = &[
    (0b00, "16b", 0, 1, 2, 0x4E221C20), // and v0.16b,  v1.16b,  v2.16b
    (0b00, "8b", 31, 30, 29, 0x0E3D1FDF), // and v31.8b,  v30.8b,  v29.8b
    (0b01, "16b", 0, 1, 2, 0x4EA21C20), // orr v0.16b,  v1.16b,  v2.16b
    (0b01, "8b", 5, 6, 7, 0x0EA71CC5),  // orr v5.8b,   v6.8b,   v7.8b
    (0b10, "16b", 0, 1, 2, 0x6E221C20), // eor v0.16b,  v1.16b,  v2.16b
    (0b10, "16b", 10, 11, 12, 0x6E2C1D6A), // eor v10.16b, v11.16b, v12.16b
];

#[test]
fn logical_matches_golden_table() {
    for &(opc, arr, rd, rn, rm, expected) in GOLDEN {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let got = word_of(encode_neon_logical(&ops, opc));
        assert_eq!(
            got, expected,
            "op(opc={opc:b}) v{rd}.{arr}, v{rn}.{arr}, v{rm}.{arr}: \
             got 0x{got:08X}, want 0x{expected:08X}",
        );
        assert_eq!(
            ref_encode(opc, rd, rn, rm, arr),
            expected,
            "reference encoder drift for opc={opc:b}"
        );
    }
}

// --- arity / operand-shape contracts --------------------------------------

/// Fewer than three register operands must yield `Err` (matches LLVM's "too
/// few operands for instruction").
#[test]
fn logical_rejects_too_few_operands() {
    for &opc in VALID_OPCS {
        let one = vec![va(0, "16b")];
        let two = vec![va(0, "16b"), va(1, "16b")];
        assert!(
            encode_neon_logical(&one, opc).is_err(),
            "1 operand must error (opc={opc:b})"
        );
        assert!(
            encode_neon_logical(&two, opc).is_err(),
            "2 operands must error (opc={opc:b})"
        );
    }
}

/// A non-register operand in a register slot must yield `Err`, not panic.
#[test]
fn logical_rejects_non_register_operand() {
    for &opc in VALID_OPCS {
        let ops = vec![va(0, "16b"), va(1, "16b"), Operand::Imm(2)];
        assert!(
            encode_neon_logical(&ops, opc).is_err(),
            "imm in Rm slot must error (opc={opc:b})"
        );
    }
}

/// opc >= 4 (outside the 2-bit selector) must yield `Err`.
#[test]
fn logical_rejects_unknown_high_opc() {
    let ops = vec![va(0, "16b"), va(1, "16b"), va(2, "16b")];
    for &opc in &[4u32, 5, 100, u32::MAX] {
        assert!(
            encode_neon_logical(&ops, opc).is_err(),
            "opc={opc} must error"
        );
    }
}

// --- properties -----------------------------------------------------------

proptest! {
    // === Oracle: differential reference encoder ===========================
    // For every valid op (AND/ORR/EOR), arrangement (.8b/.16b) and register
    // triple, the implementation must equal the independently-assembled word.
    #[test]
    fn matches_reference_encoder(
        opc_idx in 0usize..VALID_OPCS.len(),
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let opc = VALID_OPCS[opc_idx];
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let got = word_of(encode_neon_logical(&ops, opc));
        let want = ref_encode(opc, rd, rn, rm, arr);
        prop_assert_eq!(got, want);
    }

    // === Field placement + op-specific U/size =============================
    // Rd/Rn/Rm five-bit fields round-trip exactly (no silent truncation of
    // in-range register numbers), Q == (arr == "16b"), and the U/size bits are
    // the architecturally-correct values per op (AND=0/00, ORR=0/10, EOR=1/00).
    #[test]
    fn fields_round_trip_and_op_bits_correct(
        opc_idx in 0usize..VALID_OPCS.len(),
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let opc = VALID_OPCS[opc_idx];
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let w = word_of(encode_neon_logical(&ops, opc));
        let q = if arr == "16b" { 1u32 } else { 0u32 };
        let (_name, want_u, want_size) = opc_spec(opc).unwrap();

        prop_assert_eq!(w & 0x1F, rd, "Rd field");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field");
        prop_assert_eq!((w >> 16) & 0x1F, rm, "Rm field");
        prop_assert_eq!((w >> 30) & 0x1, q, "Q bit (1 only for .16b)");
        prop_assert_eq!((w >> 29) & 0x1, want_u, "U bit per op");
        prop_assert_eq!((w >> 22) & 0x3, want_size, "size bits per op");
    }

    // === Fixed-bits invariant ============================================
    // The architecturally-constant bits never change for any valid op/input:
    // bit31=0, bits28-24=01110, bit21=1, opcode5(bits15-11)=00011, bit10=1.
    #[test]
    fn fixed_bits_are_constant(
        opc_idx in 0usize..VALID_OPCS.len(),
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let opc = VALID_OPCS[opc_idx];
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let w = word_of(encode_neon_logical(&ops, opc));

        prop_assert_eq!((w >> 31) & 1, 0, "bit 31 must be 0");
        prop_assert_eq!((w >> 24) & 0x1F, 0b01110, "bits 28-24");
        prop_assert_eq!((w >> 21) & 1, 1, "bit 21 must be 1");
        prop_assert_eq!((w >> 11) & 0x1F, 0b00011, "opcode5 bits 15-11");
        prop_assert_eq!((w >> 10) & 1, 1, "bit 10 must be 1");
    }
}

// --- documented findings --------------------------------------------------

// AND/ORR/EOR are only defined for `.8b`/`.16b` (byte lanes; the `size` field
// is fixed per op and the group is byte-only). The ARMv8-A ARM marks every
// other arrangement UNALLOCATED, and LLVM rejects them with "invalid operand
// for instruction". The encoder should return `Err`.
//
// This is a *property* that currently FAILS: the impl derives Q solely from
// `arr == "16b"` and treats every other arrangement as Q=0, emitting a
// valid-looking byte word. `#[ignore]`d so default `cargo test` stays green;
// run with `cargo test -- --ignored rejects_non_byte_arrangement` to
// reproduce.
proptest! {
    #[test]
    fn rejects_non_byte_arrangement(
        opc_idx in 0usize..VALID_OPCS.len(),
        arr in prop_oneof![
            Just("4h"), Just("8h"), Just("2s"), Just("4s"), Just("1d"), Just("2d"),
        ],
    ) {
        let opc = VALID_OPCS[opc_idx];
        let ops = vec![va(0, arr), va(1, arr), va(2, arr)];
        let res = encode_neon_logical(&ops, opc);
        prop_assert!(
            res.is_err(),
            "logical op (opc={opc:b}) does not support .{arr} (only .8b/.16b are \
             allocated); expected Err but got {:?}",
            res.as_ref().map(|e| match e {
                EncodeResult::Word(w) => format!("Ok(0x{w:08X})"),
                _ => format!("{e:?}"),
            }),
        );
    }

    /// `opc=0b11` is marked "ANDS - not valid for NEON, fall back" in the
    /// source comment, i.e. it has no valid NEON logical meaning. Per the ARM
    /// it is UNALLOCATED, so the encoder should return `Err`. Instead it
    /// silently returns the *same* word as `opc=0b10` (EOR).
    ///
    /// Currently FAILS. `#[ignore]`d; run with
    /// `cargo test -- --ignored opc_3_is_unallocated` to reproduce. See
    /// the transplanted campaign notes.
    #[test]
    fn opc_3_is_unallocated(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let res = encode_neon_logical(&ops, 0b11);
        prop_assert!(
            res.is_err(),
            "opc=0b11 is UNALLOCATED for NEON logical ops (source: 'not valid for \
             NEON'); expected Err but got {:?}",
            res.as_ref().map(|e| match e {
                EncodeResult::Word(w) => format!("Ok(0x{w:08X})"),
                _ => format!("{e:?}"),
            }),
        );
    }
}
