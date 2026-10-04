//! Property-based tests for `encode_neon_mls`.
//!
//! `encode_neon_mls` encodes the AArch64 NEON `MLS` (vector) instruction
//! (multiply-subtract) — `MLS Vd.T, Vn.T, Vm.T` — in the
//! "Advanced SIMD three same" encoding group:
//!
//! ```text
//!   31 30 29 28-24 23-22 21 20-16 15-11 10 9-5 4-0
//!    0  Q  U  01110  size  1   Rm   10010  1  Rn  Rd
//! ```
//! with **U=1** (this is the only difference from the sibling `MLA`,
//! which shares opcode `10010` but has U=0). `Q` and `size` are derived
//! from the arrangement `T`.
//!
//! ## Oracle
//! The golden words below were validated against **`llvm-mc-18
//! --triple=aarch64`** (`--show-encoding`), an independent authoritative
//! assembler — they are not derived from this crate's implementation.
//! As a cross-check, each `MLS` word equals the corresponding `MLA`
//! word with bit 29 (U) set, exactly as the ARMv8-A ARM layout requires.
//!
//! `MLS` is defined only for `size != 0b11`; the active regression rejects the
//! unallocated `.1d` and `.2d` arrangements. The encoder also requires exact
//! arity and identical arrangements for all three vector operands.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::{encode_neon_mls, neon_arr_to_q_size};
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

/// Arrangements that are architecturally VALID for MLS (size 00/01/10).
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
/// documented layout with U=1. Used together with the golden table so a bug
/// shared by both this and the real impl would still be caught by the
/// absolute hex check.
fn ref_encode_mls(rd: u32, rn: u32, rm: u32, arr: &str) -> u32 {
    let (q, size) = neon_arr_to_q_size(arr).unwrap();
    let mut w = 0u32;
    w |= q << 30; // bit 31 is 0, bit 29 (U) is 1
    w |= 1u32 << 29; // U=1 distinguishes MLS from MLA
    w |= 0b01110u32 << 24;
    w |= size << 22;
    w |= 1u32 << 21;
    w |= rm << 16;
    w |= 0b10010u32 << 11; // opcode (bits 15-11)
    w |= 1u32 << 10; // fixed '1'
    w |= rn << 5;
    w |= rd;
    w
}

fn word_of(res: Result<EncodeResult, String>) -> u32 {
    match res {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {other:?}"),
    }
}

// --- golden table (absolute oracle, llvm-mc-18 verified) ------------------

/// Each expected word is the big-endian u32 of `llvm-mc`'s `[b0,b1,b2,b3]`
/// little-endian encoding bytes — i.e. `b3<<24 | b2<<16 | b1<<8 | b0`.
/// Verified with: `llvm-mc-18 --triple=aarch64 --assemble --show-encoding`.
const GOLDEN: &[(u32, u32, u32, &str, u32)] = &[
    // (Rd, Rn, Rm, arrangement, expected_word)
    (0, 1, 2, "4s", 0x6EA29420), // mls v0.4s, v1.4s, v2.4s   [20 94 a2 6e]
    (0, 1, 2, "2s", 0x2EA29420), // mls v0.2s, v1.2s, v2.2s   (Q=0)
    (5, 6, 7, "8h", 0x6E6794C5), // mls v5.8h, v6.8h, v7.8h   [c5 94 67 6e]
    (31, 30, 29, "16b", 0x6E3D97DF), // mls v31.16b, v30.16b, v29.16b [df 97 3d 6e]
    (0, 0, 0, "8b", 0x2E209400), // mls v0.8b, v0.8b, v0.8b   (Q=0,size=00)
    (10, 20, 30, "4h", 0x2E7E968A), // mls v10.4h, v20.4h, v30.4h (Q=0)
];

#[test]
fn mls_matches_golden_table() {
    for &(rd, rn, rm, arr, expected) in GOLDEN {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let got = word_of(encode_neon_mls(&ops));
        assert_eq!(
            got, expected,
            "mls v{rd}.{arr}, v{rn}.{arr}, v{rm}.{arr}: got 0x{got:08X}, want 0x{expected:08X}",
        );
        // Cross-check the reference encoder against the golden values too.
        assert_eq!(
            ref_encode_mls(rd, rn, rm, arr),
            expected,
            "reference encoder drift"
        );
    }
}

// --- properties -----------------------------------------------------------

proptest! {
    // === Oracle: reference encoder (differential) =========================
    // For every valid MLS arrangement and register triple, the implementation
    // must equal the independently-assembled reference word.
    #[test]
    fn matches_reference_encoder(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let got = word_of(encode_neon_mls(&ops));
        let want = ref_encode_mls(rd, rn, rm, arr);
        prop_assert_eq!(got, want);
    }

    // === Field placement: Rd/Rn/Rm round-trip + Q/size mapping ===========
    // The five-bit register fields must round-trip exactly, and the Q/size
    // bits must equal the arrangement-derived values. No silent truncation of
    // register numbers is permitted for in-range inputs.
    #[test]
    fn fields_round_trip_and_map_arrangement(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let w = word_of(encode_neon_mls(&ops));
        let (q, size) = neon_arr_to_q_size(arr).unwrap();

        prop_assert_eq!(w & 0x1F, rd, "Rd field");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field");
        prop_assert_eq!((w >> 16) & 0x1F, rm, "Rm field");
        prop_assert_eq!((w >> 30) & 0x1, q, "Q bit");
        prop_assert_eq!((w >> 22) & 0x3, size, "size field");
    }

    // === Fixed-bits invariant =============================================
    // The architecturally-constant bits of the MLS encoding never change for
    // any valid input: bit31=0, U(bit29)=1 (this is what makes it MLS, not
    // MLA), bits28-24=01110, bit21=1, opcode(bits15-11)=10010, bit10=1.
    #[test]
    fn fixed_bits_are_constant(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let w = word_of(encode_neon_mls(&ops));

        prop_assert_eq!((w >> 31) & 1, 0, "bit 31 must be 0");
        prop_assert_eq!((w >> 29) & 1, 1, "U bit must be 1 for MLS");
        prop_assert_eq!((w >> 24) & 0x1F, 0b01110, "bits 28-24");
        prop_assert_eq!((w >> 21) & 1, 1, "bit 21 must be 1");
        prop_assert_eq!((w >> 11) & 0x1F, 0b10010, "opcode bits 15-11");
        prop_assert_eq!((w >> 10) & 1, 1, "bit 10 must be 1");
    }

    // === Negative contract: unsupported arrangement rejected ==============
    // Any arrangement string not defined by `neon_arr_to_q_size` must cause
    // `encode_neon_mls` to return `Err` (no silent fallthrough).
    #[test]
    fn rejects_unsupported_arrangement(
        arr in "[a-z0-9]{1,4}".prop_filter("must be an unknown arrangement", |s| {
            !matches!(s.as_str(), "8b"|"16b"|"4h"|"8h"|"2s"|"4s"|"1d"|"2d")
        }),
    ) {
        let ops = vec![va(0, arr.as_str()), va(1, arr.as_str()), va(2, arr.as_str())];
        prop_assert!(encode_neon_mls(&ops).is_err(),
            "unsupported arrangement {arr:?} should be rejected");
    }
}

// --- active negative-contract regression -----------------------------------

/// `.1d`/`.2d` are unallocated for vector MLS (`size == 0b11`).
#[test]
fn rejects_unallocated_doubleword() {
    proptest!(|(rd in 0u32..=31, rn in 0u32..=31, rm in 0u32..=31,
                 arr in prop_oneof![Just("1d"), Just("2d")])| {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        prop_assert!(
            encode_neon_mls(&ops).is_err(),
            "MLS does not support .{arr} (size=0b11 is unallocated); \
             expected Err but got Ok",
        );
    });
}

#[test]
fn mls_rejects_mismatched_arrangements_and_extra_operands() {
    assert!(encode_neon_mls(&[va(0, "4s"), va(1, "2s"), va(2, "4s")]).is_err());
    assert!(encode_neon_mls(&[va(0, "4s"), va(1, "4s"), va(2, "4s"), va(3, "4s")]).is_err());
    assert!(encode_neon_mls(&[va(0, "4s"), va(1, "4s")]).is_err());
}
