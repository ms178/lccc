//! Property-based tests for `encode_mov_wide_imm`
//! (the AArch64 MOV-wide immediate sequence encoder in `data_processing.rs`).
//!
//! `encode_mov_wide_imm(rd, is_64, imm)` lowers a full-width immediate into a
//! `MOVZ` + zero-or-more `MOVK` sequence. The ARMv8-A ARM bit-strings are:
//!
//! ```text
//! MOVZ Xd, #imm16, LSL #(hw*16):  sf 10 100101 hw imm16 Rd   (opc = 10)
//! MOVK Xd, #imm16, LSL #(hw*16):  sf 11 100101 hw imm16 Rd   (opc = 11)
//! ```
//! where `sf=1` selects 64-bit (`max_hw = 4`) and `sf=0` selects 32-bit
//! (`max_hw = 2`; `hw ∈ {2,3}` is RESERVED/UNDEFINED).
//!
//! ## Oracles
//!
//! * **P1 — simulation**: an independently-written decoder replays the
//!   `MOVZ`/`MOVK` stream and the resulting register value must equal
//!   `imm` masked to the operand width. This is a true round-trip oracle,
//!   not a field-mask check.
//! * **P2 — field placement** and **P3 — structure** verify the bit-strings
//!   against the ARM directly.
//!
//! ## Findings
//!
//! 1. **Silent truncation of 32-bit immediates (spec violation).** When
//!    `is_64 == false` the encoder only walks `hw ∈ {0,1}` and silently
//!    discards any bits of `imm` above bit 31. The ARMv8-A ARM requires a
//!    `Wd` destination immediate to fit in 32 bits, and `llvm-mc` rejects
//!    e.g. `mov w0, #0x1_0000_0001` with
//!    *"immediate must be an integer in range [0, 4294967295]"*. The
//!    implementation should return `Err` but instead returns `Ok` with the
//!    high half dropped. Witnessed by the `#[ignore]`d
//!    `mov_wide_rejects_32bit_overflow` (see
//!    `DATA_PROCESSING_MOV_WIDE_IMM_BUG_REPORT.md`).
//!
//! 2. **Dead/unreachable code.** The `if words.is_empty()` fallback at the
//!    tail of the function can never execute: for `imm == 0` the very first
//!    iteration (`hw == 0`) emits a `MOVZ` via the `hw == 0 && imm == 0`
//!    predicate, and for `imm != 0` the first non-zero chunk emits a `MOVZ`.
//!    So `words` is never empty. Documented (not a failure) by property
//!    `mov_wide_words_never_empty`.
//!
//! 3. **No `rd` range validation.** `rd` is OR'd straight into bits [4:0]
//!    with no `rd <= 31` check, so `rd > 31` silently corrupts the `imm16`
//!    field. Witnessed by the `#[ignore]`d `mov_wide_rejects_rd_out_of_range`.
//!
//! All `#[ignore]`d tests are expected to **fail** (they assert a contract the
//! code does not uphold); run them explicitly with
//! `cargo test -- --ignored mov_wide`.

use lccc::pbt_internals::*;
use proptest::prelude::*;

// ── field extractors (ARMv8 MOV-wide immediate encoding) ──────────────────
//  sf  opc(2)  100101  hw(2)  imm16(16)  Rd(5)
//  bit 31 | 30:29 | 28:23 | 22:21 | 20:5 | 4:0
fn sf_of(w: u32) -> u32 {
    (w >> 31) & 1
}
fn opc_of(w: u32) -> u32 {
    (w >> 29) & 0b11
} // 00=MOVN, 10=MOVZ, 11=MOVK
fn fixed_of(w: u32) -> u32 {
    (w >> 23) & 0x3F
} // must be 0b100101
fn hw_of(w: u32) -> u32 {
    (w >> 21) & 0b11
}
fn imm16_of(w: u32) -> u32 {
    (w >> 5) & 0xFFFF
}
fn rd_of(w: u32) -> u32 {
    w & 0x1F
}

const MOVZ_OPC: u32 = 0b10;
const MOVK_OPC: u32 = 0b11;

fn words_of(r: Result<EncodeResult, String>) -> Vec<u32> {
    match r.expect("encoder returned Err") {
        EncodeResult::Word(w) => vec![w],
        EncodeResult::Words(v) => v,
        other => panic!("expected Word/Words, got {:?}", other),
    }
}

/// Reference replay of a MOVZ/MOVK stream: MOVZ zeroes then places imm16 at
/// `hw*16`; MOVK preserves all other bits and writes its 16-bit window. Returns
/// the final 64-bit register value.
fn replay(words: &[u32]) -> u64 {
    let mut val: u64 = 0;
    for w in words {
        let hw = hw_of(*w);
        let imm16 = imm16_of(*w) as u64;
        let shift = (hw as u64) * 16;
        let window = 0xFFFFu64 << shift;
        match opc_of(*w) {
            MOVZ_OPC => val = imm16 << shift, // zero-then-set
            MOVK_OPC => val = (val & !window) | (imm16 << shift),
            other => panic!("unexpected opc {:02b} in word 0x{:08x}", other, w),
        }
    }
    val
}

fn width_mask(is_64: bool) -> u64 {
    if is_64 { u64::MAX } else { 0xFFFF_FFFF }
}

proptest! {
    // P1. Simulation / round-trip oracle: replaying the emitted MOVZ/MOVK
    //     stream yields exactly `imm` masked to the operand width. This is a
    //     genuine semantic oracle, not a field-mask tautology — it catches any
    //     wrong `hw`, wrong imm16, wrong opcode, or wrong sequencing.
    #[test]
    fn mov_wide_roundtrip_reconstructs_imm(
        rd in 0u32..=31,
        is_64 in any::<bool>(),
        imm in any::<u64>(),
    ) {
        let words = words_of(encode_mov_wide_imm(rd, is_64, imm));
        let reconstructed = replay(&words);
        prop_assert_eq!(reconstructed, imm & width_mask(is_64),
            "MOVZ/MOVK stream did not reconstruct the immediate");
    }

    // P2. Field placement: every emitted word conforms to the ARMv8 MOV-wide
    //     bit-string — fixed `100101`, opc ∈ {MOVZ,MOVK}, hw in range, imm16
    //     equals the actual chunk, Rd placed, sf derived from is_64.
    #[test]
    fn mov_wide_every_word_is_well_formed(
        rd in 0u32..=31,
        is_64 in any::<bool>(),
        imm in any::<u64>(),
    ) {
        let max_hw: u32 = if is_64 { 4 } else { 2 };
        let words = words_of(encode_mov_wide_imm(rd, is_64, imm));
        let sf = if is_64 { 1u32 } else { 0u32 };
        for w in &words {
            let hw = hw_of(*w);
            prop_assert_eq!(sf_of(*w), sf, "sf mismatch in 0x{:08x}", w);
            prop_assert_eq!(fixed_of(*w), 0b100101, "fixed bits in 0x{:08x}", w);
            let opc = opc_of(*w);
            prop_assert!(opc == MOVZ_OPC || opc == MOVK_OPC,
                "opc {:02b} not MOVZ/MOVK in 0x{:08x}", opc, w);
            prop_assert!(hw < max_hw,
                "hw={} out of range [0,{}) for 0x{:08x}", hw, max_hw, w);
            prop_assert_eq!(rd_of(*w), rd, "Rd field in 0x{:08x}", w);
            let chunk = ((imm >> ((hw as u64) * 16)) & 0xFFFF) as u32;
            prop_assert_eq!(imm16_of(*w), chunk,
                "imm16 field in 0x{:08x} (hw={})", w, hw);
        }
    }

    // P3. Structure invariant: the first word is always MOVZ (it must zero the
    //     register first), all following words are MOVK, and the word count is
    //     exactly the number of non-zero 16-bit chunks (clamped to >= 1, since
    //     imm==0 still emits one MOVZ). Also documents Finding #2: `words` is
    //     *never* empty, so the trailing `if words.is_empty()` branch is dead.
    #[test]
    fn mov_wide_movz_first_then_movk_and_never_empty(
        rd in 0u32..=31,
        is_64 in any::<bool>(),
        imm in any::<u64>(),
    ) {
        let max_hw: u32 = if is_64 { 4 } else { 2 };
        let words = words_of(encode_mov_wide_imm(rd, is_64, imm));
        prop_assert!(!words.is_empty(),
            "words empty — would hit the (dead) `words.is_empty()` fallback");

        prop_assert_eq!(opc_of(words[0]), MOVZ_OPC,
            "first word must be MOVZ, got opc {:02b}", opc_of(words[0]));
        for w in &words[1..] {
            prop_assert_eq!(opc_of(*w), MOVK_OPC,
                "non-first word must be MOVK, got opc {:02b} in 0x{:08x}",
                opc_of(*w), w);
        }

        let nonzero_chunks = (0..max_hw)
            .filter(|hw| (imm >> ((hw * 16) as u64)) & 0xFFFF != 0)
            .count();
        let expected_len = std::cmp::max(1, nonzero_chunks);
        prop_assert_eq!(words.len(), expected_len,
            "word count != max(1, nonzero_chunks)");
    }

    // P4. Zero immediate: `imm == 0` must encode as a *single* MOVZ Xd, #0
    //     (hw=0, imm16=0) for both widths.
    #[test]
    fn mov_wide_zero_immediate_is_single_movz(
        rd in 0u32..=31,
        is_64 in any::<bool>(),
    ) {
        let words = words_of(encode_mov_wide_imm(rd, is_64, 0));
        prop_assert_eq!(words.len(), 1);
        let w = words[0];
        prop_assert_eq!(opc_of(w), MOVZ_OPC);
        prop_assert_eq!(hw_of(w), 0);
        prop_assert_eq!(imm16_of(w), 0);
        prop_assert_eq!(rd_of(w), rd);
    }

    // ── Finding #1: BUG WITNESS (#[ignore], EXPECTED TO FAIL) ─────────────
    //
    // For a 32-bit destination the ARMv8-A ARM restricts the immediate to
    // 32 bits; `llvm-mc` rejects `mov w0, #0x1_0000_0001`. The encoder should
    // therefore return Err for any imm with bits above bit 31 when is_64 is
    // false. Instead it silently truncates. This test asserts the spec'd
    // contract and fails today.
    #[ignore]
    #[test]
    fn mov_wide_rejects_32bit_overflow(
        rd in 0u32..=31,
        // upper 32 bits, guaranteed non-zero so the value exceeds 0xFFFFFFFF
        hi in 1u64..=0xFFFFFFFFu64,
        lo in any::<u32>(),
    ) {
        let imm = (hi << 32) | (lo as u64);
        prop_assert!(
            encode_mov_wide_imm(rd, false, imm).is_err(),
            "32-bit mov with imm 0x{:x} > 0xFFFFFFFF should be rejected", imm
        );
    }

    // ── Finding #3: BUG WITNESS (#[ignore], EXPECTED TO FAIL) ─────────────
    //
    // `rd` occupies only bits [4:0]; values > 31 have no valid encoding and
    // must be rejected. The encoder OR's rd straight in, silently corrupting
    // the imm16 field.
    #[ignore]
    #[test]
    fn mov_wide_rejects_rd_out_of_range(
        rd in 32u32..=0x1FFFF,
        is_64 in any::<bool>(),
    ) {
        prop_assert!(
            encode_mov_wide_imm(rd, is_64, 0x1234).is_err(),
            "rd={} > 31 should be rejected", rd
        );
    }
}
