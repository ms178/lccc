//! Property-based tests for `encode_shift`
//! (the AArch64 LSL/LSR/ASR/ROR encoder in `data_processing.rs`).
//!
//! Two encoding shapes are covered:
//!
//! 1. **Immediate form** — `LSL/LSR/ASR Rd, Rn, #imm` lowers to
//!    UBFM/SBFM (`sf opc 100110 N immr imms Rn Rd`), and
//!    `ROR Rd, Rn, #imm` lowers to EXTR (`sf 0 0 100111 N 0 Rm imms Rn Rd`
//!    with `Rm = Rn`).
//!
//! 2. **Register form** — `LSL/LSR/ASR/ROR Rd, Rn, Rm` lowers to a
//!    Data-processing (2 source) instruction
//!    (`sf 0 S=0 11010110 Rm 0010 op2 Rn Rd`, with `op2 = shift_type`).
//!
//! ## Findings
//!
//! The register form and the *in-range* immediate form are encoded correctly.
//! However the immediate path performs **no range validation** on the shift
//! amount, so out-of-range / negative immediates are not rejected:
//!
//!  * `LSL #imm` with `imm >= width` causes unsigned underflow in
//!    `(width - imm)` and `width - 1 - imm` — a panic in debug builds,
//!    silent garbage in release.
//!  * `ROR #imm` / `LSR #imm` / `ASR #imm` with `imm` outside `[0, width-1]`
//!    are silently accepted (the field is truncated/shifted into neighbours).
//!  * A negative immediate (`*imm as u32`) wraps to a huge value and is not
//!    rejected.
//!
//! These are witnessed by the `#[ignore]`d properties below so the default
//! `cargo test` stays green; run them explicitly with
//! `cargo test -- --ignored`.

use lccc::pbt_internals::*;
use proptest::prelude::*;
use std::panic;

// ── helpers ──────────────────────────────────────────────────────────────

fn xreg(n: u32) -> Operand {
    Operand::Reg(format!("x{}", n))
}
fn wreg(n: u32) -> Operand {
    Operand::Reg(format!("w{}", n))
}

fn expect_word(r: Result<EncodeResult, String>) -> u32 {
    match r {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected Word, got {:?}", other),
    }
}

// field extractors for the Data-processing (2 source) / UBFM-SBFM-EXTR words
fn sf_of(w: u32) -> u32 {
    (w >> 31) & 1
}
fn rm_of(w: u32) -> u32 {
    (w >> 16) & 0x1F
}
fn rn_of(w: u32) -> u32 {
    (w >> 5) & 0x1F
}
fn rd_of(w: u32) -> u32 {
    w & 0x1F
}

proptest! {
    // ── Register form: reference encoding ────────────────────────────────
    //
    // Data-processing (2 source): `sf 0 S=0 11010110 Rm 0010 op2 Rn Rd`.
    // With all registers and op2 zeroed:
    //   32-bit: 0001 1010 1100 0000 0000 1000 0000 0000 = 0x1AC00800
    //   64-bit: 1001 1010 1100 0000 0000 1000 0000 0000 = 0x9AC00800
    // and Rm/Rn/Rd/op2 OR'd into their fields.

    #[test]
    fn shift_register_reference_encoding(
        rd in 0u32..=31,
        rn in 0u32..=31,
        rm in 0u32..=31,
        shift_type in 0u32..=3,   // 00=LSL 01=LSR 10=ASR 11=ROR
        is_64 in any::<bool>(),
    ) {
        let dst = if is_64 { xreg(rd) } else { wreg(rd) };
        let ops = vec![dst, xreg(rn), xreg(rm)];
        let w = expect_word(encode_shift(&ops, shift_type));

        let sf = if is_64 { 1u32 } else { 0u32 };
        let expected = (sf << 31)
            | (0b0011010110 << 21)
            | (rm << 16)
            | (0b0010 << 12)
            | (shift_type << 10)
            | (rn << 5)
            | rd;
        prop_assert_eq!(w, expected);
    }

    // ── Register form: field placement ───────────────────────────────────

    #[test]
    fn shift_register_field_placement(
        rd in 0u32..=31,
        rn in 0u32..=31,
        rm in 0u32..=31,
        shift_type in 0u32..=3,
        is_64 in any::<bool>(),
    ) {
        let dst = if is_64 { xreg(rd) } else { wreg(rd) };
        let ops = vec![dst, xreg(rn), xreg(rm)];
        let w = expect_word(encode_shift(&ops, shift_type));

        prop_assert_eq!(sf_of(w), if is_64 { 1 } else { 0 });
        // bits [30:21] = "0 S=0 11010110" = 0b0011010110
        prop_assert_eq!((w >> 21) & 0x3FF, 0b0011010110);
        prop_assert_eq!(rm_of(w), rm);
        // bits [15:10] = "0010 op2"
        prop_assert_eq!((w >> 10) & 0x3F, (0b0010 << 2) | shift_type);
        prop_assert_eq!(rn_of(w), rn);
        prop_assert_eq!(rd_of(w), rd);
    }

    // ── Immediate form: field oracle for LSL/LSR/ASR (UBFM/SBFM) ─────────
    //
    // For a valid `imm` in [0, width-1]:
    //   LSL -> UBFM: opc=10, immr=(width-imm)%width, imms=width-1-imm
    //   LSR -> UBFM: opc=10, immr=imm,             imms=width-1
    //   ASR -> SBFM: opc=00, immr=imm,             imms=width-1
    //   ROR -> EXTR: bits[30:23]=00100111, Rm=rn, imms=imm
    // Fixed bits [28:23]=100110 (UBFM/SBFM), N at bit 22.

    #[test]
    fn shift_immediate_field_oracle(
        rd in 0u32..=31,
        rn in 0u32..=31,
        shift_type in 0u32..=3,
        is_64 in any::<bool>(),
        imm in 0u32..64,
    ) {
        let width: u32 = if is_64 { 64 } else { 32 };
        prop_assume!(imm < width);

        let dst = if is_64 { xreg(rd) } else { wreg(rd) };
        let ops = vec![dst, xreg(rn), Operand::Imm(imm as i64)];
        let w = expect_word(encode_shift(&ops, shift_type));

        let sf = if is_64 { 1u32 } else { 0u32 };
        let n = if is_64 { 1u32 } else { 0u32 };
        prop_assert_eq!(sf_of(w), sf);

        match shift_type {
            0b00 => {
                // LSL -> UBFM
                let immr = (width - imm) % width;
                let imms = width - 1 - imm;
                prop_assert_eq!((w >> 29) & 0x3, 0b10);                 // opc = UBFM
                prop_assert_eq!((w >> 23) & 0x3F, 0b100110);            // fixed bits
                prop_assert_eq!((w >> 22) & 1, n);
                prop_assert_eq!((w >> 16) & 0x3F, immr);
                prop_assert_eq!((w >> 10) & 0x3F, imms);
            }
            0b01 => {
                // LSR -> UBFM
                prop_assert_eq!((w >> 29) & 0x3, 0b10);
                prop_assert_eq!((w >> 23) & 0x3F, 0b100110);
                prop_assert_eq!((w >> 22) & 1, n);
                prop_assert_eq!((w >> 16) & 0x3F, imm);                 // immr = imm
                prop_assert_eq!((w >> 10) & 0x3F, width - 1);           // imms = width-1
            }
            0b10 => {
                // ASR -> SBFM
                prop_assert_eq!((w >> 29) & 0x3, 0b00);                 // opc = SBFM
                prop_assert_eq!((w >> 23) & 0x3F, 0b100110);
                prop_assert_eq!((w >> 22) & 1, n);
                prop_assert_eq!((w >> 16) & 0x3F, imm);                 // immr = imm
                prop_assert_eq!((w >> 10) & 0x3F, width - 1);           // imms = width-1
            }
            0b11 => {
                // ROR -> EXTR Rd, Rn, Rn, #imm
                prop_assert_eq!((w >> 23) & 0xFF, 0b00100111);          // bits[30:23]
                prop_assert_eq!((w >> 22) & 1, n);
                prop_assert_eq!((w >> 16) & 0x1F, rn);                  // Rm = Rn
                prop_assert_eq!((w >> 10) & 0x3F, imm);                 // imms(lsb) = imm
            }
            _ => unreachable!(),
        }
        prop_assert_eq!(rn_of(w), rn);
        prop_assert_eq!(rd_of(w), rd);
    }

    // ── Negative contract (passes): a missing shift operand is rejected ──

    #[test]
    fn shift_rejects_missing_shift_operand(
        rd in 0u32..=31,
        rn in 0u32..=31,
        shift_type in 0u32..=3,
        is_64 in any::<bool>(),
    ) {
        let dst = if is_64 { xreg(rd) } else { wreg(rd) };
        let ops = vec![dst, xreg(rn)]; // only Rd, Rn — no third operand
        prop_assert!(encode_shift(&ops, shift_type).is_err());
    }

    // ── BUG WITNESS (#[ignore]): no range validation on the immediate ────
    //
    // An immediate that is negative or >= width has no valid AArch64 encoding
    // and MUST be rejected with `Err`. The current encoder instead:
    //   * wraps `*imm as u32` (negatives become huge positives), and
    //   * underflows `(width - imm)` / `width - 1 - imm` (panic in debug,
    //     silent garbage in release).
    //
    // We wrap the call in `catch_unwind` so the witness reports a clean
    // assertion failure rather than aborting, regardless of build mode.

    #[ignore]
    #[test]
    fn shift_rejects_invalid_immediate(
        shift_type in 0u32..=3,
        is_64 in any::<bool>(),
        bad_kind in 0u32..2,            // 0 = out-of-range, 1 = negative
        slack in 0i64..64,
    ) {
        let width: i64 = if is_64 { 64 } else { 32 };
        let bad_imm: i64 = if bad_kind == 0 { width + slack } else { -1 - slack };

        let dst = if is_64 { xreg(0) } else { wreg(0) };
        let ops = vec![dst, xreg(1), Operand::Imm(bad_imm)];

        let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            encode_shift(&ops, shift_type)
        }));

        match result {
            Ok(r) => prop_assert!(r.is_err(),
                "imm={} (width={}) shift_type={} was silently accepted: {:?}",
                bad_imm, width, shift_type, r),
            Err(_) => prop_assert!(false,
                "imm={} (width={}) shift_type={} panicked instead of returning Err",
                bad_imm, width, shift_type),
        }
    }
}
