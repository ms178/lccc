//! Property-based tests for `encode_int_to_float` — the AArch64 scalar
//! integer-to-floating-point conversion encoder backing `SCVTF` / `UCVTF`.
//!
//! # Reference oracle
//!
//! ARMv8-A encoding (ARM ARM §C5.6 "Floating-point<->integer conversions"):
//!   `sf 00 11110 ftype 1 00 opcode 000000 Rn Rd`
//!     [31]=sf (0=W source, 1=X source), [30:29]=00, [28:24]=11110,
//!     [23:22]=ftype (00=S dest, 01=D dest), [21]=1 (fixed),
//!     [20:19]=00 (fixed), [18:16]=opcode (3 bits: 010=SCVTF, 011=UCVTF),
//!     [15:10]=000000 (fixed), [9:5]=Rn (GP source), [4:0]=Rd (FP dest).
//!
//! Worked examples derived from the template and cross-checked against known
//! assembler output:
//!   `scvtf s0, w0` = 0x1E220000   `scvtf s0, x0` = 0x9E220000
//!   `scvtf d0, w0` = 0x1E620000   `scvtf d0, x0` = 0x9E620000
//!   `ucvtf s0, w0` = 0x1E230000   `ucvtf d0, x0` = 0x9E630000
//!
//! # Findings surfaced
//!
//! The bit-packing is **correct** for valid operands: every field lands at its
//! canonical bit position with no truncation, `sf` tracks the GP source width,
//! `ftype` tracks the FP dest width, and the SCVTF/UCVTF opcode bit is set
//! correctly (P1–P4). Arity, immediate-source and out-of-range register inputs
//! are correctly rejected (P5).
//!
//! One **validation bug** is exposed as witness property B1, marked `#[ignore]`
//! so `cargo test` stays green (run with `cargo test -- --ignored int_to_float`).
//! `SCVTF`/`UCVTF` architecturally require an FP destination (S/D) and a GP
//! integer source (W/X), but `encode_int_to_float` validates neither operand's
//! bank — it parses the register number via `get_reg`/`parse_reg_num` (which
//! accept any bank prefix) and derives `ftype`/`sf` purely from name prefixes.
//! So a GP destination or an FP source is silently accepted and mis-encoded.

use lccc::pbt_internals::Operand;
use lccc::pbt_internals::*;
use proptest::prelude::*;

// ── field extractors for the SCVTF/UCVTF layout ──────────────────────────
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
fn bit21_of(w: u32) -> u32 {
    (w >> 21) & 1
}
fn ftype_of(w: u32) -> u32 {
    (w >> 22) & 0x3
}
fn top_of(w: u32) -> u32 {
    (w >> 24) & 0x1F
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

/// Reference word for SCVTF/UCVTF per the ARMv8 template above.
fn reference_word(is_signed: bool, sf: u32, ftype: u32, rn: u32, rd: u32) -> u32 {
    let opcode: u32 = if is_signed { 0b010 } else { 0b011 };
    (0b11110u32 << 24) | (ftype << 22) | (1 << 21) | (opcode << 16) | (rn << 5) | rd | (sf << 31)
}

// ── Concrete cross-checks against known assembler output ─────────────────
#[test]
fn concrete_int_to_float_encodings_match_reference() {
    // SCVTF — all four dest-precision / source-width combinations.
    assert_eq!(
        expect_word(encode_int_to_float(
            &[Operand::Reg("s0".into()), Operand::Reg("w0".into())],
            true
        )),
        0x1E220000,
        "SCVTF S0,W0"
    );
    assert_eq!(
        expect_word(encode_int_to_float(
            &[Operand::Reg("s0".into()), Operand::Reg("x0".into())],
            true
        )),
        0x9E220000,
        "SCVTF S0,X0"
    );
    assert_eq!(
        expect_word(encode_int_to_float(
            &[Operand::Reg("d0".into()), Operand::Reg("w0".into())],
            true
        )),
        0x1E620000,
        "SCVTF D0,W0"
    );
    assert_eq!(
        expect_word(encode_int_to_float(
            &[Operand::Reg("d0".into()), Operand::Reg("x0".into())],
            true
        )),
        0x9E620000,
        "SCVTF D0,X0"
    );
    // UCVTF — opcode bit 16 distinguishes it from SCVTF.
    assert_eq!(
        expect_word(encode_int_to_float(
            &[Operand::Reg("s0".into()), Operand::Reg("w0".into())],
            false
        )),
        0x1E230000,
        "UCVTF S0,W0"
    );
    assert_eq!(
        expect_word(encode_int_to_float(
            &[Operand::Reg("d0".into()), Operand::Reg("x0".into())],
            false
        )),
        0x9E630000,
        "UCVTF D0,X0"
    );
}

proptest! {
    // ── P1: reference / field layout ───────────────────────────────────
    // Valid operands (FP dest ∈ {S,D}, GP source ∈ {W,X}, reg nums 0..32) +
    // signedness => the word matches the hand-derived reference, and every
    // field round-trips exactly into its 5-bit slot with NO truncation, while
    // all fixed fields hold their canonical values.
    #[test]
    fn prop_int_to_float_places_fields(
        rd in 0u32..32, rn in 0u32..32,
        dest_double in any::<bool>(), src_64 in any::<bool>(),
        is_signed in any::<bool>(),
    ) {
        let (dst_reg, ftype) = if dest_double { ("d".to_string(), 0b01u32) } else { ("s".to_string(), 0b00u32) };
        let (src_reg, sf)     = if src_64     { ("x".to_string(), 1u32)    } else { ("w".to_string(), 0u32)    };
        let ops = vec![
            Operand::Reg(format!("{}{}", dst_reg, rd)),
            Operand::Reg(format!("{}{}", src_reg, rn)),
        ];
        let w = expect_word(encode_int_to_float(&ops, is_signed));

        // Full-word reference oracle.
        prop_assert_eq!(w, reference_word(is_signed, sf, ftype, rn, rd));

        // Fixed fields of the scalar int<->float conversion encoding.
        prop_assert_eq!(top_of(w), 0b11110u32);      // [28:24] = 11110 (0x1E)
        prop_assert_eq!(bit21_of(w), 1u32);           // bit 21  = 1
        prop_assert_eq!(fixed_lo_of(w), 0u32);        // [15:10] = 000000
        prop_assert_eq!(opcode_of(w) >> 2, 0u32);     // opcode[2] (bit 18) = 0

        // Register fields round-trip exactly into their 5-bit slots.
        prop_assert_eq!(rd_of(w), rd);
        prop_assert_eq!(rn_of(w), rn);
    }

    // ── P2: precision fields derived independently ─────────────────────
    // `ftype` comes solely from the dest prefix (d=>01, s=>00); `sf` comes
    // solely from the source width (x=>1, w=>0). The two are independent, so
    // any dest/source width combination is legal and the encoder must set
    // each field from its own operand only.
    #[test]
    fn prop_int_to_float_precision_fields_independent(
        rd in 0u32..32, rn in 0u32..32,
        dest_double in any::<bool>(), src_64 in any::<bool>(),
    ) {
        let (dst_reg, want_ftype) = if dest_double { ("d".to_string(), 0b01u32) } else { ("s".to_string(), 0b00u32) };
        let (src_reg, want_sf)    = if src_64     { ("x".to_string(), 1u32)    } else { ("w".to_string(), 0u32)    };
        let ops = vec![
            Operand::Reg(format!("{}{}", dst_reg, rd)),
            Operand::Reg(format!("{}{}", src_reg, rn)),
        ];
        let w = expect_word(encode_int_to_float(&ops, true));
        prop_assert_eq!(ftype_of(w), want_ftype);
        prop_assert_eq!(sf_of(w), want_sf);
    }

    // ── P3: opcode from is_signed (SCVTF vs UCVTF) ─────────────────────
    // The 3-bit opcode field [18:16] is 010 for SCVTF (signed) and 011 for
    // UCVTF (unsigned); the ONLY word-level difference between the two, for
    // identical operands, is bit 16 (0x00010000).
    #[test]
    fn prop_int_to_float_opcode_selects_signedness(
        rd in 0u32..32, rn in 0u32..32,
        dest_double in any::<bool>(), src_64 in any::<bool>(),
    ) {
        let (dst_reg, _) = if dest_double { ("d".to_string(), ()) } else { ("s".to_string(), ()) };
        let (src_reg, _) = if src_64     { ("x".to_string(), ()) } else { ("w".to_string(), ()) };
        let ops = vec![
            Operand::Reg(format!("{}{}", dst_reg, rd)),
            Operand::Reg(format!("{}{}", src_reg, rn)),
        ];
        let ws = expect_word(encode_int_to_float(&ops, true));  // SCVTF
        let wu = expect_word(encode_int_to_float(&ops, false)); // UCVTF

        prop_assert_eq!(opcode_of(ws), 0b010u32);
        prop_assert_eq!(opcode_of(wu), 0b011u32);
        prop_assert_eq!(ws ^ wu, 0x0001_0000u32); // differ only in bit 16
    }

    // ── P4: determinism ────────────────────────────────────────────────
    // Same operands + signedness => identical word.
    #[test]
    fn prop_int_to_float_is_deterministic(
        rd in 0u32..32, rn in 0u32..32,
        dest_double in any::<bool>(), src_64 in any::<bool>(),
        is_signed in any::<bool>(),
    ) {
        let dst_reg = if dest_double { "d" } else { "s" };
        let src_reg = if src_64 { "x" } else { "w" };
        let ops = vec![
            Operand::Reg(format!("{}{}", dst_reg, rd)),
            Operand::Reg(format!("{}{}", src_reg, rn)),
        ];
        let w1 = expect_word(encode_int_to_float(&ops, is_signed));
        let w2 = expect_word(encode_int_to_float(&ops, is_signed));
        prop_assert_eq!(w1, w2);
    }

    // ── P5: negative contract (validated, PASSES) ─────────────────────
    // Arity < 2, an immediate source, and out-of-range register numbers must
    // all be rejected — never silently masked into a 5-bit field.
    #[test]
    fn prop_int_to_float_rejects_arity_immediate_and_out_of_range(
        n in 32u32..4096u32, imm in any::<i64>(),
    ) {
        // Too few operands.
        prop_assert!(encode_int_to_float(&[], true).is_err());
        prop_assert!(
            encode_int_to_float(&[Operand::Reg("d0".into())], true).is_err()
        );

        // Immediate as the GP source.
        let imm_ops = vec![Operand::Reg("d0".into()), Operand::Imm(imm)];
        prop_assert!(encode_int_to_float(&imm_ops, true).is_err());

        // Out-of-range dest FP register.
        let bad_dst = vec![Operand::Reg(format!("d{}", n)), Operand::Reg("x0".into())];
        prop_assert!(
            encode_int_to_float(&bad_dst, true).is_err(),
            "dest d{} must be rejected (5-bit field), not silently masked", n
        );
        // Out-of-range source GP register.
        let bad_src = vec![Operand::Reg("d0".into()), Operand::Reg(format!("x{}", n))];
        prop_assert!(
            encode_int_to_float(&bad_src, true).is_err(),
            "source x{} must be rejected (5-bit field), not silently masked", n
        );
    }

    // ── B1: FINDING (FAILS by design — #[ignore]) ─────────────────────
    // SCVTF/UCVTF architecturally require an FP destination (S/D) and a GP
    // integer source (W/X). A GP destination or an FP source is UNDEFINED and
    // must be rejected, but `encode_int_to_float` validates neither operand's
    // bank: it derives `ftype` only from the dest prefix and `sf` only from
    // the source prefix, so these illegal operands are silently mis-encoded.
    #[ignore = "BUG: encode_int_to_float does not validate operand banks (FP dest / GP source)"]
    #[test]
    fn prop_int_to_float_rejects_wrong_register_banks(n in 0u32..32) {
        // GP destination: dest must be FP (S/D), not W/X.
        let gp_dest = vec![
            Operand::Reg(format!("w{}", n)),
            Operand::Reg(format!("x{}", n)),
        ];
        prop_assert!(
            encode_int_to_float(&gp_dest, true).is_err(),
            "GP dest (W{}) is invalid for SCVTF/UCVTF; got {:?}",
            n, encode_int_to_float(&gp_dest, true)
        );

        // FP source: source must be a GP integer (W/X), not S/D.
        let fp_src = vec![
            Operand::Reg(format!("d{}", n)),
            Operand::Reg(format!("s{}", n)),
        ];
        prop_assert!(
            encode_int_to_float(&fp_src, true).is_err(),
            "FP source (S{}) is invalid for SCVTF/UCVTF; got {:?}",
            n, encode_int_to_float(&fp_src, true)
        );

        // Both wrong at once.
        let both = vec![
            Operand::Reg(format!("x{}", n)),
            Operand::Reg(format!("d{}", n)),
        ];
        prop_assert!(
            encode_int_to_float(&both, true).is_err(),
            "GP dest (X{}) + FP source (D{}) is invalid for SCVTF/UCVTF; got {:?}",
            n, n, encode_int_to_float(&both, true)
        );
    }
}
