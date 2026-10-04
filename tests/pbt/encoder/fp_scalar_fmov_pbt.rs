//! Property-based tests for `encode_fmov` — the AArch64 scalar `FMOV`
//! instruction encoder (Floating-point data-processing group).
//!
//! `encode_fmov` covers four syntactic forms:
//!   1. **FP→FP register move** : `FMOV <Sd>,<Sn>` / `FMOV <Dd>,<Dn>`
//!   2. **GP→FP transfer**      : `FMOV <Sd>,<Wn>` / `FMOV <Dd>,<Xn>`
//!   3. **FP→GP transfer**      : `FMOV <Wd>,<Sn>` / `FMOV <Xd>,<Dn>`
//!   4. **FP immediate**        : `FMOV <Dd>,#<imm>` — *not yet implemented*
//!      (the encoder deliberately returns `Err`).
//!
//! # Reference oracle
//!
//! ARMv8-A encoding templates (ARM ARM §C4.1.25 "FMOV"). All variants share
//! bits[4:0]=Rd, bits[9:5]=Rn(source); they differ in the fixed high fields:
//!   FP→FP : `0 00 11110 ftype 1 0000 00 10000 Rn Rd`   (0x1E204000 | ft<<22 | Rn<<5 | Rd)
//!   GP→FP : `sf 00 11110 ftype 1 00 111 000000 Rn Rd`  (rmode==111)
//!   FP→GP : `sf 00 11110 ftype 1 00 110 000000 Rn Rd`  (rmode==110)
//!
//! Worked examples derived from the templates (cross-checked against known
//! assembler output): `fmov s0,s0`=0x1E204000, `fmov d0,d0`=0x1E604000,
//! `fmov d0,x0`=0x9E670000, `fmov s0,w0`=0x1E270000, `fmov x0,d0`=0x9E660000,
//! `fmov w0,s0`=0x1E260000.
//!
//! # Findings surfaced
//!
//! The bit-packing of `encode_fmov` is **correct** for valid operands: the
//! ftype, sf, rmode, opcode and Rd/Rn fields all land at their canonical bit
//! positions with no truncation, and out-of-range register numbers (>=32),
//! immediates and sub-2 arity are all correctly rejected. Properties P1–P5
//! below pin this down.
//!
//! Two **validation bugs** are also exposed (witness properties B1 and B2).
//! They are marked `#[ignore]` so `cargo test` stays green; run them with
//! `cargo test -- --ignored fmov`. The encoder silently accepts operand
//! combinations the architecture treats as UNDEFINED:
//!   * B1 — mixed-precision FP moves (`FMOV Dd,Sn` / `FMOV Sd,Dn`);
//!   * B2 — GP/FP width mismatches (`FMOV Dd,Wn`, `FMOV Sd,Xn`,
//!     `FMOV Xd,Sn`, `FMOV Wd,Dn`).

use lccc::pbt_internals::Operand;
use lccc::pbt_internals::*;
use proptest::prelude::*;

// ── FMOV field extractors (shared encoding layout) ───────────────────────
fn rd_of(w: u32) -> u32 {
    w & 0x1F
}
fn rn_of(w: u32) -> u32 {
    (w >> 5) & 0x1F
} // source register field
fn opcode_of(w: u32) -> u32 {
    (w >> 10) & 0x3F
}
fn rmode_of(w: u32) -> u32 {
    (w >> 16) & 0x7
}
fn ftype_of(w: u32) -> u32 {
    (w >> 22) & 0x3
}
fn sf_of(w: u32) -> u32 {
    (w >> 31) & 1
}

fn word(r: Result<EncodeResult, String>) -> u32 {
    match r {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {:?}", other),
    }
}

proptest! {
    // ── P1. FP→FP register move: reference encoding + exact round-trip ───
    #[test]
    fn prop_fmov_fp_to_fp_reference(src in 0u32..32, dst in 0u32..32, dbl in any::<bool>()) {
        let (p, ftype) = if dbl { ("d", 0b01u32) } else { ("s", 0b00u32) };
        let ops = vec![
            Operand::Reg(format!("{}{}", p, dst)),
            Operand::Reg(format!("{}{}", p, src)),
        ];
        let w = word(encode_fmov(&ops));
        // Golden word from the ARM template.
        let base = (0b00011110u32 << 24) | (ftype << 22) | (0b100000 << 16) | (0b10000 << 10);
        prop_assert_eq!(w, base | (src << 5) | dst);
        // No truncation: both register fields round-trip through their 5-bit slots.
        prop_assert_eq!(rn_of(w), src);
        prop_assert_eq!(rd_of(w), dst);
        prop_assert_eq!(ftype_of(w), ftype);
        prop_assert_eq!(sf_of(w), 0);                  // FP→FP never sets sf
        prop_assert_eq!(opcode_of(w) & 0b11, 0b00);     // bits[11:10] == 00 (FMOV opcode)
    }

    // ── P2. GP→FP transfer: `FMOV <Dd>,<Xn>` / `FMOV <Sd>,<Wn>` ──────────
    // rmode == 111, sf/ftype derived from the FP dest, source sits in Rn.
    #[test]
    fn prop_fmov_gp_to_fp_reference(src in 0u32..32, dst in 0u32..32, dbl in any::<bool>()) {
        let (dst_p, src_p, sf, ftype) = if dbl {
            ("d", "x", 1u32, 0b01u32)
        } else {
            ("s", "w", 0u32, 0b00u32)
        };
        let ops = vec![
            Operand::Reg(format!("{}{}", dst_p, dst)),
            Operand::Reg(format!("{}{}", src_p, src)),
        ];
        let w = word(encode_fmov(&ops));
        prop_assert_eq!(sf_of(w), sf);
        prop_assert_eq!(ftype_of(w), ftype);
        prop_assert_eq!(rmode_of(w), 0b111);   // GP→FP conversion select
        prop_assert_eq!(opcode_of(w), 0);       // bits[15:10] == 000000
        prop_assert_eq!((w >> 21) & 1, 1u32);   // bit 21 fixed to 1
        prop_assert_eq!(rn_of(w), src);
        prop_assert_eq!(rd_of(w), dst);
    }

    // ── P3. FP→GP transfer: `FMOV <Xd>,<Dn>` / `FMOV <Wd>,<Sn>` ──────────
    // rmode == 110, sf/ftype derived from the FP source, dest sits in Rd.
    #[test]
    fn prop_fmov_fp_to_gp_reference(src in 0u32..32, dst in 0u32..32, dbl in any::<bool>()) {
        let (dst_p, src_p, sf, ftype) = if dbl {
            ("x", "d", 1u32, 0b01u32)
        } else {
            ("w", "s", 0u32, 0b00u32)
        };
        let ops = vec![
            Operand::Reg(format!("{}{}", dst_p, dst)),
            Operand::Reg(format!("{}{}", src_p, src)),
        ];
        let w = word(encode_fmov(&ops));
        prop_assert_eq!(sf_of(w), sf);
        prop_assert_eq!(ftype_of(w), ftype);
        prop_assert_eq!(rmode_of(w), 0b110);   // FP→GP conversion select
        prop_assert_eq!(opcode_of(w), 0);
        prop_assert_eq!((w >> 21) & 1, 1u32);
        prop_assert_eq!(rn_of(w), src);   // FP source lands in the Rn field
        prop_assert_eq!(rd_of(w), dst);   // GP dest lands in the Rd field
    }

    // ── P4. Determinism: identical operands always yield the identical word ─
    #[test]
    fn prop_fmov_is_deterministic(src in 0u32..32, dst in 0u32..32, form in 0u32..4u32) {
        let ops = match form {
            0 => vec![Operand::Reg(format!("s{}", dst)), Operand::Reg(format!("s{}", src))],
            1 => vec![Operand::Reg(format!("d{}", dst)), Operand::Reg(format!("d{}", src))],
            2 => vec![Operand::Reg(format!("d{}", dst)), Operand::Reg(format!("x{}", src))],
            _ => vec![Operand::Reg(format!("x{}", dst)), Operand::Reg(format!("d{}", src))],
        };
        let w1 = word(encode_fmov(&ops));
        let w2 = word(encode_fmov(&ops));
        prop_assert_eq!(w1, w2);
    }

    // ── P5. Negative contract (validated, PASSES): bad inputs rejected ───
    // Immediates, arity < 2, and out-of-range register numbers (>= 32) must
    // all return Err — NO silent masking into 5-bit register fields.
    #[test]
    fn prop_fmov_rejects_immediate_arity_and_oor(imm in any::<i64>(), n in 33u32..256u32) {
        // FP immediate form is unimplemented -> Err (documented behaviour).
        let imm_ops = vec![Operand::Reg("s0".into()), Operand::Imm(imm)];
        prop_assert!(encode_fmov(&imm_ops).is_err());

        // Arity: 0 or 1 operands -> Err ("requires 2 operands").
        prop_assert!(encode_fmov(&[]).is_err());
        prop_assert!(encode_fmov(&[Operand::Reg("s0".into())]).is_err());

        // Out-of-range destination / source register numbers must be rejected.
        let oor_dst = vec![Operand::Reg(format!("s{}", n)), Operand::Reg("s0".into())];
        prop_assert!(encode_fmov(&oor_dst).is_err(),
            "register s{} must be rejected, not masked into 5 bits", n);
        let oor_src = vec![Operand::Reg("s0".into()), Operand::Reg(format!("s{}", n))];
        prop_assert!(encode_fmov(&oor_src).is_err(),
            "register s{} must be rejected, not masked into 5 bits", n);
    }

    // ── B1. BUG WITNESS (ignored): FP→FP mixed precision must be rejected ─
    // FMOV only permits homogeneous-precision FP moves: `Dd,Dn` or `Sd,Sn`.
    // Mixed `Dd,Sn` / `Sd,Dn` are UNDEFINED and must be rejected, but
    // encode_fmov silently encodes them (ftype is taken from whichever
    // operand happens to be 'd').
    #[test]
    #[ignore = "bug witness: FMOV accepts mixed-precision FP operands (Dd,Sn)/(Sd,Dn)"]
    fn prop_fmov_rejects_mixed_precision_fp_to_fp(n in 0u32..32) {
        let d_dst_s_src = vec![
            Operand::Reg(format!("d{}", n)),
            Operand::Reg(format!("s{}", n)),
        ];
        let s_dst_d_src = vec![
            Operand::Reg(format!("s{}", n)),
            Operand::Reg(format!("d{}", n)),
        ];
        prop_assert!(encode_fmov(&d_dst_s_src).is_err(),
            "FMOV D{},S{} must be rejected (precision mismatch)", n, n);
        prop_assert!(encode_fmov(&s_dst_d_src).is_err(),
            "FMOV S{},D{} must be rejected (precision mismatch)", n, n);
    }

    // ── B2. BUG WITNESS (ignored): GP↔FP width mismatch must be rejected ──
    // The GP↔FP transfer forms only allow Wn↔Sd and Xn↔Dd. Crossing the width
    // (`Dd,Wn`, `Sd,Xn`, `Xd,Sn`, `Wd,Dn`) is invalid, but encode_fmov derives
    // width from a single operand and silently mis-encodes the other.
    #[test]
    #[ignore = "bug witness: FMOV accepts GP/FP width mismatches (e.g. Dd,Wn)"]
    fn prop_fmov_rejects_gp_fp_width_mismatch(n in 0u32..32) {
        let cases: [(Vec<Operand>, &str); 4] = [
            (vec![Operand::Reg(format!("d{}", n)), Operand::Reg(format!("w{}", n))], "FMOV Dd,Wn"),
            (vec![Operand::Reg(format!("s{}", n)), Operand::Reg(format!("x{}", n))], "FMOV Sd,Xn"),
            (vec![Operand::Reg(format!("x{}", n)), Operand::Reg(format!("s{}", n))], "FMOV Xd,Sn"),
            (vec![Operand::Reg(format!("w{}", n)), Operand::Reg(format!("d{}", n))], "FMOV Wd,Dn"),
        ];
        for (ops, label) in cases {
            prop_assert!(encode_fmov(&ops).is_err(),
                "{} (operands {:?}) must be rejected: GP/FP width mismatch", label, ops);
        }
    }
}
