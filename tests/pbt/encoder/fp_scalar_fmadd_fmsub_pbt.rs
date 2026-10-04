//! Property-based tests for `encode_fmadd_fmsub` — the AArch64 scalar
//! `FMADD` / `FMSUB` fused multiply-accumulate encoder
//! (`Rd = Ra +/- (Rn * Rm)`), Floating-point data-processing (3 source).
//!
//! # Reference oracle
//!
//! ARMv8-A encoding (ARM ARM §C4.1.18, "Floating-point data-processing
//! (3 source)"):
//!   `0 00 11111 ftype 0 Rm o1 Ra Rn Rd`
//!     [31:24]=00011111 (0x1F), [23:22]=ftype (00=S, 01=D),
//!     [21]=0 (the `1` marks FNMADD/FNMSUB), [20:16]=Rm,
//!     [15]=o1 (0=FMADD, 1=FMSUB), [14:10]=Ra, [9:5]=Rn, [4:0]=Rd.
//!
//! Worked examples derived from the template and cross-checked against known
//! assembler output (Rn=Rd=Rm=Ra=0):
//!   `fmadd s0,s0,s0,s0` = 0x1F000000   `fmadd d0,d0,d0,d0` = 0x1F400000
//!   `fmsub s0,s0,s0,s0` = 0x1F008000   `fmsub d0,d0,d0,d0` = 0x1F408000
//!
//! # Findings surfaced
//!
//! The bit-packing is **correct** for valid operands: every field lands at its
//! canonical bit position with no truncation, bit 21 stays 0, and FMADD vs
//! FMSUB differ *only* in bit 15 (o1). Arity (<4 operands), non-register
//! operands, and out-of-range register numbers (>=32) are all correctly
//! rejected (properties P1–P4).
//!
//! One **validation bug** is exposed as witness property B1. It is marked
//! `#[ignore]` so `cargo test` stays green; run it with
//! `cargo test fp_scalar_fmadd_fmsub -- --ignored`:
//!   * B1 — FMADD/FMSUB operate only on FP registers and require all four
//!     operands to share the same precision (all-S or all-D). The encoder
//!     derives `ftype` solely from `operands[0]` and never validates operand
//!     banks or precision homogeneity, so it silently produces illegal
//!     encodings for mixed-precision FP operands and for GP-bank (W/X)
//!     operands.

use lccc::pbt_internals::Operand;
use lccc::pbt_internals::*;
use proptest::prelude::*;

// ── Field extractors for the FMADD/FMSUB layout ──────────────────────────
fn rd_of(w: u32) -> u32 {
    w & 0x1F
}
fn rn_of(w: u32) -> u32 {
    (w >> 5) & 0x1F
}
fn ra_of(w: u32) -> u32 {
    (w >> 10) & 0x1F
}
fn o1_of(w: u32) -> u32 {
    (w >> 15) & 1
}
fn rm_of(w: u32) -> u32 {
    (w >> 16) & 0x1F
}
fn bit21_of(w: u32) -> u32 {
    (w >> 21) & 1
}
fn ftype_of(w: u32) -> u32 {
    (w >> 22) & 0x3
}
fn top_of(w: u32) -> u32 {
    w >> 24
}
fn sf_of(w: u32) -> u32 {
    (w >> 31) & 1
}

fn expect_word(r: Result<EncodeResult, String>) -> u32 {
    match r {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected Word, got {:?}", other),
    }
}

// ── Concrete cross-checks against known assembler output ─────────────────
#[test]
fn concrete_fmadd_fmsub_match_reference() {
    // FMADD (o1=0): single then double.
    assert_eq!(
        expect_word(encode_fmadd_fmsub(
            &[
                Operand::Reg("s0".into()),
                Operand::Reg("s0".into()),
                Operand::Reg("s0".into()),
                Operand::Reg("s0".into()),
            ],
            false
        )),
        0x1F000000,
        "FMADD S0,S0,S0,S0"
    );
    assert_eq!(
        expect_word(encode_fmadd_fmsub(
            &[
                Operand::Reg("d0".into()),
                Operand::Reg("d0".into()),
                Operand::Reg("d0".into()),
                Operand::Reg("d0".into()),
            ],
            false
        )),
        0x1F400000,
        "FMADD D0,D0,D0,D0"
    );
    // FMSUB (o1=1): single then double — only bit 15 differs from FMADD.
    assert_eq!(
        expect_word(encode_fmadd_fmsub(
            &[
                Operand::Reg("s0".into()),
                Operand::Reg("s0".into()),
                Operand::Reg("s0".into()),
                Operand::Reg("s0".into()),
            ],
            true
        )),
        0x1F008000,
        "FMSUB S0,S0,S0,S0"
    );
    assert_eq!(
        expect_word(encode_fmadd_fmsub(
            &[
                Operand::Reg("d0".into()),
                Operand::Reg("d0".into()),
                Operand::Reg("d0".into()),
                Operand::Reg("d0".into()),
            ],
            true
        )),
        0x1F408000,
        "FMSUB D0,D0,D0,D0"
    );
}

proptest! {
    // P1 — Oracle: reference / field layout. Homogeneous-precision FP
    // operands (all S or all D) with in-range register numbers + is_sub =>
    // every field lands at its canonical ARMv8 bit position with no
    // truncation, bit 21 stays 0, and the word equals an independent
    // reconstruction of the template.
    #[test]
    fn prop_fmadd_fmsub_field_layout(
        rd in 0u32..32, rn in 0u32..32, rm in 0u32..32, ra in 0u32..32,
        dbl in any::<bool>(), is_sub in any::<bool>(),
    ) {
        let p = if dbl { "d" } else { "s" };
        let ops = vec![
            Operand::Reg(format!("{}{}", p, rd)),
            Operand::Reg(format!("{}{}", p, rn)),
            Operand::Reg(format!("{}{}", p, rm)),
            Operand::Reg(format!("{}{}", p, ra)),
        ];
        let w = expect_word(encode_fmadd_fmsub(&ops, is_sub));
        let ftype = if dbl { 0b01u32 } else { 0b00u32 };
        let o1 = if is_sub { 1u32 } else { 0u32 };

        // Fixed high fields of the scalar FP 3-source encoding.
        prop_assert_eq!(top_of(w), 0x1Fu32);      // [31:24] = 00011111
        prop_assert_eq!(bit21_of(w), 0u32);        // [21] = 0 (not FNMADD/FNMSUB)
        prop_assert_eq!(sf_of(w), 0u32);           // scalar FP, sf always 0

        // ftype, o1, and all four register fields round-trip exactly.
        prop_assert_eq!(ftype_of(w), ftype);
        prop_assert_eq!(o1_of(w), o1);
        prop_assert_eq!(rd_of(w), rd);             // [4:0]
        prop_assert_eq!(rn_of(w), rn);             // [9:5]
        prop_assert_eq!(rm_of(w), rm);             // [20:16]
        prop_assert_eq!(ra_of(w), ra);             // [14:10]

        // Independent reference reconstruction from the template.
        let expected = (0b00011111u32 << 24) | (ftype << 22) | (rm << 16)
            | (o1 << 15) | (ra << 10) | (rn << 5) | rd;
        prop_assert_eq!(w, expected);
    }

    // P2 — Oracle: ground-truth reference. Pins each (precision, is_sub)
    // corner for the Rn=Rd=Rm=Ra=0 case to an externally verified 32-bit word.
    // Unlike the self-derived reconstruction in P1 (which re-implements the
    // encoder's own bit math), this guards against a bug shared between
    // encoder and test.
    #[test]
    fn prop_fmadd_fmsub_ground_truth(dbl in any::<bool>(), is_sub in any::<bool>()) {
        let p = if dbl { "d" } else { "s" };
        let ops = vec![
            Operand::Reg(format!("{}0", p)),
            Operand::Reg(format!("{}0", p)),
            Operand::Reg(format!("{}0", p)),
            Operand::Reg(format!("{}0", p)),
        ];
        let w = expect_word(encode_fmadd_fmsub(&ops, is_sub));
        let expected = match (dbl, is_sub) {
            (false, false) => 0x1F000000u32, // FMADD S0,S0,S0,S0
            (true,  false) => 0x1F400000u32, // FMADD D0,D0,D0,D0
            (false, true)  => 0x1F008000u32, // FMSUB S0,S0,S0,S0
            (true,  true)  => 0x1F408000u32, // FMSUB D0,D0,D0,D0
        };
        prop_assert_eq!(w, expected);
    }

    // P3 — Oracle: o1 derivation. is_sub selects bit 15; FMADD vs FMSUB on
    // identical operands differ ONLY in bit 15 — no other field is perturbed.
    #[test]
    fn prop_fmadd_fmsub_add_vs_sub_differ_only_in_bit15(
        rd in 0u32..32, rn in 0u32..32, rm in 0u32..32, ra in 0u32..32,
        dbl in any::<bool>(),
    ) {
        let p = if dbl { "d" } else { "s" };
        let ops = vec![
            Operand::Reg(format!("{}{}", p, rd)),
            Operand::Reg(format!("{}{}", p, rn)),
            Operand::Reg(format!("{}{}", p, rm)),
            Operand::Reg(format!("{}{}", p, ra)),
        ];
        let w_add = expect_word(encode_fmadd_fmsub(&ops, false));
        let w_sub = expect_word(encode_fmadd_fmsub(&ops, true));
        prop_assert_eq!(o1_of(w_add), 0u32); // FMADD
        prop_assert_eq!(o1_of(w_sub), 1u32); // FMSUB
        prop_assert_eq!(w_add ^ w_sub, 1u32 << 15);
    }

    // P4 — Negative contract (validated, PASSES): out-of-range register
    // numbers (>= 32) MUST be rejected by get_reg (parse_reg_num caps at 31),
    // not masked into 5 bits; too-few operands (< 4) and non-register operands
    // must be rejected.
    #[test]
    fn prop_fmadd_fmsub_rejects_bad_regs_arity_immediates(
        n in 32u32..256u32, pos in 0u32..4u32, imm in any::<i64>(),
    ) {
        // Out-of-range register in any of the four operand positions.
        let mut names = vec![
            "d0".to_string(), "d0".to_string(),
            "d0".to_string(), "d0".to_string(),
        ];
        names[pos as usize] = format!("d{}", n);
        let ops: Vec<Operand> = names.into_iter().map(Operand::Reg).collect();
        prop_assert!(
            encode_fmadd_fmsub(&ops, false).is_err(),
            "register d{} must be rejected (5-bit field), not silently masked", n
        );
        // Too few operands (< 4).
        prop_assert!(encode_fmadd_fmsub(&[], false).is_err());
        prop_assert!(
            encode_fmadd_fmsub(&[Operand::Reg("d0".into())], false).is_err()
        );
        prop_assert!(encode_fmadd_fmsub(&[
            Operand::Reg("d0".into()), Operand::Reg("d0".into()),
            Operand::Reg("d0".into()),
        ], false).is_err());
        // Non-register operand in any slot.
        let bad = vec![
            Operand::Reg("d0".into()), Operand::Reg("d0".into()),
            Operand::Reg("d0".into()), Operand::Imm(imm),
        ];
        prop_assert!(encode_fmadd_fmsub(&bad, false).is_err());
    }

    // B1 — Negative contract (FINDING — #[ignore] witness). FMADD/FMSUB
    // operate ONLY on floating-point registers, and all four operands MUST
    // share one precision (all-S or all-D). But encode_fmadd_fmsub derives
    // ftype solely from operands[0] and never validates operand banks or
    // precision homogeneity, so it silently produces illegal encodings for
    // mixed-precision FP operands and for GP-bank (W/X) operands. Run with
    // `cargo test fp_scalar_fmadd_fmsub -- --ignored`.
    #[test]
    #[ignore = "witness: encoder accepts illegal mixed-precision / GP-bank FMADD operands"]
    fn prop_fmadd_fmsub_rejects_mixed_precision_and_gp_bank(n in 0u32..32) {
        // Mixed precision: dest double, sources single.
        let mix = vec![
            Operand::Reg(format!("d{}", n)),
            Operand::Reg(format!("s{}", n)),
            Operand::Reg(format!("s{}", n)),
            Operand::Reg(format!("s{}", n)),
        ];
        prop_assert!(
            encode_fmadd_fmsub(&mix, false).is_err(),
            "mixed precision (Dd, Sn, Sm, Sa) must be rejected; got {:?}",
            encode_fmadd_fmsub(&mix, false)
        );
        // GP-bank operands are not valid FP 3-source operands.
        let gp = vec![
            Operand::Reg(format!("x{}", n)),
            Operand::Reg(format!("x{}", n)),
            Operand::Reg(format!("x{}", n)),
            Operand::Reg(format!("x{}", n)),
        ];
        prop_assert!(
            encode_fmadd_fmsub(&gp, false).is_err(),
            "GP registers (x{}) are not valid FMADD/FMSUB operands; got {:?}",
            n, encode_fmadd_fmsub(&gp, false)
        );
    }
}
