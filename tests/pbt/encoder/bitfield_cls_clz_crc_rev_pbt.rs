//! Property-based tests for the AArch64 "data-processing (1 source)" and
//! CRC32 encoders in `bitfield.rs`:
//!   * `encode_clz`  — Count Leading Zeros
//!   * `encode_cls`  — Count Leading Sign bits
//!   * `encode_rev`  — Byte-Reverse (register)
//!   * `encode_crc32` — CRC32 / CRC32C (checksum)
//!
//! ## Oracle
//!
//! **Field placement (ARM ARM bit layout) + differential (XOR vs sibling
//! encoders that share the same template) + reference (known-constant
//! anchors) + negative error contract (malformed operands).** Every
//! reference constant was hand-derived from the encoder and cross-checked
//! against the ARMv8-A ARM:
//!
//! ```text
//!   CLZ/CLS/RBIT/REV (Data-processing, 1 source):
//!     sf 1 0 11010110 00000 opc[15:10] Rn Rd
//!       CLZ  opc = 000100   (X0,X0 = 0xDAC01000 ; W0,W0 = 0x5AC01000)
//!       CLS  opc = 000101   (X0,X0 = 0xDAC01400 ; W0,W0 = 0x5AC01400)
//!       REV  opc = 000010 (W) / 000011 (X)
//!                       (X0,X0 = 0xDAC00C00 ; W0,W0 = 0x5AC00800)
//!
//!   CRC32 / CRC32C:
//!     sf 0 0 11010110 Rm 010 C sz Rn Rd
//!       crc32w W0,W0,W0 = 0x1AC04800 ; crc32x X0,W0,X0 = 0x9AC04C00
//! ```
//!
//! ## Regression properties (former bug witnesses, now active)
//!
//! Every defect this campaign found in these encoders — mixed Rd/Rn widths
//! for CLZ/CLS/REV, unknown CRC mnemonics, and per-variant CRC register
//! widths — is fixed; the former `#[ignore]`d witnesses run in the default
//! suite and lock the contracts in.

use lccc::pbt_internals::*;
use proptest::prelude::*;

// ── shared helpers ───────────────────────────────────────────────────────

fn word(r: Result<EncodeResult, String>) -> u32 {
    match r {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected Word, got {:?}", other),
    }
}

/// `true` -> `x{n}`, `false` -> `w{n}`.
fn reg(n: u32, x64: bool) -> String {
    if x64 {
        format!("x{}", n)
    } else {
        format!("w{}", n)
    }
}

fn reg_op(n: u32, x64: bool) -> Operand {
    Operand::Reg(reg(n, x64))
}

// ===========================================================================
//  encode_clz  —  Count Leading Zeros
// ===========================================================================

mod clz {
    use super::*;

    // sf [31] | 1 [30] | 0 [29] | 1101011 [28:22] (N fixed=1)
    // | 000000 [21:16] | 000100 [15:10] | Rn [9:5] | Rd [4:0]
    //
    // NOTE: bit[22] ("N") is FIXED to 1 by the architecture — it does NOT
    // track sf (unlike the UBFM/SBFM/BFM bitfield family). So the "N==sf"
    // invariant must NOT be asserted here.
    const MASK_SF: u32 = 0x8000_0000;
    const FIXED_28_22: u32 = 0b1101011 << 22; // 0x1AC0_0000 (incl. N=1)
    const MASK_21_16: u32 = 0x003F_0000;
    const FIXED_15_10: u32 = 0b000100 << 10; // 0x0000_1000 (CLZ opcode)
    const MASK_15_10: u32 = 0x0000_FC00;
    const MASK_RN: u32 = 0x0000_03E0;
    const MASK_RD: u32 = 0x0000_001F;

    fn enc(ops: &[Operand]) -> u32 {
        word(encode_clz(ops))
    }

    proptest! {
        // Field placement: every fixed opcode bit lands where the ARM ARM
        // mandates; Rn/Rd reconstruct to inputs; sf tracks the destination
        // width; N[22] is fixed to 1 even for the 32-bit form.
        #[test]
        fn prop_clz_field_placement(
            rd in 0u32..=30u32, rn in 0u32..=30u32, x64 in any::<bool>(),
        ) {
            let ops = vec![reg_op(rd, x64), reg_op(rn, x64)];
            let w = enc(&ops);
            prop_assert_eq!(w & MASK_SF, if x64 { MASK_SF } else { 0 });
            prop_assert_eq!(w & (1u32 << 30), 1u32 << 30, "bit30 must be 1");
            prop_assert_eq!(w & (1u32 << 29), 0, "bit29 must be 0");
            prop_assert_eq!(w & 0x1FC0_0000, FIXED_28_22, "[28:22] = 1101011 (N=1)");
            prop_assert_eq!(w & MASK_21_16, 0, "opcode2 [21:16] must be 0");
            prop_assert_eq!(w & MASK_15_10, FIXED_15_10, "[15:10] = 000100 (CLZ)");
            prop_assert_eq!((w & MASK_RN) >> 5, rn);
            prop_assert_eq!(w & MASK_RD, rd);
            prop_assert_eq!((w >> 22) & 1, 1, "N bit fixed to 1");
        }

        // Differential vs sibling CLS: identical template, differ ONLY in
        // bit[10] (CLZ opc=000100 vs CLS opc=000101).
        #[test]
        fn prop_clz_xor_cls_is_only_bit_10(
            rd in 0u32..=30u32, rn in 0u32..=30u32, x64 in any::<bool>(),
        ) {
            let ops = vec![reg_op(rd, x64), reg_op(rn, x64)];
            prop_assert_eq!(enc(&ops) ^ word(encode_cls(&ops)), 1u32 << 10);
        }

        // Differential vs scalar RBIT: RBIT opc=000000, CLZ opc=000100 ->
        // differ only in bit[12].
        #[test]
        fn prop_clz_xor_rbit_is_only_bit_12(
            rd in 0u32..=30u32, rn in 0u32..=30u32, x64 in any::<bool>(),
        ) {
            let ops = vec![reg_op(rd, x64), reg_op(rn, x64)];
            prop_assert_eq!(enc(&ops) ^ word(encode_rbit(&ops)), 1u32 << 12);
        }

        // Width differential: x{N} vs w{N} (same partner) differs only in sf.
        #[test]
        fn prop_width_changes_only_sf(num in 0u32..=30u32) {
            let ops64 = vec![Operand::Reg(format!("x{}", num)), Operand::Reg("x0".into())];
            let ops32 = vec![Operand::Reg(format!("w{}", num)), Operand::Reg("w0".into())];
            prop_assert_eq!(enc(&ops64) ^ enc(&ops32), MASK_SF);
        }

        // Malformed-operands negative contract (should pass): missing operand
        // or a non-register in either fixed slot must yield Err.
        #[test]
        fn prop_rejects_malformed_operands(
            kind in prop_oneof![Just(0u8), Just(1u8), Just(2u8), Just(3u8)],
            n in 0u32..=30u32, v in -16i64..=16i64,
        ) {
            let r = match kind {
                0 => encode_clz(&[]),
                1 => encode_clz(&[reg_op(n, true)]),
                2 => encode_clz(&[Operand::Imm(v), Operand::Reg("x1".into())]),
                _ => encode_clz(&[reg_op(n, true), Operand::Imm(v)]),
            };
            prop_assert!(r.is_err(), "expected Err, got {:?}", r);
        }

        // ── REGRESSION (former bug witness, now active) ─────────────────────────────────────
        // ARM ARM CLZ is defined only as "CLZ <Wd>,<Wn>" / "CLZ <Xd>,<Xn>":
        // source and destination MUST share the same register size. A
        // correct assembler MUST reject mismatched-width pairs such as
        // "CLZ Xd, Wn". The encoder reads Rn but DISCARDS its width
        // (`let (rn, _) = get_reg(...)`), deriving sf solely from Rd, so it
        // silently accepts mismatches. This asserts the correct contract and
        // FAILS against the current SUT — kept out of the default run.
        #[test]
        fn prop_rejects_mismatched_register_widths(d in 0u32..=30u32, n in 0u32..=30u32) {
            let xd_wn = vec![reg_op(d, true), reg_op(n, false)];
            let wd_xn = vec![reg_op(d, false), reg_op(n, true)];
            prop_assert!(encode_clz(&xd_wn).is_err(), "CLZ x{},w{} must be Err", d, n);
            prop_assert!(encode_clz(&wd_xn).is_err(), "CLZ w{},x{} must be Err", d, n);
        }
    }

    // Known-constant reference anchor (ARM ARM):
    //   CLZ X0,X0 = 0xDAC01000 ; CLZ W0,W0 = 0x5AC01000.
    #[test]
    fn prop_clz_known_constants() {
        let w64 = enc(&[Operand::Reg("x0".into()), Operand::Reg("x0".into())]);
        let w32 = enc(&[Operand::Reg("w0".into()), Operand::Reg("w0".into())]);
        assert_eq!(w64, 0xDAC0_1000u32);
        assert_eq!(w32, 0x5AC0_1000u32);
    }
}

// ===========================================================================
//  encode_cls  —  Count Leading Sign bits
// ===========================================================================

mod cls {
    use super::*;

    // sf [31] | 10 [30:29] | 11010110 [28:21] | 00000 [20:16]
    // | 000101 [15:10] (CLS opcode) | Rn [9:5] | Rd [4:0]
    const MASK_SF: u32 = 0x8000_0000;
    const FIXED_30_29: u32 = 0b10 << 29; // 0x4000_0000
    const MASK_30_29: u32 = 0b11 << 29;
    const FIXED_28_21: u32 = 0b11010110u32 << 21; // 0x1AC0_0000
    const MASK_28_21: u32 = 0xFFu32 << 21;
    const MASK_OP2: u32 = 0x1Fu32 << 16; // [20:16] must be 0
    const FIXED_15_10: u32 = 0b000101u32 << 10; // CLS opcode
    const MASK_15_10: u32 = 0x3Fu32 << 10;
    const MASK_RN: u32 = 0x1Fu32 << 5;
    const MASK_RD: u32 = 0x1Fu32;

    fn enc(ops: &[Operand]) -> u32 {
        word(encode_cls(ops))
    }

    proptest! {
        #[test]
        fn prop_cls_field_placement(
            rd in 0u32..=30u32, rn in 0u32..=30u32, x64 in any::<bool>(),
        ) {
            let ops = vec![reg_op(rd, x64), reg_op(rn, x64)];
            let w = enc(&ops);
            prop_assert_eq!(w & MASK_30_29, FIXED_30_29, "[30:29] = 10");
            prop_assert_eq!(w & MASK_28_21, FIXED_28_21, "[28:21] = 11010110");
            prop_assert_eq!(w & MASK_OP2, 0, "op2 [20:16] must be 0");
            prop_assert_eq!(w & MASK_15_10, FIXED_15_10, "[15:10] = 000101 (CLS)");
            prop_assert_eq!(w & MASK_SF, if x64 { MASK_SF } else { 0 });
            prop_assert_eq!((w & MASK_RN) >> 5, rn);
            prop_assert_eq!(w & MASK_RD, rd);
        }

        // Differential vs CLZ: identical template, differ ONLY in bit[10].
        #[test]
        fn prop_cls_xor_clz_is_only_bit_10(
            rd in 0u32..=30u32, rn in 0u32..=30u32, x64 in any::<bool>(),
        ) {
            let ops = vec![reg_op(rd, x64), reg_op(rn, x64)];
            prop_assert_eq!(enc(&ops) ^ word(encode_clz(&ops)), 0x0000_0400u32);
        }

        // Width differential: differs only in sf[31].
        #[test]
        fn prop_width_changes_only_sf(num in 0u32..=30u32) {
            let ops64 = vec![Operand::Reg(format!("x{}", num)), Operand::Reg("x0".into())];
            let ops32 = vec![Operand::Reg(format!("w{}", num)), Operand::Reg("w0".into())];
            prop_assert_eq!(enc(&ops64) ^ enc(&ops32), MASK_SF);
        }

        // Malformed-operands negative contract (should pass).
        #[test]
        fn prop_rejects_malformed_operands(
            kind in prop_oneof![Just(0u8), Just(1u8), Just(2u8)],
            n in 0u32..=30u32, v in -16i64..=16i64,
        ) {
            let r = match kind {
                0 => encode_cls(&[]),
                1 => encode_cls(&[reg_op(n, true)]),
                _ => encode_cls(&[Operand::Imm(v), Operand::Reg("x1".into())]),
            };
            prop_assert!(r.is_err(), "expected Err, got {:?}", r);
        }

        // ── REGRESSION (former bug witness, now active) ─────────────────────────────────────
        // CLS shares a single sf field: Rd and Rn MUST be the same width
        // (ARM ARM, CLS). `CLS x0, w1` / `CLS w0, x1` are UNPREDICTABLE and
        // a correct assembler MUST reject them. The encoder discards Rn's
        // width, so it silently accepts mismatches. Asserts the correct
        // contract and FAILS against the current SUT.
        #[test]
        fn prop_rejects_mixed_width_operands(d in 0u32..=30u32, n in 0u32..=30u32) {
            let a = vec![reg_op(d, true), reg_op(n, false)];
            let b = vec![reg_op(d, false), reg_op(n, true)];
            prop_assert!(encode_cls(&a).is_err(), "CLS x{},w{} must be Err", d, n);
            prop_assert!(encode_cls(&b).is_err(), "CLS w{},x{} must be Err", d, n);
        }
    }

    // Known-constant reference anchor (ARM ARM):
    //   CLS X0,X0 = 0xDAC01400 ; CLS W0,W0 = 0x5AC01400.
    #[test]
    fn prop_cls_known_constants() {
        let w64 = enc(&[Operand::Reg("x0".into()), Operand::Reg("x0".into())]);
        let w32 = enc(&[Operand::Reg("w0".into()), Operand::Reg("w0".into())]);
        assert_eq!(w64, 0xDAC0_1400u32);
        assert_eq!(w32, 0x5AC0_1400u32);
    }
}

// ===========================================================================
//  encode_rev  —  Byte-Reverse register
// ===========================================================================

mod rev {
    use super::*;

    // sf 1 0 11010110 00000 opc[15:10] Rn Rd
    //   W form: sf=0, opc=000010 -> base 0x5AC00800
    //   X form: sf=1, opc=000011 -> base 0xDAC00C00
    // REV correctly SWAPS opc with sf (contrast the buggy encode_rev32).
    // Single sf field => Rd and Rn must share the same width.
    const BASE_64: u32 = 0xDAC0_0C00;
    const BASE_32: u32 = 0x5AC0_0800;

    const MASK_SF: u32 = 0x8000_0000;
    const MASK_OPC: u32 = 0x3F << 10;
    const MASK_RN: u32 = 0x1F << 5;
    const MASK_RD: u32 = 0x1F;

    fn enc(ops: &[Operand]) -> u32 {
        word(encode_rev(ops))
    }

    /// ARM ARM reference word for REV, both widths.
    fn ref_rev(x64: bool, rn: u32, rd: u32) -> u32 {
        let base = if x64 { BASE_64 } else { BASE_32 };
        base | (rn << 5) | rd
    }

    proptest! {
        #[test]
        fn prop_rev_field_placement(
            rd in 0u32..=30u32, rn in 0u32..=30u32, x64 in any::<bool>(),
        ) {
            let ops = vec![reg_op(rd, x64), reg_op(rn, x64)];
            let w = enc(&ops);
            prop_assert_eq!(w & MASK_SF, if x64 { MASK_SF } else { 0 }, "sf tracks width");
            prop_assert_eq!(w & (0b11u32 << 29), 0b10u32 << 29, "bit30=1, bit29=0");
            prop_assert_eq!(w & (0xFFu32 << 21), 0xD6u32 << 21, "[28:21] = 11010110");
            prop_assert_eq!(w & (0x1Fu32 << 16), 0, "[20:16] must be 0");
            let want_opc: u32 = if x64 { 0b000011 } else { 0b000010 };
            prop_assert_eq!((w & MASK_OPC) >> 10, want_opc, "opc varies with width");
            prop_assert_eq!((w & MASK_RN) >> 5, rn, "Rn reconstruct");
            prop_assert_eq!(w & MASK_RD, rd, "Rd reconstruct");
        }

        // Reference oracle: the emitted word must equal the hand-derived
        // ARM ARM base OR'd with (Rn<<5)|Rd, for BOTH widths. Passing for
        // both widths is precisely what makes encode_rev correct.
        #[test]
        fn prop_rev_matches_arm_reference(
            rd in 0u32..=30u32, rn in 0u32..=30u32, x64 in any::<bool>(),
        ) {
            let ops = vec![reg_op(rd, x64), reg_op(rn, x64)];
            prop_assert_eq!(enc(&ops), ref_rev(x64, rn, rd));
        }

        // Width differential: x{N} vs w{N} differs only in sf[31] and the
        // low bit of opc[10] (opc is 000011 for X and 000010 for W).
        #[test]
        fn prop_width_changes_only_sf_and_opc_bit0(
            num in 0u32..=30u32, rn in 0u32..=30u32,
        ) {
            let ops64 = vec![Operand::Reg(format!("x{}", num)), Operand::Reg(format!("x{}", rn))];
            let ops32 = vec![Operand::Reg(format!("w{}", num)), Operand::Reg(format!("w{}", rn))];
            prop_assert_eq!(enc(&ops64) ^ enc(&ops32), MASK_SF | (1u32 << 10));
        }

        // Determinism: the encoder is a pure function.
        #[test]
        fn prop_deterministic(
            rd in 0u32..=30u32, rn in 0u32..=30u32, x64 in any::<bool>(),
        ) {
            let ops = vec![reg_op(rd, x64), reg_op(rn, x64)];
            prop_assert_eq!(enc(&ops), enc(&ops));
        }

        // Malformed-operands negative contract (should pass).
        #[test]
        fn prop_rejects_malformed_operands(
            kind in prop_oneof![Just(0u8), Just(1u8), Just(2u8), Just(3u8)],
            n in 0u32..=30u32, v in -16i64..=16i64,
        ) {
            let r = match kind {
                0 => encode_rev(&[]),
                1 => encode_rev(&[reg_op(n, true)]),
                2 => encode_rev(&[Operand::Imm(v), Operand::Reg("x1".into())]),
                _ => encode_rev(&[reg_op(n, true), Operand::Imm(v)]),
            };
            prop_assert!(r.is_err(), "expected Err, got {:?}", r);
        }

        // ── REGRESSION (former bug witness, now active) ─────────────────────────────────────
        // REV carries a SINGLE sf field, so the ARM ARM requires Rd and Rn
        // to share the same width: `REV Xd, Wn` / `REV Wd, Xn` are
        // UNPREDICTABLE / UNALLOCATED and MUST be rejected. The encoder
        // derives sf ONLY from Rd and silently accepts a mismatched-width
        // source register. Asserts the correct contract and FAILS against
        // the current SUT.
        #[test]
        fn prop_rejects_mismatched_widths(
            rd in 0u32..=30u32, rn in 0u32..=30u32, rd_x64 in any::<bool>(),
        ) {
            let ops = vec![reg_op(rd, rd_x64), reg_op(rn, !rd_x64)];
            let r = encode_rev(&ops);
            prop_assert!(r.is_err(),
                "REV {}/{} (mismatched widths) must be Err, got {:?}",
                reg(rd, rd_x64), reg(rn, !rd_x64), r);
        }
    }

    // Known-constant reference anchor (ARM ARM):
    //   REV X0,X0 = 0xDAC00C00 ; REV W0,W0 = 0x5AC00800.
    #[test]
    fn prop_rev_known_constants() {
        let w64 = enc(&[Operand::Reg("x0".into()), Operand::Reg("x0".into())]);
        let w32 = enc(&[Operand::Reg("w0".into()), Operand::Reg("w0".into())]);
        assert_eq!(w64, 0xDAC0_0C00u32);
        assert_eq!(w32, 0x5AC0_0800u32);
    }
}

// ===========================================================================
//  encode_crc32  —  CRC32 / CRC32C (checksum)
// ===========================================================================

mod crc32 {
    use super::*;

    // sf [31] | 00 [30:29] | 11010110 [28:21] | Rm [20:16]
    // | 010 [15:13] | C [12] | sz [11:10] | Rn [9:5] | Rd [4:0]
    //
    // Exactly eight mnemonics: crc32{b,h,w,x} (C=0) and crc32c{b,h,w,x} (C=1).
    // sf=1 only for the 'x' (doubleword) variants.
    const MASK_SF: u32 = 0x8000_0000;
    const FIXED_30_21: u32 = 0b0011010110u32 << 21; // 0x1AC0_0000
    const MASK_30_21: u32 = 0x3FFu32 << 21;
    const MASK_RM: u32 = 0x1Fu32 << 16;
    const FIXED_15_13: u32 = 0b010u32 << 13;
    const MASK_15_13: u32 = 0x7u32 << 13;
    const MASK_C: u32 = 0x1u32 << 12;
    const MASK_SZ: u32 = 0x3u32 << 10;
    const MASK_RN: u32 = 0x1Fu32 << 5;
    const MASK_RD: u32 = 0x1Fu32;

    fn enc(mn: &str, ops: &[Operand]) -> u32 {
        word(encode_crc32(mn, ops))
    }

    /// Expected (sf, sz, c) for each architecturally valid mnemonic.
    fn expected(mn: &str) -> (u32, u32, u32) {
        match mn {
            "crc32b" => (0, 0b00, 0),
            "crc32h" => (0, 0b01, 0),
            "crc32w" => (0, 0b10, 0),
            "crc32x" => (1, 0b11, 0),
            "crc32cb" => (0, 0b00, 1),
            "crc32ch" => (0, 0b01, 1),
            "crc32cw" => (0, 0b10, 1),
            "crc32cx" => (1, 0b11, 1),
            _ => unreachable!("invalid mnemonic in expected()"),
        }
    }

    fn valid_mnemonic() -> impl Strategy<Value = &'static str> {
        prop_oneof![
            Just("crc32b"),
            Just("crc32h"),
            Just("crc32w"),
            Just("crc32x"),
            Just("crc32cb"),
            Just("crc32ch"),
            Just("crc32cw"),
            Just("crc32cx"),
        ]
    }

    proptest! {
        // Field placement: for every valid mnemonic and any register triple,
        // every fixed bit and field lands where the encoding mandates;
        // sf/sz/C reconstruct to the per-mnemonic values; Rm/Rn/Rd to inputs.
        #[test]
        fn prop_crc32_field_placement(
            mn in valid_mnemonic(),
            rd in 0u32..=30u32, rn in 0u32..=30u32, rm in 0u32..=30u32,
        ) {
            let ops = vec![reg_op(rd, false), reg_op(rn, false), reg_op(rm, mn.ends_with('x'))];
            let w = enc(mn, &ops);
            let (sf, sz, c) = expected(mn);
            prop_assert_eq!(w & MASK_30_21, FIXED_30_21, "fixed bits [30:21]");
            prop_assert_eq!(w & MASK_15_13, FIXED_15_13, "[15:13] = 010");
            prop_assert_eq!(w & MASK_SF, sf << 31);
            prop_assert_eq!(w & MASK_C, c << 12);
            prop_assert_eq!((w & MASK_SZ) >> 10, sz);
            prop_assert_eq!((w & MASK_RM) >> 16, rm);
            prop_assert_eq!((w & MASK_RN) >> 5, rn);
            prop_assert_eq!(w & MASK_RD, rd);
        }

        // sf/sz/C are determined purely by the mnemonic (size class from the
        // suffix); Rm must be X for the 64-bit data variants, W otherwise,
        // while Rd/Rn are always the W view (ARMv8 ARM, CRC32/CRC32C).
        #[test]
        fn prop_mnemonic_drives_size_bits(
            mn in valid_mnemonic(),
            rd in 0u32..=30u32, rn in 0u32..=30u32, rm in 0u32..=30u32,
        ) {
            let ops = vec![reg_op(rd, false), reg_op(rn, false), reg_op(rm, mn.ends_with('x'))];
            let w = enc(mn, &ops);
            let (sf, sz, c) = expected(mn);
            prop_assert_eq!((w >> 31) & 1, sf);
            prop_assert_eq!((w >> 10) & 0b11, sz);
            prop_assert_eq!((w >> 12) & 1, c);
        }

        // Malformed-operands negative contract (should pass): fewer than 3
        // operands or a non-register in a fixed slot must yield Err.
        #[test]
        fn prop_rejects_malformed_operands(
            kind in prop_oneof![Just(0u8), Just(1u8), Just(2u8), Just(3u8)],
            n in 0u32..=30u32, v in -16i64..=16i64,
        ) {
            let r = match kind {
                0 => encode_crc32("crc32w", &[]),
                1 => encode_crc32("crc32w", &[reg_op(n, false), Operand::Reg("w1".into())]),
                2 => encode_crc32("crc32w", &[
                    Operand::Imm(v), Operand::Reg("w1".into()), Operand::Reg("w2".into())]),
                _ => encode_crc32("crc32w", &[
                    Operand::Reg("w0".into()), Operand::Reg("w1".into()), Operand::Imm(v)]),
            };
            prop_assert!(r.is_err(), "expected Err, got {:?}", r);
        }
    }

    // Known-constant reference anchor (ARM ARM):
    //   CRC32W W0,W0,W0 = 0x1AC04800 ; CRC32X W0,W0,X0 = 0x9AC04C00.
    // They differ in EXACTLY sf[31] and sz bit0[10] (sz 11 vs 10): 0x8000_0400.
    // (The 64-bit data variants keep the W accumulator; only Rm widens —
    //  the transplanted anchor claimed an X accumulator, corrected against
    //  the ARMv8 ARM and GNU as 2.47: `crc32x w0, w1, x2` -> 0x9ac24c20.)
    #[test]
    fn prop_crc32_known_constants() {
        let w32 = enc(
            "crc32w",
            &[reg_op(0, false), reg_op(0, false), reg_op(0, false)],
        );
        let x64 = enc(
            "crc32x",
            &[reg_op(0, false), reg_op(0, false), reg_op(0, true)],
        );
        assert_eq!(w32, 0x1AC0_4800u32, "CRC32W W0,W0,W0");
        assert_eq!(x64, 0x9AC0_4C00u32, "CRC32X X0,W0,X0");
        assert_eq!(
            w32 ^ x64,
            0x8000_0400u32,
            "CRC32X vs CRC32W differ only in sf[31] and sz bit0 [10]"
        );
    }

    // ── REGRESSION (former bug witness, now active) ───────────────────────────────────────────
    // The AArch64 CRC32 family admits exactly eight mnemonics
    // (crc32{b,h,w,x} and crc32c{b,h,w,x}). An assembler MUST reject any
    // other mnemonic. The encoder's `match` arm `_ => (0, 0b00)` combined
    // with `mnemonic.contains("crc32c")` accepts arbitrary strings ("crc32",
    // "crc32d", "crc32c", "nop", "foo", "") and emits a Word. Asserts the
    // correct contract and FAILS against the current SUT.
    #[test]
    fn prop_rejects_unknown_mnemonics() {
        let cases = [
            "crc32", "crc32d", "crc32y", "crc32c", "crc32cd", "crc32cz", "nop", "foo", "",
        ];
        for c in cases {
            let r = encode_crc32(c, &[reg_op(0, false), reg_op(1, false), reg_op(2, false)]);
            assert!(
                r.is_err(),
                "mnemonic {:?} should be rejected, got {:?}",
                c,
                r
            );
        }
    }

    // ── REGRESSION (former bug witness, now active) ───────────────────────────────────────────
    // ARM ARM (CRC32 / CRC32C) fixes exactly one legal register-width
    // combination per size class:
    //   crc32{b,h,w} / crc32c{b,h,w}: <Wd>, <Wn>, <Wm>   (all three 32-bit)
    //   crc32{x}     / crc32c{x}:     <Wd>, <Wn>, <Xm>   (Rm 64-bit; Rd,Rn 32-bit)
    // Any mnemonic/width mismatch MUST be rejected. The encoder discards
    // EVERY width flag — `let (rd, _)`, `(rn, _)`, `(rm, _)` — and derives sf
    // purely from the mnemonic suffix, so it silently emits a Word for these
    // illegal combinations. Asserts the correct contract and FAILS against
    // the current SUT.
    #[test]
    fn prop_rejects_width_mismatch_per_variant() {
        // [Rd, Rn, Rm] legal widths per variant: true = 64-bit X.
        // Rd and Rn are the 32-bit accumulator/initial-CRC view for EVERY
        // variant; only Rm (the data operand) widens to X for the *x forms
        // (ARMv8 ARM CRC32/CRC32C; GNU as 2.47: `crc32x w0, w1, x2` ->
        // 0x9ac24c20, every other combination is an "operand mismatch").
        // The transplanted witness claimed an X accumulator — corrected.
        let wants: &[(&str, [bool; 3])] = &[
            ("crc32b", [false, false, false]),
            ("crc32h", [false, false, false]),
            ("crc32w", [false, false, false]),
            ("crc32x", [false, false, true]),
            ("crc32cb", [false, false, false]),
            ("crc32ch", [false, false, false]),
            ("crc32cw", [false, false, false]),
            ("crc32cx", [false, false, true]),
        ];
        for &(mn, want) in wants {
            for slot in 0..3usize {
                let mut widths = want;
                widths[slot] = !widths[slot]; // introduce an illegal mismatch
                let ops = vec![
                    reg_op(0, widths[0]),
                    reg_op(1, widths[1]),
                    reg_op(2, widths[2]),
                ];
                let r = encode_crc32(mn, &ops);
                let label = |b: bool| if b { 'X' } else { 'W' };
                assert!(
                    r.is_err(),
                    "{:?} with Rd={}0, Rn={}1, Rm={}2 (slot {} mismatched) must be rejected, got {:?}",
                    mn,
                    label(widths[0]),
                    label(widths[1]),
                    label(widths[2]),
                    slot,
                    r
                );
            }
        }
    }
}
