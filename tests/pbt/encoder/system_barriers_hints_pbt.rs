//! Property-based tests for the AArch64 barrier / hint / exception-entry
//! encoders in `system.rs`:
//!
//!   * `encode_dmb` — Data Memory Barrier
//!   * `encode_dsb` — Data Synchronization Barrier
//!   * `encode_hint` — HINT #imm (NOP / YIELD / WFE / ... family)
//!   * `encode_svc` — SVC #imm16  (Supervisor Call / syscall)
//!   * `encode_brk` — BRK #imm16  (Breakpoint)
//!
//! # Encodings under test
//!
//! ```text
//!   dmb <opt> : 0xD503_30BF | (CRm << 8)        // op2 = 0b101
//!   dsb <opt> : 0xD503_309F | (CRm << 8)        // op2 = 0b100
//!   hint #imm : 0xD503_201F | (CRm << 8) | (op2 << 5)
//!               where CRm = (imm >> 3) & 0xF, op2 = imm & 7   (imm ∈ [0,127])
//!   svc #imm16: 0xD400_0001 | (imm16 << 5)      // LL = 0b00001
//!   brk #imm16: 0xD420_0000 | (imm16 << 5)      // LL = 0b00000
//! ```
//!
//! The barrier `<opt>` name selects CRm directly:
//!
//! ```text
//!   sy=0xF  st=0xE  ld=0xD | ish=0xB  ishst=0xA  ishld=0x9
//!   nsh=0x7 nshst=0x6 nshld=0x5 | osh=0x3 oshst=0x2 oshld=0x1
//! ```
//!
//! # Reference oracle
//!
//! All known-word anchors were cross-checked against `llvm-mc-14
//! --triple=aarch64-linux-gnu`, e.g.:
//!   `dmb sy`   = 0xD503_3FBF      `dsb sy`    = 0xD503_3F9F
//!   `dmb nsh`  = 0xD503_37BF      `dsb ishst` = 0xD503_3A9F
//!   `hint #0`  = 0xD503_201F (NOP) `hint #127` = 0xD503_2FFF
//!   `svc #0`   = 0xD400_0001      `svc #0xffff`= 0xD41F_FFE1
//!   `brk #0`   = 0xD420_0000      `brk #0xffff`= 0xD43F_FFE0
//!
//! `llvm-mc` was also used as the differential oracle for the *negative*
//! contracts: it rejects out-of-range immediates and non-barrier operands,
//! whereas the encoder silently masks / defaults them. See the `#[ignore]`d
//! witnesses at the foot of this file.
//!
//! # Findings surfaced
//!
//! The bit-packing is **correct** for every valid operand: the fixed opcode
//! fields are invariant, every CRm/imm16/7-bit-hint field round-trips out of
//! the word, distinct inputs yield distinct words, `Barrier` and `Symbol`
//! operands agree, names are case-insensitive, and all KAT words match
//! `llvm-mc`.
//!
//! Five **validation bugs** are exposed as `#[ignore]`d witness properties so
//! the default `cargo test` stays green. Run them explicitly with
//! `cargo test --test pbt_arm_system system_barriers_hints -- --ignored`:
//!   * **S1** — `svc #imm` performs no range check; the immediate is masked
//!     with `& 0xFFFF`, so values outside `[0, 0xFFFF]` (e.g. `#65536`,
//!     `#-1`, `#0x1_0000`) are silently accepted. `llvm-mc` rejects them with
//!     *"immediate must be an integer in range [0, 65535]."*.
//!   * **S2** — `brk #imm` has the identical defect (mask `& 0xFFFF`).
//!   * **S3** — `hint #imm` performs no range check; CRm/op2 are masked with
//!     `& 0xF` / `& 0x7`, so values outside `[0, 127]` (e.g. `#128`, `#256`,
//!     `#-1`) are silently accepted. `llvm-mc` rejects them with
//!     *"immediate must be an integer in range [0, 127]."*.
//!   * **S4** — `dmb`/`dsb` with a non-barrier operand silently default to the
//!     `SY` option (CRm = 0xF). `llvm-mc` rejects a register operand
//!     (`dmb x0`) with *"invalid barrier option name"*, and — for a numeric
//!     operand — accepts `dmb #5` as CRm = 5 (`nshld`, 0xD503_35BF), whereas
//!     the encoder emits CRm = 15 (`sy`). So both an invalid operand and a
//!     valid-but-numeric operand are mishandled.

use lccc::pbt_internals::Operand;
use lccc::pbt_internals::*;
use proptest::prelude::*;

// ── helpers ──────────────────────────────────────────────────────────────

/// Unwrap an encoder result, panicking if it is not `EncodeResult::Word`.
fn expect_word(r: Result<EncodeResult, String>) -> u32 {
    match r {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {:?}", other),
    }
}

/// `(barrier_option_name, CRm)` for every option recognised by the encoder.
/// The CRm value is also what `llvm-mc` places in bits[11:8].
const BARRIER_OPTIONS: &[(&str, u32)] = &[
    ("sy", 0xF),
    ("st", 0xE),
    ("ld", 0xD),
    ("ish", 0xB),
    ("ishst", 0xA),
    ("ishld", 0x9),
    ("nsh", 0x7),
    ("nshst", 0x6),
    ("nshld", 0x5),
    ("osh", 0x3),
    ("oshst", 0x2),
    ("oshld", 0x1),
];

// =========================================================================
// encode_dmb — DMB <opt>   (base 0xD503_30BF; CRm at bits[11:8])
// =========================================================================

proptest! {
    // D1. Fixed opcode: for every recognised option, the bits outside the CRm
    //     field are the constant DMB base.
    #[test]
    fn dmb_fixed_opcode_for_known_options(idx in 0usize..BARRIER_OPTIONS.len()) {
        let (name, _crm) = BARRIER_OPTIONS[idx];
        let w = expect_word(encode_dmb(&[Operand::Barrier(name.into())]));
        prop_assert_eq!(w & 0xFFFF_F0FF, 0xD503_30BFu32, "fixed bits for dmb {}", name);
    }

    // D2. CRm round-trips: the option name selects CRm at bits[11:8].
    #[test]
    fn dmb_crm_roundtrips_from_name(idx in 0usize..BARRIER_OPTIONS.len()) {
        let (name, crm) = BARRIER_OPTIONS[idx];
        let w = expect_word(encode_dmb(&[Operand::Barrier(name.into())]));
        prop_assert_eq!((w >> 8) & 0xF, crm, "CRm for dmb {}", name);
    }

    // D3. `Barrier` and `Symbol` spellings are interchangeable.
    #[test]
    fn dmb_barrier_and_symbol_agree(idx in 0usize..BARRIER_OPTIONS.len()) {
        let (name, _crm) = BARRIER_OPTIONS[idx];
        let wb = expect_word(encode_dmb(&[Operand::Barrier(name.into())]));
        let ws = expect_word(encode_dmb(&[Operand::Symbol(name.into())]));
        prop_assert_eq!(wb, ws);
    }

    // D4. Case-insensitivity: the option name is lowercased before lookup.
    #[test]
    fn dmb_case_insensitive(idx in 0usize..BARRIER_OPTIONS.len()) {
        let (name, _crm) = BARRIER_OPTIONS[idx];
        let lo = expect_word(encode_dmb(&[Operand::Barrier(name.into())]));
        let up = expect_word(encode_dmb(&[Operand::Barrier(name.to_uppercase())]));
        prop_assert_eq!(lo, up);
    }

    // D5. Injectivity: distinct CRm values produce distinct words (the option
    //     space is small and closed, so exhaustively comparing pairs is cheap).
    #[test]
    fn dmb_distinct_options_distinct_words(a in 0usize..BARRIER_OPTIONS.len(),
                                           b in 0usize..BARRIER_OPTIONS.len()) {
        let (na, ca) = BARRIER_OPTIONS[a];
        let (nb, cb) = BARRIER_OPTIONS[b];
        prop_assume!(ca != cb);
        let wa = expect_word(encode_dmb(&[Operand::Barrier(na.into())]));
        let wb = expect_word(encode_dmb(&[Operand::Barrier(nb.into())]));
        prop_assert_ne!(wa, wb);
    }

    // D6. Unknown barrier name is rejected (matches `llvm-mc`).
    #[test]
    fn dmb_rejects_unknown_option_name(idx in 0u32..1000) {
        // Names outside the recognised set.
        let bad = format!("badopt{}", idx);
        prop_assert!(encode_dmb(&[Operand::Barrier(bad.clone())]).is_err());
        prop_assert!(encode_dmb(&[Operand::Symbol(bad)]).is_err());
    }
}

// Deterministic known-answer anchors (cross-checked with llvm-mc).
#[test]
fn dmb_known_words_from_llvm_mc() {
    let cases = [
        ("sy", 0xD503_3FBFu32),
        ("st", 0xD503_3EBF),
        ("ld", 0xD503_3DBF),
        ("ish", 0xD503_3BBF),
        ("ishst", 0xD503_3ABF),
        ("ishld", 0xD503_39BF),
        ("nsh", 0xD503_37BF),
        ("nshst", 0xD503_36BF),
        ("nshld", 0xD503_35BF),
        ("osh", 0xD503_33BF),
        ("oshst", 0xD503_32BF),
        ("oshld", 0xD503_31BF),
    ];
    for (name, expected) in cases {
        let w = expect_word(encode_dmb(&[Operand::Barrier(name.into())]));
        assert_eq!(w, expected, "dmb {}", name);
    }
}

// Empty operand list defaults to the `sy` option (gas-compatible alias for
// `dmb sy`). This is a passing characterization of the documented default.
#[test]
fn dmb_empty_operands_defaults_to_sy() {
    let w = expect_word(encode_dmb(&[]));
    assert_eq!(w, 0xD503_3FBF, "bare `dmb` should equal `dmb sy`");
}

// =========================================================================
// encode_dsb — DSB <opt>   (base 0xD503_309F; CRm at bits[11:8])
// =========================================================================

proptest! {
    #[test]
    fn dsb_fixed_opcode_for_known_options(idx in 0usize..BARRIER_OPTIONS.len()) {
        let (name, _crm) = BARRIER_OPTIONS[idx];
        let w = expect_word(encode_dsb(&[Operand::Barrier(name.into())]));
        prop_assert_eq!(w & 0xFFFF_F0FF, 0xD503_309Fu32, "fixed bits for dsb {}", name);
    }

    #[test]
    fn dsb_crm_roundtrips_from_name(idx in 0usize..BARRIER_OPTIONS.len()) {
        let (name, crm) = BARRIER_OPTIONS[idx];
        let w = expect_word(encode_dsb(&[Operand::Barrier(name.into())]));
        prop_assert_eq!((w >> 8) & 0xF, crm, "CRm for dsb {}", name);
    }

    #[test]
    fn dsb_barrier_and_symbol_agree(idx in 0usize..BARRIER_OPTIONS.len()) {
        let (name, _crm) = BARRIER_OPTIONS[idx];
        let wb = expect_word(encode_dsb(&[Operand::Barrier(name.into())]));
        let ws = expect_word(encode_dsb(&[Operand::Symbol(name.into())]));
        prop_assert_eq!(wb, ws);
    }

    #[test]
    fn dsb_case_insensitive(idx in 0usize..BARRIER_OPTIONS.len()) {
        let (name, _crm) = BARRIER_OPTIONS[idx];
        let lo = expect_word(encode_dsb(&[Operand::Barrier(name.into())]));
        let up = expect_word(encode_dsb(&[Operand::Barrier(name.to_uppercase())]));
        prop_assert_eq!(lo, up);
    }

    #[test]
    fn dsb_distinct_options_distinct_words(a in 0usize..BARRIER_OPTIONS.len(),
                                           b in 0usize..BARRIER_OPTIONS.len()) {
        let (na, ca) = BARRIER_OPTIONS[a];
        let (nb, cb) = BARRIER_OPTIONS[b];
        prop_assume!(ca != cb);
        let wa = expect_word(encode_dsb(&[Operand::Barrier(na.into())]));
        let wb = expect_word(encode_dsb(&[Operand::Barrier(nb.into())]));
        prop_assert_ne!(wa, wb);
    }

    #[test]
    fn dsb_rejects_unknown_option_name(idx in 0u32..1000) {
        let bad = format!("badopt{}", idx);
        prop_assert!(encode_dsb(&[Operand::Barrier(bad.clone())]).is_err());
        prop_assert!(encode_dsb(&[Operand::Symbol(bad)]).is_err());
    }
}

#[test]
fn dsb_known_words_from_llvm_mc() {
    let cases = [
        ("sy", 0xD503_3F9Fu32),
        ("st", 0xD503_3E9F),
        ("ld", 0xD503_3D9F),
        ("ish", 0xD503_3B9F),
        ("ishst", 0xD503_3A9F),
        ("ishld", 0xD503_399F),
        ("nsh", 0xD503_379F),
        ("nshst", 0xD503_369F),
        ("nshld", 0xD503_359F),
        ("osh", 0xD503_339F),
        ("oshst", 0xD503_329F),
        ("oshld", 0xD503_319F),
    ];
    for (name, expected) in cases {
        let w = expect_word(encode_dsb(&[Operand::Barrier(name.into())]));
        assert_eq!(w, expected, "dsb {}", name);
    }
}

#[test]
fn dsb_empty_operands_defaults_to_sy() {
    let w = expect_word(encode_dsb(&[]));
    assert_eq!(w, 0xD503_3F9F, "bare `dsb` should equal `dsb sy`");
}

// =========================================================================
// encode_hint — HINT #imm   (base 0xD503_201F; CRm[11:8], op2[7:5])
//   imm = (CRm << 3) | op2,  imm ∈ [0, 127]
// =========================================================================

proptest! {
    // H1. Exact-word oracle over the full legal range: every 7-bit immediate
    //     reconstructs to the canonical HINT word.
    #[test]
    fn hint_in_range_exact_word(imm in 0u32..=127) {
        let w = expect_word(encode_hint(&[Operand::Imm(imm as i64)]));
        let crm = (imm >> 3) & 0xF;
        let op2 = imm & 0x7;
        let expected = 0xD503_201Fu32 | (crm << 8) | (op2 << 5);
        prop_assert_eq!(w, expected);
    }

    // H2. Round-trip: the 7-bit immediate is fully recoverable from the word.
    #[test]
    fn hint_imm7_roundtrips(imm in 0u32..=127) {
        let w = expect_word(encode_hint(&[Operand::Imm(imm as i64)]));
        let crm = (w >> 8) & 0xF;
        let op2 = (w >> 5) & 0x7;
        prop_assert_eq!((crm << 3) | op2, imm);
    }

    // H3. Fixed opcode: bits outside CRm[11:8] and op2[7:5] are constant.
    #[test]
    fn hint_fixed_opcode(imm in 0u32..=127) {
        let w = expect_word(encode_hint(&[Operand::Imm(imm as i64)]));
        prop_assert_eq!(w & 0xFFFF_F01F, 0xD503_201Fu32);
    }

    // H4. Injectivity over the legal range.
    #[test]
    fn hint_distinct_imm_distinct_words(a in 0u32..=127, b in 0u32..=127) {
        prop_assume!(a != b);
        let wa = expect_word(encode_hint(&[Operand::Imm(a as i64)]));
        let wb = expect_word(encode_hint(&[Operand::Imm(b as i64)]));
        prop_assert_ne!(wa, wb);
    }

    // H5. Error contract: a missing or non-Imm first operand is rejected.
    #[test]
    fn hint_rejects_non_imm_first_operand(kind in 0u8..3) {
        let ops: Vec<Operand> = match kind {
            0 => vec![],
            1 => vec![Operand::Reg("x0".into())],
            _ => vec![Operand::Symbol("foo".into())],
        };
        prop_assert!(encode_hint(&ops).is_err(), "expected Err for {:?}", ops);
    }
}

#[test]
fn hint_known_words_from_llvm_mc() {
    // hint #0  == nop, hint #1 == yield, hint #127 == maximum hint code.
    assert_eq!(expect_word(encode_hint(&[Operand::Imm(0)])), 0xD503_201F);
    assert_eq!(expect_word(encode_hint(&[Operand::Imm(1)])), 0xD503_203F);
    assert_eq!(expect_word(encode_hint(&[Operand::Imm(127)])), 0xD503_2FFF);
}

// =========================================================================
// encode_svc — SVC #imm16   (base 0xD400_0001; imm16 at bits[20:5])
// =========================================================================

proptest! {
    // V1. Fixed opcode + LL field: bits[31:21] and bits[4:0] are constant.
    #[test]
    fn svc_fixed_opcode_and_ll_field(imm in 0u32..=0xFFFF) {
        let w = expect_word(encode_svc(&[Operand::Imm(imm as i64)]));
        prop_assert_eq!(w & 0xFFE0_001F, 0xD400_0001u32);
    }

    // V2. imm16 round-trips out of bits[20:5].
    #[test]
    fn svc_imm16_roundtrips(imm in 0u32..=0xFFFF) {
        let w = expect_word(encode_svc(&[Operand::Imm(imm as i64)]));
        prop_assert_eq!((w >> 5) & 0xFFFF, imm);
    }

    // V3. Injectivity over the 16-bit range.
    #[test]
    fn svc_distinct_imm_distinct_words(a in 0u32..=0xFFFF, b in 0u32..=0xFFFF) {
        prop_assume!(a != b);
        let wa = expect_word(encode_svc(&[Operand::Imm(a as i64)]));
        let wb = expect_word(encode_svc(&[Operand::Imm(b as i64)]));
        prop_assert_ne!(wa, wb);
    }

    // V4. Error contract: a missing or non-Imm first operand is rejected.
    #[test]
    fn svc_rejects_non_imm_first_operand(kind in 0u8..3) {
        let ops: Vec<Operand> = match kind {
            0 => vec![],
            1 => vec![Operand::Reg("x0".into())],
            _ => vec![Operand::Symbol("foo".into())],
        };
        prop_assert!(encode_svc(&ops).is_err(), "expected Err for {:?}", ops);
    }
}

#[test]
fn svc_known_words_from_llvm_mc() {
    assert_eq!(expect_word(encode_svc(&[Operand::Imm(0)])), 0xD400_0001);
    assert_eq!(expect_word(encode_svc(&[Operand::Imm(1)])), 0xD400_0021);
    assert_eq!(
        expect_word(encode_svc(&[Operand::Imm(0xFFFF)])),
        0xD41F_FFE1
    );
}

// =========================================================================
// encode_brk — BRK #imm16   (base 0xD420_0000; imm16 at bits[20:5])
// =========================================================================

proptest! {
    // K1. Fixed opcode + LL field: bits[31:21] and bits[4:0] are constant.
    #[test]
    fn brk_fixed_opcode_and_ll_field(imm in 0u32..=0xFFFF) {
        let w = expect_word(encode_brk(&[Operand::Imm(imm as i64)]));
        prop_assert_eq!(w & 0xFFE0_001F, 0xD420_0000u32);
    }

    // K2. imm16 round-trips out of bits[20:5].
    #[test]
    fn brk_imm16_roundtrips(imm in 0u32..=0xFFFF) {
        let w = expect_word(encode_brk(&[Operand::Imm(imm as i64)]));
        prop_assert_eq!((w >> 5) & 0xFFFF, imm);
    }

    // K3. Injectivity over the 16-bit range.
    #[test]
    fn brk_distinct_imm_distinct_words(a in 0u32..=0xFFFF, b in 0u32..=0xFFFF) {
        prop_assume!(a != b);
        let wa = expect_word(encode_brk(&[Operand::Imm(a as i64)]));
        let wb = expect_word(encode_brk(&[Operand::Imm(b as i64)]));
        prop_assert_ne!(wa, wb);
    }

    // K4. Error contract: a missing or non-Imm first operand is rejected.
    #[test]
    fn brk_rejects_non_imm_first_operand(kind in 0u8..3) {
        let ops: Vec<Operand> = match kind {
            0 => vec![],
            1 => vec![Operand::Reg("x0".into())],
            _ => vec![Operand::Symbol("foo".into())],
        };
        prop_assert!(encode_brk(&ops).is_err(), "expected Err for {:?}", ops);
    }
}

#[test]
fn brk_known_words_from_llvm_mc() {
    assert_eq!(expect_word(encode_brk(&[Operand::Imm(0)])), 0xD420_0000);
    assert_eq!(expect_word(encode_brk(&[Operand::Imm(1)])), 0xD420_0020);
    assert_eq!(
        expect_word(encode_brk(&[Operand::Imm(0xFFFF)])),
        0xD43F_FFE0
    );
}

// =========================================================================
// Cross-function differential: DMB and DSB differ only in op2 (bits[7:5]).
// DMB op2 = 0b101 (base byte 0xBF), DSB op2 = 0b100 (base byte 0x9F). The
// two words for the *same* option must therefore differ only in bit 5
// (the op2 LSB: 0b101 vs 0b100).
// =========================================================================

proptest! {
    #[test]
    fn dmb_and_dsb_differ_only_in_op2_bit(idx in 0usize..BARRIER_OPTIONS.len()) {
        let (name, _crm) = BARRIER_OPTIONS[idx];
        let dmb = expect_word(encode_dmb(&[Operand::Barrier(name.into())]));
        let dsb = expect_word(encode_dsb(&[Operand::Barrier(name.into())]));
        // Identical except bit 5: DMB op2=0b101 sets it, DSB op2=0b100 clears it.
        prop_assert_eq!(dmb ^ dsb, 1u32 << 5);
        prop_assert_eq!(dmb & !(1u32 << 5), dsb);
        prop_assert_ne!(dmb, dsb);
    }
}

// =========================================================================
// Bug witnesses — #[ignore] so the default `cargo test` stays green.
// Run with:  cargo test --test pbt_arm_system system_barriers_hints -- --ignored
// =========================================================================

/// **S1** — `svc #imm` must reject immediates outside `[0, 0xFFFF]`. `llvm-mc`
/// rejects them with *"immediate must be an integer in range [0, 65535]."*;
/// the encoder instead masks `& 0xFFFF` and silently accepts them (e.g.
/// `#65536` aliases `#0`, `#0x1_0001` aliases `#1`).
#[test]
#[ignore = "documented bug: out-of-range SVC immediate is masked (& 0xFFFF) instead of rejected; llvm-mc says 'immediate must be an integer in range [0, 65535].'"]
fn b_s1_svc_rejects_out_of_range_immediate() {
    let bad = [65536i64, 65537, 0x1_0000, 0x1_FFFF, -1, -2, i64::MAX];
    for imm in bad {
        let r = encode_svc(&[Operand::Imm(imm)]);
        assert!(
            r.is_err(),
            "svc #{:#x} should be rejected (out of [0,0xFFFF]), got {:?}",
            imm,
            r,
        );
    }
}

/// **S2** — `brk #imm` has the identical defect (mask `& 0xFFFF`).
#[test]
#[ignore = "documented bug: out-of-range BRK immediate is masked (& 0xFFFF) instead of rejected; llvm-mc says 'immediate must be an integer in range [0, 65535].'"]
fn b_s2_brk_rejects_out_of_range_immediate() {
    let bad = [65536i64, 65537, 0x1_0000, 0x1_FFFF, -1, -2, i64::MAX];
    for imm in bad {
        let r = encode_brk(&[Operand::Imm(imm)]);
        assert!(
            r.is_err(),
            "brk #{:#x} should be rejected (out of [0,0xFFFF]), got {:?}",
            imm,
            r,
        );
    }
}

/// **S3** — `hint #imm` must reject immediates outside `[0, 127]`. `llvm-mc`
/// rejects them with *"immediate must be an integer in range [0, 127]."*; the
/// encoder masks CRm/op2 with `& 0xF` / `& 0x7`, so e.g. `#128` aliases `#0`
/// (NOP) and `#256` aliases `#0`.
#[test]
#[ignore = "documented bug: out-of-range HINT immediate is masked (& 0xF / & 0x7) instead of rejected; llvm-mc says 'immediate must be an integer in range [0, 127].'"]
fn b_s3_hint_rejects_out_of_range_immediate() {
    let bad = [128i64, 129, 255, 256, 1000, -1, i64::MAX];
    for imm in bad {
        let r = encode_hint(&[Operand::Imm(imm)]);
        assert!(
            r.is_err(),
            "hint #{:#x} should be rejected (out of [0,127]), got {:?}",
            imm,
            r,
        );
    }
}

/// **S4a** — `dmb`/`dsb` must reject a register operand. `llvm-mc` rejects
/// `dmb x0` / `dsb x0` with *"invalid barrier option name"*; the encoder's
/// catch-all arm instead defaults to the `sy` option and emits a word.
#[test]
#[ignore = "documented bug: register operand to dmb/dsb silently defaults to SY; llvm-mc rejects with 'invalid barrier option name'"]
fn b_s4a_dmb_dsb_reject_register_operand() {
    let bad_regs = ["x0", "w5", "sp", "d0", "v3"];
    for rname in bad_regs {
        let r = encode_dmb(&[Operand::Reg(rname.into())]);
        assert!(
            r.is_err(),
            "dmb {} should be rejected (not a barrier option), got {:?}",
            rname,
            r,
        );
        let r = encode_dsb(&[Operand::Reg(rname.into())]);
        assert!(
            r.is_err(),
            "dsb {} should be rejected (not a barrier option), got {:?}",
            rname,
            r,
        );
    }
}

/// **S4b** — A *numeric* barrier operand is a valid `llvm-mc` input and selects
/// CRm directly: `dmb #5` == `dmb nshld` (0xD503_35BF), `dsb #7` == `dsb nsh`
/// (0xD503_379F). The encoder's catch-all instead emits the `sy` option
/// (CRm = 0xF), producing the wrong instruction for a valid input.
#[test]
#[ignore = "documented bug: numeric dmb/dsb operand defaults to SY instead of mapping to CRm; llvm-mc encodes dmb #5 as CRm=5 (nshld), dmb #7 as CRm=7 (nsh)"]
fn b_s4b_dmb_dsb_numeric_operand_maps_to_crm() {
    // dmb #5 -> CRm = 5 (nshld) per llvm-mc.
    let w = expect_word(encode_dmb(&[Operand::Imm(5)]));
    assert_eq!(w, 0xD503_35BF, "dmb #5 should equal dmb nshld");
    // dsb #7 -> CRm = 7 (nsh) per llvm-mc.
    let w = expect_word(encode_dsb(&[Operand::Imm(7)]));
    assert_eq!(w, 0xD503_379F, "dsb #7 should equal dsb nsh");
}
