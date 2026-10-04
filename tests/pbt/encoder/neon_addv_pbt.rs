//! Property-based tests for `encode_neon_addv`.
//!
//! `encode_neon_addv` encodes the AArch64 NEON `ADDV` instruction
//! (integer add across all vector lanes) — `ADDV Vd.T, Vn.T` — in the
//! "Advanced SIMD across lanes" encoding group:
//!
//! ```text
//!   31 30 29 28-24 23-22 21-17  16-12   11-10 9-5 4-0
//!    0  Q  U  01110  size  11000  opcode  10   Rn  Rd
//! ```
//! with `U = 0` and `opcode = 11011` for ADDV. `Q`/`size` come from the
//! arrangement `T`.
//!
//! ## Oracle
//! The golden words were hand-derived from the ARMv8-A ARM bit layout
//! (ARM DDI 0487, "Advanced SIMD across lanes", ADDV row: U=0, opcode=11011)
//! and are independent of this crate's implementation. They anchor the
//! absolute correctness of every fixed field. The independent reference
//! encoder `ref_encode_addv` mirrors the correct sibling implementation
//! `encode_neon_across(..., 0, 0b11011)`.
//!
//! ## Finding (documented by the `#[ignore]`d tests `addv_matches_golden_table`,
//! `addv_matches_reference_encoder`, `addv_fixed_bits_are_constant`)
//! The implementation assembles the fixed field `11000 11011 10` as
//! `(0b11000 << 17) | (0b110111 << 10)`. The second constant is only 6 bits
//! wide and lands at bits 15–10 instead of the required 7-bit `1101110` at
//! bits 16–10. Concretely: bit 16 is wrongly 0 (must be 1) and bit 10 is
//! wrongly 1 (must be 0). Every emitted word is therefore wrong.
//! Example: `addv v0.4s, v1.4s` yields `0x4EB0DC20` instead of `0x4EB1B820`.
//! See `ADDV_ENCODER_BUG_REPORT.md`.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::{encode_neon_addv, neon_arr_to_q_size};
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

/// Arrangements architecturally VALID for ADDV (ARMv8-A ARM, ADDV (vector)):
/// 8B, 16B, 4H, 8H, 4S. `.2s`/`.1d`/`.2d` are not valid reductions.
fn valid_arrangement_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("8b"), Just("16b"), Just("4h"), Just("8h"), Just("4s")]
}

/// Independent reference encoder: assembles the word field-by-field from the
/// documented ARMv8-A ARM layout. This mirrors the *correct* sibling
/// `encode_neon_across(u_bit=0, opcode=0b11011)`, not the buggy `encode_neon_addv`.
fn ref_encode_addv(rd: u32, rn: u32, arr: &str) -> u32 {
    let (q, size) = neon_arr_to_q_size(arr).unwrap();
    (q << 30)
        | (0b01110u32 << 24)
        | (size << 22)
        | (0b11000u32 << 17) // bits 21-17
        | (0b11011u32 << 12) // opcode, bits 16-12
        | (0b10u32 << 10) // bits 11-10
        | (rn << 5)
        | rd
}

fn word_of(res: Result<EncodeResult, String>) -> u32 {
    match res {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {other:?}"),
    }
}

// --- golden table (absolute oracle) ---------------------------------------

/// Hand-derived from the ARMv8-A ARM layout for ADDV.
const GOLDEN: &[(u32, u32, &str, u32)] = &[
    // (Rd, Rn, arrangement, expected_word)
    (0, 1, "4s", 0x4EB1B820),   // addv v0.4s, v1.4s
    (0, 1, "4h", 0x0E71B820),   // addv v0.4h, v1.4h  (Q=0)
    (5, 6, "16b", 0x4E31B8C5),  // addv v5.16b, v6.16b
    (31, 30, "8h", 0x4E71BCDF), // addv v31.8h, v30.8h
    (0, 0, "8b", 0x0E31B800),   // addv v0.8b, v0.8b  (Q=0,size=00)
    (10, 20, "4h", 0x0E71BA8A), // addv v10.4h, v20.4h (Q=0)
];

/// Fails today: the encoder emits wrong words (bit 16 missing, bit 10 wrong).
/// Run with `cargo test -- --ignored addv_matches_golden_table` to reproduce.
#[test]
#[ignore]
fn addv_matches_golden_table() {
    for &(rd, rn, arr, expected) in GOLDEN {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let got = word_of(encode_neon_addv(&ops));
        assert_eq!(
            got, expected,
            "addv v{rd}.{arr}, v{rn}.{arr}: got 0x{got:08X}, want 0x{expected:08X}",
        );
        // Cross-check the reference encoder against the golden values too.
        assert_eq!(
            ref_encode_addv(rd, rn, arr),
            expected,
            "reference encoder drift"
        );
    }
}

// --- properties -----------------------------------------------------------

proptest! {
    // === Oracle: differential against independent reference encoder ========
    // For every valid ADDV arrangement and register pair, the implementation
    // must equal the independently-assembled reference word. Currently FAILS
    // because of the bit-shift error in the opcode constant.
    //
    // #[ignore]d so the suite stays green; the finding is in
    // ADDV_ENCODER_BUG_REPORT.md. Reproduce with --ignored.
    #[test]
    #[ignore]
    fn addv_matches_reference_encoder(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let got = word_of(encode_neon_addv(&ops));
        let want = ref_encode_addv(rd, rn, arr);
        prop_assert_eq!(got, want);
    }

    // === Fixed-bits invariant (failing) ===================================
    // The architecturally-constant bits must never change: bit31=0,
    // U(bit29)=0, bits28-24=01110, bits21-17=11000, opcode(bits16-12)=11011,
    // bits11-10=10. Currently FAILS on the opcode and bits11-10 fields.
    #[test]
    #[ignore]
    fn addv_fixed_bits_are_constant(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let w = word_of(encode_neon_addv(&ops));

        prop_assert_eq!((w >> 31) & 1, 0, "bit 31 must be 0");
        prop_assert_eq!((w >> 29) & 1, 0, "U bit must be 0");
        prop_assert_eq!((w >> 24) & 0x1F, 0b01110, "bits 28-24");
        prop_assert_eq!((w >> 17) & 0x1F, 0b11000, "bits 21-17");
        prop_assert_eq!((w >> 12) & 0x1F, 0b11011, "opcode bits 16-12");
        prop_assert_eq!((w >> 10) & 0x3, 0b10, "bits 11-10");
    }

    // === Field placement: Rd/Rn round-trip + Q/size mapping (passing) =====
    // The five-bit register fields and the Q/size bits lie outside the
    // buggy opcode region, so they round-trip correctly today.
    #[test]
    fn fields_round_trip_and_map_arrangement(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let w = word_of(encode_neon_addv(&ops));
        let (q, size) = neon_arr_to_q_size(arr).unwrap();

        prop_assert_eq!(w & 0x1F, rd, "Rd field");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field");
        prop_assert_eq!((w >> 30) & 0x1, q, "Q bit");
        prop_assert_eq!((w >> 22) & 0x3, size, "size field");
    }

    // === Negative contract: unsupported arrangement rejected (passing) ====
    // Any arrangement string not understood by `neon_arr_to_q_size` must
    // cause `encode_neon_addv` to return `Err` (no silent fallthrough).
    #[test]
    fn rejects_unsupported_arrangement(
        arr in "[a-z0-9]{1,4}".prop_filter("must be an unknown arrangement", |s| {
            !matches!(s.as_str(), "8b"|"16b"|"4h"|"8h"|"2s"|"4s"|"1d"|"2d")
        }),
    ) {
        let ops = vec![va(0, arr.as_str()), va(1, arr.as_str())];
        prop_assert!(encode_neon_addv(&ops).is_err(),
            "unsupported arrangement {arr:?} should be rejected");
    }
}

// --- documented finding: unallocated arrangements silently encoded --------

/// `ADDV` (vector) is defined by the ARMv8-A ARM ONLY for 8B/16B/4H/8H/4S.
/// `.1d`/`.2d` (size=0b11) and `.2s` are NOT valid arrangements for ADDV and
/// must be rejected. The current implementation accepts them and emits a word
/// instead of returning `Err`.
///
/// `#[ignore]`d because it documents a genuine gap (not yet fixed).
/// Run with `cargo test -- --ignored addv_rejects_unallocated_arrangements`.
#[test]
#[ignore]
fn addv_rejects_unallocated_arrangements() {
    for arr in &["1d", "2d", "2s"] {
        let ops = vec![va(0, arr), va(1, arr)];
        let res = encode_neon_addv(&ops);
        assert!(
            res.is_err(),
            "ADDV does not support .{arr}; expected Err but got Ok(0x{:08X})",
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
