//! Property-based tests for `encode_mov` **immediate dispatch** and its
//! interaction with the `movz` / `movk` / `movn` wide-immediate encoders
//! (`data_processing.rs`).
//!
//! ## Scope / what is NEW here
//!
//! The three known defect classes — immediate truncation, shift
//! normalization, and W-register invalid shift on `encode_mov{z,k,n}` — are
//! already reported under `pbt-out/bug_reports/encode_mov{z,k,n}_*.md` and
//! witnessed by the (failing) inline `*_rejects_*` tests in
//! `data_processing.rs`. This file deliberately targets a **different** defect:
//! the `mov <Rd>, #imm` dispatch in `encode_mov` and how it (mis)handles the
//! `lsl #N` shift operand that the wide-immediate alias permits.
//!
//! `MOV (wide immediate)` is an alias of `MOVZ`: the ARMv8-A ARM spells it
//! `MOV <Wd|WSP>, #<imm>{, LSL #<shift>}`. A real assembler therefore treats
//! `mov x0, #0x1234, lsl #16` as `movz x0, #0x1234, lsl #16` (hw=1, materialising
//! `0x12340000`). `llvm-mc` confirms:
//!
//! ```text
//! $ echo 'mov x0, #0x1234, lsl #16' | llvm-mc --triple=aarch64 -show-encoding
//!     movz   x0, #4660, lsl #16       // encoding: [0x80,0x46,0xa2,0xd2]
//! ```
//! i.e. word `0xD2A24680` (sf=1, opc=10, hw=1, imm16=0x1234).
//!
//! ## Oracles
//!
//! * **Differential (passing)** — `encode_mov` with no shift must agree with
//!   `encode_movz` / `encode_movn` on the same operands. Guards the
//!   non-defective dispatch paths.
//! * **Differential (witness, `#[ignore]`)** — `encode_mov` **with** a valid
//!   `lsl #N` must agree with `encode_movz` (which honours the shift). This is
//!   the headline bug: today `encode_mov` ignores `operands[2]`.
//! * **Negative contract (witness, `#[ignore]`)** — `mov Wd, #imm, lsl #N`
//!   with `N >= 32` (UNALLOCATED for `sf=0`) must be rejected.
//! * **Reference (witness, `#[ignore]`)** — the concrete `mov x0,#0x1234,lsl#16`
//!   KAT must equal `0xD2A24680`.
//!
//! All `#[ignore]`d tests are expected to **fail** against the current SUT; run
//! them explicitly with `cargo test --lib data_processing_mov_dispatch -- --ignored`.
//! Default `cargo test` stays green (only the passing characterisation properties
//! run).

use lccc::pbt_internals::Operand;
use lccc::pbt_internals::*;
use proptest::prelude::*;

fn opc_of(w: u32) -> u32 {
    (w >> 29) & 0b11
} // 00=MOVN, 10=MOVZ, 11=MOVK
fn hw_of(w: u32) -> u32 {
    (w >> 21) & 0b11
}
fn imm16_of(w: u32) -> u32 {
    (w >> 5) & 0xFFFF
}
const MOVN_OPC: u32 = 0b00;

/// Build a destination register operand. `is_64` selects `xN` vs `wN`.
fn dst_reg(rd: u32, is_64: bool) -> Operand {
    Operand::Reg(format!("{}{}", if is_64 { "x" } else { "w" }, rd))
}

fn imm_op(v: i64) -> Operand {
    Operand::Imm(v)
}
fn lsl(amount: u32) -> Operand {
    Operand::Shift {
        kind: "lsl".into(),
        amount,
    }
}

/// Reference MOVZ word, independently assembled from the ARMv8-A ARM
/// bit-string `sf 10 100101 hw imm16 Rd`.
fn ref_movz(sf: u32, hw: u32, imm16: u32, rd: u32) -> u32 {
    (sf << 31) | (0b10 << 29) | (0b100101 << 23) | (hw << 21) | ((imm16 & 0xFFFF) << 5) | rd
}

fn expect_word(r: Result<EncodeResult, String>) -> u32 {
    match r.unwrap() {
        EncodeResult::Word(w) => w,
        other => panic!("expected Word, got {:?}", other),
    }
}

proptest! {
    // ── P1. Differential (passing): the no-shift immediate path of
    //       `encode_mov` agrees bit-for-bit with `encode_movz` for every
    //       in-range 16-bit immediate and both widths. This guards the
    //       well-behaved dispatch branch and establishes `encode_movz` as a
    //       sound oracle for the shift-honouring witness below.
    #[test]
    fn mov_imm_no_shift_matches_movz(
        rd in 0u32..=30,
        is_64 in any::<bool>(),
        imm in 0i64..=0xFFFF,
    ) {
        let ops = vec![dst_reg(rd, is_64), imm_op(imm)];
        let via_mov = expect_word(encode_mov(&ops));
        let via_movz = expect_word(encode_movz(&ops));
        prop_assert_eq!(via_mov, via_movz);
        // and both match the independent reference word (hw=0)
        let sf = if is_64 { 1 } else { 0 };
        prop_assert_eq!(via_mov, ref_movz(sf, 0, imm as u32, rd));
    }

    // ── P2. Differential (passing): the negative-immediate dispatch path of
    //       `encode_mov` (`imm < 0` with `!imm` in 16 bits) agrees bit-for-bit
    //       with `encode_movn`. `mov x0, #-N` is the canonical alias of
    //       `movn x0, #(!N)`. Guards the MOVN branch of the dispatcher.
    #[test]
    fn mov_neg_imm_matches_movn(
        rd in 0u32..=30,
        is_64 in any::<bool>(),
        // neg ∈ [-0xFFFF,-1]  ⇒  !neg ∈ [0,0xFFFE], well inside the MOVN range.
        neg in (-0xFFFFi64)..(-0i64),
    ) {
        let not_imm = !neg; // i64 bitwise NOT, what encode_mov itself computes
        let via_mov = expect_word(encode_mov(&[dst_reg(rd, is_64), imm_op(neg)]));
        let via_movn = expect_word(encode_movn(&[dst_reg(rd, is_64), imm_op(not_imm)]));
        prop_assert_eq!(via_mov, via_movn);
        prop_assert_eq!(opc_of(via_mov), MOVN_OPC);
        prop_assert_eq!(hw_of(via_mov), 0);
        prop_assert_eq!(imm16_of(via_mov), (not_imm as u32) & 0xFFFF);
    }

    // ── P3. Soundness of the oracle (passing): `encode_movz` itself honours a
    //       valid `lsl #N`, placing `hw = N/16` and the imm16 untouched. This
    //       is the property `encode_mov` *should* also satisfy via the alias,
    //       and it establishes that any disagreement in W1 is `encode_mov`'s
    //       fault, not the oracle's. Restricted to the spec-valid domain:
    //       X ⇒ {0,16,32,48}, W ⇒ {0,16}.
    #[test]
    fn movz_honours_valid_lsl_shift(
        rd in 0u32..=30,
        is_64 in any::<bool>(),
        imm in 0i64..=0xFFFF,
        hw in 0u32..=3,
    ) {
        // For 32-bit (sf=0) only hw in {0,1} is architecturally valid.
        prop_assume!(is_64 || hw < 2);
        let amount = hw * 16;
        let ops = vec![dst_reg(rd, is_64), imm_op(imm), lsl(amount)];
        let w = expect_word(encode_movz(&ops));
        let sf = if is_64 { 1 } else { 0 };
        prop_assert_eq!(w, ref_movz(sf, hw, imm as u32, rd));
        prop_assert_eq!(hw_of(w), hw);
        prop_assert_eq!(imm16_of(w), imm as u32);
    }

    // ── P4. Reference (passing): `mov x0, #0x1234` (no shift) encodes to the
    //       documented MOVZ x0,#0x1234 word. Parameterised over Rd so it lives
    //       inside proptest!.
    #[test]
    fn mov_imm_kat_matches_reference_param(
        rd in 0u32..=30,
    ) {
        let ops = vec![dst_reg(rd, true), imm_op(0x1234)];
        let w = expect_word(encode_mov(&ops));
        prop_assert_eq!(w, ref_movz(1, 0, 0x1234, rd));
        prop_assert_eq!(hw_of(w), 0);
    }
}

// ── P4b. Plain (non-proptest) KAT: `mov x0, #0x1234` encodes to the
//        documented MOVZ x0,#0x1234 word (rd=0).
#[test]
fn mov_imm_kat_matches_reference() {
    let ops = vec![dst_reg(0, true), imm_op(0x1234)];
    let w = expect_word(encode_mov(&ops));
    assert_eq!(w, ref_movz(1, 0, 0x1234, 0));
    assert_eq!(hw_of(w), 0);
}

proptest! {
    // =====================================================================
    //  BUG WITNESSES — all #[ignore]'d, all EXPECTED TO FAIL today.
    //  Root cause: `encode_mov`'s `mov <Rd>, #imm` branch reads only
    //  `operands[0]` and `operands[1]`; it never inspects `operands[2]`
    //  (the `Shift`), so a valid `lsl #N` is silently dropped.
    // =====================================================================

    // ── W1. HEADLINE differential witness: `mov <Rd>, #imm, lsl #N` is the
    //       alias of `movz <Rd>, #imm, lsl #N`, so the two encoders MUST emit
    //       the identical 32-bit word. Today `encode_mov` drops the shift
    //       (hw=0) while `encode_movz` honours it (hw=N/16), so they disagree
    //       for every non-zero valid shift. Restricted to the spec-valid
    //       shift domain so the failure isolates the dispatch drop and is not
    //       masked by the (separately reported) W-register-shift defect of
    //       `encode_movz` itself.
    #[ignore = "documented bug: encode_mov dispatch drops lsl shift on mov <Rd>,#imm; movz honours it"]
    #[test]
    fn mov_dispatch_honours_lsl_like_movz(
        rd in 0u32..=30,
        is_64 in any::<bool>(),
        imm in 0i64..=0xFFFF,
        hw in 1u32..=3, // hw>=1 => non-zero shift => the drop is observable
    ) {
        prop_assume!(is_64 || hw < 2); // keep to the spec-valid shift domain
        let amount = hw * 16;
        let ops = vec![dst_reg(rd, is_64), imm_op(imm), lsl(amount)];
        let via_mov = expect_word(encode_mov(&ops));
        let via_movz = expect_word(encode_movz(&ops));
        prop_assert_eq!(via_mov, via_movz,
            "mov dropped lsl #{}: mov hw={} vs movz hw={}",
            amount, hw_of(via_mov), hw_of(via_movz));
        // and the dropped word must match the independent reference (hw=N/16)
        let sf = if is_64 { 1 } else { 0 };
        prop_assert_eq!(via_mov, ref_movz(sf, hw, imm as u32, rd));
    }

    // ── W2. Negative-contract witness: for a 32-bit destination only
    //       `lsl #{0,16}` is allocated; `lsl #32` / `lsl #48` are UNALLOCATED
    //       (hw in {2,3} with sf=0). Because `encode_mov` drops the shift it
    //       silently accepts `mov w0, #1, lsl #32` and emits `movz w0, #1`
    //       instead of rejecting it.
    #[ignore = "documented bug: encode_mov accepts mov Wd,#imm,lsl#>=32 (dropped, UNALLOCATED for sf=0)"]
    #[test]
    fn mov_dispatch_rejects_unallocated_w_lsl(
        rd in 0u32..=30,
        imm in 0i64..=0xFFFF,
        bad_amount in 32u32..=63,
    ) {
        let ops = vec![dst_reg(rd, false), imm_op(imm), lsl(bad_amount)];
        prop_assert!(encode_mov(&ops).is_err(),
            "mov w{}, #0x{:x}, lsl #{} should be rejected (UNALLOCATED for W)",
            rd, imm, bad_amount);
    }
}

// ── W3. Plain (non-proptest) reference KAT witness (minimal reproducer): the
//        exact input from the `llvm-mc` transcript above must encode to
//        `0xD2A24680`. Today `encode_mov` returns a word with hw=0 — i.e. it
//        materialises `0x1234` instead of `0x12340000`.
#[ignore = "documented bug: mov x0,#0x1234,lsl#16 must be 0xD2A24680 (hw=1), not 0xD2802468 (hw=0)"]
#[test]
fn mov_dispatch_kat_lsl16() {
    let ops = vec![dst_reg(0, true), imm_op(0x1234), lsl(16)];
    let w = expect_word(encode_mov(&ops));
    assert_eq!(
        w,
        0xD2A24680,
        "mov x0,#0x1234,lsl#16: got 0x{:08X} (hw={}, imm16=0x{:x}), expected 0xD2A24680 (hw=1)",
        w,
        hw_of(w),
        imm16_of(w)
    );
}
