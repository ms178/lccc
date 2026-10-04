//! Property-based tests for `encode_fcvt_rounding` — the AArch64 scalar
//! float-to-integer rounding-conversion encoder (`FCVT{N,M,P,Z,A}{S,U}`).
//!
//! # Reference oracle
//!
//! ARMv8-A encoding (ARM ARM §C5.6 "Floating-point<->integer conversions"):
//!   `sf 00 11110 ftype 1 rmode opcode 000000 Rn Rd`
//!     [31]=sf (0=W dest, 1=X dest), [30:29]=00, [28:24]=11110,
//!     [23:22]=ftype (00=S source, 01=D source), [21]=1 (fixed),
//!     [20:19]=rmode (2 bits), [18:16]=opcode (3 bits), [15:10]=000000,
//!     [9:5]=Rn (FP source), [4:0]=Rd (GP dest).
//!
//! Worked examples derived from the template and cross-checked against known
//! assembler output:
//!   `fcvtzs w0,s0` = 0x1E380000   `fcvtzs x0,s0` = 0x9E380000
//!   `fcvtzs w0,d0` = 0x1E780000   `fcvtzs x0,d0` = 0x9E780000
//!   `fcvtns w0,s0` = 0x1E200000   `fcvtns x0,s0` = 0x9E200000
//!   `fcvtas w0,s0` = 0x1E240000
//!
//! # Findings surfaced
//!
//! The bit-packing is **correct** for valid operands: every field lands at its
//! canonical bit position with no truncation, and arity / immediate-source /
//! out-of-range register inputs are correctly rejected (properties P1–P3).
//!
//! Three **validation bugs** are also exposed as witness properties B1–B3.
//! They are marked `#[ignore]` so `cargo test` stays green; run them with
//! `cargo test -- --ignored fcvt_rounding`. The encoder silently accepts
//! inputs the architecture treats as UNDEFINED / unallocatable:
//!   * B1 — `rmode` is a 2-bit field but any `u32` is OR'd in with no range
//!     check (values >= 4 corrupt bit 21 / ftype);
//!   * B2 — `opcode` is a 3-bit field but any `u32` is OR'd in with no range
//!     check (values >= 8 corrupt the rmode field);
//!   * B3 — float-to-int FCVT* requires a GP destination (W/X) and an FP
//!     source (S/D); neither bank is validated.

use lccc::pbt_internals::Operand;
use lccc::pbt_internals::*;
use proptest::prelude::*;

// ── field extractors for the FCVT rounding layout ────────────────────────
fn rd_of(w: u32) -> u32 {
    w & 0x1F
}
fn rn_of(w: u32) -> u32 {
    (w >> 5) & 0x1F
}
fn fixed_lo_of(w: u32) -> u32 {
    (w >> 10) & 0x3F
}
fn opcode_of(w: u32) -> u32 {
    (w >> 16) & 0x7
}
fn rmode_of(w: u32) -> u32 {
    (w >> 19) & 0x3
}
fn bit21_of(w: u32) -> u32 {
    (w >> 21) & 1
}
fn ftype_of(w: u32) -> u32 {
    (w >> 22) & 0x3
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
fn concrete_fcvt_encodings_match_reference() {
    // FCVTZS (rmode=11, opcode=000) — all four width/precision combos.
    assert_eq!(
        expect_word(encode_fcvt_rounding(
            &[Operand::Reg("w0".into()), Operand::Reg("s0".into())],
            0b11,
            0b000
        )),
        0x1E380000,
        "FCVTZS W0,S0"
    );
    assert_eq!(
        expect_word(encode_fcvt_rounding(
            &[Operand::Reg("x0".into()), Operand::Reg("s0".into())],
            0b11,
            0b000
        )),
        0x9E380000,
        "FCVTZS X0,S0"
    );
    assert_eq!(
        expect_word(encode_fcvt_rounding(
            &[Operand::Reg("w0".into()), Operand::Reg("d0".into())],
            0b11,
            0b000
        )),
        0x1E780000,
        "FCVTZS W0,D0"
    );
    assert_eq!(
        expect_word(encode_fcvt_rounding(
            &[Operand::Reg("x0".into()), Operand::Reg("d0".into())],
            0b11,
            0b000
        )),
        0x9E780000,
        "FCVTZS X0,D0"
    );
    // FCVTNS (rmode=00, opcode=000).
    assert_eq!(
        expect_word(encode_fcvt_rounding(
            &[Operand::Reg("w0".into()), Operand::Reg("s0".into())],
            0b00,
            0b000
        )),
        0x1E200000,
        "FCVTNS W0,S0"
    );
    assert_eq!(
        expect_word(encode_fcvt_rounding(
            &[Operand::Reg("x0".into()), Operand::Reg("s0".into())],
            0b00,
            0b000
        )),
        0x9E200000,
        "FCVTNS X0,S0"
    );
    // FCVTAS (rmode=00, opcode=100).
    assert_eq!(
        expect_word(encode_fcvt_rounding(
            &[Operand::Reg("w0".into()), Operand::Reg("s0".into())],
            0b00,
            0b100
        )),
        0x1E240000,
        "FCVTAS W0,S0"
    );
}

proptest! {
    /// P1 — Reference / field-layout oracle. For a valid GP destination, a valid
    /// FP source, and in-range `rmode` (2-bit) / `opcode` (3-bit), every field
    /// lands at its canonical ARMv8 bit position with no truncation, and the
    /// whole word equals the reference template.
    #[test]
    fn prop_fcvt_rounding_places_fields(
        rd_num in 0u32..32,
        rn_num in 0u32..32,
        dest_x in any::<bool>(),
        src_d in any::<bool>(),
        rmode in 0u32..4,
        opcode in 0u32..8,
    ) {
        let rd_name = if dest_x { format!("x{}", rd_num) } else { format!("w{}", rd_num) };
        let rn_name = if src_d { format!("d{}", rn_num) } else { format!("s{}", rn_num) };
        let ops = vec![Operand::Reg(rd_name), Operand::Reg(rn_name)];

        let sf: u32 = if dest_x { 1 } else { 0 };
        let ftype: u32 = if src_d { 0b01 } else { 0b00 };
        let w = expect_word(encode_fcvt_rounding(&ops, rmode, opcode));

        // Reference template.
        let expected = (sf << 31) | (0b11110 << 24) | (ftype << 22)
            | (1 << 21) | (rmode << 19) | (opcode << 16) | (rn_num << 5) | rd_num;
        prop_assert_eq!(w, expected);

        // Field-by-field extraction.
        prop_assert_eq!(rd_of(w), rd_num);
        prop_assert_eq!(rn_of(w), rn_num);
        prop_assert_eq!(fixed_lo_of(w), 0b000000); // [15:10] == 0
        prop_assert_eq!(opcode_of(w), opcode); // [18:16]
        prop_assert_eq!(rmode_of(w), rmode); // [20:19]
        prop_assert_eq!(bit21_of(w), 1); // fixed 1
        prop_assert_eq!(ftype_of(w), ftype); // [23:22]
        prop_assert_eq!(sf_of(w), sf); // [31]
        prop_assert_eq!((w >> 24) & 0x1F, 0b11110); // [28:24] == 11110
        prop_assert_eq!((w >> 29) & 0x3, 0); // [30:29] == 00
    }

    /// P2 — sf derived solely from destination width (X→1, W→0); ftype derived
    /// solely from the source prefix (D→01, S→00).
    #[test]
    fn prop_fcvt_rounding_sf_ftype_derivation(
        rd_num in 0u32..32,
        rn_num in 0u32..32,
        dest_x in any::<bool>(),
        src_d in any::<bool>(),
    ) {
        let rd_name = if dest_x { format!("x{}", rd_num) } else { format!("w{}", rd_num) };
        let rn_name = if src_d { format!("d{}", rn_num) } else { format!("s{}", rn_num) };
        let ops = vec![Operand::Reg(rd_name), Operand::Reg(rn_name)];
        let w = expect_word(encode_fcvt_rounding(&ops, 0b00, 0b000));
        prop_assert_eq!(sf_of(w), if dest_x { 1 } else { 0 });
        prop_assert_eq!(ftype_of(w), if src_d { 0b01 } else { 0b00 });
    }

    /// P3 — Determinism: identical operands + rmode + opcode ⇒ identical word.
    #[test]
    fn prop_fcvt_rounding_is_deterministic(
        rd_num in 0u32..32,
        rn_num in 0u32..32,
        dest_x in any::<bool>(),
        src_d in any::<bool>(),
        rmode in 0u32..4,
        opcode in 0u32..8,
    ) {
        let rd_name = if dest_x { format!("x{}", rd_num) } else { format!("w{}", rd_num) };
        let rn_name = if src_d { format!("d{}", rn_num) } else { format!("s{}", rn_num) };
        let ops = vec![Operand::Reg(rd_name), Operand::Reg(rn_name)];
        let w1 = expect_word(encode_fcvt_rounding(&ops, rmode, opcode));
        let w2 = expect_word(encode_fcvt_rounding(&ops, rmode, opcode));
        prop_assert_eq!(w1, w2);
    }

    /// P4 — Negative contract (validated, PASSES): sub-2 arity, an immediate
    /// source operand, and out-of-range register numbers (>= 32) MUST all be
    /// rejected — never silently masked into a 5-bit field.
    #[test]
    fn prop_fcvt_rounding_rejects_arity_imm_and_oob_reg(
        n in 32u32..256u32,
        imm in any::<i64>(),
        rmode in 0u32..4,
        opcode in 0u32..8,
    ) {
        // Arity: 0 or 1 operands ⇒ Err ("requires 2 operands").
        prop_assert!(encode_fcvt_rounding(&[], rmode, opcode).is_err());
        prop_assert!(encode_fcvt_rounding(&[Operand::Reg("w0".into())], rmode, opcode).is_err());

        // Immediate source: source must be a register.
        let imm_src = vec![Operand::Reg("w0".into()), Operand::Imm(imm)];
        prop_assert!(encode_fcvt_rounding(&imm_src, rmode, opcode).is_err());

        // Out-of-range destination register (>= 32).
        let oob_dest = vec![Operand::Reg(format!("w{}", n)), Operand::Reg("s0".into())];
        prop_assert!(
            encode_fcvt_rounding(&oob_dest, rmode, opcode).is_err(),
            "dest w{} must be rejected (5-bit field), not masked", n
        );
        // Out-of-range source register (>= 32).
        let oob_src = vec![Operand::Reg("w0".into()), Operand::Reg(format!("s{}", n))];
        prop_assert!(
            encode_fcvt_rounding(&oob_src, rmode, opcode).is_err(),
            "source s{} must be rejected (5-bit field), not masked", n
        );
    }

    // ── Bug witnesses (FAIL by design; #[ignore] keeps `cargo test` green) ──

    /// B1 — FINDING (FAILS): `rmode` is a 2-bit field at [20:19] but the encoder
    /// ORs any `u32` in with no range check. Values >= 4 leak past bit 19 into
    /// the fixed bit 21 (and then ftype) and silently corrupt the word instead
    /// of being rejected. Run with `cargo test -- --ignored fcvt_rounding`.
    #[test]
    #[ignore]
    fn prop_fcvt_rounding_rejects_oversized_rmode(bad in 4u32..1024u32) {
        let ops = vec![Operand::Reg("w0".into()), Operand::Reg("s0".into())];
        prop_assert!(
            encode_fcvt_rounding(&ops, bad, 0b000).is_err(),
            "rmode={} must be rejected (2-bit field [20:19]); it is OR'd in and corrupts bit21/ftype", bad
        );
    }

    /// B2 — FINDING (FAILS): `opcode` is a 3-bit field at [18:16] but the
    /// encoder ORs any `u32` in with no range check. Values >= 8 leak into the
    /// rmode field (and beyond) and silently corrupt the word.
    #[test]
    #[ignore]
    fn prop_fcvt_rounding_rejects_oversized_opcode(bad in 8u32..1024u32) {
        let ops = vec![Operand::Reg("w0".into()), Operand::Reg("s0".into())];
        prop_assert!(
            encode_fcvt_rounding(&ops, 0b00, bad).is_err(),
            "opcode={} must be rejected (3-bit field [18:16]); it is OR'd in and corrupts the rmode field", bad
        );
    }

    /// B3 — FINDING (FAILS): float-to-int FCVT* requires a GP destination
    /// (W/X) and an FP source (S/D). The encoder derives `sf` from the dest
    /// width and `ftype` from the source prefix but validates NEITHER bank, so
    /// it silently accepts illegal operands such as an FP destination or a GP
    /// source.
    #[test]
    #[ignore]
    fn prop_fcvt_rounding_rejects_wrong_bank_operands(n in 0u32..32) {
        // FP destination (must be a GP W/X register).
        let fp_dest = vec![Operand::Reg(format!("d{}", n)), Operand::Reg("s0".into())];
        prop_assert!(
            encode_fcvt_rounding(&fp_dest, 0b11, 0b000).is_err(),
            "FP destination d{} is invalid for float-to-int FCVT*; got {:?}",
            n,
            encode_fcvt_rounding(&fp_dest, 0b11, 0b000)
        );
        // GP source (must be an FP S/D register).
        let gp_src = vec![Operand::Reg("w0".into()), Operand::Reg(format!("x{}", n))];
        prop_assert!(
            encode_fcvt_rounding(&gp_src, 0b11, 0b000).is_err(),
            "GP source x{} is invalid for float-to-int FCVT*; got {:?}",
            n,
            encode_fcvt_rounding(&gp_src, 0b11, 0b000)
        );
    }
}
