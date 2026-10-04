//! Property-based tests for `encode_div` and `encode_logical` focusing on
//! **register-class acceptance** — specifically FP/SIMD register prefixes and
//! SP (stack pointer) operand aliasing.
//!
//! ## Scope (complementary to existing suites)
//! `encode_logical_pbt.rs` already covers reference encoding, field placement,
//! `sf` derivation, bitmask-immediate validity and shift-amount range checks.
//! `data_processing.rs::tests` covers `mul`/`smull`/`smaddl` reference oracles.
//! The existing bug reports cover *mixed-width* and *SP-as-XZR* aliasing for
//! some encoders, but **not** the cases exercised here:
//!
//! 1. **`encode_div` silently accepts FP/SIMD register names** (`d0`, `v0`,
//!    `s1`, `h2`, `b3`, `q4`). SDIV/UDIV are the *integer* "data-processing
//!    (2 source)" class (`<Wd>,<Wn>,<Wm>` / `<Xd>,<Xn>,<Xm>`); a vector/FP
//!    register has no valid encoding and a correct assembler rejects it.
//!    `parse_reg_num` maps every `d/s/q/v/h/b` prefix to a number, so
//!    `sdiv d0,d1,d2` encodes verbatim as a 32-bit integer SDIV.
//! 2. **`encode_div` silently aliases SP→XZR**. The data-processing (2 source)
//!    register class is **Zr-only** (encoding `31` == `XZR`/`WZR`, never SP).
//!    `sdiv sp,x1,x2` is accepted and emitted with `Rd=31`, which decodes as
//!    `SDIV XZR,X1,X2` — the programmer's SP destination is dropped.
//! 3. **`encode_logical` accepts SP in the Rm position** of the shifted-register
//!    form. For `ORR/AND/EOR` (shifted register) the **Rm** field is Zr-only
//!    (`31`==ZR); SP is legal only for **Rd** and **Rn**. The encoder routes
//!    Rm through `parse_reg_num`, so `orr x0,x1,sp` is accepted and silently
//!    becomes `ORR X0,X1,XZR`.
//!
//! ## Oracle
//! The SDIV/UDIV golden words below are the documented ARMv8-A encodings:
//!
//! ```text
//!   sdiv x0,x0,x0 -> 0x9AC00C00   ; 1 0 0 11010110 Rm 000011 Rn Rd
//!   udiv x0,x0,x0 -> 0x9AC00800   ; opcode 000010 (o1=0)
//!   sdiv w0,w0,w0 -> 0x1AC00C00
//!   udiv w0,w0,w0 -> 0x1AC00800
//! ```
//! The reference expression is built field-by-field from the bit-string
//! `sf | 0 | S=0 | 11010110 | Rm | 00001 | o1 | Rn | Rd`, independently of the
//! crate's single-OR expression.
//!
//! All confirmed bugs are captured by `#[ignore]`d properties so the default
//! `cargo test` stays green; run them with `cargo test -- --ignored`.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::{encode_div, encode_logical};
use proptest::prelude::*;

// --- helpers --------------------------------------------------------------

fn xreg(n: u32) -> Operand {
    Operand::Reg(format!("x{}", n))
}
fn wreg(n: u32) -> Operand {
    Operand::Reg(format!("w{}", n))
}

/// An FP/SIMD register operand: one of d/s/q/v/h/b + number.
fn fpsimd_reg(prefix: &str, n: u32) -> Operand {
    Operand::Reg(format!("{}{}", prefix, n))
}

fn expect_word(r: Result<EncodeResult, String>) -> u32 {
    match r {
        Ok(EncodeResult::Word(w)) => w,
        Ok(other) => panic!("expected Word, got {:?}", other),
        Err(e) => panic!("expected Ok, got Err: {}", e),
    }
}

// Field extractors for the data-processing (2 source) layout.
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

/// Reference encoder for SDIV/UDIV, built from the documented bit-string.
/// `sf | 0 | S=0 | 11010110 | Rm | 00001 | o1 | Rn | Rd`.
fn div_ref(rd: u32, rn: u32, rm: u32, sf: u32, o1: u32) -> u32 {
    (sf << 31)
        | (0b0011010110u32 << 21)
        | (rm << 16)
        | (0b00001u32 << 11)
        | (o1 << 10)
        | (rn << 5)
        | rd
}

proptest! {
    // ════════════════════════════════════════════════════════════════════════
    //  encode_div — positive oracles (these PASS against the current SUT)
    // ════════════════════════════════════════════════════════════════════════

    // P1. Reference oracle + field placement: for valid general-purpose
    //     registers (x0..x30 / w0..w30) the encoded word equals the
    //     spec-derived constant, and every fixed bit-group + register field
    //     lands where the data-processing (2 source) encoding dictates.
    #[test]
    fn prop_div_reference_and_fields(
        rd in 0u32..=30,
        rn in 0u32..=30,
        rm in 0u32..=30,
        is_64 in any::<bool>(),
        unsigned in any::<bool>(),
    ) {
        let dst = if is_64 { xreg(rd) } else { wreg(rd) };
        let src = if is_64 { xreg(rn) } else { wreg(rn) };
        let src2 = if is_64 { xreg(rm) } else { wreg(rm) };
        let ops = vec![dst, src, src2];
        let o1 = if unsigned { 0u32 } else { 1u32 };
        let sf = if is_64 { 1u32 } else { 0u32 };

        let w = expect_word(encode_div(&ops, unsigned));

        prop_assert_eq!(w, div_ref(rd, rn, rm, sf, o1));
        // Fixed bits 30..21 == 0_11010110_0 (the 10-bit constant), opcode
        // nibble at 15..11 == 00001, o1 at bit 10, and the three reg fields.
        prop_assert_eq!(sf_of(w), sf);
        prop_assert_eq!((w >> 21) & 0x3FF, 0b0011010110);
        prop_assert_eq!((w >> 10) & 0x3F, (0b00001 << 1) | o1); // opcode[15:11]=00001, bit10=o1
        prop_assert_eq!(rm_of(w), rm);
        prop_assert_eq!(rn_of(w), rn);
        prop_assert_eq!(rd_of(w), rd);
    }

    // P2. sf derives ONLY from the destination register's width, regardless of
    //     (possibly mismatched) source widths.
    #[test]
    fn prop_div_rejects_mixed_widths(
        n in 0u32..=30,
        rn_is_x in any::<bool>(),
        rm_is_x in any::<bool>(),
        rd_is_x in any::<bool>(),
        unsigned in any::<bool>(),
    ) {
        // Mixed x/w operand widths must be REJECTED (previously sf was
        // silently taken from the destination alone).
        prop_assume!(rd_is_x != rn_is_x || rn_is_x != rm_is_x);
        let rd = if rd_is_x { xreg(n) } else { wreg(n) };
        let rn = if rn_is_x { xreg(n) } else { wreg(n) };
        let rm = if rm_is_x { xreg(n) } else { wreg(n) };
        prop_assert!(encode_div(&[rd, rn, rm], unsigned).is_err(),
            "sdiv/udiv with mixed x/w widths must be rejected");
    }

    // ════════════════════════════════════════════════════════════════════════
    //  encode_div — BUG WITNESSES (#[ignore])
    // ════════════════════════════════════════════════════════════════════════

    // W1. FP/SIMD register acceptance (the finding NOT covered by existing
    //     reports). SDIV/UDIV accept only general-purpose registers. Any
    //     FP/SIMD prefix (d/s/q/v/h/b) in any operand position MUST be
    //     rejected. The encoder accepts it today, so this FAILS and is
    //     `#[ignore]`d. `cargo test -- --ignored div_rejects_fpsimd`.
    #[test]
    fn prop_div_rejects_fpsimd_registers(
        prefix in "[dsqvhb]",
        n in 0u32..=31u32,
        bad_pos in 0u32..3u32,
        unsigned in any::<bool>(),
    ) {
        let mut ops = vec![xreg(0), xreg(1), xreg(2)];
        ops[bad_pos as usize] = fpsimd_reg(&prefix, n);
        prop_assert!(
            encode_div(&ops, unsigned).is_err(),
            "sdiv/udiv with FP/SIMD reg {}{} in position {} must be Err",
            prefix, n, bad_pos
        );
    }

    // W2. SP operand aliasing. The data-processing (2 source) register class
    //     is Zr-only: encoding 31 == XZR/WZR, never SP. So `sdiv sp,x1,x2` is
    //     not representable and MUST be rejected. The encoder accepts SP and
    //     emits Rd/Rn/Rm=31, silently turning the destination/source into
    //     XZR. This FAILS and is `#[ignore]`d.
    #[test]
    fn prop_div_rejects_sp_operand(
        bad_pos in 0u32..3u32,
        use_wsp in any::<bool>(),
        unsigned in any::<bool>(),
    ) {
        let sp_name = if use_wsp { "wsp" } else { "sp" };
        let mut ops = vec![xreg(0), xreg(1), xreg(2)];
        ops[bad_pos as usize] = Operand::Reg(sp_name.to_string());
        prop_assert!(
            encode_div(&ops, unsigned).is_err(),
            "sdiv/udiv with {} in position {} must be Err (Zr-only class)",
            sp_name, bad_pos
        );
    }
}

proptest! {
    // ════════════════════════════════════════════════════════════════════════
    //  encode_logical — SP operand acceptance (shifted-register + immediate)
    // ════════════════════════════════════════════════════════════════════════
    //
    // Per ARMv8-A ARM, `ORR/AND/EOR` (non-flag-setting logical):
    //   * shifted-register form: `Rd` and `Rn` are **SP-allowed** (31 == SP);
    //     **Rm** is **Zr-only** (31 == ZR).
    //   * immediate form: `Rd` and `Rn` are **SP-allowed** (31 == SP).
    // (ORR opc = 0b01.)

    // P3. SP is ACCEPTED in the Rd and Rn positions (shifted register), and the
    //     31 lands in the correct field — i.e. the encoder's correct half of
    //     SP handling. This PASSES.
    #[test]
    fn prop_logical_rejects_sp_in_register_forms(rm in 0u32..=30) {
        // The shifted-register forms take no SP in ANY position
        // (GNU as: `and sp, x1, x2` / `and x0, sp, x2` are rejected).
        let a = encode_logical(&[Operand::Reg("sp".into()), xreg(rm), xreg(rm)], 0b01);
        prop_assert!(a.is_err(), "orr sp, x, x (register form) must be rejected");
        let b = encode_logical(&[xreg(rm), Operand::Reg("sp".into()), xreg(rm)], 0b01);
        prop_assert!(b.is_err(), "orr x, sp, x (register form) must be rejected");
    }

    // W3. SP in the Rm position of the shifted-register form is silently
    //     accepted. The Rm field is Zr-only (31 == XZR), so `orr x0,x1,sp`
    //     must be rejected; the encoder accepts it and emits Rm=31, silently
    //     aliasing SP→XZR. This FAILS and is `#[ignore]`d.
    #[ignore = "documented bug: encode_logical accepts SP in Rm (Zr-only field), silently aliasing SP to XZR"]
    #[test]
    fn prop_logical_rejects_sp_in_rm(use_wsp in any::<bool>()) {
        let rm_name = if use_wsp { "wsp" } else { "sp" };
        let ops = vec![xreg(0), xreg(1), Operand::Reg(rm_name.to_string())];
        prop_assert!(
            encode_logical(&ops, 0b01).is_err(),
            "orr x0,x1,{} must be Err (Rm is Zr-only)", rm_name
        );
    }
}

// P4. SP is ACCEPTED in the immediate form for both Rd and Rn (constant case,
//      asserted outside the proptest macro).
#[test]
fn logical_immediate_sp_contract() {
    // SP is legal ONLY as the destination of the non-flags immediate forms:
    // `and sp, x1, #15` assembles in GNU as; SP in Rn (and every position of
    // the register forms, and ANDS) is rejected.
    let w = expect_word(encode_logical(
        &[Operand::Reg("sp".into()), xreg(1), Operand::Imm(0xff)],
        0b01,
    ));
    assert_eq!(sf_of(w), 1);
    assert_eq!(rd_of(w), 31);
    assert_eq!(rn_of(w), 1);
    let rn_sp = encode_logical(
        &[xreg(0), Operand::Reg("sp".into()), Operand::Imm(0xff)],
        0b01,
    );
    assert!(rn_sp.is_err(), "SP in Rn must be rejected");
    let ands_sp = encode_logical(
        &[Operand::Reg("sp".into()), xreg(1), Operand::Imm(0xff)],
        0b11,
    );
    assert!(ands_sp.is_err(), "SP in ANDS destination must be rejected");
}

// ── Known-constant reference anchors (ARMv8-A ARM golden words) ───────────
#[test]
fn div_known_constants() {
    // sdiv x0,x0,x0 = 0x9AC00C00 ; udiv x0,x0,x0 = 0x9AC00800
    assert_eq!(
        expect_word(encode_div(&[xreg(0), xreg(0), xreg(0)], false)),
        0x9AC0_0C00u32
    );
    assert_eq!(
        expect_word(encode_div(&[xreg(0), xreg(0), xreg(0)], true)),
        0x9AC0_0800u32
    );
    // sdiv w0,w0,w0 = 0x1AC00C00 ; udiv w0,w0,w0 = 0x1AC00800
    assert_eq!(
        expect_word(encode_div(&[wreg(0), wreg(0), wreg(0)], false)),
        0x1AC0_0C00u32
    );
    assert_eq!(
        expect_word(encode_div(&[wreg(0), wreg(0), wreg(0)], true)),
        0x1AC0_0800u32
    );
}
