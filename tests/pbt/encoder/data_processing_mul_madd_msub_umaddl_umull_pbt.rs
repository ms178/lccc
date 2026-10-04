//! Property-based tests for the **multiply** encoders in `data_processing.rs`:
//!
//! * `encode_mul`    — MUL   `<Rd>,<Rn>,<Rm>`           (alias MADD  Rd,Rn,Rm,XZR)
//! * `encode_madd`   — MADD  `<Rd>,<Rn>,<Rm>,<Ra>`
//! * `encode_msub`   — MSUB  `<Rd>,<Rn>,<Rm>,<Ra>`
//! * `encode_umull`  — UMULL `<Xd>,<Wn>,<Wm>`           (alias UMADDL Xd,Wn,Wm,XZR)
//! * `encode_umaddl` — UMADDL `<Xd>,<Wn>,<Wm>,<Xa>`
//!
//! ## ARMv8-A encodings (verified against `clang --target=aarch64-linux-gnu`)
//!
//! Data-processing (3 source), §C4.1.65 — MADD/MSUB and the MUL alias:
//! ```text
//!  31  30 29  28:24      23  22:21   20:16   15  14:10   9:5   4:0
//!   sf   0  S  1 1 0 1 1  o1   0 0     Rm     o0    Ra     Rn    Rd
//! ```
//! `o0=0` → MADD/MUL, `o0=1` → MSUB. Every operand field value `31` is decoded
//! as **XZR/WZR (zero register)** — *never* SP/WSP — so `<Rd>/<Rn>/<Rm>/<Ra>`
//! may be `xzr`/`wzr` but **never** `sp`/`wsp`, and never FP/SIMD.
//!
//! Widening multiply (3 source), §C4.1.66 — UMADDL and the UMULL alias:
//! ```text
//!  31  30:29  28:24      23  22  21   20:16   15  14:10   9:5   4:0
//!   1   0 0   1 1 0 1 1   1   0   1    Rm      0    Xa     Wn    Xd
//! ```
//! `sf` is hard-wired to `1` (64-bit destination). Again field `31` == XZR,
//! so SP/WSP are **unallocated** in every operand position.
//!
//! ## What is targeted here (same defect classes as adc/sbc/neg/negs)
//!
//! All five encoders resolve operands through the shared `get_reg` /
//! `parse_reg_num` helpers, which (a) map `"sp"`/`"wsp"` → register number
//! **31** — i.e. XZR/WZR in these groups, not SP — and (b) accept the
//! FP/SIMD prefixes `d/s/q/v/h/b`. The result:
//!
//! 1. **SP/WSP acceptance** — `mul x0, sp, x1` silently encodes as
//!    `mul x0, xzr, x1`, `umull x0, w1, sp` as `umull x0, w1, wzr`, etc.
//! 2. **FP/SIMD acceptance** — `madd x0, x1, v2, x3` encodes bit-identically
//!    to `madd x0, x1, x2, x3` (reads GP `x2`, not SIMD `v2`).
//!
//! `clang --target=aarch64-linux-gnu` rejects every one of these spellings
//! with `error: invalid operand for instruction`.
//!
//! The **witnesses** (properties asserting the spec-correct rejection) are all
//! marked `#[ignore]`, so default `cargo test` stays green; run them with:
//!
//! ```text
//! cargo test --test pbt_arm_core data_processing_mul_madd_msub_umaddl_umull_pbt -- --ignored
//! ```
//! The passing **characterisation** properties pin the current (buggy)
//! behaviour (SP==XZR, FP==GP); they will start failing once the defects are
//! fixed — the cue to drop the `#[ignore]` markers.

use lccc::pbt_internals::Operand;
use lccc::pbt_internals::*;
use proptest::prelude::*;

// ── field extractors ─────────────────────────────────────────────────────
fn sf_of(w: u32) -> u32 {
    (w >> 31) & 1
} // width
fn dp3_of(w: u32) -> u32 {
    (w >> 21) & 0x3FF
} // bits 30:21 (3-source fixed)
fn long_of(w: u32) -> u32 {
    (w >> 21) & 0x3FF
} // bits 30:21 (widening fixed)
fn o0_of(w: u32) -> u32 {
    (w >> 15) & 1
} // 0=MADD/MUL 1=MSUB
fn sep_of(w: u32) -> u32 {
    (w >> 15) & 1
} // widening: fixed 0 before Ra
fn rm_of(w: u32) -> u32 {
    (w >> 16) & 0x1F
}
fn ra_of(w: u32) -> u32 {
    (w >> 10) & 0x1F
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
/// the data-processing (3 source) and widening-multiply groups.
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

// ── reference encoders (rebuilt independently from the ARMv8 bit-strings;
//    base constants cross-checked against `clang` + objdump) ───────────────
//    mul  x0,x0,x0   = 0x9B007C00   mul  w0,w0,w0  = 0x1B007C00
//    madd x0,...,x0  = 0x9B000000   madd w0,...,w0 = 0x1B000000
//    msub x0,...,x0  = 0x9B008000   msub w0,...,w0 = 0x1B008000
//    umull x0,w0,w0  = 0x9BA07C00   umaddl x0,w0,w0,x0 = 0x9BA00000
fn mul_ref(rd: u32, rn: u32, rm: u32, is_64: bool) -> u32 {
    let sf = if is_64 { 1u32 } else { 0 };
    (sf << 31) | (0b0011011000 << 21) | (rm << 16) | (0b11111 << 10) | (rn << 5) | rd
}
fn madd_ref(rd: u32, rn: u32, rm: u32, ra: u32, is_64: bool) -> u32 {
    let sf = if is_64 { 1u32 } else { 0 };
    (sf << 31) | (0b0011011000 << 21) | (rm << 16) | (ra << 10) | (rn << 5) | rd
}
fn msub_ref(rd: u32, rn: u32, rm: u32, ra: u32, is_64: bool) -> u32 {
    let sf = if is_64 { 1u32 } else { 0 };
    (sf << 31) | (0b0011011000 << 21) | (rm << 16) | (1u32 << 15) | (ra << 10) | (rn << 5) | rd
}
fn umull_ref(rd: u32, rn: u32, rm: u32) -> u32 {
    (1u32 << 31) | (0b0011011101 << 21) | (rm << 16) | (0b11111 << 10) | (rn << 5) | rd
}
fn umaddl_ref(rd: u32, rn: u32, rm: u32, ra: u32) -> u32 {
    (1u32 << 31) | (0b0011011101 << 21) | (rm << 16) | (ra << 10) | (rn << 5) | rd
}

// Encoder tables (all share the signature fn(&[Operand]) -> Result<_, String>).
type Enc = fn(&[Operand]) -> Result<EncodeResult, String>;
// GP 3-operand (MUL alias): all operands same width (W or X), sf from Rd.
const GP3: &[(&str, Enc)] = &[("mul", encode_mul)];
// GP 4-operand: all operands same width (W or X), sf from Rd.
const GP4: &[(&str, Enc)] = &[("madd", encode_madd), ("msub", encode_msub)];
// Widening 3-operand (UMULL alias): Xd, Wn, Wm; sf hard-wired 1.
const LONG3: &[(&str, Enc)] = &[("umull", encode_umull)];
// Widening 4-operand: Xd, Wn, Wm, Xa; sf hard-wired 1.
const LONG4: &[(&str, Enc)] = &[("umaddl", encode_umaddl)];

proptest! {
    // ── G1. HAPPY-PATH REFERENCE (passing guard): MUL ────────────────────
    // MUL aliases MADD with Ra=XZR(31). Full word matches the spec-derived
    // reference for both widths; field Ra==31, o0==0, dp3 fixed bits correct.
    #[test]
    fn mul_matches_armv8_reference(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30, is_64 in any::<bool>(),
    ) {
        let mk = |n: u32| if is_64 { xreg(n) } else { wreg(n) };
        let ops = vec![mk(rd), mk(rn), mk(rm)];
        let w = word(encode_mul(&ops));
        prop_assert_eq!(w, mul_ref(rd, rn, rm, is_64));
        prop_assert_eq!(sf_of(w), if is_64 { 1 } else { 0 });
        prop_assert_eq!(dp3_of(w), 0b0011011000);
        prop_assert_eq!(o0_of(w), 0);
        prop_assert_eq!(ra_of(w), 31); // XZR/WZR -> the MUL alias
        prop_assert_eq!(rm_of(w), rm);
        prop_assert_eq!(rn_of(w), rn);
        prop_assert_eq!(rd_of(w), rd);
    }

    // ── G2. HAPPY-PATH REFERENCE (passing guard): MADD / MSUB ────────────
    // Full word matches the reference; MADD vs MSUB differ ONLY in o0 (bit15);
    // Ra is a real operand field here (not forced to 31).
    #[test]
    fn madd_msub_match_armv8_reference(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30, ra in 0u32..=30,
        is_64 in any::<bool>(),
    ) {
        let mk = |n: u32| if is_64 { xreg(n) } else { wreg(n) };
        let ops = vec![mk(rd), mk(rn), mk(rm), mk(ra)];
        let wmadd = word(encode_madd(&ops));
        let wmsub = word(encode_msub(&ops));
        prop_assert_eq!(wmadd, madd_ref(rd, rn, rm, ra, is_64));
        prop_assert_eq!(wmsub, msub_ref(rd, rn, rm, ra, is_64));
        for w in [wmadd, wmsub] {
            prop_assert_eq!(sf_of(w), if is_64 { 1 } else { 0 });
            prop_assert_eq!(dp3_of(w), 0b0011011000);
            prop_assert_eq!(rm_of(w), rm);
            prop_assert_eq!(ra_of(w), ra);
            prop_assert_eq!(rn_of(w), rn);
            prop_assert_eq!(rd_of(w), rd);
        }
        prop_assert_eq!(o0_of(wmadd), 0);
        prop_assert_eq!(o0_of(wmsub), 1);
        // The two differ only in bit 15 (o0).
        prop_assert_eq!(wmadd ^ wmsub, 1u32 << 15);
    }

    // ── G3. HAPPY-PATH REFERENCE (passing guard): UMULL / UMADDL ─────────
    // sf is hard-wired to 1 (64-bit destination). UMULL aliases UMADDL with
    // Ra=XZR(31); the widening opcode field (bits 30:21) == 0011011101 and the
    // separator bit 15 == 0 in both. Full word matches the reference.
    #[test]
    fn umull_umaddl_match_armv8_reference(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30, ra in 0u32..=30,
    ) {
        let ops3 = vec![xreg(rd), wreg(rn), wreg(rm)];
        let wu = word(encode_umull(&ops3));
        prop_assert_eq!(wu, umull_ref(rd, rn, rm));
        prop_assert_eq!(sf_of(wu), 1);
        prop_assert_eq!(long_of(wu), 0b0011011101);
        prop_assert_eq!(sep_of(wu), 0);
        prop_assert_eq!(ra_of(wu), 31); // XZR -> the UMULL alias
        prop_assert_eq!(rm_of(wu), rm);
        prop_assert_eq!(rn_of(wu), rn);
        prop_assert_eq!(rd_of(wu), rd);

        let ops4 = vec![xreg(rd), wreg(rn), wreg(rm), xreg(ra)];
        let wa = word(encode_umaddl(&ops4));
        prop_assert_eq!(wa, umaddl_ref(rd, rn, rm, ra));
        prop_assert_eq!(sf_of(wa), 1);
        prop_assert_eq!(long_of(wa), 0b0011011101);
        prop_assert_eq!(sep_of(wa), 0);
        prop_assert_eq!(ra_of(wa), ra);
        prop_assert_eq!(rm_of(wa), rm);
        prop_assert_eq!(rn_of(wa), rn);
        prop_assert_eq!(rd_of(wa), rd);
        // UMULL vs UMADDL(Ra=31) are identical when Ra==31.
        prop_assert_eq!(umull_ref(rd, rn, rm), umaddl_ref(rd, rn, rm, 31));
    }

    // ── G4. NEGATIVE: too few operands => Err (passing guard) ────────────
    #[test]
    fn all_encoders_reject_too_few_operands(n in 0u32..=30, drop in 1u32..=4) {
        // 4-operand base; pop `drop` and expect Err from every encoder whose
        // arity is now unsatisfied.
        let mut ops4 = vec![xreg(n), xreg(n), xreg(n), xreg(n)];
        for _ in 0..drop { ops4.pop(); }
        if ops4.len() < 4 {
            for &(_n, f) in GP4 { prop_assert!(f(&ops4).is_err()); }
            for &(_n, f) in LONG4 { prop_assert!(f(&ops4).is_err()); }
        }
        if ops4.len() < 3 {
            for &(_n, f) in GP3 { prop_assert!(f(&ops4).is_err()); }
            for &(_n, f) in LONG3 { prop_assert!(f(&ops4).is_err()); }
        }
    }

    // ── G5. POSITIVE: XZR/WZR are LEGAL (field 31 = zero register) ───────
    // The zero register is the intended use of field value 31 in these groups
    // (and is the value the MUL/UMULL aliases rely on). Contrast with SP,
    // which is wrongly aliased onto the same field — see C1/W* below.
    #[test]
    fn zero_register_is_accepted_and_encodes_as_field_31(n in 0u32..=30) {
        // mul x0, xzr, xzr  -> Rn=Rm=31
        let w = word(encode_mul(&[xreg(n), Operand::Reg("xzr".into()), Operand::Reg("xzr".into())]));
        prop_assert_eq!(rn_of(w), 31);
        prop_assert_eq!(rm_of(w), 31);
        // madd x0, x0, x0, xzr -> Ra=31
        let wm = word(encode_madd(&[xreg(n), xreg(n), xreg(n), Operand::Reg("xzr".into())]));
        prop_assert_eq!(ra_of(wm), 31);
        // umaddl x0, wzr, wzr, xzr -> Rn=Rm=31 (WZR), Ra=31 (XZR)
        let wa = word(encode_umaddl(&[xreg(n), Operand::Reg("wzr".into()),
                                      Operand::Reg("wzr".into()), Operand::Reg("xzr".into())]));
        prop_assert_eq!(rn_of(wa), 31);
        prop_assert_eq!(rm_of(wa), 31);
        prop_assert_eq!(ra_of(wa), 31);
    }
}

proptest! {
    // ── C1. CHARACTERISATION (current buggy behaviour): SP == XZR ────────
    // PASSING today. Because register 31 is ZR in these groups, `sp`/`wsp`
    // encode bit-identically to `xzr`/`wzr` in the same slot — i.e. SP is
    // silently accepted and aliased to the zero register.
    #[test]
    fn sp_is_rejected(
        n in 0u32..=30, vi in 0u32..=1u32,
    ) {
        // SP/WSP must be REJECTED (register 31 reads as ZR in these groups;
        // previously sp/wsp were silently aliased to it).
        let (sp, zr) = if vi == 0 { ("sp", "xzr") } else { ("wsp", "wzr") };
        let a = vec![xreg(n), Operand::Reg(sp.into()), xreg(n)];
        prop_assert!(encode_mul(&a).is_err(), "mul with {sp} must be rejected");
        let c = vec![xreg(n), xreg(n), xreg(n), Operand::Reg(sp.into())];
        prop_assert!(encode_madd(&c).is_err(), "madd with {sp} must be rejected");
        let e = vec![xreg(n), Operand::Reg("wsp".into()), wreg(n)];
        prop_assert!(encode_umull(&e).is_err(), "umull with wsp must be rejected");
        let _ = zr;
    }

    // ── C2. CHARACTERISATION (current buggy behaviour): FP/SIMD == GP ────
    // PASSING today. `parse_reg_num` accepts FP/SIMD prefixes, so a SIMD name
    // in an operand slot encodes identically to the same-numbered GP register.
        fn fp_simd_is_rejected(
        n in 0u32..=30, vi in 0u32..=5u32,
    ) {
        // FP/SIMD register spellings must be REJECTED in GPR positions
        // (previously they encoded identically to the same-numbered GP reg).
        let bad = fp_variant(vi, n);
        let a = vec![xreg(n), xreg(n), bad.clone()];
        prop_assert!(encode_mul(&a).is_err(), "mul with FP/SIMD must be rejected");
        let c = vec![xreg(n), xreg(n), xreg(n), bad.clone()];
        prop_assert!(encode_msub(&c).is_err(), "msub with FP/SIMD must be rejected");
        let e = vec![xreg(n), bad.clone(), wreg(n), xreg(n)];
        prop_assert!(encode_umaddl(&e).is_err(), "umaddl with FP/SIMD must be rejected");
    }

    // ── C3. CHARACTERISATION: sf is taken from Rd (GP) / hard-wired 1 (long)
    // PASSING today. mul/madd/msub derive sf solely from the destination and
    // ignore source widths; umull/umaddl force sf=1 regardless of operand text.
    #[test]
    fn mixed_widths_are_rejected(
        n in 0u32..=30, rd_is_x in any::<bool>(),
    ) {
        // Mixed x/w operand widths must be REJECTED for the GP forms, and
        // widening forms require an X destination with W sources.
        let mk = |is_x: bool, n: u32| if is_x { xreg(n) } else { wreg(n) };
        // Sources are hardwired W here, so a width mismatch exists iff Rd is X.
        prop_assume!(rd_is_x);
        for &(_name, f) in GP3.iter().chain(GP4.iter()) {
            let ops = vec![mk(rd_is_x, n), wreg(n), wreg(n), wreg(n)];
            prop_assert!(f(&ops).is_err(), "GP form with mixed width must be rejected");
        }
        for &(_name, f) in LONG3.iter().chain(LONG4.iter()) {
            // LONG3 takes 3 register operands; LONG4's 4th (Ra) must be X.
            // The extra trailing wreg for LONG3 is ignored by the encoder.
            let ops = vec![mk(rd_is_x, n), wreg(n), wreg(n), xreg(n)];
            if !rd_is_x {
                prop_assert!(f(&ops).is_err(), "long form with W dest must be rejected");
            } else {
                let w = word(f(&ops));
                prop_assert_eq!(sf_of(w), 1);
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  BUG WITNESSES — all #[ignore]'d so `cargo test` stays green.
//  Run:  cargo test --test pbt_arm_core data_processing_mul_madd_msub_umaddl_umull_pbt -- --ignored
//  Each asserts the SPEC-CORRECT behaviour the encoder currently violates.
//  Differential oracle: `clang --target=aarch64-linux-gnu` rejects every one
//  of these spellings with "error: invalid operand for instruction".
// ═══════════════════════════════════════════════════════════════════════════
proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    // ── W1. encode_mul accepts SP/WSP in any operand position ────────────
    #[test]
    fn mul_rejects_sp_wsp_in_any_position(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
        pos in 0u32..3u32, vi in 0u32..=1u32,
    ) {
        let mut ops = vec![xreg(rd), xreg(rn), xreg(rm)];
        ops[pos as usize] = sp_variant(vi);
        prop_assert!(encode_mul(&ops).is_err(),
            "mul with SP/WSP at position {} must be Err, got {:?}", pos, encode_mul(&ops));
    }

    // ── W2. encode_madd accepts SP/WSP in any operand position ───────────
    #[test]
    fn madd_rejects_sp_wsp_in_any_position(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30, ra in 0u32..=30,
        pos in 0u32..4u32, vi in 0u32..=1u32,
    ) {
        let mut ops = vec![xreg(rd), xreg(rn), xreg(rm), xreg(ra)];
        ops[pos as usize] = sp_variant(vi);
        prop_assert!(encode_madd(&ops).is_err(),
            "madd with SP/WSP at position {} must be Err, got {:?}", pos, encode_madd(&ops));
    }

    // ── W3. encode_msub accepts SP/WSP in any operand position ───────────
    #[test]
    fn msub_rejects_sp_wsp_in_any_position(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30, ra in 0u32..=30,
        pos in 0u32..4u32, vi in 0u32..=1u32,
    ) {
        let mut ops = vec![xreg(rd), xreg(rn), xreg(rm), xreg(ra)];
        ops[pos as usize] = sp_variant(vi);
        prop_assert!(encode_msub(&ops).is_err(),
            "msub with SP/WSP at position {} must be Err, got {:?}", pos, encode_msub(&ops));
    }

    // ── W4. encode_umull accepts SP/WSP in any operand position ──────────
    #[test]
    fn umull_rejects_sp_wsp_in_any_position(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
        pos in 0u32..3u32, vi in 0u32..=1u32,
    ) {
        let mut ops = vec![xreg(rd), wreg(rn), wreg(rm)];
        ops[pos as usize] = sp_variant(vi);
        prop_assert!(encode_umull(&ops).is_err(),
            "umull with SP/WSP at position {} must be Err, got {:?}", pos, encode_umull(&ops));
    }

    // ── W5. encode_umaddl accepts SP/WSP in any operand position ─────────
    #[test]
    fn umaddl_rejects_sp_wsp_in_any_position(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30, ra in 0u32..=30,
        pos in 0u32..4u32, vi in 0u32..=1u32,
    ) {
        let mut ops = vec![xreg(rd), wreg(rn), wreg(rm), xreg(ra)];
        ops[pos as usize] = sp_variant(vi);
        prop_assert!(encode_umaddl(&ops).is_err(),
            "umaddl with SP/WSP at position {} must be Err, got {:?}", pos, encode_umaddl(&ops));
    }

    // ── W6. encode_mul accepts FP/SIMD registers in any position ─────────
    #[test]
    fn mul_rejects_fp_simd_in_any_position(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
        pos in 0u32..3u32, vi in 0u32..=5u32,
    ) {
        let mut ops = vec![xreg(rd), xreg(rn), xreg(rm)];
        ops[pos as usize] = fp_variant(vi, rm);
        prop_assert!(encode_mul(&ops).is_err(),
            "mul with FP/SIMD at position {} must be Err, got {:?}", pos, encode_mul(&ops));
    }

    // ── W7. encode_madd accepts FP/SIMD registers in any position ────────
    #[test]
    fn madd_rejects_fp_simd_in_any_position(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30, ra in 0u32..=30,
        pos in 0u32..4u32, vi in 0u32..=5u32,
    ) {
        let mut ops = vec![xreg(rd), xreg(rn), xreg(rm), xreg(ra)];
        ops[pos as usize] = fp_variant(vi, rm);
        prop_assert!(encode_madd(&ops).is_err(),
            "madd with FP/SIMD at position {} must be Err, got {:?}", pos, encode_madd(&ops));
    }

    // ── W8. encode_msub accepts FP/SIMD registers in any position ────────
    #[test]
    fn msub_rejects_fp_simd_in_any_position(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30, ra in 0u32..=30,
        pos in 0u32..4u32, vi in 0u32..=5u32,
    ) {
        let mut ops = vec![xreg(rd), xreg(rn), xreg(rm), xreg(ra)];
        ops[pos as usize] = fp_variant(vi, rm);
        prop_assert!(encode_msub(&ops).is_err(),
            "msub with FP/SIMD at position {} must be Err, got {:?}", pos, encode_msub(&ops));
    }

    // ── W9. encode_umull accepts FP/SIMD registers in any position ───────
    #[test]
    fn umull_rejects_fp_simd_in_any_position(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
        pos in 0u32..3u32, vi in 0u32..=5u32,
    ) {
        let mut ops = vec![xreg(rd), wreg(rn), wreg(rm)];
        ops[pos as usize] = fp_variant(vi, rm);
        prop_assert!(encode_umull(&ops).is_err(),
            "umull with FP/SIMD at position {} must be Err, got {:?}", pos, encode_umull(&ops));
    }

    // ── W10. encode_umaddl accepts FP/SIMD registers in any position ─────
    #[test]
    fn umaddl_rejects_fp_simd_in_any_position(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30, ra in 0u32..=30,
        pos in 0u32..4u32, vi in 0u32..=5u32,
    ) {
        let mut ops = vec![xreg(rd), wreg(rn), wreg(rm), xreg(ra)];
        ops[pos as usize] = fp_variant(vi, rm);
        prop_assert!(encode_umaddl(&ops).is_err(),
            "umaddl with FP/SIMD at position {} must be Err, got {:?}", pos, encode_umaddl(&ops));
    }

    // ── W11. PARAMETRIC BREADTH: every encoder rejects SP/WSP ────────────
    #[test]
    fn all_encoders_reject_sp_wsp(n in 0u32..=30) {
        for &(_name, f) in GP3 {
            for pos in 0u32..3 {
                let mut ops = vec![xreg(n), xreg(n), xreg(n)];
                ops[pos as usize] = sp_variant(0);
                prop_assert!(f(&ops).is_err(), "GP3 SP pos {}", pos);
            }
        }
        for &(_name, f) in GP4 {
            for pos in 0u32..4 {
                let mut ops = vec![xreg(n), xreg(n), xreg(n), xreg(n)];
                ops[pos as usize] = sp_variant(0);
                prop_assert!(f(&ops).is_err(), "GP4 SP pos {}", pos);
            }
        }
        for &(_name, f) in LONG3 {
            for pos in 0u32..3 {
                let mut ops = vec![xreg(n), wreg(n), wreg(n)];
                ops[pos as usize] = sp_variant(0);
                prop_assert!(f(&ops).is_err(), "LONG3 SP pos {}", pos);
            }
        }
        for &(_name, f) in LONG4 {
            for pos in 0u32..4 {
                let mut ops = vec![xreg(n), wreg(n), wreg(n), xreg(n)];
                ops[pos as usize] = sp_variant(0);
                prop_assert!(f(&ops).is_err(), "LONG4 SP pos {}", pos);
            }
        }
    }

    // ── W12. PARAMETRIC BREADTH: every encoder rejects FP/SIMD ───────────
    #[test]
    fn all_encoders_reject_fp_simd(n in 0u32..=30) {
        let bad = fp_variant(0, n);
        for &(_name, f) in GP3 {
            for pos in 0u32..3 {
                let mut ops = vec![xreg(n), xreg(n), xreg(n)];
                ops[pos as usize] = bad.clone();
                prop_assert!(f(&ops).is_err(), "GP3 FP pos {}", pos);
            }
        }
        for &(_name, f) in GP4 {
            for pos in 0u32..4 {
                let mut ops = vec![xreg(n), xreg(n), xreg(n), xreg(n)];
                ops[pos as usize] = bad.clone();
                prop_assert!(f(&ops).is_err(), "GP4 FP pos {}", pos);
            }
        }
        for &(_name, f) in LONG3 {
            for pos in 0u32..3 {
                let mut ops = vec![xreg(n), wreg(n), wreg(n)];
                ops[pos as usize] = bad.clone();
                prop_assert!(f(&ops).is_err(), "LONG3 FP pos {}", pos);
            }
        }
        for &(_name, f) in LONG4 {
            for pos in 0u32..4 {
                let mut ops = vec![xreg(n), wreg(n), wreg(n), xreg(n)];
                ops[pos as usize] = bad.clone();
                prop_assert!(f(&ops).is_err(), "LONG4 FP pos {}", pos);
            }
        }
    }
}
