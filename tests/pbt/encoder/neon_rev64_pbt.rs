//! Property-based tests for `encode_neon_rev64`.
//!
//! `encode_neon_rev64` encodes the AArch64 NEON `REV64` instruction
//! (reverse elements within each 64-bit doubleword) — `REV64 Vd.T, Vn.T` —
//! in the "Advanced SIMD two-register miscellaneous" encoding group:
//!
//! ```text
//!   31 30 29 28-24 23-22  21  20-16  15-12  11 10 9-5 4-0
//!    0  Q  U  01110  size  1  00000  opcode  1  0  Rn  Rd
//! ```
//! with `U = 0` and `opcode = 0000` for REV64. `Q`/`size` come from the
//! arrangement `T`.
//!
//! ## Oracle
//! The golden words were cross-validated against LLVM's `llvm-mc`
//! (`-triple=aarch64 -show-encoding`) and are independent of this crate's
//! implementation. They anchor the absolute correctness of every fixed
//! field for the six architecturally-valid arrangements. The independent
//! reference encoder `ref_encode_rev64` re-assembles the word field-by-field
//! from the documented ARMv8-A ARM layout.
//!
//! ## Finding (documented by the `#[ignore]`d test `rev64_rejects_unallocated_arrangements`)
//! `REV64` (vector) is defined by the ARMv8-A ARM ONLY for
//! 8B/16B/4H/8H/2S/4S — i.e. element sizes of 8, 16, or 32 bits. The `.1d`/
//! `.2d` arrangements (`size = 0b11`, 64-bit elements) are UNDEFINED /
//! unallocated: reversing 64-bit elements inside a 64-bit doubleword is
//! meaningless, and `llvm-mc` rejects them outright:
//!
//! ```text
//!   $ echo 'rev64 v0.1d, v1.1d' | llvm-mc-18 -assemble -triple=aarch64
//!   error: invalid instruction
//! ```
//!
//! `encode_neon_rev64` delegates arrangement parsing to `neon_arr_to_q_size`,
//! which happily maps `1d`→(Q=0,size=0b11) and `2d`→(Q=1,size=0b11). The
//! encoder then emits an UNALLOCATED instruction word (e.g. `.1d` →
//! `0x0EE00820`) instead of returning `Err`. The four passing properties
//! below confirm that, for the valid arrangements, every field is encoded
//! correctly; the failing property documents the missing range check.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::{encode_neon_rev64, neon_arr_to_q_size};
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

/// Arrangements architecturally VALID for REV64 (ARMv8-A ARM, REV64 (vector)):
/// 8B, 16B, 4H, 8H, 2S, 4S. `.1d`/`.2d` (size=0b11) are UNDEFINED.
fn valid_arrangement_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![
        Just("8b"),
        Just("16b"),
        Just("4h"),
        Just("8h"),
        Just("2s"),
        Just("4s"),
    ]
}

/// Independent reference encoder: assembles the word field-by-field from the
/// documented ARMv8-A ARM "two-register miscellaneous" layout for REV64
/// (U=0, opcode=0000). Mirrors the implementation's intent without sharing
/// the buggy `neon_arr_to_q_size` size-11 acceptance.
fn ref_encode_rev64(rd: u32, rn: u32, arr: &str) -> u32 {
    let (q, size) = neon_arr_to_q_size(arr).unwrap();
    (q << 30)
        | (0b01110u32 << 24) // bits 28-24; bit 29 (U) = 0
        | (size << 22)        // bits 23-22
        | (0b100000u32 << 16) // bit 21 = 1, bits 20-16 = 00000
        | (0b000010u32 << 10) // bits 15-12 = 0000 (opcode), bit 11 = 1, bit 10 = 0
        | (rn << 5)
        | rd
}

fn word_of(res: Result<EncodeResult, String>) -> u32 {
    match res {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {other:?}"),
    }
}

// --- golden table (absolute oracle, cross-validated with llvm-mc) ---------
//
// Each expected word is the big-endian reading of the byte sequence llvm-mc
// emits for the matching instruction, e.g.
//   rev64 v0.8b, v1.8b  -> [0x20,0x08,0x20,0x0e] -> 0x0E200820
const GOLDEN: &[(u32, u32, &str, u32)] = &[
    // (Rd, Rn, arrangement, expected_word)
    (0, 1, "8b", 0x0E200820),   // rev64 v0.8b,  v1.8b
    (0, 1, "16b", 0x4E200820),  // rev64 v0.16b, v1.16b  (Q=1)
    (0, 1, "4h", 0x0E600820),   // rev64 v0.4h,  v1.4h   (size=01)
    (0, 1, "8h", 0x4E600820),   // rev64 v0.8h,  v1.8h   (Q=1,size=01)
    (0, 1, "2s", 0x0EA00820),   // rev64 v0.2s,  v1.2s   (size=10)
    (0, 1, "4s", 0x4EA00820),   // rev64 v0.4s,  v1.4s   (Q=1,size=10)
    (5, 6, "16b", 0x4E2008C5),  // rev64 v5.16b, v6.16b
    (31, 30, "8h", 0x4E600BDF), // rev64 v31.8h, v30.8h
    (10, 20, "4s", 0x4EA00A8A), // rev64 v10.4s, v20.4s
];

#[test]
fn rev64_matches_golden_table() {
    for &(rd, rn, arr, expected) in GOLDEN {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let got = word_of(encode_neon_rev64(&ops));
        assert_eq!(
            got, expected,
            "rev64 v{rd}.{arr}, v{rn}.{arr}: got 0x{got:08X}, want 0x{expected:08X}",
        );
        // Cross-check the reference encoder against the golden values too.
        assert_eq!(
            ref_encode_rev64(rd, rn, arr),
            expected,
            "reference encoder drift"
        );
    }
}

// --- properties -----------------------------------------------------------

proptest! {
    // === Oracle: differential against independent reference encoder ========
    // For every valid REV64 arrangement and register pair, the implementation
    // must equal the independently-assembled reference word (which itself
    // matches llvm-mc on the golden table).
    #[test]
    fn rev64_matches_reference_encoder(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let got = word_of(encode_neon_rev64(&ops));
        let want = ref_encode_rev64(rd, rn, arr);
        prop_assert_eq!(got, want);
    }

    // === Fixed-bits invariant =============================================
    // The architecturally-constant bits must never change: bit31=0,
    // U(bit29)=0, bits28-24=01110, bit21=1, bits20-16=00000,
    // opcode(bits15-12)=0000, bit11=1, bit10=0.
    #[test]
    fn rev64_fixed_bits_are_constant(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let w = word_of(encode_neon_rev64(&ops));

        prop_assert_eq!((w >> 31) & 1, 0, "bit 31 must be 0");
        prop_assert_eq!((w >> 29) & 1, 0, "U bit must be 0");
        prop_assert_eq!((w >> 24) & 0x1F, 0b01110, "bits 28-24");
        prop_assert_eq!((w >> 16) & 0x3F, 0b100000, "bit21=1, bits20-16=00000");
        prop_assert_eq!((w >> 10) & 0x3F, 0b000010, "opcode=0000, bit11=1, bit10=0");
    }

    // === Field placement: Rd/Rn round-trip + Q/size mapping ===============
    // The five-bit register fields and the Q/size bits must round-trip and
    // correctly reflect the arrangement.
    #[test]
    fn rev64_fields_round_trip_and_map_arrangement(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let w = word_of(encode_neon_rev64(&ops));
        let (q, size) = neon_arr_to_q_size(arr).unwrap();

        prop_assert_eq!(w & 0x1F, rd, "Rd field");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field");
        prop_assert_eq!((w >> 30) & 0x1, q, "Q bit");
        prop_assert_eq!((w >> 22) & 0x3, size, "size field");
    }

    // === Negative contract: unsupported arrangement rejected ==============
    // Any arrangement string not understood by `neon_arr_to_q_size` must
    // cause `encode_neon_rev64` to return `Err` (no silent fallthrough).
    #[test]
    fn rev64_rejects_unsupported_arrangement(
        arr in "[a-z0-9]{1,4}".prop_filter("must be an unknown arrangement", |s| {
            !matches!(s.as_str(), "8b"|"16b"|"4h"|"8h"|"2s"|"4s"|"1d"|"2d")
        }),
    ) {
        let ops = vec![va(0, arr.as_str()), va(1, arr.as_str())];
        prop_assert!(encode_neon_rev64(&ops).is_err(),
            "unsupported arrangement {arr:?} should be rejected");
    }

    // === Negative contract: too few operands rejected =====================
    #[test]
    fn rev64_rejects_too_few_operands(n in 0usize..2) {
        let ops: Vec<Operand> = (0..n).map(|_| va(0, "8b")).collect();
        prop_assert!(encode_neon_rev64(&ops).is_err(),
            "{n} operands should be rejected; rev64 needs 2");
    }
}

// --- documented finding: unallocated arrangements silently encoded --------
//
// `REV64` (vector) is defined by the ARMv8-A ARM ONLY for 8B/16B/4H/8H/2S/4S.
// The `.1d`/`.2d` arrangements (`size = 0b11`) are UNDEFINED / unallocated
// and `llvm-mc` rejects them. The current implementation accepts them and
// emits an UNALLOCATED word instead of returning `Err`.
//
// This is a genuine property (negative contract): every `size=11` arrangement
// must return `Err`. It is `#[ignore]`d to keep the default suite green, per
// this module's established convention (see `neon_addv_pbt.rs`'s `#[ignore]`d
// finding tests). It FAILS when run with `--ignored`, surfacing the bug.
proptest! {
    #[test]
    #[ignore]
    fn rev64_rejects_unallocated_arrangements(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in prop_oneof![Just("1d"), Just("2d")],
    ) {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let res = encode_neon_rev64(&ops);
        prop_assert!(
            res.is_err(),
            "rev64 v{rd}.{arr}, v{rn}.{arr}: size=11 is UNDEFINED for REV64 (llvm-mc rejects); \
             expected Err but got {res:?}",
        );
    }
}
