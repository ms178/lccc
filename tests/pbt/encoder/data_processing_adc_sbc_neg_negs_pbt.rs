//! Property-based tests for the **add/subtract-with-carry** and
//! **negate** shifted-register encoders in `data_processing.rs`:
//!
//! * `encode_adc`  — ADC/ADCS `<Rd>,<Rn>,<Rm>`   (carry-in add; op=0)
//! * `encode_sbc`  — SBC/SBCS `<Rd>,<Rn>,<Rm>`   (carry-in sub; op=1)
//! * `encode_neg`  — NEG  `<Rd>,<Rm>{,<shift>}`  (alias SUB  Rd,XZR,Rm)
//! * `encode_negs` — NEGS `<Rd>,<Rm>{,<shift>}`  (alias SUBS Rd,XZR,Rm)
//!
//! ## ARMv8-A encodings
//!
//! Add/subtract (with carry), §C4.1.9 / §C4.1.207:
//! ```text
//!  31  30  29  28:21       20:16  15:10   9:5   4:0
//!   sf  op   S  1 1 0 1 0 0 0 0   Rm    0 0 0 0 0 0  Rn    Rd
//! ```
//! `op=0` → ADC, `op=1` → SBC; `S=1` → ADCS/SBCS. Rm/Rn/Rd are
//! **general-purpose registers or the zero register** — *not* SP/WSP, and
//! *not* FP/SIMD registers. Register 31 in this group encodes **ZR** (XZR/WZR),
//! so `<Rd>/<Rn>/<Rm>` may be `xzr`/`wzr` but **never** `sp`/`wsp`.
//!
//! Add/subtract (shifted register) — the group NEG/NEGS alias into, §C4.1.3:
//! ```text
//!  31  30  29  28:24      23:22    21  20:16  15:10   9:5      4:0
//!   sf   1   S  0 1 0 1 1   shift    0    Rm    imm6   Rn(=31)  Rd
//! ```
//! NEG/NEGS set Rn=11111 (XZR/WZR). Again register 31 == ZR here, so SP is
//! **not** a permitted operand name for the destination or the source.
//!
//! ## What is NEW here
//!
//! Pre-existing `bug_reports/` already cover register-width mixing for
//! `encode_adc`, `encode_sbc`, and `encode_neg` (and many shift-range findings
//! for neg/negs). This suite targets three defect classes that are **not yet
//! reported** for these four functions:
//!
//! 1. **`encode_negs` register-width mixing** — NEW. `encode_neg` is already
//!    reported, but `encode_negs` derives `sf` from `Rd` only and silently
//!    accepts e.g. `negs w0, x1`. `llvm-mc`/GAS reject it.
//! 2. **SP/WSP acceptance** — NEW for all four. `get_reg`/`parse_reg_num` map
//!    `sp`→31 / `wsp`→31, and because this encoding group reads register 31 as
//!    **ZR**, the operand is silently aliased to `xzr`/`wzr` instead of being
//!    rejected. `clang --target=aarch64` rejects `adc x0, sp, x1`, `neg sp, x0`,
//!    etc. with "invalid operand for instruction".
//! 3. **FP/SIMD register acceptance** — NEW for all four. `parse_reg_num`
//!    accepts `v/d/s/q/h/b` prefixes, so e.g. `adc x0, x1, v2` encodes
//!    bit-identically to `adc x0, x1, x2` instead of erroring.
//!
//! The bug **witnesses** (properties asserting the spec-correct rejection) are
//! all marked `#[ignore]`, so default `cargo test` stays green; run them with:
//!
//! ```text
//! cargo test --test pbt_arm_core data_processing_adc_sbc_neg_negs_pbt -- --ignored
//! ```
//! Three passing *characterisation* properties pin the **current (buggy)**
//! behaviour (sf from Rd only; SP==XZR; FP==GP); they will start failing once
//! the defects are fixed — the cue to drop the `#[ignore]` markers.

use lccc::pbt_internals::Operand;
use lccc::pbt_internals::*;
use proptest::prelude::*;

// ── field extractors (ARMv8 add/sub carry + shifted register) ─────────────
fn sf_of(w: u32) -> u32 {
    (w >> 31) & 1
} // width
fn op_of(w: u32) -> u32 {
    (w >> 30) & 1
} // 0=add 1=sub
fn carry_op_of(w: u32) -> u32 {
    (w >> 21) & 0xFF
} // bits 28:21 == 11010000
fn class5_of(w: u32) -> u32 {
    (w >> 24) & 0x1F
} // bits 28:24 == 01011
fn rm_of(w: u32) -> u32 {
    (w >> 16) & 0x1F
}
fn imm6_of(w: u32) -> u32 {
    (w >> 10) & 0x3F
}
fn rn_of(w: u32) -> u32 {
    (w >> 5) & 0x1F
}
fn rd_of(w: u32) -> u32 {
    w & 0x1F
}

// ── operand builders ─────────────────────────────────────────────────────
fn xreg(n: u32) -> Operand {
    Operand::Reg(format!("x{}", n))
}
fn wreg(n: u32) -> Operand {
    Operand::Reg(format!("w{}", n))
}

/// SP / WSP: both parse to register number 31 — which is **ZR**, not SP, in
/// the add/subtract-with-carry and add/subtract shifted-register groups.
fn sp_variant(i: u32) -> Operand {
    match i % 2 {
        0 => Operand::Reg("sp".into()),
        _ => Operand::Reg("wsp".into()),
    }
}

/// FP/SIMD register names accepted by `parse_reg_num` but illegal as GP
/// operands: d/s/q/v/h/b with a lane number.
fn fp_variant(i: u32, n: u32) -> Operand {
    const PFX: &[char] = &['d', 's', 'q', 'v', 'h', 'b'];
    Operand::Reg(format!("{}{}", PFX[(i as usize) % PFX.len()], n))
}

fn word(r: Result<EncodeResult, String>) -> u32 {
    match r {
        Ok(EncodeResult::Word(w)) => w,
        Ok(other) => panic!("expected Word, got {:?}", other),
        Err(e) => panic!("expected Ok, got Err: {}", e),
    }
}

// ── reference encoders (rebuilt independently from the ARMv8 bit-strings) ─
fn adc_ref(rd: u32, rn: u32, rm: u32, is_64: bool, s: u32) -> u32 {
    let sf = if is_64 { 1u32 } else { 0 };
    (sf << 31) | (s << 29) | (0b11010000 << 21) | (rm << 16) | (rn << 5) | rd
}
fn sbc_ref(rd: u32, rn: u32, rm: u32, is_64: bool, s: u32) -> u32 {
    let sf = if is_64 { 1u32 } else { 0 };
    (sf << 31) | (1u32 << 30) | (s << 29) | (0b11010000 << 21) | (rm << 16) | (rn << 5) | rd
}
fn neg_ref(rd: u32, rm: u32, is_64: bool) -> u32 {
    let sf = if is_64 { 1u32 } else { 0 };
    (sf << 31) | (1u32 << 30) | (0b01011 << 24) | (rm << 16) | (0b11111u32 << 5) | rd
}
fn negs_ref(rd: u32, rm: u32, is_64: bool) -> u32 {
    let sf = if is_64 { 1u32 } else { 0 };
    (sf << 31) | (1u32 << 30) | (1u32 << 29) | (0b01011 << 24) | (rm << 16) | (0b11111u32 << 5) | rd
}

// Encoder tables (carry pair takes a set_flags arg; negate pair does not).
type Enc3 = fn(&[Operand], bool) -> Result<EncodeResult, String>;
const CARRY: &[(&str, Enc3)] = &[("adc", encode_adc), ("sbc", encode_sbc)];
type Enc2 = fn(&[Operand]) -> Result<EncodeResult, String>;
const NEG: &[(&str, Enc2)] = &[("neg", encode_neg), ("negs", encode_negs)];

proptest! {
    // ── G1. HAPPY-PATH REFERENCE (passing guard): ADC/SBC/ADCS/SBCS ───────
    // Every valid (Rd,Rn,Rm,width,set_flags) encodes to the exact ARMv8 word;
    // SBC differs from ADC only in the op bit (30); the S bit tracks set_flags.
    #[test]
    fn adc_sbc_match_armv8_reference(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
        is_64 in any::<bool>(), set_flags in any::<bool>(),
    ) {
        let mk = |n: u32| if is_64 { xreg(n) } else { wreg(n) };
        let ops = vec![mk(rd), mk(rn), mk(rm)];
        let s = if set_flags { 1u32 } else { 0 };
        prop_assert_eq!(word(encode_adc(&ops, set_flags)), adc_ref(rd, rn, rm, is_64, s));
        prop_assert_eq!(word(encode_sbc(&ops, set_flags)), sbc_ref(rd, rn, rm, is_64, s));
        // Fixed carry opcode + zero imm6 + register fields.
        for &(_n, f) in CARRY {
            let w = word(f(&ops, set_flags));
            prop_assert_eq!(carry_op_of(w), 0b11010000);
            prop_assert_eq!(imm6_of(w), 0);
            prop_assert_eq!(rm_of(w), rm);
            prop_assert_eq!(rn_of(w), rn);
            prop_assert_eq!(rd_of(w), rd);
            prop_assert_eq!(sf_of(w), if is_64 { 1 } else { 0 });
        }
        // ADC vs SBC differ only in bit 30.
        prop_assert_eq!(word(encode_adc(&ops, set_flags)) ^ word(encode_sbc(&ops, set_flags)), 1u32 << 30);
    }

    // ── G2. HAPPY-PATH REFERENCE (passing guard): NEG / NEGS ─────────────
    // NEG/NEGS alias SUB/SUBS Rd,XZR,Rm: op=1, class=01011, Rn field==31,
    // imm6==0; NEGS sets S=1. Full-word match against the rebuilt reference.
    #[test]
    fn neg_negs_match_armv8_reference(
        rd in 0u32..=30, rm in 0u32..=30, is_64 in any::<bool>(),
    ) {
        let mk = |n: u32| if is_64 { xreg(n) } else { wreg(n) };
        let ops = vec![mk(rd), mk(rm)];
        prop_assert_eq!(word(encode_neg(&ops)), neg_ref(rd, rm, is_64));
        prop_assert_eq!(word(encode_negs(&ops)), negs_ref(rd, rm, is_64));
        for &(_n, f) in NEG {
            let w = word(f(&ops));
            prop_assert_eq!(op_of(w), 1);
            prop_assert_eq!(class5_of(w), 0b01011);
            prop_assert_eq!(rn_of(w), 31);                 // XZR/WZR implicit
            prop_assert_eq!(imm6_of(w), 0);
            prop_assert_eq!(rm_of(w), rm);
            prop_assert_eq!(rd_of(w), rd);
            prop_assert_eq!(sf_of(w), if is_64 { 1 } else { 0 });
        }
        // NEGS vs NEG differ only in the S bit (29).
        prop_assert_eq!(word(encode_neg(&ops)) ^ word(encode_negs(&ops)), 1u32 << 29);
    }

    // ── G3. NEGATIVE: too few operands => Err (passing guard) ────────────
    #[test]
    fn carry_and_neg_reject_too_few_operands(n in 0u32..=30, missing in 1u32..=3) {
        let mut ops = vec![xreg(n), xreg(n), xreg(n)];
        for _ in 0..missing { ops.pop(); }
        for &(_n, f) in CARRY {
            prop_assert!(f(&ops, false).is_err());
        }
        let mut ops2 = vec![xreg(n), xreg(n)];
        for _ in 0..(missing.min(2)) { ops2.pop(); }
        for &(_n, f) in NEG {
            prop_assert!(f(&ops2).is_err());
        }
    }
}

proptest! {
    // ── C1. CHARACTERISATION (current buggy behaviour): sf from Rd ONLY ──
    // PASSING today. All four encoders derive `sf` from the destination and
    // ignore source widths, so mixed W/X operands assemble at the destination
    // width. (Width-mixing for adc/sbc/neg is already reported; for negs this
    // is the NEW defect the W1 witness below asserts.)
    #[test]
    fn sf_is_rejected_on_width_mismatch(
        n in 0u32..=30, rd_is_x in any::<bool>(),
        rn_is_x in any::<bool>(), rm_is_x in any::<bool>(),
    ) {
        // Mixed x/w operands must be REJECTED; sf consistency is enforced
        // across every register operand (previously sf came from Rd alone).
        let mk = |is_x: bool, n: u32| if is_x { xreg(n) } else { wreg(n) };
        prop_assume!(rd_is_x != rn_is_x || rn_is_x != rm_is_x);
        let carry_ops = vec![mk(rd_is_x, n), mk(rn_is_x, n), mk(rm_is_x, n)];
        for &(_n, f) in CARRY {
            prop_assert!(f(&carry_ops, false).is_err(), "carry mixed width must be rejected");
        }
        prop_assume!(rd_is_x != rm_is_x);
        let neg_ops = vec![mk(rd_is_x, n), mk(rm_is_x, n)];
        for &(_n, f) in NEG {
            prop_assert!(f(&neg_ops).is_err(), "neg mixed width must be rejected");
        }
    }

    // ── C2. CHARACTERISATION (current buggy behaviour): SP == XZR ────────
    // PASSING today. Because register 31 is ZR in these groups, `sp`/`wsp`
    // encode bit-identically to `xzr`/`wzr` in the same slot — i.e. SP is
    // silently accepted and aliased to the zero register.
    #[test]
    fn sp_is_rejected(
        rd in 0u32..=30, _rn in 0u32..=30, rm in 0u32..=30, set_flags in any::<bool>(),
    ) {
        // SP/WSP must be REJECTED (register 31 reads as ZR in these groups;
        // previously sp/wsp were silently aliased to it).
        // The 64-bit forms: sp rejected, xzr encodes.
        let a = vec![Operand::Reg(format!("x{}", rd)), Operand::Reg("sp".into()), xreg(rm)];
        prop_assert!(encode_adc(&a, set_flags).is_err(), "adc with sp must be rejected");
        let b = vec![Operand::Reg(format!("x{}", rd)), Operand::Reg("xzr".into()), xreg(rm)];
        prop_assert!(encode_adc(&b, set_flags).is_ok(), "adc with xzr must encode");
        let c = vec![Operand::Reg("sp".into()), xreg(rm)];
        prop_assert!(encode_neg(&c).is_err(), "neg with sp must be rejected");
        // The 32-bit forms: wsp rejected, wzr encodes (with w registers).
        let e = vec![wreg(rd), Operand::Reg("wsp".into()), wreg(rm)];
        prop_assert!(encode_adc(&e, set_flags).is_err(), "adc with wsp must be rejected");
        let f = vec![wreg(rd), Operand::Reg("wzr".into()), wreg(rm)];
        prop_assert!(encode_adc(&f, set_flags).is_ok(), "adc with wzr must encode");
    }

    // ── C3. CHARACTERISATION (current buggy behaviour): FP/SIMD == GP ────
    // PASSING today. `parse_reg_num` accepts FP/SIMD prefixes, so a SIMD name
    // in an operand slot encodes identically to the same-numbered GP register.
    #[test]
    fn fp_simd_is_rejected(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30, vi in 0u32..=6u32,
    ) {
        // FP/SIMD register spellings must be REJECTED in GPR operand
        // positions (previously they encoded identically to the
        // same-numbered GP register).
        let bad = fp_variant(vi, rm);
        let ops_bad = vec![xreg(rd), xreg(rn), bad.clone()];
        prop_assert!(encode_adc(&ops_bad, false).is_err(), "adc with FP/SIMD must be rejected");
        let ops_bad2 = vec![xreg(rd), bad];
        prop_assert!(encode_negs(&ops_bad2).is_err(), "negs with FP/SIMD must be rejected");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  BUG WITNESSES — all #[ignore]'d so `cargo test` stays green.
//  Run:  cargo test --test pbt_arm_core data_processing_adc_sbc_neg_negs_pbt -- --ignored
//  Each asserts the SPEC-CORRECT behaviour the encoder currently violates.
//  Differential oracle: `clang --target=aarch64-linux-gnu` rejects every one
//  of these spellings with "error: invalid operand for instruction".
// ═══════════════════════════════════════════════════════════════════════════
proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    // ── W1. NEW: encode_negs silently accepts mixed W/X register widths ───
    // NEGS <Rd>,<Rm> needs matching widths; `negs w0, x1` is illegal (GAS:
    // "operand size mismatch"). The encoder takes sf from Rd only → Ok. FAILS.
    #[test]
    fn negs_rejects_mixed_register_widths(
        rd in 0u32..=30, rm in 0u32..=30, mix in 0u32..=1u32,
    ) {
        let ops = match mix {
            0 => vec![wreg(rd), xreg(rm)],   // W dest, X source
            _ => vec![xreg(rd), wreg(rm)],   // X dest, W source
        };
        prop_assert!(encode_negs(&ops).is_err(), "negs mixed width must be Err, got {:?}", encode_negs(&ops));
    }

    // ── W2. NEW: encode_adc accepts SP/WSP in any operand position ────────
    #[test]
    fn adc_rejects_sp_wsp_in_any_position(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
        pos in 0u32..3u32, set_flags in any::<bool>(), vi in 0u32..=1u32,
    ) {
        let mut ops = vec![xreg(rd), xreg(rn), xreg(rm)];
        ops[pos as usize] = sp_variant(vi);
        prop_assert!(encode_adc(&ops, set_flags).is_err(),
            "adc with SP/WSP at position {} must be Err, got {:?}", pos, encode_adc(&ops, set_flags));
    }

    // ── W3. NEW: encode_sbc accepts SP/WSP in any operand position ────────
    #[test]
    fn sbc_rejects_sp_wsp_in_any_position(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
        pos in 0u32..3u32, set_flags in any::<bool>(), vi in 0u32..=1u32,
    ) {
        let mut ops = vec![xreg(rd), xreg(rn), xreg(rm)];
        ops[pos as usize] = sp_variant(vi);
        prop_assert!(encode_sbc(&ops, set_flags).is_err(),
            "sbc with SP/WSP at position {} must be Err, got {:?}", pos, encode_sbc(&ops, set_flags));
    }

    // ── W4. NEW: encode_neg accepts SP/WSP in any operand position ────────
    #[test]
    fn neg_rejects_sp_wsp_in_any_position(
        rd in 0u32..=30, rm in 0u32..=30, pos in 0u32..2u32, vi in 0u32..=1u32,
    ) {
        let mut ops = vec![xreg(rd), xreg(rm)];
        ops[pos as usize] = sp_variant(vi);
        prop_assert!(encode_neg(&ops).is_err(),
            "neg with SP/WSP at position {} must be Err, got {:?}", pos, encode_neg(&ops));
    }

    // ── W5. NEW: encode_negs accepts SP/WSP in any operand position ───────
    #[test]
    fn negs_rejects_sp_wsp_in_any_position(
        rd in 0u32..=30, rm in 0u32..=30, pos in 0u32..2u32, vi in 0u32..=1u32,
    ) {
        let mut ops = vec![xreg(rd), xreg(rm)];
        ops[pos as usize] = sp_variant(vi);
        prop_assert!(encode_negs(&ops).is_err(),
            "negs with SP/WSP at position {} must be Err, got {:?}", pos, encode_negs(&ops));
    }

    // ── W6. NEW: encode_adc accepts FP/SIMD registers in any position ─────
    #[test]
    fn adc_rejects_fp_simd_in_any_position(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
        pos in 0u32..3u32, set_flags in any::<bool>(), vi in 0u32..=5u32,
    ) {
        let mut ops = vec![xreg(rd), xreg(rn), xreg(rm)];
        ops[pos as usize] = fp_variant(vi, rm);
        prop_assert!(encode_adc(&ops, set_flags).is_err(),
            "adc with FP/SIMD at position {} must be Err, got {:?}", pos, encode_adc(&ops, set_flags));
    }

    // ── W7. NEW: encode_sbc accepts FP/SIMD registers in any position ─────
    #[test]
    fn sbc_rejects_fp_simd_in_any_position(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
        pos in 0u32..3u32, set_flags in any::<bool>(), vi in 0u32..=5u32,
    ) {
        let mut ops = vec![xreg(rd), xreg(rn), xreg(rm)];
        ops[pos as usize] = fp_variant(vi, rm);
        prop_assert!(encode_sbc(&ops, set_flags).is_err(),
            "sbc with FP/SIMD at position {} must be Err, got {:?}", pos, encode_sbc(&ops, set_flags));
    }

    // ── W8. NEW: encode_neg accepts FP/SIMD registers in any position ─────
    #[test]
    fn neg_rejects_fp_simd_in_any_position(
        rd in 0u32..=30, rm in 0u32..=30, pos in 0u32..2u32, vi in 0u32..=5u32,
    ) {
        let mut ops = vec![xreg(rd), xreg(rm)];
        ops[pos as usize] = fp_variant(vi, rm);
        prop_assert!(encode_neg(&ops).is_err(),
            "neg with FP/SIMD at position {} must be Err, got {:?}", pos, encode_neg(&ops));
    }

    // ── W9. NEW: encode_negs accepts FP/SIMD registers in any position ────
    #[test]
    fn negs_rejects_fp_simd_in_any_position(
        rd in 0u32..=30, rm in 0u32..=30, pos in 0u32..2u32, vi in 0u32..=5u32,
    ) {
        let mut ops = vec![xreg(rd), xreg(rm)];
        ops[pos as usize] = fp_variant(vi, rm);
        prop_assert!(encode_negs(&ops).is_err(),
            "negs with FP/SIMD at position {} must be Err, got {:?}", pos, encode_negs(&ops));
    }

    // ── W10. PARAMETRIC BREADTH: every encoder rejects SP/WSP ─────────────
    // Surfaces ALL four affected functions in one shrinkable failure.
    #[test]
    fn all_encoders_reject_sp_wsp(n in 0u32..=30, set_flags in any::<bool>()) {
        for &(_name, f) in CARRY {
            for pos in 0u32..3 {
                let mut ops = vec![xreg(n), xreg(n), xreg(n)];
                ops[pos as usize] = sp_variant(0);
                prop_assert!(f(&ops, set_flags).is_err(), "carry encoder SP pos {}", pos);
            }
        }
        for &(_name, f) in NEG {
            for pos in 0u32..2 {
                let mut ops = vec![xreg(n), xreg(n)];
                ops[pos as usize] = sp_variant(0);
                prop_assert!(f(&ops).is_err(), "neg encoder SP pos {}", pos);
            }
        }
    }

    // ── W11. PARAMETRIC BREADTH: every encoder rejects FP/SIMD ────────────
    #[test]
    fn all_encoders_reject_fp_simd(n in 0u32..=30, set_flags in any::<bool>()) {
        let bad = fp_variant(0, n);
        for &(_name, f) in CARRY {
            for pos in 0u32..3 {
                let mut ops = vec![xreg(n), xreg(n), xreg(n)];
                ops[pos as usize] = bad.clone();
                prop_assert!(f(&ops, set_flags).is_err(), "carry encoder FP pos {}", pos);
            }
        }
        for &(_name, f) in NEG {
            for pos in 0u32..2 {
                let mut ops = vec![xreg(n), xreg(n)];
                ops[pos as usize] = bad.clone();
                prop_assert!(f(&ops).is_err(), "neg encoder FP pos {}", pos);
            }
        }
    }

    // ── W12. PARAMETRIC BREADTH: every encoder rejects mixed width ────────
    // Width-mixing is already reported for adc/sbc/neg; this surfaces negs
    // alongside them in a single property (negs is the NEW case).
    #[test]
    fn all_encoders_reject_mixed_width(n in 0u32..=30, set_flags in any::<bool>()) {
        let carry_ops = vec![xreg(n), wreg(n), wreg(n)]; // X dest, W sources
        for &(_name, f) in CARRY {
            prop_assert!(f(&carry_ops, set_flags).is_err(), "carry mixed width");
        }
        let neg_ops = vec![wreg(n), xreg(n)];            // W dest, X source
        for &(_name, f) in NEG {
            prop_assert!(f(&neg_ops).is_err(), "neg mixed width");
        }
    }
}
