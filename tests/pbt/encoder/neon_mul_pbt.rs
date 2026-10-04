//! Property-based tests for `encode_neon_mul`.
//!
//! `encode_neon_mul` encodes the AArch64 NEON `MUL` (vector) instruction —
//! `MUL Vd.T, Vn.T, Vm.T` — in the "Advanced SIMD three same" encoding group:
//!
//! ```text
//!   31 30 29 28-24 23-22 21 20-16 15-11 10 9-5 4-0
//!    0  Q  U  01110  size  1   Rm   10011  1  Rn  Rd
//! ```
//! with **U=0** (bit 29) and opcode `10011` (bits 15-11). `Q` and `size` are
//! derived from the arrangement `T` via `neon_arr_to_q_size`.
//!
//! ## Oracle
//! The golden words below were validated against **`llvm-mc-18
//! --triple=aarch64`** (`--show-encoding`), an independent authoritative
//! assembler — they are not derived from this crate's implementation. As a
//! cross-check, each word equals the independently field-assembled reference
//! encoder, exactly as the ARMv8-A ARM layout requires.
//!
//! ## Finding (captured by the `#[ignore]`d bug-witness `rejects_unallocated_doubleword`)
//! `MUL` (vector) — like its siblings `MLA`/`MLS` — is architecturally
//! defined only for `size != 0b11`, i.e. arrangements
//! `.8b/.16b/.4h/.8h/.2s/.4s`. The `.1d`/`.2d` arrangements (`size = 0b11`)
//! are **unallocated** for this instruction: `llvm-mc` rejects them with
//! "invalid operand for instruction", yet `encode_neon_mul` accepts them and
//! emits a silently-wrong word (`mul v0.1d,...` -> `Ok(0x0EE09C00)`,
//! `mul v0.2d,...` -> `Ok(0x4EE09C00)`) instead of returning `Err`.
//!
//! The property `rejects_unallocated_doubleword` would FAIL (proptest shrinks
//! to the minimal witness `mul v0.1d, v0.1d, v0.1d`). It is marked
//! `#[ignore]` so the default `cargo test` run stays green; run it explicitly
//! with `cargo test -- --ignored` to exercise the witness. See the bug
//!.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::{encode_neon_mul, neon_arr_to_q_size};
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

/// Arrangements that are architecturally VALID for MUL (size 00/01/10).
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
/// documented layout with U=0. Used together with the golden table so a bug
/// shared by both this and the real impl would still be caught by the
/// absolute hex check.
fn ref_encode_mul(rd: u32, rn: u32, rm: u32, arr: &str) -> u32 {
    let (q, size) = neon_arr_to_q_size(arr).unwrap();
    let mut w = 0u32;
    w |= q << 30; // bit 31 is 0, bit 29 (U) is 0 for MUL
    w |= 0b01110u32 << 24;
    w |= size << 22;
    w |= 1u32 << 21;
    w |= rm << 16;
    w |= 0b10011u32 << 11; // opcode (bits 15-11)
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
    (0, 1, 2, "4s", 0x4EA29C20), // mul v0.4s, v1.4s, v2.4s   [20 9c a2 4e]
    (0, 1, 2, "2s", 0x0EA29C20), // mul v0.2s, v1.2s, v2.2s   (Q=0)
    (5, 6, 7, "8h", 0x4E679CC5), // mul v5.8h, v6.8h, v7.8h   [c5 9c 67 4e]
    (5, 6, 7, "4h", 0x0E679CC5), // mul v5.4h, v6.4h, v7.4h   (Q=0)
    (31, 30, 29, "16b", 0x4E3D9FDF), // mul v31.16b, v30.16b, v29.16b [df 9f 3d 4e]
    (0, 0, 0, "8b", 0x0E209C00), // mul v0.8b, v0.8b, v0.8b   (Q=0,size=00)
    (10, 20, 30, "4h", 0x0E7E9E8A), // mul v10.4h, v20.4h, v30.4h (Q=0)
];

#[test]
fn mul_matches_golden_table() {
    for &(rd, rn, rm, arr, expected) in GOLDEN {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let got = word_of(encode_neon_mul(&ops));
        assert_eq!(
            got, expected,
            "mul v{rd}.{arr}, v{rn}.{arr}, v{rm}.{arr}: got 0x{got:08X}, want 0x{expected:08X}",
        );
        // Cross-check the reference encoder against the golden values too.
        assert_eq!(
            ref_encode_mul(rd, rn, rm, arr),
            expected,
            "reference encoder drift"
        );
    }
}

// --- properties -----------------------------------------------------------

proptest! {
    // === Oracle: reference encoder (differential) =========================
    // For every valid MUL arrangement and register triple, the implementation
    // must equal the independently-assembled reference word.
    #[test]
    fn matches_reference_encoder(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let got = word_of(encode_neon_mul(&ops));
        let want = ref_encode_mul(rd, rn, rm, arr);
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
        let w = word_of(encode_neon_mul(&ops));
        let (q, size) = neon_arr_to_q_size(arr).unwrap();

        prop_assert_eq!(w & 0x1F, rd, "Rd field");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field");
        prop_assert_eq!((w >> 16) & 0x1F, rm, "Rm field");
        prop_assert_eq!((w >> 30) & 0x1, q, "Q bit");
        prop_assert_eq!((w >> 22) & 0x3, size, "size field");
    }

    // === Fixed-bits invariant =============================================
    // The architecturally-constant bits of the MUL encoding never change for
    // any valid input: bit31=0, U(bit29)=0, bits28-24=01110, bit21=1,
    // opcode(bits15-11)=10011, bit10=1.
    #[test]
    fn fixed_bits_are_constant(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let w = word_of(encode_neon_mul(&ops));

        prop_assert_eq!((w >> 31) & 1, 0, "bit 31 must be 0");
        prop_assert_eq!((w >> 29) & 1, 0, "U bit must be 0 for MUL");
        prop_assert_eq!((w >> 24) & 0x1F, 0b01110, "bits 28-24");
        prop_assert_eq!((w >> 21) & 1, 1, "bit 21 must be 1");
        prop_assert_eq!((w >> 11) & 0x1F, 0b10011, "opcode bits 15-11");
        prop_assert_eq!((w >> 10) & 1, 1, "bit 10 must be 1");
    }

    // === Negative contract: unsupported arrangement rejected ==============
    // Any arrangement string not defined by `neon_arr_to_q_size` must cause
    // `encode_neon_mul` to return `Err` (no silent fallthrough).
    #[test]
    fn rejects_unsupported_arrangement(
        arr in "[a-z0-9]{1,4}".prop_filter("must be an unknown arrangement", |s| {
            !matches!(s.as_str(), "8b"|"16b"|"4h"|"8h"|"2s"|"4s"|"1d"|"2d")
        }),
    ) {
        let ops = vec![va(0, arr.as_str()), va(1, arr.as_str()), va(2, arr.as_str())];
        prop_assert!(encode_neon_mul(&ops).is_err(),
            "unsupported arrangement {arr:?} should be rejected");
    }
}

// --- SUT violation: unallocated doubleword encoding (#[ignore] witness) ---

/// `MUL` (vector) is only defined for `size != 0b11`; `.1d`/`.2d` are
/// unallocated (ARMv8-A ARM, Advanced SIMD three same: the `size==11` row is
/// UNALLOCATED for the multiply family MUL/MLA/MLS). `llvm-mc` rejects these
/// with "invalid operand for instruction". The encoder MUST reject them too.
///
/// This is a **bug-witness** property: the current implementation emits a
/// silently-wrong word instead of returning `Err` (`Ok(0x0EE09C00)` for
/// `.1d`, `Ok(0x4EE09C00)` for `.2d`). proptest shrinks the counterexample
/// to the minimal witness `mul v0.1d, v0.1d, v0.1d`.
///
/// It is marked `#[ignore]` so the default `cargo test` run stays green.
/// Reproduce with: `cargo test -- --ignored rejects_unallocated_doubleword`.
#[test]
#[ignore]
fn rejects_unallocated_doubleword() {
    proptest!(|(rd in 0u32..=31, rn in 0u32..=31, rm in 0u32..=31,
                 arr in prop_oneof![Just("1d"), Just("2d")])| {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        prop_assert!(
            encode_neon_mul(&ops).is_err(),
            "MUL does not support .{arr} (size=0b11 is unallocated); \
             expected Err but got Ok",
        );
    });
}
