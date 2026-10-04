//! Property-based tests for `encode_neon_pmull`.
//!
//! `encode_neon_pmull(operands, is_pmull2)` encodes the AArch64 NEON
//! polynomial-multiply-long instructions `PMULL`/`PMULL2` in the
//! "Advanced SIMD three same" encoding group with **size=11** (the crypto
//! sub-space):
//!
//! ```text
//!   31 30 29 28-24 23-22 21 20-16 15-10 9-5 4-0
//!    0  Q  U  01110  size  1   Rm  111000  Rn  Rd
//! ```
//! with **U=0**, **size=11**, and fixed bits **15-10 = 111000** (opcode
//! =11100 at bits 15-11, bit 10 = 0). Per the ARMv8-A ARM, PMULL is defined
//! **only** as:
//!   * `PMULL  Vd.1Q, Vn.1D, Vm.1D`  (Q=0)
//!   * `PMULL2 Vd.1Q, Vn.2D, Vm.2D`  (Q=1)
//!
//! ## Oracle
//! The golden words below were produced and independently verified against
//! LLVM's AArch64 assembler (`clang --target=aarch64 -march=armv8-a+crypto+aes`):
//!
//! ```text
//!   pmull  v0.1q,  v1.1d,  v2.1d   -> 0x0EE2E020
//!   pmull2 v0.1q,  v1.2d,  v2.2d   -> 0x4EE2E020
//!   pmull  v5.1q,  v6.1d,  v7.1d   -> 0x0EE7E0C5
//!   pmull2 v31.1q, v30.2d, v29.2d  -> 0x4EFDE3DF
//!   pmull  v10.1q, v11.1d, v12.1d  -> 0x0EECE16A
//! ```
//! The reference encoder is assembled field-by-field from the documented
//! layout (splitting `opcode<<11` and leaving bit 10 explicitly 0), so it is
//! structurally independent of the crate's single-OR expression; the absolute
//! LLVM golden check guards against any shared field-placement error.
//!
//! ## Finding (surfaced by the FAILING `proptest!` property `rejects_non_canonical_arrangements`)
//! PMULL/PMULL2 are architecturally defined **only** for `.1Q`/`.1D` (PMULL)
//! and `.1Q`/`.2D` (PMULL2). LLVM rejects every other arrangement
//! (`.8b/.16b/.4h/.8h/.2s/.4s/.1d/.2d`) with "invalid operand for instruction".
//! `encode_neon_pmull` discards all three arrangement strings
//! (`let (rd, _) = ...`) and emits a valid-looking PMULL word for *any*
//! arrangement, silently corrupting the instruction. The property fails and
//! proptest shrinks to a minimal witness. See
//! the transplanted campaign notes.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_pmull;
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

fn is_pmull2_strategy() -> impl Strategy<Value = bool> {
    prop_oneof![Just(false), Just(true)]
}

/// Build the canonical operand list for a PMULL/PMULL2 instruction.
fn canonical_ops(rd: u32, rn: u32, rm: u32, is_pmull2: bool) -> Vec<Operand> {
    let src_arr = if is_pmull2 { "2d" } else { "1d" };
    vec![va(rd, "1q"), va(rn, src_arr), va(rm, src_arr)]
}

/// Independent reference encoder assembled field-by-field from the ARM layout.
fn ref_encode_pmull(rd: u32, rn: u32, rm: u32, is_pmull2: bool) -> u32 {
    let q: u32 = if is_pmull2 { 1 } else { 0 };
    let mut w = 0u32;
    w |= q << 30; // bit 30 = Q; bit 31 stays 0
    // bit 29 (U) = 0
    w |= 0b01110u32 << 24; // bits 28-24
    w |= 0b11u32 << 22; // bits 23-22: size = 11
    w |= 1u32 << 21; // bit 21
    w |= rm << 16; // bits 20-16
    w |= 0b11100u32 << 11; // opcode bits 15-11
    // bit 10 = 0 (left clear)
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
// (Rd, Rn, Rm, is_pmull2, expected_word)
const GOLDEN: &[(u32, u32, u32, bool, u32)] = &[
    (0, 1, 2, false, 0x0EE2E020),    // pmull  v0.1q,  v1.1d,  v2.1d
    (0, 1, 2, true, 0x4EE2E020),     // pmull2 v0.1q,  v1.2d,  v2.2d
    (5, 6, 7, false, 0x0EE7E0C5),    // pmull  v5.1q,  v6.1d,  v7.1d
    (31, 30, 29, true, 0x4EFDE3DF),  // pmull2 v31.1q, v30.2d, v29.2d
    (10, 11, 12, false, 0x0EECE16A), // pmull  v10.1q, v11.1d, v12.1d
];

#[test]
fn pmull_matches_golden_table() {
    for &(rd, rn, rm, is_pmull2, expected) in GOLDEN {
        let ops = canonical_ops(rd, rn, rm, is_pmull2);
        let got = word_of(encode_neon_pmull(&ops, is_pmull2));
        assert_eq!(
            got,
            expected,
            "pmull{} v{rd}.1q, v{rn}.{}, v{rm}.{}: got 0x{got:08X}, want 0x{expected:08X}",
            if is_pmull2 { "2" } else { "" },
            if is_pmull2 { "2d" } else { "1d" },
            if is_pmull2 { "2d" } else { "1d" },
        );
        assert_eq!(
            ref_encode_pmull(rd, rn, rm, is_pmull2),
            expected,
            "reference encoder drift",
        );
    }
}

// --- properties -----------------------------------------------------------

proptest! {
    // === Oracle: reference encoder (differential) =========================
    // For every register triple and both PMULL/PMULL2 forms, the
    // implementation must equal the independently-assembled reference word.
    #[test]
    fn matches_reference_encoder(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        is_pmull2 in is_pmull2_strategy(),
    ) {
        let ops = canonical_ops(rd, rn, rm, is_pmull2);
        let got = word_of(encode_neon_pmull(&ops, is_pmull2));
        let want = ref_encode_pmull(rd, rn, rm, is_pmull2);
        prop_assert_eq!(got, want);
    }

    // === Field placement: Rd/Rn/Rm round-trip + Q mapping ================
    // The five-bit register fields must round-trip exactly (no truncation of
    // in-range register numbers), Q (bit 30) must equal is_pmull2, and size
    // (bits 23-22) must be 11.
    #[test]
    fn fields_round_trip_and_q_maps_is_pmull2(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        is_pmull2 in is_pmull2_strategy(),
    ) {
        let ops = canonical_ops(rd, rn, rm, is_pmull2);
        let w = word_of(encode_neon_pmull(&ops, is_pmull2));

        prop_assert_eq!(w & 0x1F, rd, "Rd field");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field");
        prop_assert_eq!((w >> 16) & 0x1F, rm, "Rm field");
        prop_assert_eq!(
            (w >> 30) & 0x1,
            if is_pmull2 { 1u32 } else { 0u32 },
            "Q bit must equal is_pmull2",
        );
        prop_assert_eq!((w >> 22) & 0x3, 0b11u32, "size bits must be 11");
    }

    // === Fixed-bits invariant =============================================
    // The architecturally-constant bits of the PMULL encoding never change:
    // bit31=0, U(bit29)=0, bits28-24=01110, bit21=1, opcode(bits15-11)=11100,
    // bit10=0.
    #[test]
    fn fixed_bits_are_constant(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        is_pmull2 in is_pmull2_strategy(),
    ) {
        let ops = canonical_ops(rd, rn, rm, is_pmull2);
        let w = word_of(encode_neon_pmull(&ops, is_pmull2));

        prop_assert_eq!((w >> 31) & 1, 0, "bit 31 must be 0");
        prop_assert_eq!((w >> 29) & 1, 0, "U bit must be 0 for PMULL");
        prop_assert_eq!((w >> 24) & 0x1F, 0b01110, "bits 28-24");
        prop_assert_eq!((w >> 21) & 1, 1, "bit 21 must be 1");
        prop_assert_eq!((w >> 11) & 0x1F, 0b11100, "opcode bits 15-11");
        prop_assert_eq!((w >> 10) & 1, 0, "bit 10 must be 0 for PMULL");
    }

    // === Error contract: operand count ===================================
    // Fewer than 3 operands must be rejected with Err, regardless of the
    // PMULL/PMULL2 flag.
    #[test]
    fn rejects_too_few_operands(
        n in 0usize..3,
        is_pmull2 in is_pmull2_strategy(),
    ) {
        let ops: Vec<Operand> = (0..n).map(|_| va(0, "1d")).collect();
        let res = encode_neon_pmull(&ops, is_pmull2);
        prop_assert!(
            res.is_err(),
            "expected Err for {} operands, got {:?}",
            n, res,
        );
    }

    // === Negative contract: non-canonical arrangements MUST be rejected =====
    // PMULL/PMULL2 are defined ONLY for a .1q destination with .1d (PMULL) or
    // .2d (PMULL2) sources (ARMv8-A ARM, "Advanced SIMD three same",
    // size=11). Every other arrangement is UNALLOCATED and LLVM rejects it
    // with "invalid operand for instruction". Applying any arrangement below
    // to all three operands makes the destination non-.1q, so the encoder
    // MUST return Err.
    //
    // CURRENT STATUS: this property FAILS (real SUT bug). proptest shrinks to
    // a minimal witness; see the transplanted campaign notes.
    #[test]
    fn rejects_non_canonical_arrangements(
        arr in prop_oneof![
            Just("8b"), Just("16b"), Just("4h"), Just("8h"),
            Just("2s"), Just("4s"), Just("1d"), Just("2d"),
        ],
        is_pmull2 in is_pmull2_strategy(),
    ) {
        let ops = vec![va(0, arr), va(1, arr), va(2, arr)];
        let res = encode_neon_pmull(&ops, is_pmull2);
        prop_assert!(
            res.is_err(),
            "pmull{} with .{arr} on all operands is UNALLOCATED \
             (only .1q/.1d or .1q/.2d); expected Err, got {:?}",
            if is_pmull2 { "2" } else { "" }, res,
        );
    }
}
