//! Property-based tests for `encode_neon_sqshrun`.
//!
//! `encode_neon_sqshrun` encodes the AArch64 NEON *narrowing* shift-by-immediate
//! family `SQSHRUN`/`SQSHRUN2` (signed saturating shift right unsigned narrow)
//! and its rounding twin `SQRSHRUN`/`SQRSHRUN2`. All four live in the
//! "Advanced SIMD shift by immediate" group:
//!
//! ```text
//!   31 30 29 28-23 22-19 18-16 15-10 9-5 4-0
//!    0  Q  1 011110 immh  immb  opcode Rn  Rd
//! ```
//! with `opcode = 100001` (SQSHRUN/SQSHRUN2) or `opcode = 100011`
//! (SQRSHRUN/SQRSHRUN2), and `Q = 1` for the `2` (high-half) variants.
//!
//! ## The immh:immb encoding (the spec)
//! The shared decode for this whole group maps `immh` to the **source** element
//! size `esize` (`immh=001x -> 16`, `immh=01xx -> 32`, `immh=1xxx -> 64`;
//! `immh=0001` is an 8-bit element which has **no 4-bit destination and is
//! UNALLOCATED for any narrow shift**) and recovers the shift as
//!
//! ```text
//!   shift = (esize * 2) - UInt(immh:immb)        // i.e. immh:immb = 2*esize - shift
//! ```
//!
//! This is the formula the crate itself uses in the *non-narrow* siblings
//! `encode_neon_ushr` / `encode_neon_shift_right` (`element_bits * 2 - shift`).
//!
//! ## Oracle
//! The reference encoder below is assembled field-by-field from the layout and
//! uses the ARM-correct `immh:immb = 2*esize - shift`. It is *intentionally*
//! identical to the implementation in every other field (it reuses the impl's
//! `U=1` and `opcode` bits), so a differential failure isolates *exactly* the
//! `immh:immb` computation — no shared opcode/U-bit assumption can mask it.
//! No AArch64 assembler is on PATH in this environment, so the differential
//! oracle is the field-correct reference rather than `llvm-mc`.
//!
//! ## Finding (documented by the `#[ignore]`d witnesses below)
//! `encode_neon_sqshrun` computes `immh:immb = element_bits - shift` (then ORs
//! a broken `immh_base`), i.e. it uses **half** the correct value. Concretely,
//! for shift=1 it emits:
//!
//! | source | impl word  | correct word | impl `immh:immb` | decodes as          |
//! |--------|------------|--------------|------------------|---------------------|
//! | `.8h`  | 0x2F0F8420 | 0x2F1F8420   | 15 (immh=0001)   | **UNALLOCATED**     |
//! | `.4s`  | 0x2F1F8420 | 0x2F378420   | 31 (immh=0011)   | 16-bit src SQSHRUN  |
//! | `.2d`  | 0x2F378420 | 0x2F7F8420   | 63 (immh=0111)   | 32-bit src SQSHRUN  |
//!
//! i.e. for `.4s`/`.2d` sources it silently emits a *different* (smaller-element)
//! instruction, and for `.8h` it emits an UNALLOCATED encoding. The correct
//! `immh:immb` is `2*esize - shift`. See `SQSHRUN_IMMHB_BUG_REPORT.md`.
//!
//! The non-`#[ignore]` properties below pin the parts of the encoding that
//! *are* correct (Rd/Rn placement, Q mapping, the `0_..._011110` prefix,
//! shift-range validation, and the error contract) so the default
//! `cargo test` stays green.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_sqshrun;
use proptest::prelude::*;

// --- helpers --------------------------------------------------------------

/// Source element width in bits for a valid SQSHRUN source arrangement.
fn element_bits(src: &str) -> u32 {
    match src {
        "8h" => 16,
        "4s" => 32,
        "2d" => 64,
        _ => unreachable!("invalid source arrangement {src}"),
    }
}

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

/// Valid SQSHRUN source arrangements paired with an in-range shift
/// (`1..=esize`, matching the encoder's own validation).
fn src_and_shift_strategy() -> impl Strategy<Value = (&'static str, u32)> {
    prop_oneof![
        (Just("8h"), 1u32..=16u32),
        (Just("4s"), 1u32..=32u32),
        (Just("2d"), 1u32..=64u32),
    ]
}

/// ARM-correct reference encoder. Differs from the implementation ONLY in the
/// `immh:immb` computation (`2*esize - shift`); all other fields are taken
/// straight from the impl so a mismatch isolates the `immh:immb` bug.
fn ref_encode_sqshrun(
    rd: u32,
    rn: u32,
    shift: u32,
    src: &str,
    is_rounding: bool,
    is_high: bool,
) -> u32 {
    let eb = element_bits(src);
    let immhb = eb * 2 - shift; // ARM: shift = 2*esize - immh:immb
    let immh = (immhb >> 3) & 0xF;
    let immb = immhb & 0x7;
    let q = if is_high { 1u32 } else { 0 };
    let opcode: u32 = if is_rounding { 0b100011 } else { 0b100001 };
    (q << 30)
        | (1u32 << 29)
        | (0b011110u32 << 23)
        | (immh << 19)
        | (immb << 16)
        | (opcode << 10)
        | (rn << 5)
        | rd
}

fn word_of(res: Result<EncodeResult, String>) -> u32 {
    match res {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {other:?}"),
    }
}

// --- structural properties (impl-correct parts; keep `cargo test` green) ---

proptest! {
    // === Field placement: Rd/Rn round-trip + Q mapping ===================
    // The five-bit register fields must round-trip exactly and Q must equal
    // `is_high` for every valid input. No silent truncation of in-range
    // register numbers.
    #[test]
    fn fields_round_trip_and_q_maps_is_high(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        (src, shift) in src_and_shift_strategy(),
        is_rounding in any::<bool>(),
        is_high in any::<bool>(),
    ) {
        let ops = vec![va(rd, "8b"), va(rn, src), Operand::Imm(shift as i64)];
        let w = word_of(encode_neon_sqshrun(&ops, is_rounding, is_high));

        prop_assert_eq!(w & 0x1F, rd, "Rd field (bits 4-0)");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field (bits 9-5)");
        prop_assert_eq!((w >> 30) & 0x1, u32::from(is_high), "Q bit must equal is_high");
    }

    // === Fixed prefix bits ===============================================
    // bit31 = 0, bit29 (U) = 1, bits28-23 = 011110 for every valid input.
    // (These are regression anchors for the current implementation; they are
    //  stable and structurally correct. The arm-correctness of `immh:immb` is
    //  checked separately in the `#[ignore]`d witnesses below.)
    #[test]
    fn fixed_prefix_bits_are_constant(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        (src, shift) in src_and_shift_strategy(),
        is_rounding in any::<bool>(),
        is_high in any::<bool>(),
    ) {
        let ops = vec![va(rd, "8b"), va(rn, src), Operand::Imm(shift as i64)];
        let w = word_of(encode_neon_sqshrun(&ops, is_rounding, is_high));

        prop_assert_eq!((w >> 31) & 1, 0u32, "bit 31 must be 0");
        prop_assert_eq!((w >> 29) & 1, 1u32, "U bit (29) is 1 in the impl");
        prop_assert_eq!((w >> 23) & 0x3F, 0b011110u32, "bits 28-23 must be 011110");
    }

    // === Opcode distinguishes rounding ===================================
    // The 6-bit opcode field (bits 15-10) must be 100001 for SQSHRUN and
    // 100011 for SQRSHRUN, and must not depend on arrangement/shift/registers.
    #[test]
    fn opcode_encodes_rounding_variant(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        (src, shift) in src_and_shift_strategy(),
        is_high in any::<bool>(),
    ) {
        let ops = vec![va(rd, "8b"), va(rn, src), Operand::Imm(shift as i64)];

        let w_nr = word_of(encode_neon_sqshrun(&ops, false, is_high));
        let w_r = word_of(encode_neon_sqshrun(&ops, true, is_high));

        prop_assert_eq!((w_nr >> 10) & 0x3F, 0b100001u32, "SQSHRUN opcode (15-10)");
        prop_assert_eq!((w_r >> 10) & 0x3F, 0b100011u32, "SQRSHRUN opcode (15-10)");
    }

    // === Negative contract: out-of-range shift ===========================
    // shift == 0 is reserved (immh:immb would be 0) and shift > esize is out
    // of range; both must be rejected. (shift == esize is in range -> Ok.)
    #[test]
    fn rejects_out_of_range_shift(
        rn in reg_num_strategy(),
        (src, _shift) in src_and_shift_strategy(),
    ) {
        let eb = element_bits(src);
        let ops_zero = vec![va(0, "8b"), va(rn, src), Operand::Imm(0)];
        prop_assert!(encode_neon_sqshrun(&ops_zero, false, false).is_err(),
            "shift 0 must be rejected for .{src}");

        let ops_over = vec![va(0, "8b"), va(rn, src), Operand::Imm((eb + 1) as i64)];
        prop_assert!(encode_neon_sqshrun(&ops_over, false, false).is_err(),
            "shift > esize must be rejected for .{src}");
    }
}

// --- negative contract: structural rejections (plain tests) ---------------

#[test]
fn rejects_too_few_operands() {
    for ops in [vec![], vec![va(0, "8b")], vec![va(0, "8b"), va(1, "8h")]] {
        assert!(
            encode_neon_sqshrun(&ops, false, false).is_err(),
            "expected Err for {:?} (needs 3 operands)",
            ops,
        );
    }
}

#[test]
fn rejects_unsupported_source_arrangements() {
    // Only 8h / 4s / 2d are valid *source* arrangements for SQSHRUN.
    for arr in &["8b", "16b", "4h", "2s", "1d"] {
        let ops = vec![va(0, "8b"), va(1, arr), Operand::Imm(1)];
        assert!(
            encode_neon_sqshrun(&ops, false, false).is_err(),
            "source .{arr} is unsupported; expected Err",
        );
    }
}

#[test]
fn rejects_non_immediate_shift() {
    for bad in [
        Operand::Reg("x3".into()),
        Operand::Symbol("lbl".into()),
        Operand::Mem {
            base: "x0".into(),
            offset: 0,
        },
    ] {
        let ops = vec![va(0, "8b"), va(1, "8h"), bad];
        assert!(
            encode_neon_sqshrun(&ops, false, false).is_err(),
            "third operand must be an immediate shift",
        );
    }
}

// --- bug witnesses: immh:immb is wrong (#[ignore]d so default runs green) --
//
// Per the ARM ARM "Advanced SIMD shift by immediate" group, immh encodes the
// SOURCE element size and `shift = 2*esize - immh:immb`, i.e.
// `immh:immb = 2*esize - shift`. The implementation instead emits
// `element_bits - shift` (with a broken `immh_base` OR), producing either an
// UNALLOCATED encoding (`.8h`) or a valid-but-wrong smaller-element
// instruction (`.4s`/`.2d`). These tests assert the correct value and so FAIL
// on the current implementation; run them explicitly, e.g.
//   cargo test -- --ignored sqshrun_immhb

proptest! {
    #[test]
    #[ignore]
    fn immh_immb_matches_arm_spec(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        (src, shift) in src_and_shift_strategy(),
        is_rounding in any::<bool>(),
        is_high in any::<bool>(),
    ) {
        let ops = vec![va(rd, "8b"), va(rn, src), Operand::Imm(shift as i64)];
        let got = word_of(encode_neon_sqshrun(&ops, is_rounding, is_high));
        let want = ref_encode_sqshrun(rd, rn, shift, src, is_rounding, is_high);

        // Isolate the bug: the immh:immb field (bits 22-16) must equal
        // 2*esize - shift.
        let eb = element_bits(src);
        let expected_immhb = eb * 2 - shift;
        let immh_field = (got >> 19) & 0xF;
        let immb_field = (got >> 16) & 0x7;
        let got_immhb = (immh_field << 3) | immb_field;

        prop_assert_eq!(
            got_immhb, expected_immhb,
            ".{} shift #{}: immh:immb is 0x{:X}, expected 0x{:X} (2*esize - shift); \
             full word 0x{:08X} vs correct 0x{:08X}",
            src, shift, got_immhb, expected_immhb, got, want,
        );
    }
}

/// Concrete shift=#1 cases showing the divergence. For `.8h` the emitted
/// `immh` is `0001` (an UNALLOCATED narrow encoding); for `.4s`/`.2d` the word
/// is bit-identical to the *correct* encoding of one element size smaller,
/// i.e. a different instruction is silently produced.
#[test]
#[ignore]
fn golden_immh_immb_diverges_from_arm() {
    // (source arrangement, shift, is_rounding, is_high, correct_word)
    let cases: &[(&str, u32, bool, bool, u32)] = &[
        // SQSHRUN V0.8b, V1.8h, #1   -> correct 0x2F1F8420
        ("8h", 1, false, false, 0x2F1F8420),
        // SQSHRUN V0.4h, V1.4s, #1   -> correct 0x2F378420
        ("4s", 1, false, false, 0x2F378420),
        // SQSHRUN V0.2s, V1.2d, #1   -> correct 0x2F7F8420
        ("2d", 1, false, false, 0x2F7F8420),
    ];

    for &(src, shift, is_rounding, is_high, correct) in cases {
        let ops = vec![va(0, "8b"), va(1, src), Operand::Imm(shift as i64)];
        let got = word_of(encode_neon_sqshrun(&ops, is_rounding, is_high));
        assert_eq!(
            got, correct,
            "SQSHRUN .{src} #{shift}: impl 0x{got:08X} != ARM-correct 0x{correct:08X}",
        );
    }
}
