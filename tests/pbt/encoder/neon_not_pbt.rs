//! Property-based tests for `encode_neon_not`.
//!
//! `encode_neon_not` encodes the AArch64 NEON `NOT` (bitwise NOT) instruction —
//! `NOT Vd.T, Vn.T`, an alias of `MVN` — in the "Advanced SIMD two-register
//! miscellaneous" encoding group:
//!
//! ```text
//!   31 30 29 28-24 23-22 21-17 16-12 11-10 9-5 4-0
//!    0  Q  U  01110  size  10000  opcode 10  Rn  Rd
//! ```
//! with **U=1**, **size=00**, **opcode=00101**. Per the ARMv8-A ARM
//! (ARM DDI 0487, "Advanced SIMD two-register miscellaneous", MVN/NOT row)
//! `NOT` is defined **only for `.8b` (Q=0) and `.16b` (Q=1)** — byte lanes.
//! Every other arrangement is UNALLOCATED.
//!
//! ## Oracle
//! The golden words below were produced and independently verified against
//! LLVM's AArch64 assembler (`clang --target=aarch64`):
//!
//! ```text
//!   not v0.8b,  v1.8b    -> 0x2E205820
//!   not v0.16b, v1.16b   -> 0x6E205820
//!   mvn v5.16b, v6.16b   -> 0x6E2058C5
//!   mvn v31.8b, v30.8b   -> 0x2E205BDF
//!   not v10.16b,v11.16b  -> 0x6E20596A
//! ```
//! The reference encoder below is assembled field-by-field from the documented
//! layout and is structurally independent of the crate implementation (it builds
//! `opcode5<<12 | 0b10<<10`, whereas the impl ORs `0b00101<<12 | 0b10<<10`
//! inside the same expression), so a shared off-by-one would still be caught by
//! the absolute golden check.
//!
//! ## Finding 1 (documented by the `#[ignore]`d test `not_rejects_non_byte`)
//! `NOT` is architecturally defined only for `.8b`/`.16b` (size=00 is fixed; the
//! group is byte-only). LLVM rejects `.4h/.8h/.2s/.4s/.1d/.2d` with
//! "invalid operand for instruction", but `encode_neon_not` accepts any
//! arrangement: it maps `arr != "16b"` to `Q=0` and emits a valid-looking
//! byte-NOT word, silently corrupting the instruction.
//!
//! ## Finding 2 (documented by the `#[ignore]`d test `not_rejects_mismatched_arrangements`)
//! `NOT` requires `Vd` and `Vn` to share the same arrangement. LLVM rejects
//! mismatched forms (e.g. `not v0.8b, v1.16b`) with "invalid operand for
//! instruction", but `encode_neon_not` discards the source arrangement
//! (`let (rn, _) = ...`) and derives Q solely from `arr_d`, silently encoding
//! the mismatch as the destination's byte form. Both findings share one root
//! cause — missing arrangement validation. See
//! `pbt-out/bug_reports/encode_neon_not_non_byte_arrangement.md` and
//! `pbt-out/bug_reports/encode_neon_not_mismatched_arrangements.md`.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_not;
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

/// Arrangements that are architecturally VALID for NOT (byte lanes only).
fn valid_arrangement_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("8b"), Just("16b")]
}

/// Independent reference encoder assembled field-by-field from the ARM layout:
///   0 Q U 01110 size 10000 opcode5 10 Rn Rd   with U=1, size=00, opcode5=00101.
fn ref_encode_not(rd: u32, rn: u32, arr: &str) -> u32 {
    let q: u32 = if arr == "16b" { 1 } else { 0 };
    let mut w = 0u32;
    w |= q << 30; // bit 31 stays 0
    w |= 1u32 << 29; // U = 1
    w |= 0b01110u32 << 24; // bits 28-24
    w |= 0b00u32 << 22; // size [23:22] = 00 (fixed for NOT)
    w |= 0b10000u32 << 17; // bits 21-17
    w |= 0b00101u32 << 12; // opcode5 bits 16-12
    w |= 0b10u32 << 10; // bits 11-10
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

const GOLDEN: &[(u32, u32, &str, u32)] = &[
    // (Rd, Rn, arrangement, expected_word)
    (0, 1, "8b", 0x2E205820),    // not v0.8b,  v1.8b
    (0, 1, "16b", 0x6E205820),   // not v0.16b, v1.16b   (Q=1)
    (5, 6, "16b", 0x6E2058C5),   // mvn v5.16b, v6.16b
    (31, 30, "8b", 0x2E205BDF),  // mvn v31.8b, v30.8b
    (10, 11, "16b", 0x6E20596A), // not v10.16b,v11.16b
];

#[test]
fn not_matches_golden_table() {
    for &(rd, rn, arr, expected) in GOLDEN {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let got = word_of(encode_neon_not(&ops));
        assert_eq!(
            got, expected,
            "not v{rd}.{arr}, v{rn}.{arr}: got 0x{got:08X}, want 0x{expected:08X}",
        );
        assert_eq!(
            ref_encode_not(rd, rn, arr),
            expected,
            "reference encoder drift"
        );
    }
}

// --- arity / operand-shape contracts --------------------------------------

/// NOT requires two register operands. Fewer must yield `Err` (matches LLVM's
/// "too few operands for instruction").
#[test]
fn not_rejects_too_few_operands() {
    let one = vec![va(0, "16b")];
    assert!(encode_neon_not(&one).is_err(), "1 operand must error");
}

/// A non-register operand in a register slot must yield `Err`, not panic or
/// silently encode.
#[test]
fn not_rejects_non_register_operand() {
    let ops = vec![va(0, "16b"), Operand::Imm(2)];
    assert!(
        encode_neon_not(&ops).is_err(),
        "immediate in Rn slot must error"
    );
}

// --- properties -----------------------------------------------------------

proptest! {
    // === Oracle: reference encoder (differential) =========================
    // For every valid (.8b/.16b) arrangement and register pair, the
    // implementation must equal the independently-assembled reference word.
    #[test]
    fn matches_reference_encoder(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let got = word_of(encode_neon_not(&ops));
        let want = ref_encode_not(rd, rn, arr);
        prop_assert_eq!(got, want);
    }

    // === Field placement: Rd/Rn round-trip + Q mapping ===================
    // The five-bit register fields must round-trip exactly (no silent
    // truncation of in-range register numbers), Q must equal (arr == "16b"),
    // and size (bits 23-22) must be 00 for every valid NOT (it is a fixed
    // field, independent of arrangement).
    #[test]
    fn fields_round_trip_and_map_arrangement(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let w = word_of(encode_neon_not(&ops));
        let q = if arr == "16b" { 1u32 } else { 0u32 };

        prop_assert_eq!(w & 0x1F, rd, "Rd field");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field");
        prop_assert_eq!((w >> 30) & 0x1, q, "Q bit (1 only for .16b)");
        prop_assert_eq!((w >> 22) & 0x3, 0b00u32, "size bits must be 00 (NOT fixed field)");
    }

    // === Fixed-bits invariant =============================================
    // The architecturally-constant bits of the NOT encoding never change for
    // any valid input: bit31=0, U(bit29)=1, bits28-24=01110, bits21-17=10000,
    // opcode5(bits16-12)=00101, bits11-10=10.
    #[test]
    fn fixed_bits_are_constant(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let w = word_of(encode_neon_not(&ops));

        prop_assert_eq!((w >> 31) & 1, 0, "bit 31 must be 0");
        prop_assert_eq!((w >> 29) & 1, 1, "U bit must be 1 for NOT");
        prop_assert_eq!((w >> 24) & 0x1F, 0b01110, "bits 28-24");
        prop_assert_eq!((w >> 17) & 0x1F, 0b10000, "bits 21-17");
        prop_assert_eq!((w >> 12) & 0x1F, 0b00101, "opcode5 bits 16-12");
        prop_assert_eq!((w >> 10) & 0x3, 0b10, "bits 11-10");
    }

    // === Determinism / referential transparency ==========================
    // Re-encoding the same operands twice must yield the identical word.
    #[test]
    fn encoding_is_deterministic(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let w1 = word_of(encode_neon_not(&ops));
        let w2 = word_of(encode_neon_not(&ops));
        prop_assert_eq!(w1, w2);
    }
}

// --- documented finding: non-byte arrangements not rejected ---------------

/// `NOT` is only defined for `.8b`/`.16b` (byte lanes; size=00 is a fixed field
/// of the group). The ARMv8-A ARM "Advanced SIMD two-register miscellaneous"
/// table marks every other arrangement UNALLOCATED for MVN/NOT, and LLVM
/// rejects them with "invalid operand for instruction". The encoder should
/// return `Err`.
///
/// This test is `#[ignore]`d because the current implementation emits a
/// valid-looking byte-NOT word (mapping any `arr != "16b"` to Q=0) instead of
/// returning `Err` — i.e. it does NOT meet the contract.
/// Run with `cargo test -- --ignored not_rejects_non_byte` to reproduce.
/// See `NEON_NOT_NONBYTE_BUG_REPORT.md`.
#[test]
#[ignore]
fn not_rejects_non_byte() {
    for arr in &["4h", "8h", "2s", "4s", "1d", "2d"] {
        let ops = vec![va(0, arr), va(1, arr)];
        let res = encode_neon_not(&ops);
        assert!(
            res.is_err(),
            "NOT does not support .{arr} (only .8b/.16b are allocated); \
             expected Err but got Ok(0x{:08X})",
            res.as_ref()
                .ok()
                .map(|e| match e {
                    EncodeResult::Word(w) => *w,
                    _ => 0,
                })
                .unwrap_or(0),
        );
    }
}

// --- documented finding 2: mismatched Vd/Vn arrangements not rejected -------

/// `NOT` requires `Vd` and `Vn` to share the same arrangement (both `.8b` or
/// both `.16b`). LLVM rejects mismatched forms (e.g. `not v0.8b, v1.16b`) with
/// "invalid operand for instruction". The encoder discards the source operand's
/// arrangement (`let (rn, _) = ...`) and derives Q solely from `arr_d`, so a
/// mismatch is silently encoded as the destination's byte form instead of
/// returning `Err`.
///
/// This test is `#[ignore]`d because the current implementation does NOT
/// validate that the two arrangements match.
/// Run with `cargo test -- --ignored not_rejects_mismatched_arrangements`.
#[test]
#[ignore]
fn not_rejects_mismatched_arrangements() {
    for (ad, an) in &[("8b", "16b"), ("16b", "8b")] {
        let ops = vec![va(0, ad), va(1, an)];
        let res = encode_neon_not(&ops);
        assert!(
            res.is_err(),
            "NOT requires Vd and Vn to share arrangement; \
             not v0.{ad}, v1.{an}: expected Err but got Ok(0x{:08X})",
            res.as_ref()
                .ok()
                .map(|e| match e {
                    EncodeResult::Word(w) => *w,
                    _ => 0,
                })
                .unwrap_or(0),
        );
    }
}
