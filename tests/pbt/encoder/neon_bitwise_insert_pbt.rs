//! Property-based tests for `encode_neon_bitwise_insert`.
//!
//! `encode_neon_bitwise_insert` encodes the AArch64 NEON `BIT` (Bitwise Insert
//! if True) and `BIF` (Bitwise Insert if False) instructions —
//! `BIT/BIF Vd.T, Vn.T, Vm.T` — in the "Advanced SIMD three same" encoding
//! group:
//!
//! ```text
//!   31 30 29 28-24 23-22 21 20-16 15-11 10 9-5 4-0
//!    0  Q  U  01110  size  1   Rm  opcode  1  Rn  Rd
//! ```
//! with **U=1**, **opcode=00011**, **bit10=1**, and the `size` field selecting
//! the variant: **size=10 → BIT**, **size=11 → BIF** (the encoder is dispatched
//! with these two values from `mod.rs`). Per the ARMv8-A ARM (ARM DDI 0487,
//! "Advanced SIMD three same", BIT/BIF rows) BIT/BIF are defined **only for
//! `.8b` (Q=0) and `.16b` (Q=1)** — byte lanes, like the rest of the
//! three-same logical group (AND/BIC/ORR/ORN/EOR/BSL/BIT/BIF). Every other
//! arrangement is UNALLOCATED.
//!
//! ## Oracle
//! The golden words below were produced and independently verified against
//! LLVM's AArch64 assembler (`clang --target=aarch64`):
//!
//! ```text
//!   bit v0.16b, v1.16b, v2.16b    -> 0x6EA21C20   (size=10)
//!   bif v0.16b, v1.16b, v2.16b    -> 0x6EE21C20   (size=11)
//!   bit v0.8b,  v1.8b,  v2.8b     -> 0x2EA21C20
//!   bif v0.8b,  v1.8b,  v2.8b     -> 0x2EE21C20
//!   bit v31.16b,v30.16b,v29.16b   -> 0x6EBD1FDF
//!   bif v7.8b,  v8.8b,  v9.8b     -> 0x2EE91D07
//! ```
//! The reference encoder below is assembled field-by-field from the documented
//! layout and is structurally independent of the crate implementation (it splits
//! `opcode5<<11 | 1<<10`, whereas the impl ORs `0b000111<<10`), so a shared
//! off-by-one would still be caught by the absolute golden check.
//!
//! ## Findings (documented by the `#[ignore]`d witnesses)
//!
//! 1. **Non-byte arrangements not rejected** (`bit_bif_reject_non_byte`):
//!    BIT/BIF are architecturally defined only for `.8b`/`.16b`. LLVM rejects
//!    `.4h/.8h/.2s/.4s/.1d/.2d` with "invalid operand for instruction", but the
//!    encoder maps any `arr != "16b"` to `Q=0` and emits a valid-looking
//!    byte-BIT/BIF word. See
//!    `pbt-out/bug_reports/encode_neon_bitwise_insert_non_byte_arrangement.md`.
//!
//! 2. **`size` parameter not range-checked** (`bit_bif_rejects_unallocated_size`):
//!    SUT observation surfaced by *source-reading* (NOT a shrunk PBT property),
//!    hence documented in `pbt-out/REPORT.md` Design Caveats rather than in a
//!    bug report. The function is documented only for `size ∈ {0b10, 0b11}` but
//!    accepts any value verbatim, silently emitting a *different* logical-group
//!    instruction (e.g. `size=0b00` → ORN, `size=0b01` → BSL). Latent: the
//!    mnemonic dispatcher (`mod.rs:776-777`) only ever passes `{0b10, 0b11}`.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_bitwise_insert;
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

/// The two `size` values the dispatcher passes: 0b10 (BIT) and 0b11 (BIF).
fn size_strategy() -> impl Strategy<Value = u32> {
    prop_oneof![Just(0b10u32), Just(0b11u32)]
}

/// Arrangements that are architecturally VALID for BIT/BIF (byte lanes only).
fn valid_arrangement_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("8b"), Just("16b")]
}

/// Independent reference encoder assembled field-by-field from the ARM layout:
///   0 Q U 01110 size 1 Rm opcode5 1 Rn Rd   with U=1, opcode5=00011.
/// `size` is masked to its 2-bit field (a no-op for the {2,3} inputs we feed).
fn ref_encode_bitwise(rd: u32, rn: u32, rm: u32, arr: &str, size: u32) -> u32 {
    let q: u32 = if arr == "16b" { 1 } else { 0 };
    let mut w = 0u32;
    w |= q << 30; // bit 31 stays 0
    w |= 1u32 << 29; // U = 1
    w |= 0b01110u32 << 24; // bits 28-24
    w |= (size & 0b11) << 22; // size [23:22]
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

/// (Rd, Rn, Rm, arrangement, size, expected_word)
const GOLDEN: &[(u32, u32, u32, &str, u32, u32)] = &[
    (0, 1, 2, "16b", 0b10, 0x6EA21C20), // bit v0.16b, v1.16b, v2.16b
    (0, 1, 2, "16b", 0b11, 0x6EE21C20), // bif v0.16b, v1.16b, v2.16b
    (0, 1, 2, "8b", 0b10, 0x2EA21C20),  // bit v0.8b,  v1.8b,  v2.8b   (Q=0)
    (0, 1, 2, "8b", 0b11, 0x2EE21C20),  // bif v0.8b,  v1.8b,  v2.8b
    (31, 30, 29, "16b", 0b10, 0x6EBD1FDF), // bit v31.16b,v30.16b,v29.16b
    (7, 8, 9, "8b", 0b11, 0x2EE91D07),  // bif v7.8b,  v8.8b,  v9.8b
];

#[test]
fn bitwise_matches_golden_table() {
    for &(rd, rn, rm, arr, size, expected) in GOLDEN {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let got = word_of(encode_neon_bitwise_insert(&ops, size));
        assert_eq!(
            got, expected,
            "size={size:#04b} v{rd}.{arr}, v{rn}.{arr}, v{rm}.{arr}: \
             got 0x{got:08X}, want 0x{expected:08X}",
        );
        assert_eq!(
            ref_encode_bitwise(rd, rn, rm, arr, size),
            expected,
            "reference encoder drift",
        );
    }
}

// --- arity / operand-shape contracts --------------------------------------

/// BIT/BIF require three register operands. Fewer must yield `Err` (matches
/// LLVM's "too few operands for instruction").
#[test]
fn bitwise_rejects_too_few_operands() {
    for size in [0b10u32, 0b11u32] {
        let one = vec![va(0, "16b")];
        let two = vec![va(0, "16b"), va(1, "16b")];
        assert!(
            encode_neon_bitwise_insert(&one, size).is_err(),
            "{size:b}: 1 operand must error"
        );
        assert!(
            encode_neon_bitwise_insert(&two, size).is_err(),
            "{size:b}: 2 operands must error"
        );
    }
}

/// A non-register operand in a register slot must yield `Err`, not panic or
/// silently encode.
#[test]
fn bitwise_rejects_non_register_operand() {
    for size in [0b10u32, 0b11u32] {
        let ops = vec![va(0, "16b"), va(1, "16b"), Operand::Imm(2)];
        assert!(
            encode_neon_bitwise_insert(&ops, size).is_err(),
            "{size:b}: immediate in Rm slot must error"
        );
    }
}

// --- properties -----------------------------------------------------------

proptest! {
    // === Oracle: reference encoder (differential) =========================
    // For every valid (.8b/.16b) arrangement, both size variants (BIT/BIF),
    // and register triple, the implementation must equal the independently
    // assembled reference word.
    #[test]
    fn matches_reference_encoder(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
        size in size_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let got = word_of(encode_neon_bitwise_insert(&ops, size));
        let want = ref_encode_bitwise(rd, rn, rm, arr, size);
        prop_assert_eq!(got, want);
    }

    // === Field placement: Rd/Rn/Rm round-trip + Q + size mapping =========
    // The five-bit register fields must round-trip exactly (no silent
    // truncation of in-range register numbers), Q must equal (arr == "16b"),
    // and the size field (bits 23-22) must echo the input size for the valid
    // values {2,3}.
    #[test]
    fn fields_round_trip_and_map_arrangement(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
        size in size_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let w = word_of(encode_neon_bitwise_insert(&ops, size));
        let q = if arr == "16b" { 1u32 } else { 0u32 };

        prop_assert_eq!(w & 0x1F, rd, "Rd field");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field");
        prop_assert_eq!((w >> 16) & 0x1F, rm, "Rm field");
        prop_assert_eq!((w >> 30) & 0x1, q, "Q bit (1 only for .16b)");
        prop_assert_eq!((w >> 22) & 0x3, size, "size bits must echo input size");
    }

    // === Fixed-bits invariant =============================================
    // The architecturally-constant bits of the BIT/BIF encoding never change
    // for any valid input: bit31=0, U(bit29)=1, bits28-24=01110, bit21=1,
    // opcode5(bits15-11)=00011, bit10=1.
    #[test]
    fn fixed_bits_are_constant(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
        size in size_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let w = word_of(encode_neon_bitwise_insert(&ops, size));

        prop_assert_eq!((w >> 31) & 1, 0, "bit 31 must be 0");
        prop_assert_eq!((w >> 29) & 1, 1, "U bit must be 1 for BIT/BIF");
        prop_assert_eq!((w >> 24) & 0x1F, 0b01110, "bits 28-24");
        prop_assert_eq!((w >> 21) & 1, 1, "bit 21 must be 1");
        prop_assert_eq!((w >> 11) & 0x1F, 0b00011, "opcode5 bits 15-11");
        prop_assert_eq!((w >> 10) & 1, 1, "bit 10 must be 1");
    }

    // === Determinism / referential transparency ==========================
    // The same inputs must always produce the same word.
    #[test]
    fn encoding_is_deterministic(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
        size in size_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let w1 = word_of(encode_neon_bitwise_insert(&ops, size));
        let w2 = word_of(encode_neon_bitwise_insert(&ops, size));
        prop_assert_eq!(w1, w2);
    }

    // === BIT vs BIF differ only in the size field ========================
    // For identical registers/arrangement, BIT (size=2) and BIF (size=3) must
    // differ ONLY in bits 23-22 (the size field): all other bits identical,
    // and bit 22 set (BIF) vs cleared (BIT).
    #[test]
    fn bit_and_bif_differ_only_in_size(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let bit_w = word_of(encode_neon_bitwise_insert(&ops, 0b10));
        let bif_w = word_of(encode_neon_bitwise_insert(&ops, 0b11));
        prop_assert_eq!(bit_w & !(0b11u32 << 22), bif_w & !(0b11u32 << 22),
            "BIT/BIF must agree on every field except size");
        prop_assert_eq!((bit_w >> 22) & 1, 0, "BIT clears size[0] (bit 22)");
        prop_assert_eq!((bif_w >> 22) & 1, 1, "BIF sets size[0] (bit 22)");
    }
}

// --- documented finding 1: non-byte arrangements not rejected -------------

/// BIT/BIF are only defined for `.8b`/`.16b` (byte lanes; the three-same
/// logical group is byte-only). The ARMv8-A ARM "Advanced SIMD three same"
/// table marks every other arrangement UNALLOCATED, and LLVM rejects them with
/// "invalid operand for instruction". The encoder should return `Err`.
///
/// This test is `#[ignore]`d because the current implementation emits a
/// valid-looking byte-BIT/BIF word (mapping any `arr != "16b"` to Q=0) instead
/// of returning `Err` — i.e. it does NOT meet the contract.
/// Run with `cargo test -- --ignored bit_bif_reject_non_byte` to reproduce.
/// See `pbt-out/bug_reports/encode_neon_bitwise_insert_non_byte_arrangement.md`.
#[test]
#[ignore]
fn bit_bif_reject_non_byte() {
    for size in [0b10u32, 0b11u32] {
        for arr in &["4h", "8h", "2s", "4s", "1d", "2d"] {
            let ops = vec![va(0, arr), va(1, arr), va(2, arr)];
            let res = encode_neon_bitwise_insert(&ops, size);
            let got = res
                .as_ref()
                .ok()
                .map(|e| match e {
                    EncodeResult::Word(w) => *w,
                    _ => 0,
                })
                .unwrap_or(0);
            assert!(
                res.is_err(),
                "size={size:#04b} .{arr}: BIT/BIF only supports .8b/.16b; \
                 expected Err but got Ok(0x{got:08X})",
            );
        }
    }
}

// --- documented finding 2: unallocated `size` values not rejected ---------

/// SUT OBSERVATION (source-reading, not a PBT-surfaced bug — documented in
/// `pbt-out/REPORT.md` Design Caveats, NOT in `## Bugs Found`).
///
/// `encode_neon_bitwise_insert` is documented (neon.rs docstring) only for
/// `size = 0b10` (BIT) and `size = 0b11` (BIF). Any other `size` is a
/// *different* logical-group instruction — `size = 0b00` emits an ORN word,
/// `size = 0b01` a BSL word — under this function's name. The encoder passes
/// `size` through verbatim with no range check, so it silently mis-encodes
/// instead of returning `Err`.
///
/// Latent: the mnemonic dispatcher (`mod.rs:776-777`) only ever passes
/// `{0b10, 0b11}`, so this is not reachable via the `bit`/`bif` mnemonics
/// today — but the `pub(crate)` function is unguarded. This `#[ignore]`d
/// witness documents the gap; default `cargo test` stays green.
/// Reproduce with `cargo test -- --ignored bit_bif_rejects_unallocated_size`.
#[test]
#[ignore]
fn bit_bif_rejects_unallocated_size() {
    let ops = vec![va(0, "16b"), va(1, "16b"), va(2, "16b")];
    for size in [0u32, 1u32] {
        let res = encode_neon_bitwise_insert(&ops, size);
        let got = res
            .as_ref()
            .ok()
            .map(|e| match e {
                EncodeResult::Word(w) => *w,
                _ => 0,
            })
            .unwrap_or(0);
        assert!(
            res.is_err(),
            "size={size:#04b}: encode_neon_bitwise_insert is documented only for \
             size=0b10 (BIT) / 0b11 (BIF); expected Err but got Ok(0x{got:08X})",
        );
    }
}
