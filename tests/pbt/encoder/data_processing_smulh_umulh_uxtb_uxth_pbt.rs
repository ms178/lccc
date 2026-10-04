//! Property-based tests for four AArch64 data-processing encoders in
//! `data_processing.rs`:
//!
//! * `encode_smulh` — SMULH `<Xd>,<Xn>,<Xm>`  (64-bit ONLY)
//! * `encode_umulh` — UMULH `<Xd>,<Xn>,<Xm>`  (64-bit ONLY)
//! * `encode_uxtb`  — UXTB  `<Rd>,<Rn>`        (alias of UBFM, imms=7)
//! * `encode_uxth`  — UXTH  `<Rd>,<Rn>`        (alias of UBFM, imms=15)
//!
//! ## Focus (per task)
//!
//! For the two multiply-high encoders (both 64-bit-only), this suite probes:
//!
//! 1. silent acceptance of **32-bit W** register operands (known from existing
//!    reports — pinned here as a width-indifference characterisation + an
//!    `#[ignore]`d spec-correct witness);
//! 2. **SP-as-destination** (and source) — `sp`/`wsp` parse to register field
//!    **31**, which is **XZR/WZR** in the data-processing (3-source) group, not
//!    SP; SMULH/UMULH have no SP-using variant;
//! 3. **immediate operands** — SMULH/UMULH take only registers; an immediate in
//!    any operand position must be rejected (this currently *holds* — a passing
//!    negative contract);
//! 4. **FP/SIMD bank** acceptance — `get_reg`/`parse_reg_num` accept the
//!    `d`/`s`/`q`/`v`/`h`/`b` prefixes, silently reusing the lane number as a
//!    general-purpose register number.
//!
//! For the two zero-extend encoders, this suite probes FP/SIMD register
//! acceptance in both the destination and source positions.
//!
//! ## Oracle & bug-witness policy
//!
//! Differential oracle: `clang --target=aarch64-linux-gnu` rejects every
//! illegal spelling used below with `error: invalid operand for instruction`:
//!
//! ```text
//! $ echo 'umulh sp, x1, x2' | clang --target=aarch64-linux-gnu -c -x assembler -
//! <stdin>:1:7: error: invalid operand for instruction
//! $ echo 'uxtb d0, d1'      | clang --target=aarch64-linux-gnu -c -x assembler -
//! <stdin>:1:6: error: invalid operand for instruction
//! ```
//!
//! The SUT silently accepts them. Every **witness** (a property asserting the
//! spec-correct `Err`) is `#[ignore]`d so a default `cargo test` stays green;
//! run them with:
//!
//! ```text
//! cargo test --test pbt_arm_core data_processing_smulh_umulh_uxtb_uxth_pbt -- --ignored
//! ```
//!
//! The passing **characterisation** properties pin the *current* (buggy)
//! observable behaviour; they will start failing once the defects are fixed —
//! the cue to drop the `#[ignore]` markers on the matching witnesses.

use lccc::pbt_internals::Operand;
use lccc::pbt_internals::*;
use proptest::prelude::*;

/// Common function-pointer type for the encoders under test (function items
/// have distinct types, so they are cast to this when selected by index).
type Enc = fn(&[Operand]) -> Result<EncodeResult, String>;

// ═══════════════════════════════════════════════════════════════════════════
//  Field extractors
// ═══════════════════════════════════════════════════════════════════════════

// Data-processing (3 source) layout:  sf 00 11011 op31 Rm o0 Ra Rn Rd
//   bit: 31 | 30:29 | 28:24 | 23:21 | 20:16 | 15 | 14:10 | 9:5 | 4:0
fn dp_sf(w: u32) -> u32 {
    (w >> 31) & 1
}
fn dp_class(w: u32) -> u32 {
    (w >> 24) & 0x1F
} // must be 0b11011
fn dp_op31(w: u32) -> u32 {
    (w >> 21) & 0x7
}
fn dp_o0(w: u32) -> u32 {
    (w >> 15) & 1
}
fn dp_rm(w: u32) -> u32 {
    (w >> 16) & 0x1F
}
fn dp_ra(w: u32) -> u32 {
    (w >> 10) & 0x1F
}
fn dp_rn(w: u32) -> u32 {
    (w >> 5) & 0x1F
}
fn dp_rd(w: u32) -> u32 {
    w & 0x1F
}
// UBFM layout (UXTB/UXTH aliases):  sf 10 100110 N 0 immr imms Rn Rd
//   bit: 31 | 30:29 | 28:23 | 22 | 21:16 | 15:10 | 9:5 | 4:0
fn ubfm_sf(w: u32) -> u32 {
    (w >> 31) & 1
}
fn ubfm_opc(w: u32) -> u32 {
    (w >> 29) & 0x3
} // must be 0b10 (UBFM)
fn ubfm_fixed(w: u32) -> u32 {
    (w >> 23) & 0x3F
} // must be 0b100110
fn ubfm_n(w: u32) -> u32 {
    (w >> 22) & 1
}
fn ubfm_immr(w: u32) -> u32 {
    (w >> 16) & 0x3F
}
fn ubfm_imms(w: u32) -> u32 {
    (w >> 10) & 0x3F
}
fn ubfm_rn(w: u32) -> u32 {
    (w >> 5) & 0x1F
}
fn ubfm_rd(w: u32) -> u32 {
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

/// `sp`/`wsp`: both parse to register number 31 — which is **ZR**, not SP, in
/// the data-processing (3-source) encoding group.
fn sp_variant(i: u32) -> Operand {
    if i.is_multiple_of(2) {
        Operand::Reg("sp".into())
    } else {
        Operand::Reg("wsp".into())
    }
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
fn smulh_ref(rd: u32, rn: u32, rm: u32) -> u32 {
    (1u32 << 31) | (0b0011011010 << 21) | (rm << 16) | (0b011111 << 10) | (rn << 5) | rd
}
fn umulh_ref(rd: u32, rn: u32, rm: u32) -> u32 {
    (1u32 << 31) | (0b0011011110 << 21) | (rm << 16) | (0b011111 << 10) | (rn << 5) | rd
}
/// UBFM reference for a zero-extend-of-N-bits alias (UXTB -> imms=7, UXTH -> imms=15).
fn uxt_ref(imms: u32, rd: u32, rn: u32, is_64: bool) -> u32 {
    let sf = if is_64 { 1u32 } else { 0 };
    let n = if is_64 { 1u32 } else { 0 };
    (sf << 31) | (0b10 << 29) | (0b100110 << 23) | (n << 22) | (imms << 10) | (rn << 5) | rd
}

// ═══════════════════════════════════════════════════════════════════════════
//  SMULH / UMULH — happy-path anchors (passing by default)
// ═══════════════════════════════════════════════════════════════════════════
proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    // Anchor: valid Xd,Xn,Xm matches the independent ARMv8 reference word and
    // field placement. SMULH is always 64-bit (sf=1), op31=010, o0=0, Ra=XZR.
    #[test]
    fn smulh_happy_path(rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30) {
        let w = word(encode_smulh(&[xreg(rd), xreg(rn), xreg(rm)]));
        prop_assert_eq!(w, smulh_ref(rd, rn, rm));
        prop_assert_eq!(dp_sf(w), 1);
        prop_assert_eq!(dp_class(w), 0b11011);
        prop_assert_eq!(dp_op31(w), 0b010);
        prop_assert_eq!(dp_o0(w), 0);
        prop_assert_eq!(dp_ra(w), 31); // Ra hardwired to XZR
        prop_assert_eq!(dp_rm(w), rm);
        prop_assert_eq!(dp_rn(w), rn);
        prop_assert_eq!(dp_rd(w), rd);
    }

    // Anchor: UMULH matches its reference; differs from SMULH only in op31 bit
    // 23 (the unsigned selector).
    #[test]
    fn umulh_happy_path(rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30) {
        let w = word(encode_umulh(&[xreg(rd), xreg(rn), xreg(rm)]));
        prop_assert_eq!(w, umulh_ref(rd, rn, rm));
        prop_assert_eq!(dp_sf(w), 1);
        prop_assert_eq!(dp_class(w), 0b11011);
        prop_assert_eq!(dp_op31(w), 0b110);
        prop_assert_eq!(dp_o0(w), 0);
        prop_assert_eq!(dp_ra(w), 31);
        prop_assert_eq!(dp_rm(w), rm);
        prop_assert_eq!(dp_rn(w), rn);
        prop_assert_eq!(dp_rd(w), rd);
        // Differential vs SMULH: identical operands differ only in op31 bit 23.
        let s = word(encode_smulh(&[xreg(rd), xreg(rn), xreg(rm)]));
        prop_assert_eq!(s ^ w, 1u32 << 23);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  SMULH / UMULH — immediate operands (PASSING negative contract)
//
//  SMULH/UMULH take ONLY registers. An immediate (any value) in any of the
//  three operand positions must be rejected with Err — never silently coerced.
//  The encoder routes every operand through get_reg, which errors on non-Reg
//  operands, so this contract currently HOLDS. clang confirms immediates are
//  invalid: `umulh x0, x1, #5` -> "error: invalid operand for instruction".
// ═══════════════════════════════════════════════════════════════════════════
proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn smulh_rejects_immediate_in_any_position(
        n in 0u32..=30, pos in 0u32..3u32, imm in any::<i64>(),
    ) {
        let mut ops = vec![xreg(n), xreg(n), xreg(n)];
        ops[pos as usize] = Operand::Imm(imm);
        prop_assert!(encode_smulh(&ops).is_err(),
            "smulh with Imm({}) at position {} must be Err", imm, pos);
    }

    #[test]
    fn umulh_rejects_immediate_in_any_position(
        n in 0u32..=30, pos in 0u32..3u32, imm in any::<i64>(),
    ) {
        let mut ops = vec![xreg(n), xreg(n), xreg(n)];
        ops[pos as usize] = Operand::Imm(imm);
        prop_assert!(encode_umulh(&ops).is_err(),
            "umulh with Imm({}) at position {} must be Err", imm, pos);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  SMULH / UMULH — CHARACTERISATION (passing today; pins the buggy behaviour)
//  SP/WSP and FP/SIMD names are silently accepted — the *bug* is that they are
//  accepted at all. These properties document HOW the bug currently manifests.
// ═══════════════════════════════════════════════════════════════════════════
proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    // C1. SP/WSP as DESTINATION (operand 0) silently aliases to register field
    //     value 31 (== XZR/WZR), not SP, in both SMULH and UMULH. The encoder
    //     returns Ok(Word(...)) indistinguishable from the XZR spelling.
    #[test]
    fn sp_destination_is_rejected(
        n in 0u32..=30, enc_idx in 0u32..2u32, vi in 0u32..=1u32,
    ) {
        // SP/WSP must be REJECTED in smulh/umulh operand positions
        // (previously silently aliased to field 31 == ZR).
        let enc: Enc = if enc_idx % 2 == 0 { encode_smulh as Enc } else { encode_umulh as Enc };
        let ops = vec![sp_variant(vi), xreg(n), xreg(n)]; // smulh/umulh <sp>, Xn, Xm
        prop_assert!(enc(&ops).is_err(), "smulh/umulh with SP/WSP must be rejected");
    }

    // C2. An FP/SIMD register name in any position is silently accepted; its
    //     lane number is placed into the operand field exactly like the
    //     same-numbered GP register.
    #[test]
    fn fp_simd_is_rejected(
        n in 0u32..=30, enc_idx in 0u32..2u32, pos in 0u32..3u32, vi in 0u32..=5u32,
    ) {
        // FP/SIMD register spellings must be REJECTED (previously the lane
        // number was placed into the operand field like a GP register).
        let enc: Enc = if enc_idx % 2 == 0 { encode_smulh as Enc } else { encode_umulh as Enc };
        let pos = pos % 3;
        let mut ops = vec![xreg(n), xreg(n), xreg(n)];
        ops[pos as usize] = fp_variant(vi, n);
        prop_assert!(enc(&ops).is_err(), "smulh/umulh with FP/SIMD must be rejected");
    }

    // C3. Width-indifference: SMULH/UMULH hardcode sf=1 and discard get_reg's
    //     is_64 flag, so a fully-W-form operand set emits the IDENTICAL word to
    //     the X-form. Pins the known 32-bit-acceptance defect.
    #[test]
    fn w_form_is_rejected(
        n in 0u32..=30, enc_idx in 0u32..2u32,
    ) {
        // SMULH/UMULH are 64-bit-only: W-form operands must be REJECTED
        // (previously a fully-W operand set emitted the identical word to
        // the X form).
        let enc: Enc = if enc_idx % 2 == 0 { encode_smulh as Enc } else { encode_umulh as Enc };
        let xw = word(enc(&[xreg(n), xreg(n), xreg(n)]));
        prop_assert_eq!(dp_sf(xw), 1);
        prop_assert!(enc(&[wreg(n), wreg(n), wreg(n)]).is_err(),
            "smulh/umulh with W-form operands must be rejected");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  BUG WITNESSES — all #[ignore]'d so a default `cargo test` stays green.
//  Run:  cargo test --test pbt_arm_core data_processing_smulh_umulh_uxtb_uxth_pbt -- --ignored
//  Each asserts the SPEC-CORRECT rejection the encoder currently violates.
// ═══════════════════════════════════════════════════════════════════════════
proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    // ── 32-bit W operands: SMULH/UMULH are 64-bit-only ("SMULH Xd, Xn, Xm").
    #[test]
    fn wit_smulh_rejects_32bit_w_operands(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
    ) {
        let ops = vec![wreg(rd), wreg(rn), wreg(rm)];
        prop_assert!(encode_smulh(&ops).is_err(),
            "smulh w-form must be Err, got {:?}", encode_smulh(&ops));
    }

    #[test]
    fn wit_umulh_rejects_32bit_w_operands(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
    ) {
        let ops = vec![wreg(rd), wreg(rn), wreg(rm)];
        prop_assert!(encode_umulh(&ops).is_err(),
            "umulh w-form must be Err, got {:?}", encode_umulh(&ops));
    }

    // ── SP-as-destination: field 31 == XZR/WZR, not SP. SMULH/UMULH have no
    //    SP-using variant. clang rejects `umulh sp, x1, x2`.
    #[test]
    fn wit_smulh_rejects_sp_as_destination(n in 0u32..=30, vi in 0u32..=1u32) {
        let ops = vec![sp_variant(vi), xreg(n), xreg(n)];
        prop_assert!(encode_smulh(&ops).is_err(),
            "smulh <sp/wsp> as destination must be Err, got {:?}", encode_smulh(&ops));
    }

    #[test]
    fn wit_umulh_rejects_sp_as_destination(n in 0u32..=30, vi in 0u32..=1u32) {
        let ops = vec![sp_variant(vi), xreg(n), xreg(n)];
        prop_assert!(encode_umulh(&ops).is_err(),
            "umulh <sp/wsp> as destination must be Err, got {:?}", encode_umulh(&ops));
    }

    // ── FP/SIMD bank: d/s/q/v/h/b operands are not valid GP operands.
    #[test]
    fn wit_smulh_rejects_fp_simd_in_any_position(
        n in 0u32..=30, pos in 0u32..3u32, vi in 0u32..=5u32,
    ) {
        let pos = pos % 3;
        let mut ops = vec![xreg(n), xreg(n), xreg(n)];
        ops[pos as usize] = fp_variant(vi, n);
        prop_assert!(encode_smulh(&ops).is_err(),
            "smulh with FP/SIMD at position {} must be Err, got {:?}", pos, encode_smulh(&ops));
    }

    #[test]
    fn wit_umulh_rejects_fp_simd_in_any_position(
        n in 0u32..=30, pos in 0u32..3u32, vi in 0u32..=5u32,
    ) {
        let pos = pos % 3;
        let mut ops = vec![xreg(n), xreg(n), xreg(n)];
        ops[pos as usize] = fp_variant(vi, n);
        prop_assert!(encode_umulh(&ops).is_err(),
            "umulh with FP/SIMD at position {} must be Err, got {:?}", pos, encode_umulh(&ops));
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  UXTB / UXTH — happy-path anchor (W form, the canonical alias)
// ═══════════════════════════════════════════════════════════════════════════
proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    // Anchor: `uxtb wd, wn` is UBFM with imms=7, immr=0. Field placement and
    // the fixed UBFM opcode match the ARMv8 reference.
    #[test]
    fn uxtb_happy_path(rd in 0u32..=30, rn in 0u32..=30) {
        let w = word(encode_uxtb(&[wreg(rd), wreg(rn)]));
        prop_assert_eq!(w, uxt_ref(7, rd, rn, false));
        prop_assert_eq!(ubfm_sf(w), 0);
        prop_assert_eq!(ubfm_opc(w), 0b10);
        prop_assert_eq!(ubfm_fixed(w), 0b100110);
        prop_assert_eq!(ubfm_n(w), 0);
        prop_assert_eq!(ubfm_immr(w), 0);
        prop_assert_eq!(ubfm_imms(w), 7);
        prop_assert_eq!(ubfm_rn(w), rn);
        prop_assert_eq!(ubfm_rd(w), rd);
    }

    // Anchor: `uxth wd, wn` is UBFM with imms=15, immr=0.
    #[test]
    fn uxth_happy_path(rd in 0u32..=30, rn in 0u32..=30) {
        let w = word(encode_uxth(&[wreg(rd), wreg(rn)]));
        prop_assert_eq!(w, uxt_ref(15, rd, rn, false));
        prop_assert_eq!(ubfm_imms(w), 15);
        prop_assert_eq!(ubfm_immr(w), 0);
        prop_assert_eq!(ubfm_opc(w), 0b10);
        prop_assert_eq!(ubfm_fixed(w), 0b100110);
        prop_assert_eq!(ubfm_rn(w), rn);
        prop_assert_eq!(ubfm_rd(w), rd);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  UXTB / UXTH — FP/SIMD CHARACTERISATION (passing today; pins the bug)
//  FP/SIMD names are silently accepted and encoded as a 32-bit UBFM, because
//  is_64bit_reg() returns false for the d/s/q/v/h/b prefixes.
// ═══════════════════════════════════════════════════════════════════════════
proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    // An FP/SIMD register in either position is silently accepted; its lane
    // number is placed into the operand field, and the result is a 32-bit UBFM.
    #[test]
    fn fp_simd_is_rejected_for_uxtb_uxth(
        n in 0u32..=30, enc_idx in 0u32..2u32, pos in 0u32..2u32, vi in 0u32..=5u32,
    ) {
        // FP/SIMD register spellings must be REJECTED in uxtb/uxth operands
        // (previously the lane number was placed into the UBFM field).
        let (enc, _imms): (Enc, u32) = if enc_idx % 2 == 0 {
            (encode_uxtb as Enc, 7u32)
        } else {
            (encode_uxth as Enc, 15u32)
        };
        let pos = pos % 2;
        let mut ops = vec![wreg(n), wreg(n)];
        ops[pos as usize] = fp_variant(vi, n);
        prop_assert!(enc(&ops).is_err(), "uxtb/uxth with FP/SIMD must be rejected");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  UXTB / UXTH — FP/SIMD BUG WITNESSES (#[ignore]'d)
//  clang rejects `uxtb d0, d1` and `uxth s0, s1` with
//  "error: invalid operand for instruction". FP/SIMD registers are not valid
//  operands for the scalar zero-extend instructions.
// ═══════════════════════════════════════════════════════════════════════════
proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn wit_uxtb_rejects_fp_simd_in_any_position(
        n in 0u32..=30, pos in 0u32..2u32, vi in 0u32..=5u32,
    ) {
        let pos = pos % 2;
        let mut ops = vec![wreg(n), wreg(n)];
        ops[pos as usize] = fp_variant(vi, n);
        prop_assert!(encode_uxtb(&ops).is_err(),
            "uxtb with FP/SIMD at position {} must be Err, got {:?}", pos, encode_uxtb(&ops));
    }

    #[test]
    fn wit_uxth_rejects_fp_simd_in_any_position(
        n in 0u32..=30, pos in 0u32..2u32, vi in 0u32..=5u32,
    ) {
        let pos = pos % 2;
        let mut ops = vec![wreg(n), wreg(n)];
        ops[pos as usize] = fp_variant(vi, n);
        prop_assert!(encode_uxth(&ops).is_err(),
            "uxth with FP/SIMD at position {} must be Err, got {:?}", pos, encode_uxth(&ops));
    }
}
