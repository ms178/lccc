//! Property-based tests for the three AArch64 **sign-extend** encoders in
//! `data_processing.rs`:
//!
//! * `encode_sxtb` — SXTB  `<Rd>,<Rn>`  (alias of SBFM, imms=7)
//! * `encode_sxth` — SXTH  `<Rd>,<Rn>`  (alias of SBFM, imms=15)
//! * `encode_sxtw` — SXTW  `<Xd>,<Wn>`  (alias of SBFM, imms=31, 64-bit only)
//!
//! ## Focus (per task)
//!
//! These encoders resolve operands through the shared `get_reg` /
//! `parse_reg_num` helpers. `parse_reg_num` accepts the FP/SIMD register
//! prefixes `d`/`s`/`q`/`v`/`h`/`b` and maps them to the **same-numbered
//! general-purpose** register; `is_64bit_reg` returns `false` for every one of
//! those prefixes. The encoders never validate the register bank, so an FP/SIMD
//! register supplied in **either** operand position is silently encoded as a
//! scalar SBFM with the lane number dropped into the `Rd`/`Rn` field.
//!
//! SXTB/SXTH/SXTW are scalar general-purpose instructions; FP/SIMD operands are
//! **unallocated**.
//!
//! ## Oracle & contract policy
//!
//! Differential oracles: `clang --target=aarch64-linux-gnu` and GNU as 2.47
//! reject every illegal spelling used below (`error: invalid operand for
//! instruction`), while the valid GP forms assemble cleanly. The encoders
//! implement the fail-closed contract: FP/SIMD operands, X-spelled sources
//! (the ARMv8 ARM spells the source `Wn` for every destination width), and
//! W destinations for SXTW are all rejected with `Err` — the previously
//! `#[ignore]`d witnesses are active regression properties.
//!
//! ```text
//! $ echo 'sxtb d0, d1' | clang --target=aarch64-linux-gnu -c -x assembler -
//! <stdin>:1:6: error: invalid operand for instruction
//! $ echo 'sxtw v0, v1' | clang --target=aarch64-linux-gnu -c -x assembler -
//! <stdin>:1:6: error: invalid operand for instruction
//! ```

use lccc::pbt_internals::Operand;
use lccc::pbt_internals::*;
use proptest::prelude::*;

/// Common function-pointer type for the encoders under test (function items
/// have distinct types, so they are cast to this when selected by index).
type Enc = fn(&[Operand]) -> Result<EncodeResult, String>;

// ═══════════════════════════════════════════════════════════════════════════
//  Field extractors — SBFM layout (SXTB/SXTH/SXTW aliases):
//    sf opc(2)=00 100110 N immr(6) imms(6) Rn(5) Rd(5)
//    bit: 31 | 30:29 | 28:23 | 22 | 21:16 | 15:10 | 9:5 | 4:0
// ═══════════════════════════════════════════════════════════════════════════
fn sbfm_sf(w: u32) -> u32 {
    (w >> 31) & 1
}
fn sbfm_opc(w: u32) -> u32 {
    (w >> 29) & 0x3
} // must be 0b00 (SBFM)
fn sbfm_fixed(w: u32) -> u32 {
    (w >> 23) & 0x3F
} // must be 0b100110
fn sbfm_n(w: u32) -> u32 {
    (w >> 22) & 1
}
fn sbfm_immr(w: u32) -> u32 {
    (w >> 16) & 0x3F
}
fn sbfm_imms(w: u32) -> u32 {
    (w >> 10) & 0x3F
}
fn sbfm_rn(w: u32) -> u32 {
    (w >> 5) & 0x1F
}
fn sbfm_rd(w: u32) -> u32 {
    w & 0x1F
}
// ═══════════════════════════════════════════════════════════════════════════
//  Operand builders & helpers
// ═══════════════════════════════════════════════════════════════════════════
fn xreg(n: u32) -> Operand {
    Operand::Reg(format!("x{}", n))
}
fn wreg(n: u32) -> Operand {
    Operand::Reg(format!("w{}", n))
}

/// FP/SIMD register names accepted by `parse_reg_num` but illegal as GP
/// operands for these instructions: `d`/`s`/`q`/`v`/`h`/`b` + lane number.
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

// ── independent ARMv8 reference words (rebuilt from the bit-strings) ──────
/// SXTW Xd, Wn  ->  SBFM Xd, Xn, #0, #31   (64-bit only: sf=1, N=1, imms=31).
fn sxtw_ref(rd: u32, rn: u32) -> u32 {
    (1u32 << 31) | (0b100110 << 23) | (1 << 22) | (31 << 10) | (rn << 5) | rd
}
/// SBFM reference for a sign-extend-of-N-bits alias:
///   SXTB -> imms=7,  SXTH -> imms=15.
fn sxt_ref(imms: u32, rd: u32, rn: u32, is_64: bool) -> u32 {
    let sf = if is_64 { 1u32 } else { 0 };
    let n = if is_64 { 1u32 } else { 0 };
    (sf << 31) | (0b100110 << 23) | (n << 22) | (imms << 10) | (rn << 5) | rd
}

// ═══════════════════════════════════════════════════════════════════════════
//  HAPPY-PATH ANCHORS (passing by default)
//  Confirm the encoders produce the spec-correct SBFM for valid GP operands,
//  so the field extractors and reference words below are trustworthy.
// ═══════════════════════════════════════════════════════════════════════════
proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    // Anchor: `sxtw x0, w1` is SBFM with sf=1, opc=00, N=1, immr=0, imms=31.
    #[test]
    fn sxtw_happy_path(rd in 0u32..=30, rn in 0u32..=30) {
        let w = word(encode_sxtw(&[xreg(rd), wreg(rn)]));
        prop_assert_eq!(w, sxtw_ref(rd, rn));
        prop_assert_eq!(sbfm_sf(w), 1);
        prop_assert_eq!(sbfm_opc(w), 0b00);
        prop_assert_eq!(sbfm_fixed(w), 0b100110);
        prop_assert_eq!(sbfm_n(w), 1);
        prop_assert_eq!(sbfm_immr(w), 0);
        prop_assert_eq!(sbfm_imms(w), 31);
        prop_assert_eq!(sbfm_rn(w), rn);
        prop_assert_eq!(sbfm_rd(w), rd);
    }

    // Anchor: `sxtb/sxth Rd, Wn` (32- or 64-bit destination) match the SBFM
    // reference. The ARMv8 ARM spells the source `Wn` for BOTH destination
    // widths (GNU as rejects `sxtb x0, x1` with "operand mismatch").
    #[test]
    fn sxtb_sxth_happy_path(
        rd in 0u32..=30, rn in 0u32..=30, enc_idx in 0u32..2u32, is_64 in any::<bool>(),
    ) {
        let (enc, imms): (Enc, u32) = if enc_idx % 2 == 0 {
            (encode_sxtb as Enc, 7u32)
        } else {
            (encode_sxth as Enc, 15u32)
        };
        let (d, s) = if is_64 { (xreg(rd), wreg(rn)) } else { (wreg(rd), wreg(rn)) };
        let w = word(enc(&[d, s]));
        prop_assert_eq!(w, sxt_ref(imms, rd, rn, is_64));
        prop_assert_eq!(sbfm_sf(w), if is_64 { 1 } else { 0 });
        prop_assert_eq!(sbfm_opc(w), 0b00);
        prop_assert_eq!(sbfm_fixed(w), 0b100110);
        prop_assert_eq!(sbfm_n(w), if is_64 { 1 } else { 0 });
        prop_assert_eq!(sbfm_immr(w), 0);
        prop_assert_eq!(sbfm_imms(w), imms);
        prop_assert_eq!(sbfm_rn(w), rn);
        prop_assert_eq!(sbfm_rd(w), rd);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  FP/SIMD REJECTION REGRESSION (the former bug witnesses, now active:
//  the encoders reject FP/SIMD register names in every operand position
//  instead of placing the lane number into the SBFM fields).
// ═══════════════════════════════════════════════════════════════════════════
proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    // SXTW: an FP/SIMD register in either position is silently accepted; the
    // lane number lands in the operand field, and the word is still 64-bit
    // (sf/N hardcoded to 1, imms=31). Operand width is irrelevant — SXTW
    // ignores get_reg's is_64 flag for both operands.
    #[test]
    fn sxtw_fp_simd_is_rejected(
        n in 0u32..=30, pos in 0u32..2u32, vi in 0u32..=5u32,
    ) {
        // FP/SIMD register spellings must be REJECTED (previously the lane
        // number was placed into the SBFM field like a GP register).
        let pos = pos % 2;
        let mut ops = vec![xreg(n), wreg(n)];
        ops[pos as usize] = fp_variant(vi, n);
        prop_assert!(encode_sxtw(&ops).is_err(),
            "sxtw with FP/SIMD at position {pos} must be rejected");
    }

    // SXTB/SXTH: an FP/SIMD register in either position is silently accepted;
    // the lane number lands in the operand field, and the result is a 32-bit
    // SBFM (is_64bit_reg is false for d/s/q/v/h/b).
        fn sxtb_sxth_fp_simd_is_rejected(
        n in 0u32..=30, enc_idx in 0u32..2u32, pos in 0u32..2u32, vi in 0u32..=5u32,
    ) {
        // FP/SIMD register spellings must be REJECTED in sxtb/sxth operands
        // (previously the lane number was placed into the SBFM field).
        let (enc, _imms): (Enc, u32) = if enc_idx % 2 == 0 {
            (encode_sxtb as Enc, 7u32)
        } else {
            (encode_sxth as Enc, 15u32)
        };
        let pos = pos % 2;
        let mut ops = vec![wreg(n), wreg(n)];
        ops[pos as usize] = fp_variant(vi, n);
        prop_assert!(enc(&ops).is_err(),
            "sxtb/sxth with FP/SIMD at position {pos} must be rejected");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  WIDTH-CONTRACT REGRESSION — X-spelled sources and W SXTW destinations
//  are rejected per the ARMv8 ARM spellings (GNU as: "operand mismatch").
// ═══════════════════════════════════════════════════════════════════════════
proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn wit_sxtw_rejects_fp_simd_in_any_position(
        n in 0u32..=30, pos in 0u32..2u32, vi in 0u32..=5u32,
    ) {
        let pos = pos % 2;
        let mut ops = vec![xreg(n), wreg(n)];
        ops[pos as usize] = fp_variant(vi, n);
        prop_assert!(encode_sxtw(&ops).is_err(),
            "sxtw with FP/SIMD at position {} must be Err, got {:?}", pos, encode_sxtw(&ops));
    }

    #[test]
    fn wit_sxtb_rejects_fp_simd_in_any_position(
        n in 0u32..=30, pos in 0u32..2u32, vi in 0u32..=5u32,
    ) {
        let pos = pos % 2;
        let mut ops = vec![wreg(n), wreg(n)];
        ops[pos as usize] = fp_variant(vi, n);
        prop_assert!(encode_sxtb(&ops).is_err(),
            "sxtb with FP/SIMD at position {} must be Err, got {:?}", pos, encode_sxtb(&ops));
    }

    #[test]
    fn wit_sxth_rejects_fp_simd_in_any_position(
        n in 0u32..=30, pos in 0u32..2u32, vi in 0u32..=5u32,
    ) {
        let pos = pos % 2;
        let mut ops = vec![wreg(n), wreg(n)];
        ops[pos as usize] = fp_variant(vi, n);
        prop_assert!(encode_sxth(&ops).is_err(),
            "sxth with FP/SIMD at position {} must be Err, got {:?}", pos, encode_sxth(&ops));
    }

    #[test]
    fn sxtw_rejects_x_spelled_source(n in 0u32..=30) {
        // The ARMv8 ARM spells the SXTW source `Wn`; GNU as rejects the X
        // spelling with "operand mismatch", so it is diagnosed, not re-banked.
        prop_assert!(encode_sxtw(&[xreg(n), xreg(n)]).is_err(),
            "sxtw x{n}, x{n} must be rejected; spell the source w{n}");
    }

    #[test]
    fn sxtw_rejects_w_destination(n in 0u32..=30) {
        prop_assert!(encode_sxtw(&[wreg(n), wreg(n)]).is_err(),
            "sxtw has no 32-bit destination form");
    }

    #[test]
    fn sxtb_sxth_rejects_x_spelled_source(
        n in 0u32..=30, enc_idx in 0u32..2u32, is_64 in any::<bool>(),
    ) {
        let (enc, name): (Enc, &str) = if enc_idx % 2 == 0 {
            (encode_sxtb as Enc, "sxtb")
        } else {
            (encode_sxth as Enc, "sxth")
        };
        let d = if is_64 { xreg(n) } else { wreg(n) };
        prop_assert!(enc(&[d, xreg(n)]).is_err(),
            "{name} source must be spelled w{n} (GNU as rejects the x form)");
    }
}
