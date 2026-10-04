//! Property-based tests for the **conditional-select** encoders defined in
//! `compare_branch.rs`:
//!
//!   * `encode_csel`  — CSEL  Rd, Rn, Rm, cond
//!   * `encode_csinc` — CSINC Rd, Rn, Rm, cond
//!   * `encode_csinv` — CSINV Rd, Rn, Rm, cond
//!   * `encode_csneg` — CSNEG Rd, Rn, Rm, cond
//!   * `encode_cinc`  — CINC  Rd, Rn, cond   (alias -> CSINC, Rm==Rn, invert cond)
//!   * `encode_cinv`  — CINV  Rd, Rn, cond   (alias -> CSINV, Rm==Rn, invert cond)
//!   * `encode_cneg`  — CNEG  Rd, Rn, cond   (alias -> CSNEG, Rm==Rn, invert cond)
//!
//! ## Focus (per request)
//!
//! 1. **FP/SIMD and SP operand acceptance** for `encode_csel`, `encode_cinc`,
//!    `encode_cinv`, `encode_cneg`. The whole conditional-select group is an
//!    *integer-only* instruction whose operands belong to the "Z" class — a
//!    general-purpose register *or the zero register*, **never SP** and **never
//!    a FP/SIMD register** (V/D/S/Q/H/B). All four encoders resolve operands
//!    through the shared `get_reg` / `parse_reg_num` helpers, which:
//!      * map `"sp"`/`"wsp"` -> register number 31 (== ZR encoding, *not* SP);
//!      * accept the FP/SIMD prefixes `d/s/q/v/h/b` and map them to the
//!        same-numbered general-purpose register.
//!        Both spellings are UNALLOCATED here. (The aliases `cinc`/`cinv`/`cneg`
//!        additionally misuse any width, but that is tracked separately under (2).)
//!
//! 2. **Mixed W/X register widths** for `encode_csel`, `encode_csinc`,
//!    `encode_csinv`, `encode_csneg`. A single `sf` field governs the width of
//!    Rd, Rn and Rm collectively, so non-uniform width is UNALLOCATED. The
//!    encoders derive `sf` from **Rd only** and discard the widths of Rn/Rm,
//!    silently coercing e.g. `csel x0, w1, w2, eq` to a 64-bit instruction.
//!
//! The **condition-code** bug (the aliases accepting the reserved AL/NV
//! condition codes) is already covered by `compare_branch_cond_pbt.rs` and is
//! deliberately *not* exercised here — all condition codes used below are drawn
//! from the 14 architecturally-valid values 0..=13.
//!
//! ## Oracle
//!
//! Register-class and width contracts are taken from the ARMv8-A Architecture
//! Reference Manual (Conditional-select group) and cross-checked against the
//! system conforming assembler `clang --target=aarch64-linux-gnu`, which:
//!   * rejects every "should be Err" input witnessed below, e.g.
//!     `csel x0, sp, x1, eq`   -> "error: invalid operand for instruction"
//!     `csel x0, v0, x1, eq`   -> "error: invalid operand for instruction"
//!     `cinc x0, sp, eq`       -> "error: invalid operand for instruction"
//!     `csel x0, w1, w2, eq`   -> "error: invalid operand width"
//!     `csel x0, w1, x2, eq`   -> "error: invalid operand width"
//!     `cinc w0, x1, eq`       -> "error: invalid operand width"
//!   * accepts every positive-domain spelling, e.g.
//!     `csel x0, x1, x2, eq`, `csel w0, w1, w2, ne`, `csel x0, xzr, xzr, al`,
//!     `csinc w3, w4, w5, lt`, `csinv x6, x7, x8, ge`, `csneg w9, w10, w11, hi`,
//!     `cinc x0, x1, eq`, `cinv w0, w1, ne`, `cneg x0, x1, mi`.
//!
//! ## Bug witnesses
//!
//! Every property that *witnesses a defect* is `#[ignore]`'d so a default
//! `cargo test` stays green. Run them explicitly with:
//!
//! ```text
//! cargo test --lib compare_branch_condselect_regclass_pbt -- --ignored
//! ```
//!
//! The passing **characterisation** properties pin the current (buggy)
//! behaviour — SP/WSP -> field 31 (== ZR); FP/SIMD lane number placed as a GP
//! register; `sf` taken only from Rd under mixed widths. They will start
//! failing once the defects are fixed, which is the cue to drop the
//! `#[ignore]` markers.

use lccc::pbt_internals::Operand;
use lccc::pbt_internals::*;
use proptest::prelude::*;

// ── Conditional-select field layout (ARMv8-A) ────────────────────────────
//   sf[31] op[30] 0[29] 11010100[28:21] Rm[20:16] cond[15:12] o2[11] o1[10]
//   Rn[9:5] Rd[4:0]
//     CSEL  : op=0, o1=0, o2=0
//     CSINC : op=0, o1=1, o2=0
//     CSINV : op=1, o1=0, o2=0
//     CSNEG : op=1, o1=1, o2=0
const OPSEL: u32 = 0b11010100u32 << 21; // opcode field [28:21]

fn sf_of(w: u32) -> u32 {
    (w >> 31) & 1
}
fn op30_of(w: u32) -> u32 {
    (w >> 30) & 1
}
fn rm_of(w: u32) -> u32 {
    (w >> 16) & 0x1F
}
fn cond_of(w: u32) -> u32 {
    (w >> 12) & 0xF
}
fn o1_of(w: u32) -> u32 {
    (w >> 10) & 1
}
fn rn_of(w: u32) -> u32 {
    (w >> 5) & 0x1F
}
fn rd_of(w: u32) -> u32 {
    w & 0x1F
}

fn word(r: Result<EncodeResult, String>) -> u32 {
    match r {
        Ok(EncodeResult::Word(w)) => w,
        Ok(other) => panic!("expected Word, got {:?}", other),
        Err(e) => panic!("expected Ok, got Err: {}", e),
    }
}

// ── operand builders ─────────────────────────────────────────────────────
fn xreg(n: u32) -> Operand {
    Operand::Reg(format!("x{}", n))
}
fn wreg(n: u32) -> Operand {
    Operand::Reg(format!("w{}", n))
}
fn gp_reg(n: u32, is_64: bool) -> Operand {
    if is_64 { xreg(n) } else { wreg(n) }
}

/// `sp` / `wsp`: both parse to register number 31 — which is **ZR**, not SP,
/// in the conditional-select group.
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

/// Valid (non-AL/NV) condition codes mirroring `encode_cond`. Restricting to
/// 0..=13 keeps this suite clear of the reported condition-code bug.
const VALID_CONDS: &[(u32, &str)] = &[
    (0, "eq"),
    (1, "ne"),
    (2, "cs"),
    (3, "cc"),
    (4, "mi"),
    (5, "pl"),
    (6, "vs"),
    (7, "vc"),
    (8, "hi"),
    (9, "ls"),
    (10, "ge"),
    (11, "lt"),
    (12, "gt"),
    (13, "le"),
];

// ── reference encoders (rebuilt independently from the ARMv8 bit-strings) ─
fn csel_ref(rd: u32, rn: u32, rm: u32, sf: u32, cond: u32) -> u32 {
    (sf << 31) | OPSEL | (rm << 16) | (cond << 12) | (rn << 5) | rd
}
fn csinc_ref(rd: u32, rn: u32, rm: u32, sf: u32, cond: u32) -> u32 {
    (sf << 31) | OPSEL | (rm << 16) | (cond << 12) | (0b01 << 10) | (rn << 5) | rd
}
fn csinv_ref(rd: u32, rn: u32, rm: u32, sf: u32, cond: u32) -> u32 {
    (sf << 31) | (1u32 << 30) | OPSEL | (rm << 16) | (cond << 12) | (rn << 5) | rd
}
fn csneg_ref(rd: u32, rn: u32, rm: u32, sf: u32, cond: u32) -> u32 {
    (sf << 31) | (1u32 << 30) | OPSEL | (rm << 16) | (cond << 12) | (0b01 << 10) | (rn << 5) | rd
}
fn cinc_ref(rd: u32, rn: u32, sf: u32, cond: u32) -> u32 {
    let inv = cond ^ 1;
    (sf << 31) | OPSEL | (rn << 16) | (inv << 12) | (0b01 << 10) | (rn << 5) | rd
}
fn cinv_ref(rd: u32, rn: u32, sf: u32, cond: u32) -> u32 {
    let inv = cond ^ 1;
    (sf << 31) | (1u32 << 30) | OPSEL | (rn << 16) | (inv << 12) | (rn << 5) | rd
}
fn cneg_ref(rd: u32, rn: u32, sf: u32, cond: u32) -> u32 {
    let inv = cond ^ 1;
    (sf << 31) | (1u32 << 30) | OPSEL | (rn << 16) | (inv << 12) | (0b01 << 10) | (rn << 5) | rd
}

// ── encoder / operand tables shared by characterisation + witnesses ──────
type Enc = fn(&[Operand]) -> Result<EncodeResult, String>;

/// Group 1 targets for the FP/SIMD + SP focus: (name, encoder, reg-arity).
const G1: &[(&str, Enc, u32)] = &[
    ("csel", encode_csel, 3),
    ("cinc", encode_cinc, 2),
    ("cinv", encode_cinv, 2),
    ("cneg", encode_cneg, 2),
];

/// Group 2 targets for the mixed-width focus: (name, encoder).
const G2: &[(&str, Enc)] = &[
    ("csel", encode_csel),
    ("csinc", encode_csinc),
    ("csinv", encode_csinv),
    ("csneg", encode_csneg),
];

/// Valid-GP base operand vector for a Group-1 encoder (all operands x{n}).
fn g1_base(name: &str, n: u32) -> Vec<Operand> {
    let r = xreg(n);
    let cond = Operand::Cond("eq".into());
    match name {
        "csel" => vec![r.clone(), r.clone(), r.clone(), cond],
        _ => vec![r.clone(), r.clone(), cond],
    }
}

proptest! {
    // ── HAPPY-PATH GUARDS (passing): valid GP inputs match the ARMv8 reference
    //    word and field placement. These anchor that the harness reaches the
    //    real encoders before the characterisation / witnesses below. Note the
    //    base forms (CSEL/CSINC/CSINV/CSNEG) carry the condition VERBATIM
    //    while the aliases (CINC/CINV/CNEG) INVERT it.
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn csel_happy_path(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
        is_64 in any::<bool>(), ci in 0usize..VALID_CONDS.len(),
    ) {
        let (cv, cn) = VALID_CONDS[ci];
        let sf = u32::from(is_64);
        let ops = vec![gp_reg(rd, is_64), gp_reg(rn, is_64), gp_reg(rm, is_64),
                       Operand::Cond(cn.into())];
        let w = word(encode_csel(&ops));
        prop_assert_eq!(w, csel_ref(rd, rn, rm, sf, cv));
        prop_assert_eq!(sf_of(w), sf);
        prop_assert_eq!(op30_of(w), 0); // CSEL
        prop_assert_eq!(o1_of(w), 0);
        prop_assert_eq!(cond_of(w), cv); // verbatim
        prop_assert_eq!(rm_of(w), rm);
        prop_assert_eq!(rn_of(w), rn);
        prop_assert_eq!(rd_of(w), rd);
    }

    #[test]
    fn csinc_happy_path(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
        is_64 in any::<bool>(), ci in 0usize..VALID_CONDS.len(),
    ) {
        let (cv, cn) = VALID_CONDS[ci];
        let sf = u32::from(is_64);
        let ops = vec![gp_reg(rd, is_64), gp_reg(rn, is_64), gp_reg(rm, is_64),
                       Operand::Cond(cn.into())];
        let w = word(encode_csinc(&ops));
        prop_assert_eq!(w, csinc_ref(rd, rn, rm, sf, cv));
        prop_assert_eq!(sf_of(w), sf);
        prop_assert_eq!(op30_of(w), 0); // CSINC
        prop_assert_eq!(o1_of(w), 1);
        prop_assert_eq!(cond_of(w), cv);
        prop_assert_eq!(rm_of(w), rm);
        prop_assert_eq!(rn_of(w), rn);
        prop_assert_eq!(rd_of(w), rd);
    }

    #[test]
    fn csinv_happy_path(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
        is_64 in any::<bool>(), ci in 0usize..VALID_CONDS.len(),
    ) {
        let (cv, cn) = VALID_CONDS[ci];
        let sf = u32::from(is_64);
        let ops = vec![gp_reg(rd, is_64), gp_reg(rn, is_64), gp_reg(rm, is_64),
                       Operand::Cond(cn.into())];
        let w = word(encode_csinv(&ops));
        prop_assert_eq!(w, csinv_ref(rd, rn, rm, sf, cv));
        prop_assert_eq!(sf_of(w), sf);
        prop_assert_eq!(op30_of(w), 1); // CSINV
        prop_assert_eq!(o1_of(w), 0);
        prop_assert_eq!(cond_of(w), cv);
        prop_assert_eq!(rm_of(w), rm);
        prop_assert_eq!(rn_of(w), rn);
        prop_assert_eq!(rd_of(w), rd);
    }

    #[test]
    fn csneg_happy_path(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
        is_64 in any::<bool>(), ci in 0usize..VALID_CONDS.len(),
    ) {
        let (cv, cn) = VALID_CONDS[ci];
        let sf = u32::from(is_64);
        let ops = vec![gp_reg(rd, is_64), gp_reg(rn, is_64), gp_reg(rm, is_64),
                       Operand::Cond(cn.into())];
        let w = word(encode_csneg(&ops));
        prop_assert_eq!(w, csneg_ref(rd, rn, rm, sf, cv));
        prop_assert_eq!(sf_of(w), sf);
        prop_assert_eq!(op30_of(w), 1); // CSNEG
        prop_assert_eq!(o1_of(w), 1);
        prop_assert_eq!(cond_of(w), cv);
        prop_assert_eq!(rm_of(w), rm);
        prop_assert_eq!(rn_of(w), rn);
        prop_assert_eq!(rd_of(w), rd);
    }

    #[test]
    fn cinc_happy_path(
        rd in 0u32..=30, rn in 0u32..=30, is_64 in any::<bool>(),
        ci in 0usize..VALID_CONDS.len(),
    ) {
        let (cv, cn) = VALID_CONDS[ci];
        let sf = u32::from(is_64);
        let ops = vec![gp_reg(rd, is_64), gp_reg(rn, is_64), Operand::Cond(cn.into())];
        let w = word(encode_cinc(&ops));
        prop_assert_eq!(w, cinc_ref(rd, rn, sf, cv));
        prop_assert_eq!(sf_of(w), sf);
        prop_assert_eq!(op30_of(w), 0);
        prop_assert_eq!(o1_of(w), 1);
        // CINC -> CSINC with Rm forced == Rn.
        prop_assert_eq!(rm_of(w), rn);
        prop_assert_eq!(rn_of(w), rn);
        prop_assert_eq!(rd_of(w), rd);
        // The alias INVERTS the condition (inv_cond = cond ^ 1).
        prop_assert_eq!(cond_of(w), cv ^ 1);
    }

    #[test]
    fn cinv_happy_path(
        rd in 0u32..=30, rn in 0u32..=30, is_64 in any::<bool>(),
        ci in 0usize..VALID_CONDS.len(),
    ) {
        let (cv, cn) = VALID_CONDS[ci];
        let sf = u32::from(is_64);
        let ops = vec![gp_reg(rd, is_64), gp_reg(rn, is_64), Operand::Cond(cn.into())];
        let w = word(encode_cinv(&ops));
        prop_assert_eq!(w, cinv_ref(rd, rn, sf, cv));
        prop_assert_eq!(sf_of(w), sf);
        prop_assert_eq!(op30_of(w), 1);
        prop_assert_eq!(o1_of(w), 0);
        prop_assert_eq!(rm_of(w), rn);
        prop_assert_eq!(rn_of(w), rn);
        prop_assert_eq!(rd_of(w), rd);
        prop_assert_eq!(cond_of(w), cv ^ 1);
    }

    #[test]
    fn cneg_happy_path(
        rd in 0u32..=30, rn in 0u32..=30, is_64 in any::<bool>(),
        ci in 0usize..VALID_CONDS.len(),
    ) {
        let (cv, cn) = VALID_CONDS[ci];
        let sf = u32::from(is_64);
        let ops = vec![gp_reg(rd, is_64), gp_reg(rn, is_64), Operand::Cond(cn.into())];
        let w = word(encode_cneg(&ops));
        prop_assert_eq!(w, cneg_ref(rd, rn, sf, cv));
        prop_assert_eq!(sf_of(w), sf);
        prop_assert_eq!(op30_of(w), 1);
        prop_assert_eq!(o1_of(w), 1);
        prop_assert_eq!(rm_of(w), rn);
        prop_assert_eq!(rn_of(w), rn);
        prop_assert_eq!(rd_of(w), rd);
        prop_assert_eq!(cond_of(w), cv ^ 1);
    }
}

proptest! {
    // ── CHARACTERISATION (PASSING today; pins the current buggy behaviour).
    //
    // (A) FP/SIMD + SP acceptance for encode_csel / cinc / cinv / cneg.
    //     The *bug* is that these spellings are accepted at all; these
    //     properties merely document HOW the encoder currently mishandles them.

    // A1. SP/WSP alias onto register-field value 31 (== ZR) in every operand
    //     position of every Group-1 encoder.
    #[test]
    fn sp_wsp_is_rejected(
        n in 0u32..=30, gidx in 0u32..=3u32, pos in 0u32..=2u32, vi in 0u32..=1u32,
    ) {
        // SP/WSP must be REJECTED (the encoders previously aliased them to
        // field 31 == ZR). Strict register classes are now enforced.
        let (name, enc, arity) = G1[(gidx as usize) % G1.len()];
        let pos = pos % arity;
        let mut ops = g1_base(name, n);
        ops[pos as usize] = sp_variant(vi);
        prop_assert!(enc(&ops).is_err(),
            "{} with SP/WSP at position {} must be rejected", name, pos);
    }

    // A2. An FP/SIMD register name is silently accepted; its lane number is
    //     placed into the operand field exactly like the same-numbered GP reg.
    #[test]
    fn fp_simd_is_rejected(
        n in 0u32..=30, gidx in 0u32..=3u32, pos in 0u32..=2u32, vi in 0u32..=5u32,
    ) {
        // FP/SIMD register spellings must be REJECTED in GPR operand
        // positions (previously the lane number was placed verbatim).
        let (name, enc, arity) = G1[(gidx as usize) % G1.len()];
        let pos = pos % arity;
        let mut ops = g1_base(name, n);
        ops[pos as usize] = fp_variant(vi, n);
        prop_assert!(enc(&ops).is_err(),
            "{} with FP/SIMD at position {} must be rejected", name, pos);
    }

    // (B) Mixed-width acceptance for encode_csel / csinc / csinv / csneg.
    //     `sf` is taken ONLY from Rd; Rn/Rm widths are ignored and their
    //     register numbers are placed verbatim.

    // B1. Under a width mismatch, sf tracks ONLY Rd, while the register fields
    //     are placed by NUMBER (width-independent).
    #[test]
    fn mixed_width_is_rejected(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
        rd64 in any::<bool>(), rn64 in any::<bool>(), rm64 in any::<bool>(),
        gidx in 0u32..=3u32,
    ) {
        prop_assume!(!(rd64 == rn64 && rn64 == rm64)); // require a width mismatch
        // Mixed x/w operand widths must be REJECTED (previously sf was
        // silently taken from Rd alone).
        let (name, enc) = G2[(gidx as usize) % G2.len()];
        let ops = vec![gp_reg(rd, rd64), gp_reg(rn, rn64), gp_reg(rm, rm64),
                       Operand::Cond("eq".into())];
        prop_assert!(enc(&ops).is_err(),
            "{} with mixed x/w widths must be rejected", name);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  BUG WITNESSES — all #[ignore]'d so a default `cargo test` stays green.
//  Run:  cargo test --lib compare_branch_condselect_regclass_pbt -- --ignored
//  Each asserts the SPEC-CORRECT rejection the encoder currently violates.
//  Differential oracle: `clang --target=aarch64-linux-gnu` rejects every one
//  of these spellings ("error: invalid operand for instruction" / "invalid
//  operand width").
// ═══════════════════════════════════════════════════════════════════════════
proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    // ── encode_csel : SP/WSP + FP/SIMD ────────────────────────────────────
    #[test]
    fn csel_rejects_sp_wsp_in_any_position(
        n in 0u32..=30, pos in 0u32..3u32, vi in 0u32..=1u32,
    ) {
        let mut ops = g1_base("csel", n);
        ops[pos as usize] = sp_variant(vi);
        prop_assert!(encode_csel(&ops).is_err(),
            "csel with SP/WSP at position {} must be Err, got {:?}", pos, encode_csel(&ops));
    }

    #[test]
    fn csel_rejects_fp_simd_in_any_position(
        n in 0u32..=30, pos in 0u32..3u32, vi in 0u32..=5u32,
    ) {
        let mut ops = g1_base("csel", n);
        ops[pos as usize] = fp_variant(vi, n);
        prop_assert!(encode_csel(&ops).is_err(),
            "csel with FP/SIMD at position {} must be Err, got {:?}", pos, encode_csel(&ops));
    }

    // ── encode_cinc : SP/WSP + FP/SIMD ────────────────────────────────────
    #[test]
    fn cinc_rejects_sp_wsp_in_any_position(
        n in 0u32..=30, pos in 0u32..2u32, vi in 0u32..=1u32,
    ) {
        let mut ops = g1_base("cinc", n);
        ops[pos as usize] = sp_variant(vi);
        prop_assert!(encode_cinc(&ops).is_err(),
            "cinc with SP/WSP at position {} must be Err, got {:?}", pos, encode_cinc(&ops));
    }

    #[test]
    fn cinc_rejects_fp_simd_in_any_position(
        n in 0u32..=30, pos in 0u32..2u32, vi in 0u32..=5u32,
    ) {
        let mut ops = g1_base("cinc", n);
        ops[pos as usize] = fp_variant(vi, n);
        prop_assert!(encode_cinc(&ops).is_err(),
            "cinc with FP/SIMD at position {} must be Err, got {:?}", pos, encode_cinc(&ops));
    }

    // ── encode_cinv : SP/WSP + FP/SIMD ────────────────────────────────────
    #[test]
    fn cinv_rejects_sp_wsp_in_any_position(
        n in 0u32..=30, pos in 0u32..2u32, vi in 0u32..=1u32,
    ) {
        let mut ops = g1_base("cinv", n);
        ops[pos as usize] = sp_variant(vi);
        prop_assert!(encode_cinv(&ops).is_err(),
            "cinv with SP/WSP at position {} must be Err, got {:?}", pos, encode_cinv(&ops));
    }

    #[test]
    fn cinv_rejects_fp_simd_in_any_position(
        n in 0u32..=30, pos in 0u32..2u32, vi in 0u32..=5u32,
    ) {
        let mut ops = g1_base("cinv", n);
        ops[pos as usize] = fp_variant(vi, n);
        prop_assert!(encode_cinv(&ops).is_err(),
            "cinv with FP/SIMD at position {} must be Err, got {:?}", pos, encode_cinv(&ops));
    }

    // ── encode_cneg : SP/WSP + FP/SIMD ────────────────────────────────────
    #[test]
    fn cneg_rejects_sp_wsp_in_any_position(
        n in 0u32..=30, pos in 0u32..2u32, vi in 0u32..=1u32,
    ) {
        let mut ops = g1_base("cneg", n);
        ops[pos as usize] = sp_variant(vi);
        prop_assert!(encode_cneg(&ops).is_err(),
            "cneg with SP/WSP at position {} must be Err, got {:?}", pos, encode_cneg(&ops));
    }

    #[test]
    fn cneg_rejects_fp_simd_in_any_position(
        n in 0u32..=30, pos in 0u32..2u32, vi in 0u32..=5u32,
    ) {
        let mut ops = g1_base("cneg", n);
        ops[pos as usize] = fp_variant(vi, n);
        prop_assert!(encode_cneg(&ops).is_err(),
            "cneg with FP/SIMD at position {} must be Err, got {:?}", pos, encode_cneg(&ops));
    }

    // ── encode_csel / csinc / csinv / csneg : mixed W/X widths ───────────
    #[test]
    fn csel_rejects_mixed_widths(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
        rd64 in any::<bool>(), rn64 in any::<bool>(), rm64 in any::<bool>(),
        ci in 0usize..VALID_CONDS.len(),
    ) {
        prop_assume!(!(rd64 == rn64 && rn64 == rm64)); // require a width mismatch
        let (_, cn) = VALID_CONDS[ci];
        let ops = vec![gp_reg(rd, rd64), gp_reg(rn, rn64), gp_reg(rm, rm64),
                       Operand::Cond(cn.into())];
        prop_assert!(encode_csel(&ops).is_err(),
            "csel with mixed W/X widths must be Err, got {:?}", encode_csel(&ops));
    }

    #[test]
    fn csinc_rejects_mixed_widths(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
        rd64 in any::<bool>(), rn64 in any::<bool>(), rm64 in any::<bool>(),
        ci in 0usize..VALID_CONDS.len(),
    ) {
        prop_assume!(!(rd64 == rn64 && rn64 == rm64));
        let (_, cn) = VALID_CONDS[ci];
        let ops = vec![gp_reg(rd, rd64), gp_reg(rn, rn64), gp_reg(rm, rm64),
                       Operand::Cond(cn.into())];
        prop_assert!(encode_csinc(&ops).is_err(),
            "csinc with mixed W/X widths must be Err, got {:?}", encode_csinc(&ops));
    }

    #[test]
    fn csinv_rejects_mixed_widths(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
        rd64 in any::<bool>(), rn64 in any::<bool>(), rm64 in any::<bool>(),
        ci in 0usize..VALID_CONDS.len(),
    ) {
        prop_assume!(!(rd64 == rn64 && rn64 == rm64));
        let (_, cn) = VALID_CONDS[ci];
        let ops = vec![gp_reg(rd, rd64), gp_reg(rn, rn64), gp_reg(rm, rm64),
                       Operand::Cond(cn.into())];
        prop_assert!(encode_csinv(&ops).is_err(),
            "csinv with mixed W/X widths must be Err, got {:?}", encode_csinv(&ops));
    }

    #[test]
    fn csneg_rejects_mixed_widths(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
        rd64 in any::<bool>(), rn64 in any::<bool>(), rm64 in any::<bool>(),
        ci in 0usize..VALID_CONDS.len(),
    ) {
        prop_assume!(!(rd64 == rn64 && rn64 == rm64));
        let (_, cn) = VALID_CONDS[ci];
        let ops = vec![gp_reg(rd, rd64), gp_reg(rn, rn64), gp_reg(rm, rm64),
                       Operand::Cond(cn.into())];
        prop_assert!(encode_csneg(&ops).is_err(),
            "csneg with mixed W/X widths must be Err, got {:?}", encode_csneg(&ops));
    }
}
