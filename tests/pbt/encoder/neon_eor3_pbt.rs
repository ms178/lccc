//! Property-based tests for `encode_neon_eor3`.
//!
//! `encode_neon_eor3(operands)` encodes the AArch64 NEON three-way XOR
//! instruction `EOR3` (FEAT_SHA3):
//!
//! ```text
//!   EOR3 Vd.16B, Vn.16B, Vm.16B, Va.16B
//! ```
//! in the "Crypto three-register, SHA3" encoding group:
//!
//! ```text
//!   31-24    23-22  21   20-16  15   14-10   9-5   4-0
//!   11001110  sz     0     Rm     0     Ra      Rn    Rd
//! ```
//! with **sz = 00** (the only size value that decodes as EOR3; BCAX uses the
//! same template with a different fixed bit, RAX1 another). There is **no Q
//! bit** — bits 31-30 are fixed `11` — so EOR3 is architecturally defined
//! **only** for the `.16B` arrangement on all four operands.
//!
//! ## Oracle
//! The golden words below are derived from the ARMv8-A ARM documented bit
//! layout (the field shifts are simple enough that the reference encoder is
//! structurally similar to the SUT, so the absolute golden table is the
//! primary independent oracle):
//!
//! ```text
//!   eor3 v0.16b,  v1.16b,  v2.16b,  v3.16b   -> 0xCE020C20
//!   eor3 v31.16b, v30.16b, v29.16b, v28.16b  -> 0xCE1D73DF
//!   eor3 v5.16b,  v6.16b,  v7.16b,  v8.16b   -> 0xCE0720C5
//!   eor3 v10.16b, v11.16b, v12.16b, v13.16b  -> 0xCE0C356A
//! ```
//! (No aarch64 assembler/llvm-mc was available in this environment to
//! cross-check; the values follow directly from the spec layout above.)
//!
//! ## Finding (surfaced by the FAILING `proptest!` property `rejects_non_canonical_arrangements`)
//! EOR3 is defined ONLY for `.16B` (no Q field exists in the encoding). LLVM's
//! assembler rejects every other arrangement (`8b/4h/8h/2s/4s/1d/2d`) with
//! "invalid operand for instruction". `encode_neon_eor3` discards all four
//! arrangement strings (`let (rd, _) = ...`) and emits an identical,
//! valid-looking EOR3 word for ANY arrangement, silently corrupting the
//! instruction. The property fails and proptest shrinks to a minimal witness.
//! See `pbt-out/bug_reports/eor3-no-arrangement-validation.md`.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_eor3;
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

/// Build the canonical operand list for `EOR3 Vd.16B, Vn.16B, Vm.16B, Va.16B`.
fn canonical_ops(rd: u32, rn: u32, rm: u32, ra: u32) -> Vec<Operand> {
    vec![va(rd, "16b"), va(rn, "16b"), va(rm, "16b"), va(ra, "16b")]
}

/// Independent reference encoder assembled field-by-field from the ARM layout.
/// Computed as a shift-free accumulation of the constant high byte plus a
/// separately packed low word to keep it structurally distinct from the SUT's
/// single OR-chain.
fn ref_encode_eor3(rd: u32, rn: u32, rm: u32, ra: u32) -> u32 {
    let high = 0xCE00_0000u32; // bits 31-24 = 11001110, sz=00, bit21=0
    let mut low = 0u32;
    low |= rm << 16; // bits 20-16
    // bit 15 stays 0
    low |= ra << 10; // bits 14-10 (Ra / Va)
    low |= rn << 5; // bits 9-5
    low |= rd; // bits 4-0
    high | low
}

fn word_of(res: Result<EncodeResult, String>) -> u32 {
    match res {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {other:?}"),
    }
}

// --- golden table (absolute oracle) --------------------------------------
// (Rd, Rn, Rm, Ra, expected_word)
const GOLDEN: &[(u32, u32, u32, u32, u32)] = &[
    (0, 1, 2, 3, 0xCE020C20),     // eor3 v0.16b,  v1.16b,  v2.16b,  v3.16b
    (31, 30, 29, 28, 0xCE1D73DF), // eor3 v31.16b, v30.16b, v29.16b, v28.16b
    (5, 6, 7, 8, 0xCE0720C5),     // eor3 v5.16b,  v6.16b,  v7.16b,  v8.16b
    (10, 11, 12, 13, 0xCE0C356A), // eor3 v10.16b, v11.16b, v12.16b, v13.16b
];

#[test]
fn eor3_matches_golden_table() {
    for &(rd, rn, rm, ra, expected) in GOLDEN {
        let ops = canonical_ops(rd, rn, rm, ra);
        let got = word_of(encode_neon_eor3(&ops));
        assert_eq!(
            got, expected,
            "eor3 v{rd}.16b, v{rn}.16b, v{rm}.16b, v{ra}.16b: \
             got 0x{got:08X}, want 0x{expected:08X}",
        );
        assert_eq!(
            ref_encode_eor3(rd, rn, rm, ra),
            expected,
            "reference encoder drift",
        );
    }
}

// --- properties -----------------------------------------------------------

proptest! {
    // === Oracle: reference encoder (differential) =========================
    // For every register quadruple, the implementation must equal the
    // independently-assembled reference word.
    #[test]
    fn matches_reference_encoder(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        ra in reg_num_strategy(),
    ) {
        let ops = canonical_ops(rd, rn, rm, ra);
        let got = word_of(encode_neon_eor3(&ops));
        let want = ref_encode_eor3(rd, rn, rm, ra);
        prop_assert_eq!(got, want);
    }

    // === Field placement: Rd/Rn/Rm/Ra round-trip =========================
    // The four 5-bit register fields must round-trip exactly with no
    // truncation or cross-field bleed for in-range register numbers.
    #[test]
    fn register_fields_round_trip(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        ra in reg_num_strategy(),
    ) {
        let ops = canonical_ops(rd, rn, rm, ra);
        let w = word_of(encode_neon_eor3(&ops));

        prop_assert_eq!(w & 0x1F, rd, "Rd field (bits 4-0)");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field (bits 9-5)");
        prop_assert_eq!((w >> 10) & 0x1F, ra, "Ra/Va field (bits 14-10)");
        prop_assert_eq!((w >> 16) & 0x1F, rm, "Rm field (bits 20-16)");
    }

    // === Fixed-bits invariant =============================================
    // The architecturally-constant bits never change: bits 31-24 = 11001110,
    // sz (bits 23-22) = 00, bit 21 = 0, bit 15 = 0. (Bits 31-30 being 11 is
    // why no Q bit / no .8B form exists.)
    #[test]
    fn fixed_bits_are_constant(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        ra in reg_num_strategy(),
    ) {
        let ops = canonical_ops(rd, rn, rm, ra);
        let w = word_of(encode_neon_eor3(&ops));

        prop_assert_eq!((w >> 24) & 0xFF, 0xCE, "bits 31-24 must be 0b11001110");
        prop_assert_eq!((w >> 22) & 0x3, 0b00, "sz bits 23-22 must be 00");
        prop_assert_eq!((w >> 21) & 1, 0, "bit 21 must be 0");
        prop_assert_eq!((w >> 15) & 1, 0, "bit 15 must be 0");
    }

    // === Error contract: operand count ===================================
    // Fewer than 4 operands must be rejected with Err.
    #[test]
    fn rejects_too_few_operands(n in 0usize..4) {
        let ops: Vec<Operand> = (0..n).map(|_| va(0, "16b")).collect();
        let res = encode_neon_eor3(&ops);
        prop_assert!(
            res.is_err(),
            "expected Err for {} operands, got {:?}",
            n, res,
        );
    }

    // === Negative contract: non-canonical arrangements MUST be rejected ====
    // EOR3 is defined ONLY for `.16B` (the encoding has no Q field, so an
    // `.8B` form does not exist; every other arrangement is UNALLOCATED and
    // LLVM rejects it with "invalid operand for instruction"). Applying any
    // arrangement below to all four operands therefore MUST yield Err.
    //
    // CURRENT STATUS: this property FAILS (real SUT bug). The encoder ignores
    // all four arrangement strings, emitting an identical valid-looking EOR3
    // word for any arrangement. See
    // `pbt-out/bug_reports/eor3-no-arrangement-validation.md`.
    #[test]
    fn rejects_non_canonical_arrangements(
        arr in prop_oneof![
            Just("8b"), Just("4h"), Just("8h"),
            Just("2s"), Just("4s"), Just("1d"), Just("2d"),
        ],
    ) {
        let ops = vec![va(0, arr), va(1, arr), va(2, arr), va(3, arr)];
        let res = encode_neon_eor3(&ops);
        prop_assert!(
            res.is_err(),
            "eor3 with .{arr} on all operands is UNALLOCATED \
             (only .16b is defined); expected Err, got {:?}",
            res,
        );
    }
}
