//! Property-based tests for `encode_neon_mla`.
//!
//! `encode_neon_mla` encodes the AArch64 NEON `MLA` (vector) instruction
//! (multiply-accumulate) — `MLA Vd.T, Vn.T, Vm.T` — in the
//! "Advanced SIMD three same" encoding group:
//!
//! ```text
//!   31 30 29 28-24 23-22 21 20-16 15-11 10 9-5 4-0
//!    0  Q  U  01110  size  1   Rm   10010  1  Rn  Rd
//! ```
//! with U=0. `Q` and `size` are derived from the arrangement `T`.
//!
//! ## Oracle
//! The golden words below were hand-derived from the ARMv8-A ARM bit layout
//! (ARM DDI 0487, "Advanced SIMD three same", MLA row: U=0, opcode=10010) and
//! are independent of this crate's implementation. They anchor the absolute
//! correctness of every fixed field. As a cross-check, the canonical
//! `mul v0.4s, v1.4s, v2.4s` (= `0x4EA29C20`, opcode 10011) differs from
//! `mla v0.4s, v1.4s, v2.4s` by exactly bit 11, as expected.
//!
//! ## Finding (documented by the `#[ignore]`d test `mla_rejects_doubleword`)
//! `MLA` (like `MUL`/`MLS`) is architecturally defined only for `size != 0b11`,
//! i.e. arrangements `.8b/.16b/.4h/.8h/.2s/.4s`. The `.1d`/`.2d` arrangements
//! (`size = 0b11`) are **unallocated** for this instruction, yet
//! `encode_neon_mla` accepts them and emits a silently-wrong word instead of
//! returning `Err`. See `MLA_DWORD_BUG_REPORT.md`.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::{encode_neon_mla, neon_arr_to_q_size};
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

/// Arrangements that are architecturally VALID for MLA (size 00/01/10).
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
/// documented layout. Used together with the golden table so a bug shared by
/// both this and the real impl would still be caught by the absolute hex check.
fn ref_encode_mla(rd: u32, rn: u32, rm: u32, arr: &str) -> u32 {
    let (q, size) = neon_arr_to_q_size(arr).unwrap();
    let mut w = 0u32;
    w |= q << 30; // bit 31 is 0, bit 29 (U) is 0
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

// --- golden table (absolute oracle) ---------------------------------------

/// Hand-derived from the ARMv8-A ARM layout for MLA (vector).
const GOLDEN: &[(u32, u32, u32, &str, u32)] = &[
    // (Rd, Rn, Rm, arrangement, expected_word)
    (0, 1, 2, "4s", 0x4EA29420),     // mla v0.4s, v1.4s, v2.4s
    (0, 1, 2, "2s", 0x0EA29420),     // mla v0.2s, v1.2s, v2.2s  (Q=0)
    (5, 6, 7, "8h", 0x4E6794C5),     // mla v5.8h, v6.8h, v7.8h
    (31, 30, 29, "16b", 0x4E3D97DF), // mla v31.16b, v30.16b, v29.16b
    (0, 0, 0, "8b", 0x0E209400),     // mla v0.8b, v0.8b, v0.8b  (Q=0,size=00)
    (10, 20, 30, "4h", 0x0E7E968A),  // mla v10.4h, v20.4h, v30.4h (Q=0)
];

#[test]
fn mla_matches_golden_table() {
    for &(rd, rn, rm, arr, expected) in GOLDEN {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let got = word_of(encode_neon_mla(&ops));
        assert_eq!(
            got, expected,
            "mla v{rd}.{arr}, v{rn}.{arr}, v{rm}.{arr}: got 0x{got:08X}, want 0x{expected:08X}",
        );
        // Cross-check the reference encoder against the golden values too.
        assert_eq!(
            ref_encode_mla(rd, rn, rm, arr),
            expected,
            "reference encoder drift"
        );
    }
}

// --- properties -----------------------------------------------------------

proptest! {
    // === Oracle: reference encoder (differential) =========================
    // For every valid MLA arrangement and register triple, the implementation
    // must equal the independently-assembled reference word.
    #[test]
    fn matches_reference_encoder(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let got = word_of(encode_neon_mla(&ops));
        let want = ref_encode_mla(rd, rn, rm, arr);
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
        let w = word_of(encode_neon_mla(&ops));
        let (q, size) = neon_arr_to_q_size(arr).unwrap();

        prop_assert_eq!(w & 0x1F, rd, "Rd field");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field");
        prop_assert_eq!((w >> 16) & 0x1F, rm, "Rm field");
        prop_assert_eq!((w >> 30) & 0x1, q, "Q bit");
        prop_assert_eq!((w >> 22) & 0x3, size, "size field");
    }

    // === Fixed-bits invariant =============================================
    // The architecturally-constant bits of the MLA encoding never change for
    // any valid input: bit31=0, U(bit29)=0, bits28-24=01110, bit21=1,
    // opcode(bits15-11)=10010, bit10=1.
    #[test]
    fn fixed_bits_are_constant(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let w = word_of(encode_neon_mla(&ops));

        prop_assert_eq!((w >> 31) & 1, 0, "bit 31 must be 0");
        prop_assert_eq!((w >> 29) & 1, 0, "U bit must be 0 for MLA");
        prop_assert_eq!((w >> 24) & 0x1F, 0b01110, "bits 28-24");
        prop_assert_eq!((w >> 21) & 1, 1, "bit 21 must be 1");
        prop_assert_eq!((w >> 11) & 0x1F, 0b10010, "opcode bits 15-11");
        prop_assert_eq!((w >> 10) & 1, 1, "bit 10 must be 1");
    }

    // === Negative contract: unsupported arrangement rejected ==============
    // Any arrangement string not defined by `neon_arr_to_q_size` must cause
    // `encode_neon_mla` to return `Err` (no silent fallthrough).
    #[test]
    fn rejects_unsupported_arrangement(
        arr in "[a-z0-9]{1,4}".prop_filter("must be an unknown arrangement", |s| {
            !matches!(s.as_str(), "8b"|"16b"|"4h"|"8h"|"2s"|"4s"|"1d"|"2d")
        }),
    ) {
        let ops = vec![va(0, arr.as_str()), va(1, arr.as_str()), va(2, arr.as_str())];
        prop_assert!(encode_neon_mla(&ops).is_err(),
            "unsupported arrangement {arr:?} should be rejected");
    }
}

// --- documented finding: unallocated doubleword encoding -----------------

/// `MLA` (vector) is only defined for `size != 0b11`; `.1d`/`.2d` are
/// unallocated (ARMv8-A ARM, Advanced SIMD three same: the `size==11` row is
/// UNALLOCATED for the multiply family). The encoder should reject them.
///
/// This test is `#[ignore]`d because the current implementation emits a word
/// instead of returning `Err`, i.e. it does NOT meet the contract.
/// Run with `cargo test -- --ignored mla_rejects_doubleword` to reproduce.
/// See `MLA_DWORD_BUG_REPORT.md`.
#[test]
#[ignore]
fn mla_rejects_doubleword() {
    for arr in &["1d", "2d"] {
        let ops = vec![va(0, arr), va(1, arr), va(2, arr)];
        let res = encode_neon_mla(&ops);
        assert!(
            res.is_err(),
            "MLA does not support .{arr} (size=0b11 is unallocated); \
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
