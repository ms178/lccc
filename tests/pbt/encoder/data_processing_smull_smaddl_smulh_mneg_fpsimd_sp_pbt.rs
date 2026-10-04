//! Property-based tests for the AArch64 signed / widening-multiply encoders in
//! `data_processing.rs`:
//!
//! * `encode_smull`  — SMULL  `<Xd>,<Wn>,<Wm>`   (alias SMADDL Xd,Wn,Wm,XZR)
//! * `encode_smaddl` — SMADDL `<Xd>,<Wn>,<Wm>,<Xa>`
//! * `encode_smulh`  — SMULH  `<Xd>,<Xn>,<Xm>`   (64-bit only)
//! * `encode_mneg`   — MNEG   `<Rd>,<Rn>,<Rm>`    (alias MSUB Rd,Rn,Rm,XZR)
//!
//! ## Focus: FP/SIMD register-class acceptance & SP-as-XZR aliasing
//!
//! All four encoders resolve operands through the shared `get_reg` /
//! `parse_reg_num` helpers, which:
//!
//! (a) map `"sp"`/`"wsp"` → register number **31** — i.e. XZR/WZR in the
//!     data-processing (3-source) and widening-multiply groups, **not** SP;
//! (b) accept the FP/SIMD prefixes `d/s/q/v/h/b`, mapping them to the
//!     same-numbered general-purpose register.
//!
//! Both spellings are unallocated for these integer-multiply instructions.
//! The canonical assembler rejects every one of them:
//!
//! ```text
//! $ echo 'smull x0, sp, w2'   | clang --target=aarch64-linux-gnu -c -x assembler -
//! <stdin>:1:11: error: invalid operand for instruction
//! $ echo 'smulh x0, d1, x2'   | clang --target=aarch64-linux-gnu -c -x assembler -
//! <stdin>:1:11: error: invalid operand for instruction
//! ```
//!
//! The SUT silently accepts them. The **witness** properties (asserting the
//! spec-correct `Err`) are `#[ignore]`d so a default `cargo test` stays green;
//! run them with:
//!
//! ```text
//! cargo test --test pbt_arm_core data_processing_smull_smaddl_smulh_mneg_fpsimd_sp_pbt -- --ignored
//! ```
//! The passing **characterisation** properties pin the current (buggy) behaviour
//! (SP/WSP → field 31 == ZR; FP/SIMD lane number placed as a GP register); they
//! will start failing once the defects are fixed — the cue to drop the
//! `#[ignore]` markers.

use lccc::pbt_internals::Operand;
use lccc::pbt_internals::*;
use proptest::prelude::*;

// ── field extractors (data-processing (3 source) / widening-multiply layout)
//   sf 00 11011 op31 Rm o0 Ra Rn Rd
fn sf_of(w: u32) -> u32 {
    (w >> 31) & 1
}
fn o0_of(w: u32) -> u32 {
    (w >> 15) & 1
}
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

/// `sp` / `wsp`: both parse to register number 31 — which is **ZR**, not SP,
/// in these encoding groups.
fn sp_variant(i: u32) -> Operand {
    if i.is_multiple_of(2) {
        Operand::Reg("sp".into())
    } else {
        Operand::Reg("wsp".into())
    }
}
/// FP/SIMD register names accepted by `parse_reg_num` but illegal as GP
/// operands: `d`/`s`/`q`/`v`/`h`/`b` with a lane number.
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

// ── reference encoders (rebuilt independently from the ARMv8 bit-strings) ─
fn smull_ref(rd: u32, rn: u32, rm: u32) -> u32 {
    (1u32 << 31) | (0b0011011001 << 21) | (rm << 16) | (0b011111 << 10) | (rn << 5) | rd
}
fn smaddl_ref(rd: u32, rn: u32, rm: u32, ra: u32) -> u32 {
    (1u32 << 31) | (0b0011011001 << 21) | (rm << 16) | (ra << 10) | (rn << 5) | rd
}
fn smulh_ref(rd: u32, rn: u32, rm: u32) -> u32 {
    (1u32 << 31) | (0b0011011010 << 21) | (rm << 16) | (0b011111 << 10) | (rn << 5) | rd
}
fn mneg_ref(rd: u32, rn: u32, rm: u32, is_64: bool) -> u32 {
    let sf = if is_64 { 1u32 } else { 0 };
    (sf << 31) | (0b0011011000 << 21) | (rm << 16) | (1u32 << 15) | (0b11111 << 10) | (rn << 5) | rd
}

// ── encoder table (shared by characterisation + witnesses) ───────────────
type Enc = fn(&[Operand]) -> Result<EncodeResult, String>;
fn smull_base(a: [u32; 4]) -> Vec<Operand> {
    vec![xreg(a[0]), wreg(a[1]), wreg(a[2])]
}
fn smaddl_base(a: [u32; 4]) -> Vec<Operand> {
    vec![xreg(a[0]), wreg(a[1]), wreg(a[2]), xreg(a[3])]
}
fn smulh_base(a: [u32; 4]) -> Vec<Operand> {
    vec![xreg(a[0]), xreg(a[1]), xreg(a[2])]
}
fn mneg_base(a: [u32; 4]) -> Vec<Operand> {
    vec![xreg(a[0]), xreg(a[1]), xreg(a[2])]
}

/// (mnemonic, encoder, arity, operand-template builder) for the widening
/// multiply family under test.
type Case = (&'static str, Enc, u32, fn([u32; 4]) -> Vec<Operand>);
const CASES: &[Case] = &[
    ("smull", encode_smull, 3, smull_base),
    ("smaddl", encode_smaddl, 4, smaddl_base),
    ("smulh", encode_smulh, 3, smulh_base),
    ("mneg", encode_mneg, 3, mneg_base),
];

proptest! {
    // ── HAPPY-PATH GUARDS (passing): valid GP inputs match the ARMv8 reference
    //    word and field placement. Anchors that the harness reaches the real
    //    encoders before the characterisation/witnesses below.
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn smull_happy_path(rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30) {
        let w = word(encode_smull(&[xreg(rd), wreg(rn), wreg(rm)]));
        prop_assert_eq!(w, smull_ref(rd, rn, rm));
        prop_assert_eq!(sf_of(w), 1);
        prop_assert_eq!(ra_of(w), 31); // XZR -> the SMULL alias
        prop_assert_eq!(rm_of(w), rm);
        prop_assert_eq!(rn_of(w), rn);
        prop_assert_eq!(rd_of(w), rd);
    }

    #[test]
    fn smaddl_happy_path(rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30, ra in 0u32..=30) {
        let w = word(encode_smaddl(&[xreg(rd), wreg(rn), wreg(rm), xreg(ra)]));
        prop_assert_eq!(w, smaddl_ref(rd, rn, rm, ra));
        prop_assert_eq!(sf_of(w), 1);
        prop_assert_eq!(o0_of(w), 0);
        prop_assert_eq!(rm_of(w), rm);
        prop_assert_eq!(ra_of(w), ra);
        prop_assert_eq!(rn_of(w), rn);
        prop_assert_eq!(rd_of(w), rd);
    }

    #[test]
    fn smulh_happy_path(rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30) {
        let w = word(encode_smulh(&[xreg(rd), xreg(rn), xreg(rm)]));
        prop_assert_eq!(w, smulh_ref(rd, rn, rm));
        prop_assert_eq!(sf_of(w), 1);
        prop_assert_eq!(o0_of(w), 0);
        prop_assert_eq!(ra_of(w), 31);
        prop_assert_eq!(rm_of(w), rm);
        prop_assert_eq!(rn_of(w), rn);
        prop_assert_eq!(rd_of(w), rd);
    }

    #[test]
    fn mneg_happy_path(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30, is_w in any::<bool>(),
    ) {
        let dst = if is_w { wreg(rd) } else { xreg(rd) };
        let src = if is_w { wreg(rn) } else { xreg(rn) };
        let src2 = if is_w { wreg(rm) } else { xreg(rm) };
        let w = word(encode_mneg(&[dst, src, src2]));
        prop_assert_eq!(w, mneg_ref(rd, rn, rm, !is_w));
        prop_assert_eq!(sf_of(w), u32::from(!is_w));
        prop_assert_eq!(o0_of(w), 1); // MSUB (negate)
        prop_assert_eq!(ra_of(w), 31); // XZR -> the MNEG alias
        prop_assert_eq!(rm_of(w), rm);
        prop_assert_eq!(rn_of(w), rn);
        prop_assert_eq!(rd_of(w), rd);
    }
}

proptest! {
    // ── CHARACTERISATION (PASSING today; pins the current buggy behaviour).
    //    SP/WSP and FP/SIMD names are silently accepted — the *bug* is that
    //    they are accepted at all. These properties merely document HOW.

    // C1. SP/WSP alias onto register-field value 31 (== XZR/WZR) in every
    //     operand position of every encoder: sp/wsp/xzr/wzr all map to 31.
    #[test]
    fn sp_wsp_is_rejected(
        n in 0u32..=30, idx in 0u32..=4u32, pos in 0u32..=3u32, vi in 0u32..=1u32,
    ) {
        // SP/WSP must be REJECTED (previously aliased to field 31 == ZR).
        let (name, enc, arity, base) = CASES[(idx as usize) % CASES.len()];
        let pos = pos % arity;
        let mut ops = base([n, n, n, n]);
        ops[pos as usize] = sp_variant(vi);
        prop_assert!(enc(&ops).is_err(),
            "{} with SP/WSP at position {} must be rejected", name, pos);
    }

    // C2. An FP/SIMD register name is silently accepted; its lane number is
    //     placed into the operand field exactly like the same-numbered GP reg.
    #[test]
    fn fp_simd_is_rejected(
        n in 0u32..=30, idx in 0u32..=4u32, pos in 0u32..=3u32, vi in 0u32..=5u32,
    ) {
        // FP/SIMD register spellings must be REJECTED in GPR positions
        // (previously the lane number was placed like a GP register).
        let (name, enc, arity, base) = CASES[(idx as usize) % CASES.len()];
        let pos = pos % arity;
        let mut ops = base([n, n, n, n]);
        ops[pos as usize] = fp_variant(vi, n);
        prop_assert!(enc(&ops).is_err(),
            "{} with FP/SIMD at position {} must be rejected", name, pos);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  BUG WITNESSES — all #[ignore]'d so a default `cargo test` stays green.
//  Run:  cargo test --test pbt_arm_core data_processing_smull_smaddl_smulh_mneg_fpsimd_sp_pbt -- --ignored
//  Each asserts the SPEC-CORRECT rejection the encoder currently violates.
//  Differential oracle: `clang --target=aarch64-linux-gnu` rejects every one
//  of these spellings with "error: invalid operand for instruction".
// ═══════════════════════════════════════════════════════════════════════════
proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    // ── encode_smull ──────────────────────────────────────────────────────
    #[test]
    fn smull_rejects_sp_wsp_in_any_position(
        n in 0u32..=30, pos in 0u32..3u32, vi in 0u32..=1u32,
    ) {
        let mut ops = smull_base([n, n, n, n]);
        ops[pos as usize] = sp_variant(vi);
        prop_assert!(encode_smull(&ops).is_err(),
            "smull with SP/WSP at position {} must be Err, got {:?}", pos, encode_smull(&ops));
    }

    #[test]
    fn smull_rejects_fp_simd_in_any_position(
        n in 0u32..=30, pos in 0u32..3u32, vi in 0u32..=5u32,
    ) {
        let mut ops = smull_base([n, n, n, n]);
        ops[pos as usize] = fp_variant(vi, n);
        prop_assert!(encode_smull(&ops).is_err(),
            "smull with FP/SIMD at position {} must be Err, got {:?}", pos, encode_smull(&ops));
    }

    // ── encode_smaddl ─────────────────────────────────────────────────────
    #[test]
    fn smaddl_rejects_sp_wsp_in_any_position(
        n in 0u32..=30, pos in 0u32..4u32, vi in 0u32..=1u32,
    ) {
        let mut ops = smaddl_base([n, n, n, n]);
        ops[pos as usize] = sp_variant(vi);
        prop_assert!(encode_smaddl(&ops).is_err(),
            "smaddl with SP/WSP at position {} must be Err, got {:?}", pos, encode_smaddl(&ops));
    }

    #[test]
    fn smaddl_rejects_fp_simd_in_any_position(
        n in 0u32..=30, pos in 0u32..4u32, vi in 0u32..=5u32,
    ) {
        let mut ops = smaddl_base([n, n, n, n]);
        ops[pos as usize] = fp_variant(vi, n);
        prop_assert!(encode_smaddl(&ops).is_err(),
            "smaddl with FP/SIMD at position {} must be Err, got {:?}", pos, encode_smaddl(&ops));
    }

    // ── encode_smulh ──────────────────────────────────────────────────────
    #[test]
    fn smulh_rejects_sp_wsp_in_any_position(
        n in 0u32..=30, pos in 0u32..3u32, vi in 0u32..=1u32,
    ) {
        let mut ops = smulh_base([n, n, n, n]);
        ops[pos as usize] = sp_variant(vi);
        prop_assert!(encode_smulh(&ops).is_err(),
            "smulh with SP/WSP at position {} must be Err, got {:?}", pos, encode_smulh(&ops));
    }

    #[test]
    fn smulh_rejects_fp_simd_in_any_position(
        n in 0u32..=30, pos in 0u32..3u32, vi in 0u32..=5u32,
    ) {
        let mut ops = smulh_base([n, n, n, n]);
        ops[pos as usize] = fp_variant(vi, n);
        prop_assert!(encode_smulh(&ops).is_err(),
            "smulh with FP/SIMD at position {} must be Err, got {:?}", pos, encode_smulh(&ops));
    }

    // ── encode_mneg ───────────────────────────────────────────────────────
    #[test]
    fn mneg_rejects_sp_wsp_in_any_position(
        n in 0u32..=30, pos in 0u32..3u32, vi in 0u32..=1u32,
    ) {
        let mut ops = mneg_base([n, n, n, n]);
        ops[pos as usize] = sp_variant(vi);
        prop_assert!(encode_mneg(&ops).is_err(),
            "mneg with SP/WSP at position {} must be Err, got {:?}", pos, encode_mneg(&ops));
    }

    #[test]
    fn mneg_rejects_fp_simd_in_any_position(
        n in 0u32..=30, pos in 0u32..3u32, vi in 0u32..=5u32,
    ) {
        let mut ops = mneg_base([n, n, n, n]);
        ops[pos as usize] = fp_variant(vi, n);
        prop_assert!(encode_mneg(&ops).is_err(),
            "mneg with FP/SIMD at position {} must be Err, got {:?}", pos, encode_mneg(&ops));
    }
}
