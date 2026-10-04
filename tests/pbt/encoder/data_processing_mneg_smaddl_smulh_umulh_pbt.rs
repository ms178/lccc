//! Property-based tests for the AArch64 data-processing (3-source) encoders:
//! `encode_mneg`, `encode_smaddl`, `encode_smulh`, and `encode_umulh` in
//! `data_processing.rs`.
//!
//! ## Layout
//! All four share the data-processing (3-source) encoding shape
//! `sf 00 11011 op31 Rm o0 Ra Rn Rd`:
//!
//! ```text
//!  bit 31     : sf          (size)
//!  bits 30:29 : 00
//!  bits 28:24 : 11011       (3-source class)
//!  bits 23:21 : op31        (selects MADD/MSUB/SMADDL/UMADDL/SMULH/UMULH)
//!  bits 20:16 : Rm
//!  bit 15     : o0
//!  bits 14:10 : Ra
//!  bits  9:5  : Rn
//!  bits  4:0  : Rd
//! ```
//!
//! ## Coverage
//! For each function we check:
//!   * full-word equality vs an independently constructed ARMv8 reference,
//!   * fixed opcode-field invariants (sf / op31 / o0 / Ra),
//!   * per-register field placement & orthogonality,
//!   * negative contracts that HOLD (operand count, non-register operands,
//!     out-of-range register numbers).
//!
//! ## Bug witnesses (`#[ignore]`)
//! `encode_smaddl`, `encode_smulh`, and `encode_umulh` hard-code `sf = 1` and
//! discard the `is_64` flag returned by `get_reg`, so they never validate
//! register width. They also delegate register parsing to `parse_reg_num`,
//! which accepts every prefix (`x|w|d|s|q|v|h|b`), so they never validate the
//! register *file*. The negative-contract properties that pin these defects are
//! kept here as `#[ignore]`d witnesses — they FAIL against the current SUT, so
//! they are excluded from a default `cargo test` to keep it green. Run them
//! explicitly with:
//!
//! ```text
//! cargo test --test pbt_arm_core -- --ignored data_processing_mneg_smaddl_smulh_umulh_pbt
//! ```
//!
//! (`encode_mneg` is correct for both W and X widths — `sf` is derived from the
//! destination — so it carries no ignored witness.)

use lccc::pbt_internals::*;
use proptest::prelude::*;

// ── shared helpers ──────────────────────────────────────────────────────────

fn xreg(n: u32) -> Operand {
    Operand::Reg(format!("x{}", n))
}
fn wreg(n: u32) -> Operand {
    Operand::Reg(format!("w{}", n))
}

fn expect_word(r: Result<EncodeResult, String>) -> u32 {
    match r.unwrap() {
        EncodeResult::Word(w) => w,
        other => panic!("expected Word, got {:?}", other),
    }
}

// Field extractors for the data-processing (3-source) layout.
fn sf_of(w: u32) -> u32 {
    (w >> 31) & 1
}
fn opcode5_of(w: u32) -> u32 {
    (w >> 24) & 0x1F
}
fn op31_of(w: u32) -> u32 {
    (w >> 21) & 0x7
}
fn rm_of(w: u32) -> u32 {
    (w >> 16) & 0x1F
}
fn o0_of(w: u32) -> u32 {
    (w >> 15) & 1
}
fn ra_of(w: u32) -> u32 {
    (w >> 10) & 0x1F
}
fn rn_of(w: u32) -> u32 {
    (w >> 5) & 0x1F
}
fn rd_of(w: u32) -> u32 {
    w & 0x1F
}

// ── independent ARMv8 reference encoders ────────────────────────────────────
// (MNEG = MSUB Xd,Xn,Xm,XZR ; SMADDL/SMULH/UMULH per the ARMv8 ARM bit-strings.)

/// `MNEG Xd, Xn, Xm` -> MSUB with Ra = XZR (o0=bit15=1, Ra=11111).
fn mneg_ref(rd: u32, rn: u32, rm: u32, is_64: bool) -> u32 {
    let sf = u32::from(is_64);
    (sf << 31) | (0b0011011000 << 21) | (rm << 16) | (1u32 << 15) | (0b11111 << 10) | (rn << 5) | rd
}
/// `SMADDL Xd, Wn, Wm, Xa`: sf=1, op31=001, o0=0.
fn smaddl_ref(rd: u32, rn: u32, rm: u32, ra: u32) -> u32 {
    (1u32 << 31) | (0b0011011001 << 21) | (rm << 16) | (ra << 10) | (rn << 5) | rd
}
/// `SMULH Xd, Xn, Xm`: sf=1, op31=010, o0=0, Ra=11111.
fn smulh_ref(rd: u32, rn: u32, rm: u32) -> u32 {
    (1u32 << 31) | (0b0011011010 << 21) | (rm << 16) | (0b11111 << 10) | (rn << 5) | rd
}
/// `UMULH Xd, Xn, Xm`: sf=1, op31=110, o0=0, Ra=11111.
fn umulh_ref(rd: u32, rn: u32, rm: u32) -> u32 {
    (1u32 << 31) | (0b0011011110 << 21) | (rm << 16) | (0b11111 << 10) | (rn << 5) | rd
}

// ════════════════════════════════════════════════════════════════════════════
// encode_mneg — MNEG Xd, Xn, Xm  (== MSUB Xd, Xn, Xm, XZR)
// ════════════════════════════════════════════════════════════════════════════
proptest! {
    // Oracle: reference (literal ARMv8 bit-string). MNEG is defined in BOTH the
    // 32-bit (W) and 64-bit (X) forms, so `sf` is taken from the destination.
    #![proptest_config(proptest::test_runner::Config {
        cases: 256,
        ..proptest::test_runner::Config::default()
    })]

    // 1. Full-word equality vs the independent reference, for W and X dests.
    #[test]
    fn mneg_matches_reference(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30, is_w in any::<bool>(),
    ) {
        let rd_op = if is_w { wreg(rd) } else { xreg(rd) };
        let rn_op = if is_w { wreg(rn) } else { xreg(rn) };
        let rm_op = if is_w { wreg(rm) } else { xreg(rm) };
        let ops = vec![rd_op, rn_op, rm_op];
        let w = expect_word(encode_mneg(&ops));
        prop_assert_eq!(w, mneg_ref(rd, rn, rm, !is_w));
    }

    // 2. Fixed opcode fields are invariant across every operand combination:
    //    class=11011, op31=000, o0(bit15)=1 (MSUB, i.e. negate), Ra=11111 (XZR).
    #[test]
    fn mneg_fixed_fields_invariant(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30, is_w in any::<bool>(),
    ) {
        let rd_op = if is_w { wreg(rd) } else { xreg(rd) };
        let rn_op = if is_w { wreg(rn) } else { xreg(rn) };
        let rm_op = if is_w { wreg(rm) } else { xreg(rm) };
        let w = expect_word(encode_mneg(&[rd_op, rn_op, rm_op]));
        prop_assert_eq!(sf_of(w), u32::from(!is_w));
        prop_assert_eq!((w >> 29) & 0x3, 0b00);
        prop_assert_eq!(opcode5_of(w), 0b11011);
        prop_assert_eq!(op31_of(w), 0b000);
        prop_assert_eq!(o0_of(w), 1);          // MSUB (negate), never MADD
        prop_assert_eq!(ra_of(w), 0b11111);    // Ra hardwired to XZR
    }

    // 3. Each register perturbs ONLY its own 5-bit field; no bleed.
    #[test]
    fn mneg_register_fields_isolated(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
        rd2 in 0u32..=30, rn2 in 0u32..=30, rm2 in 0u32..=30,
    ) {
        let base = expect_word(encode_mneg(&[xreg(rd), xreg(rn), xreg(rm)]));
        // Rd -> bits 4:0
        let d = base ^ expect_word(encode_mneg(&[xreg(rd2), xreg(rn), xreg(rm)]));
        prop_assert_eq!(d & !0x0000001Fu32, 0);
        prop_assert_eq!(d & 0x1F, rd ^ rd2);
        // Rn -> bits 9:5
        let n = base ^ expect_word(encode_mneg(&[xreg(rd), xreg(rn2), xreg(rm)]));
        prop_assert_eq!(n & !0x000003E0u32, 0);
        prop_assert_eq!((n >> 5) & 0x1F, rn ^ rn2);
        // Rm -> bits 20:16
        let m = base ^ expect_word(encode_mneg(&[xreg(rd), xreg(rn), xreg(rm2)]));
        prop_assert_eq!(m & !0x001F0000u32, 0);
        prop_assert_eq!((m >> 16) & 0x1F, rm ^ rm2);
    }

    // 4. Architectural alias: MNEG Xd,Xn,Xm bit-identically equals
    //    MSUB Xd,Xn,Xm,XZR.
    #[test]
    fn mneg_alias_equals_msub_xzr(rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30) {
        let mneg = expect_word(encode_mneg(&[xreg(rd), xreg(rn), xreg(rm)]));
        let msub = expect_word(encode_msub(&[
            xreg(rd), xreg(rn), xreg(rm), Operand::Reg("xzr".into()),
        ]));
        prop_assert_eq!(mneg, msub);
        prop_assert_eq!(o0_of(mneg), 1);
        prop_assert_eq!(o0_of(msub), 1);
    }

    // 5. NEGATIVE CONTRACTS (these HOLD): too few operands, a non-register
    //    operand anywhere, and an out-of-range register number are all Err.
    #[test]
    fn mneg_rejects_invalid_operands(
        n in 0u32..=2u32,           // too few operands
        pos in 0u32..=2u32,         // which slot is non-register
        bad in 32u32..=4096u32,     // out-of-range register number
    ) {
        let too_few: Vec<Operand> = (0..n).map(|i| xreg(i % 31)).collect();
        prop_assert!(encode_mneg(&too_few).is_err());

        let mut ops = vec![xreg(0), xreg(1), xreg(2)];
        ops[pos as usize] = Operand::Imm(7);
        prop_assert!(encode_mneg(&ops).is_err());

        let oor = vec![Operand::Reg(format!("x{}", bad)), xreg(1), xreg(2)];
        prop_assert!(encode_mneg(&oor).is_err());
    }
}

// ════════════════════════════════════════════════════════════════════════════
// encode_smaddl — SMADDL Xd, Wn, Wm, Xa  (signed multiply-add long)
// ════════════════════════════════════════════════════════════════════════════
proptest! {
    #![proptest_config(proptest::test_runner::Config {
        cases: 256,
        ..proptest::test_runner::Config::default()
    })]

    // 1. Reference match for architecturally-correct widths (Xd, Wn, Wm, Xa).
    #[test]
    fn smaddl_matches_reference(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30, ra in 0u32..=30,
    ) {
        let ops = vec![xreg(rd), wreg(rn), wreg(rm), xreg(ra)];
        let w = expect_word(encode_smaddl(&ops));
        prop_assert_eq!(w, smaddl_ref(rd, rn, rm, ra));
    }

    // 2. Fixed fields: sf=1, bits30:29=00, class=11011, op31=001, o0=0.
    #[test]
    fn smaddl_fixed_fields(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30, ra in 0u32..=30,
    ) {
        let ops = vec![xreg(rd), wreg(rn), wreg(rm), xreg(ra)];
        let w = expect_word(encode_smaddl(&ops));
        prop_assert_eq!(sf_of(w), 1);
        prop_assert_eq!((w >> 29) & 0x3, 0b00);
        prop_assert_eq!(opcode5_of(w), 0b11011);
        prop_assert_eq!(op31_of(w), 0b001);
        prop_assert_eq!(o0_of(w), 0);
    }

    // 3. Register field placement: Rd/Rn/Rm/Ra -> 4:0 / 9:5 / 20:16 / 14:10.
    #[test]
    fn smaddl_register_fields(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30, ra in 0u32..=30,
    ) {
        let ops = vec![xreg(rd), wreg(rn), wreg(rm), xreg(ra)];
        let w = expect_word(encode_smaddl(&ops));
        prop_assert_eq!(rd_of(w), rd);
        prop_assert_eq!(rn_of(w), rn);
        prop_assert_eq!(rm_of(w), rm);
        prop_assert_eq!(ra_of(w), ra);
    }

    // 4. NEGATIVE CONTRACT (HOLDS): fewer than four operands -> Err.
    #[test]
    fn smaddl_rejects_too_few_operands(n in 0u32..=3u32) {
        let ops: Vec<Operand> = (0..n).map(xreg).collect();
        prop_assert!(encode_smaddl(&ops).is_err());
    }

    // 5. NEGATIVE CONTRACT (HOLDS): non-register operand anywhere, and
    //    out-of-range register number, are Err.
    #[test]
    fn smaddl_rejects_non_register_and_oor(
        pos in 0u32..=3u32, bad in 32u32..=4096u32,
    ) {
        let mut ops = vec![xreg(0), wreg(1), wreg(2), xreg(3)];
        ops[pos as usize] = Operand::Imm(9);
        prop_assert!(encode_smaddl(&ops).is_err());

        let bad_ops = vec![
            Operand::Reg(format!("x{}", bad)),
            wreg(1),
            wreg(2),
            xreg(3),
        ];
        prop_assert!(encode_smaddl(&bad_ops).is_err());
    }

    // ── Bug witness (FAILS by design; #[ignore] keeps `cargo test` green) ──
    // B1. SMADDL is `Xd, Wn, Wm, Xa` — 64-bit-only (sf=1). A 32-bit W
    //     destination has no valid encoding and MUST be Err. The encoder
    //     hard-codes sf=1 and discards `is_64`, so it silently accepts a W
    //     destination and re-emits a 64-bit instruction. See
    //     the transplanted campaign notes.
    //     Run: cargo test --test pbt_arm_core -- --ignored smaddl_rejects_w_destination
    #[test]
    fn smaddl_rejects_w_destination(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30, ra in 0u32..=30,
    ) {
        let ops = vec![wreg(rd), wreg(rn), wreg(rm), xreg(ra)];
        prop_assert!(
            encode_smaddl(&ops).is_err(),
            "SMADDL <Xd>,<Wn>,<Wm>,<Xa> rejects a W destination; got Ok"
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// encode_smulh — SMULH Xd, Xn, Xm  (signed multiply high; 64-bit only)
// ════════════════════════════════════════════════════════════════════════════
proptest! {
    #![proptest_config(proptest::test_runner::Config {
        cases: 256,
        ..proptest::test_runner::Config::default()
    })]

    // 1. Reference match.
    #[test]
    fn smulh_matches_reference(rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30) {
        let ops = vec![xreg(rd), xreg(rn), xreg(rm)];
        let w = expect_word(encode_smulh(&ops));
        prop_assert_eq!(w, smulh_ref(rd, rn, rm));
    }

    // 2. Fixed fields: sf=1, bits30:29=00, class=11011, op31=010, o0=0,
    //    Ra=11111.
    #[test]
    fn smulh_fixed_fields(rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30) {
        let ops = vec![xreg(rd), xreg(rn), xreg(rm)];
        let w = expect_word(encode_smulh(&ops));
        prop_assert_eq!(sf_of(w), 1);
        prop_assert_eq!((w >> 29) & 0x3, 0b00);
        prop_assert_eq!(opcode5_of(w), 0b11011);
        prop_assert_eq!(op31_of(w), 0b010);
        prop_assert_eq!(o0_of(w), 0);
        prop_assert_eq!(ra_of(w), 0b11111);
    }

    // 3. Differential vs UMULH: identical operands differ ONLY in op31 bit 23
    //    (the sign selector). All register fields, Ra, o0, and the class opcode
    //    are shared.
    #[test]
    fn smulh_vs_umulh_only_sign_bit_differs(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
    ) {
        let ops = vec![xreg(rd), xreg(rn), xreg(rm)];
        let s = expect_word(encode_smulh(&ops));
        let u = expect_word(encode_umulh(&ops));
        prop_assert_eq!(s ^ u, 1u32 << 23);
        prop_assert_eq!((s >> 23) & 1, 0); // SMULH: op31=010
        prop_assert_eq!((u >> 23) & 1, 1); // UMULH: op31=110
    }

    // 4. NEGATIVE CONTRACT (HOLDS): too few operands, non-register operand, and
    //    out-of-range register number are Err.
    #[test]
    fn smulh_rejects_invalid_operands(
        n in 0u32..=2u32, pos in 0u32..=2u32, bad in 32u32..=4096u32,
    ) {
        let too_few: Vec<Operand> = (0..n).map(xreg).collect();
        prop_assert!(encode_smulh(&too_few).is_err());

        let mut ops = vec![xreg(0), xreg(1), xreg(2)];
        ops[pos as usize] = Operand::Imm(5);
        prop_assert!(encode_smulh(&ops).is_err());

        let oor = vec![Operand::Reg(format!("x{}", bad)), xreg(1), xreg(2)];
        prop_assert!(encode_smulh(&oor).is_err());
    }

    // ── Bug witness (FAILS by design; #[ignore] keeps `cargo test` green) ──
    // B2. SMULH is `Xd, Xn, Xm` — 64-bit ONLY. There is no 32-bit (W) form, so
    //     any W operand must be Err. The encoder discards `is_64` and silently
    //     re-emits a 64-bit instruction. See
    //     the transplanted campaign notes.
    //     Run: cargo test --test pbt_arm_core -- --ignored smulh_rejects_w_registers
    #[test]
    fn smulh_rejects_w_registers(rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30) {
        let ops = vec![wreg(rd), wreg(rn), wreg(rm)];
        prop_assert!(
            encode_smulh(&ops).is_err(),
            "SMULH <Xd>,<Xn>,<Xm> is 64-bit-only; W operands must be rejected; got Ok"
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// encode_umulh — UMULH Xd, Xn, Xm  (unsigned multiply high; 64-bit only)
// ════════════════════════════════════════════════════════════════════════════
proptest! {
    #![proptest_config(proptest::test_runner::Config {
        cases: 256,
        ..proptest::test_runner::Config::default()
    })]

    // 1. Reference match.
    #[test]
    fn umulh_matches_reference(rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30) {
        let ops = vec![xreg(rd), xreg(rn), xreg(rm)];
        let w = expect_word(encode_umulh(&ops));
        prop_assert_eq!(w, umulh_ref(rd, rn, rm));
    }

    // 2. Fixed fields: sf=1, bits30:29=00, class=11011, op31=110, o0=0,
    //    Ra=11111.
    #[test]
    fn umulh_fixed_fields(rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30) {
        let ops = vec![xreg(rd), xreg(rn), xreg(rm)];
        let w = expect_word(encode_umulh(&ops));
        prop_assert_eq!(sf_of(w), 1);
        prop_assert_eq!((w >> 29) & 0x3, 0b00);
        prop_assert_eq!(opcode5_of(w), 0b11011);
        prop_assert_eq!(op31_of(w), 0b110);
        prop_assert_eq!(o0_of(w), 0);
        prop_assert_eq!(ra_of(w), 0b11111);
    }

    // 3. Register field orthogonality: varying one operand changes ONLY its own
    //    field (Rd=4:0, Rn=9:5, Rm=20:16), leaving every fixed opcode bit and
    //    the other fields bit-for-bit identical.
    #[test]
    fn umulh_register_fields_orthogonal(a in 0u32..=30, b in 0u32..=30) {
        // Rd
        let r0 = expect_word(encode_umulh(&[xreg(a), xreg(b), xreg(b)]));
        let r1 = expect_word(encode_umulh(&[xreg(b), xreg(b), xreg(b)]));
        prop_assert_eq!((r0 ^ r1) & !0x1Fu32, 0);
        // Rn
        let n0 = expect_word(encode_umulh(&[xreg(a), xreg(a), xreg(b)]));
        let n1 = expect_word(encode_umulh(&[xreg(a), xreg(b), xreg(b)]));
        prop_assert_eq!((n0 ^ n1) & !0x3E0u32, 0);
        // Rm
        let m0 = expect_word(encode_umulh(&[xreg(a), xreg(a), xreg(a)]));
        let m1 = expect_word(encode_umulh(&[xreg(a), xreg(a), xreg(b)]));
        prop_assert_eq!((m0 ^ m1) & !0x1F0000u32, 0);
    }

    // 4. NEGATIVE CONTRACT (HOLDS): too few operands, non-register operand, and
    //    out-of-range register number are Err.
    #[test]
    fn umulh_rejects_invalid_operands(
        n in 0u32..=2u32, pos in 0u32..=2u32, bad in 32u32..=4096u32,
    ) {
        let too_few: Vec<Operand> = (0..n).map(xreg).collect();
        prop_assert!(encode_umulh(&too_few).is_err());

        let mut ops = vec![xreg(0), xreg(1), xreg(2)];
        ops[pos as usize] = Operand::Imm(5);
        prop_assert!(encode_umulh(&ops).is_err());

        let oor = vec![Operand::Reg(format!("x{}", bad)), xreg(1), xreg(2)];
        prop_assert!(encode_umulh(&oor).is_err());
    }

    // ── Bug witness (FAILS by design; #[ignore] keeps `cargo test` green) ──
    // B3. UMULH is `Xd, Xn, Xm` — 64-bit ONLY. There is no 32-bit (W) form, so
    //     any W operand must be Err. The encoder discards `is_64` and silently
    //     re-emits a 64-bit instruction. See
    //     the transplanted campaign notes and
    //     umulh_accepts_32bit_w_registers.md.
    //     Run: cargo test --test pbt_arm_core -- --ignored umulh_rejects_w_registers
    #[test]
    fn umulh_rejects_w_registers(rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30) {
        let ops = vec![wreg(rd), wreg(rn), wreg(rm)];
        prop_assert!(
            encode_umulh(&ops).is_err(),
            "UMULH <Xd>,<Xn>,<Xm> is 64-bit-only; W operands must be rejected; got Ok"
        );
    }

    // ── Bug witness (FAILS by design; #[ignore] keeps `cargo test` green) ──
    // B4. UMULH operates ONLY on general-purpose (X) registers. An FP/SIMD
    //     register name (D/S/Q/V/H/B) is the wrong register FILE and must be
    //     Err. The encoder delegates to parse_reg_num, which accepts every
    //     prefix, so e.g. `umulh d0,d1,d2` silently encodes identically to
    //     `umulh x0,x1,x2`. See
    //     the transplanted campaign notes.
    //     Run: cargo test --test pbt_arm_core -- --ignored umulh_rejects_fp_simd_registers
    #[test]
    fn umulh_rejects_fp_simd_registers(pos in 0u32..=2u32, pfx in 0u32..=5u32, num in 0u32..=30) {
        let prefixes = ["d", "s", "q", "v", "h", "b"];
        let bad = Operand::Reg(format!("{}{}", prefixes[pfx as usize], num));
        let mut ops = vec![xreg(num), xreg(num), xreg(num)];
        ops[pos as usize] = bad;
        prop_assert!(
            encode_umulh(&ops).is_err(),
            "UMULH is GP-only; FP/SIMD register must be rejected; got Ok"
        );
    }
}
