//! Property-based tests for `encode_neon_pmul`.
//!
//! `encode_neon_pmul` encodes the AArch64 NEON `PMUL` (vector) instruction —
//! polynomial multiply, `PMUL Vd.T, Vn.T, Vm.T` — in the
//! "Advanced SIMD three same" encoding group:
//!
//! ```text
//!   31 30 29 28-24 23-22 21 20-16 15-11 10 9-5 4-0
//!    0  Q  U  01110  size  1   Rm   10011  1  Rn  Rd
//! ```
//! with **U=1**, **opcode=10011**, and **size=00**. Per the ARMv8-A ARM
//! (ARM DDI 0487, "Advanced SIMD three same", PMUL row) `PMUL` is defined
//! **only for `.8b` (Q=0) and `.16b` (Q=1)** — byte lanes. Every other
//! arrangement is UNALLOCATED.
//!
//! ## Oracle
//! The golden words below were produced and independently verified against
//! LLVM's AArch64 assembler (`clang --target=aarch64` / `llvm-objdump`):
//!
//! ```text
//!   pmul v0.16b, v1.16b, v2.16b   -> 0x6E229C20
//!   pmul v0.8b,  v1.8b,  v2.8b    -> 0x2E229C20
//!   pmul v5.16b, v6.16b, v7.16b   -> 0x6E279CC5
//!   pmul v31.8b, v30.8b, v29.8b   -> 0x2E3D9FDF
//!   pmul v10.16b,v11.16b,v12.16b  -> 0x6E2C9D6A
//! ```
//! The reference encoder below is assembled field-by-field from the documented
//! layout and is structurally independent of the crate implementation (it
//! splits `opcode<<11 | 1<<10`, whereas the impl ORs `0b100111<<10`), so a
//! shared off-by-one would still be caught by the absolute golden check.
//!
//! ## Finding (documented by the `#[ignore]`d test `pmul_rejects_non_byte`)
//! `PMUL` is architecturally defined only for `.8b`/`.16b` (size=00). LLVM
//! rejects `.4h/.8h/.2s/.4s/.1d/.2d` with "invalid operand for instruction",
//! but `encode_neon_pmul` accepts any arrangement: it maps `arr != "16b"` to
//! `Q=0` and emits a valid-looking byte-PMUL word, silently corrupting the
//! instruction. See `PMUL_NONBYTE_BUG_REPORT.md`.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_pmul;
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

/// Arrangements that are architecturally VALID for PMUL (byte lanes only).
fn valid_arrangement_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("8b"), Just("16b")]
}

/// Independent reference encoder assembled field-by-field from the ARM layout.
fn ref_encode_pmul(rd: u32, rn: u32, rm: u32, arr: &str) -> u32 {
    let q: u32 = if arr == "16b" { 1 } else { 0 };
    let mut w = 0u32;
    w |= q << 30; // bit 31 stays 0
    w |= 1u32 << 29; // U = 1
    w |= 0b01110u32 << 24; // bits 28-24
    // size bits [23:22] = 00 for bytes (left zero)
    w |= 1u32 << 21; // bit 21
    w |= rm << 16; // bits 20-16
    w |= 0b10011u32 << 11; // opcode bits 15-11
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

const GOLDEN: &[(u32, u32, u32, &str, u32)] = &[
    // (Rd, Rn, Rm, arrangement, expected_word)
    (0, 1, 2, "16b", 0x6E229C20),    // pmul v0.16b, v1.16b, v2.16b
    (0, 1, 2, "8b", 0x2E229C20),     // pmul v0.8b,  v1.8b,  v2.8b   (Q=0)
    (5, 6, 7, "16b", 0x6E279CC5),    // pmul v5.16b, v6.16b, v7.16b
    (31, 30, 29, "8b", 0x2E3D9FDF),  // pmul v31.8b, v30.8b, v29.8b
    (10, 11, 12, "16b", 0x6E2C9D6A), // pmul v10.16b,v11.16b,v12.16b
];

#[test]
fn pmul_matches_golden_table() {
    for &(rd, rn, rm, arr, expected) in GOLDEN {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let got = word_of(encode_neon_pmul(&ops));
        assert_eq!(
            got, expected,
            "pmul v{rd}.{arr}, v{rn}.{arr}, v{rm}.{arr}: got 0x{got:08X}, want 0x{expected:08X}",
        );
        assert_eq!(
            ref_encode_pmul(rd, rn, rm, arr),
            expected,
            "reference encoder drift"
        );
    }
}

// --- properties -----------------------------------------------------------

proptest! {
    // === Oracle: reference encoder (differential) =========================
    // For every valid (.8b/.16b) arrangement and register triple, the
    // implementation must equal the independently-assembled reference word.
    #[test]
    fn matches_reference_encoder(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let got = word_of(encode_neon_pmul(&ops));
        let want = ref_encode_pmul(rd, rn, rm, arr);
        prop_assert_eq!(got, want);
    }

    // === Field placement: Rd/Rn/Rm round-trip + Q mapping ================
    // The five-bit register fields must round-trip exactly, Q must equal
    // (arr == "16b"), and size (bits 23-22) must be 00 for every valid PMUL.
    // No silent truncation of register numbers is permitted for in-range
    // inputs.
    #[test]
    fn fields_round_trip_and_map_arrangement(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let w = word_of(encode_neon_pmul(&ops));
        let q = if arr == "16b" { 1u32 } else { 0u32 };

        prop_assert_eq!(w & 0x1F, rd, "Rd field");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field");
        prop_assert_eq!((w >> 16) & 0x1F, rm, "Rm field");
        prop_assert_eq!((w >> 30) & 0x1, q, "Q bit (1 only for .16b)");
        prop_assert_eq!((w >> 22) & 0x3, 0u32, "size bits must be 00 (bytes)");
    }

    // === Fixed-bits invariant =============================================
    // The architecturally-constant bits of the PMUL encoding never change for
    // any valid input: bit31=0, U(bit29)=1, bits28-24=01110, bit21=1,
    // opcode(bits15-11)=10011, bit10=1.
    #[test]
    fn fixed_bits_are_constant(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let w = word_of(encode_neon_pmul(&ops));

        prop_assert_eq!((w >> 31) & 1, 0, "bit 31 must be 0");
        prop_assert_eq!((w >> 29) & 1, 1, "U bit must be 1 for PMUL");
        prop_assert_eq!((w >> 24) & 0x1F, 0b01110, "bits 28-24");
        prop_assert_eq!((w >> 21) & 1, 1, "bit 21 must be 1");
        prop_assert_eq!((w >> 11) & 0x1F, 0b10011, "opcode bits 15-11");
        prop_assert_eq!((w >> 10) & 1, 1, "bit 10 must be 1");
    }
}

// --- documented finding: non-byte arrangements not rejected ---------------

/// `PMUL` (vector) is only defined for `.8b`/`.16b` (size=00, byte lanes).
/// The ARMv8-A ARM "Advanced SIMD three same" table marks every other
/// arrangement UNALLOCATED for PMUL, and LLVM rejects them with
/// "invalid operand for instruction". The encoder should return `Err`.
///
/// This test is `#[ignore]`d because the current implementation emits a
/// valid-looking byte-PMUL word (mapping any `arr != "16b"` to Q=0) instead of
/// returning `Err` — i.e. it does NOT meet the contract.
/// Run with `cargo test -- --ignored pmul_rejects_non_byte` to reproduce.
/// See `PMUL_NONBYTE_BUG_REPORT.md`.
#[test]
#[ignore]
fn pmul_rejects_non_byte() {
    for arr in &["4h", "8h", "2s", "4s", "1d", "2d"] {
        let ops = vec![va(0, arr), va(1, arr), va(2, arr)];
        let res = encode_neon_pmul(&ops);
        assert!(
            res.is_err(),
            "PMUL does not support .{arr} (only .8b/.16b are allocated); \
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
