//! Property-based tests for the **logical-NOT / inverted-operand** shifted-register
//! encoders in `data_processing.rs`:
//!
//! * `encode_eon`  — EON  <Rd>,<Rn>,<Rm>{,<shift>}  (opc=10, N=1)
//! * `encode_orn`  — ORN  <Rd>,<Rn>,<Rm>{,<shift>}  (opc=01, N=1)
//! * `encode_bic`  — BIC  <Rd>,<Rn>,<Rm>{,<shift>}  (opc=00, N=1)
//! * `encode_bics` — BICS <Rd>,<Rn>,<Rm>{,<shift>}  (opc=11, N=1)
//! * `encode_mvn`  — MVN  <Rd>,<Rm>{,<shift>}        (alias ORN <Rd>,XZR,<Rm>; opc=01, N=1, Rn=11111)
//!
//! ## ARMv8-A encoding (Logical shifted register), §C4.1.115
//!
//! ```text
//!  31  30:29  28:24  23:22  21  20:16  15:10   9:5   4:0
//!   sf   opc  0 1 0 1 0  shift  N    Rm    imm6   Rn    Rd
//! ```
//! `opc` ∈ {00=AND/BIC, 01=ORR/ORN, 10=EOR/EON, 11=ANDS/BICS}; `N=1` selects the
//! inverted-operand variant. For `sf=0` the legal shift range is `0..=31`
//! (imm6 bit 5 must be 0); for `sf=1` it is `0..=63`. Only the four shift kinds
//! `LSL/LSR/ASR/ROR` are architecturally defined.
//!
//! ## What is NEW here
//!
//! This suite targets two defect classes that are **not yet reported** for the
//! bulk of these functions (the pre-existing inline suite + `bug_reports/`
//! already cover EON and ORN width-mixing, and EON unknown-shift-kind):
//!
//! 1. **Register-width mixing** — `encode_bic`, `encode_bics`, and `encode_mvn`
//!    derive `sf` solely from the destination and discard the source widths, so
//!    mixed X/W operands assemble silently at the destination width. NEW for
//!    these three.
//! 2. **Unknown shift kinds** — `encode_orn`, `encode_bic`, `encode_bics`, and
//!    `encode_mvn` map any unrecognized shift kind to `LSL` via a catch-all
//!    `_ => 0b00` arm instead of returning `Err`. NEW for these four.
//!
//! The bug **witnesses** (properties asserting the spec-correct rejection) are
//! all marked `#[ignore]`, so default `cargo test` stays green; run them with:
//!
//! ```text
//! cargo test --test pbt_arm_core data_processing_logical_not_pbt -- --ignored
//! ```
//! Two passing *characterisation* properties pin the **current (buggy)**
//! behaviour (`sf` from Rd only; unknown kind ⇒ LSL); they will start failing
//! once the defects are fixed — the cue to drop the `#[ignore]` markers.

use lccc::pbt_internals::Operand;
use lccc::pbt_internals::*;
use proptest::prelude::*;

// ── field extractors (ARMv8 Logical shifted register) ─────────────────────
fn sf_of(w: u32) -> u32 {
    (w >> 31) & 1
}
fn opc_of(w: u32) -> u32 {
    (w >> 29) & 0x3
}
fn opcode5_of(w: u32) -> u32 {
    (w >> 24) & 0x1F
} // bits 28:24 == 01010
fn shift_type_of(w: u32) -> u32 {
    (w >> 22) & 0x3
}
fn n21_of(w: u32) -> u32 {
    (w >> 21) & 1
} // N=1 ⇒ inverted operand
fn rm_of(w: u32) -> u32 {
    (w >> 16) & 0x1F
}
fn imm6_of(w: u32) -> u32 {
    (w >> 10) & 0x3F
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
fn shift(kind: &str, amount: u32) -> Operand {
    Operand::Shift {
        kind: kind.into(),
        amount,
    }
}
/// The four architecturally-defined shift kinds and their 2-bit field encodings.
const KNOWN_SHIFTS: &[(&str, u32)] = &[("lsl", 0b00), ("lsr", 0b01), ("asr", 0b10), ("ror", 0b11)];
/// Unrecognized shift kinds that must be rejected (used by index selection so
/// proptest can shrink to a minimal witness without closure-typing friction).
const BOGUS_SHIFTS: &[&str] = &["foo", "bar", "LSL", "xxy", "zz", "msl", "extend"];

fn word(r: Result<EncodeResult, String>) -> u32 {
    match r {
        Ok(EncodeResult::Word(w)) => w,
        Ok(other) => panic!("expected Word, got {:?}", other),
        Err(e) => panic!("expected Ok, got Err: {}", e),
    }
}

/// Encoder table for the three-operand logical-NOT encoders, paired with each
/// instruction's `opc` field so parametric properties can check them uniformly.
type Enc = fn(&[Operand]) -> Result<EncodeResult, String>;
const THREE_OP: &[(&str, u32, Enc)] = &[
    ("eon", 0b10, encode_eon),
    ("orn", 0b01, encode_orn),
    ("bic", 0b00, encode_bic),
    ("bics", 0b11, encode_bics),
];

proptest! {
    // ── P1. HAPPY-PATH FIELD PLACEMENT (passing guard) ────────────────────
    // For each 3-operand logical-NOT encoder, all-X operands with a valid LSL
    // shift must place every fixed field (opc, class=01010, N=1, sf=1) and every
    // register field exactly per the ARMv8 encoding.
    #[test]
    fn happy_path_field_placement_all_encoders(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
        amount in 0u32..=63u32,
    ) {
        let ops = vec![xreg(rd), xreg(rn), xreg(rm), shift("lsl", amount)];
        for &(name, opc, f) in THREE_OP {
            let w = word(f(&ops));
            prop_assert_eq!(sf_of(w), 1, "{} sf", name);
            prop_assert_eq!(opc_of(w), opc, "{} opc", name);
            prop_assert_eq!(opcode5_of(w), 0b01010, "{} class", name);
            prop_assert_eq!(n21_of(w), 1, "{} N", name);
            prop_assert_eq!(shift_type_of(w), 0b00, "{} shift_type", name);
            prop_assert_eq!(imm6_of(w), amount, "{} imm6", name);
            prop_assert_eq!(rm_of(w), rm, "{} rm", name);
            prop_assert_eq!(rn_of(w), rn, "{} rn", name);
            prop_assert_eq!(rd_of(w), rd, "{} rd", name);
        }
    }

    // ── P2. KNOWN SHIFT KINDS MAP CORRECTLY (passing guard) ───────────────
    // lsl/lsr/asr/ror map to the 2-bit shift field 00/01/10/11 for every
    // encoder; the amount round-trips through imm6 (X-register range).
    #[test]
    fn known_shift_kinds_map_to_2bit_field(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
        si in 0u32..=3u32, amount in 0u32..=63u32,
    ) {
        let (kind, want) = KNOWN_SHIFTS[si as usize];
        let ops3 = vec![xreg(rd), xreg(rn), xreg(rm), shift(kind, amount)];
        for &(name, _opc, f) in THREE_OP {
            let w = word(f(&ops3));
            prop_assert_eq!(shift_type_of(w), want, "{} {} shift_type", name, kind);
            prop_assert_eq!(imm6_of(w), amount, "{} {} imm6", name, kind);
        }
        // mvn (2 operands: Rd, Rm)
        let ops_mv = vec![xreg(rd), xreg(rm), shift(kind, amount)];
        let w = word(encode_mvn(&ops_mv));
        prop_assert_eq!(shift_type_of(w), want, "mvn {} shift_type", kind);
        prop_assert_eq!(imm6_of(w), amount, "mvn {} imm6", kind);
    }

    // ── P3. MVN ALIAS = ORN Rd, XZR, Rm (passing differential) ────────────
    // `mvn Rd, Rm{,shift}` must encode bit-identically to
    // `orn Rd, XZR, Rm{,shift}` for matched widths — the defining equivalence.
    #[test]
    fn mvn_equals_orn_with_xzr_rn(
        rd in 0u32..=30, rm in 0u32..=30,
        sk in 0u32..=3u32, amount in 0u32..=63u32, is_64 in any::<bool>(),
    ) {
        let (kind, _) = KNOWN_SHIFTS[sk as usize];
        // imm6 is width-dependent: 0..=31 for W, 0..=63 for X (out-of-range
        // amounts are unallocated, not silently masked).
        let max_amt: u32 = if is_64 { 63 } else { 31 };
        let amount = amount % (max_amt + 1);
        let mk = |n: u32| if is_64 { xreg(n) } else { wreg(n) };
        let zr = Operand::Reg(if is_64 { "xzr".into() } else { "wzr".into() });
        let mvn_ops = vec![mk(rd), mk(rm), shift(kind, amount)];
        let orn_ops = vec![mk(rd), zr, mk(rm), shift(kind, amount)];
        prop_assert_eq!(word(encode_mvn(&mvn_ops)), word(encode_orn(&orn_ops)));
    }

    // ── P4. BICS vs BIC differ ONLY in opc (passing differential) ─────────
    // BICS (opc=11) is the flag-setting twin of BIC (opc=00); identical
    // operands must produce words differing in exactly bits 30:29 (0x6000_0000).
    #[test]
    fn bics_xor_bic_is_opc_bits(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
        sk in 0u32..=3u32, amount in 0u32..=63u32,
    ) {
        let (kind, _) = KNOWN_SHIFTS[sk as usize];
        let ops = vec![xreg(rd), xreg(rn), xreg(rm), shift(kind, amount)];
        let b = word(encode_bic(&ops));
        let s = word(encode_bics(&ops));
        prop_assert_eq!(s ^ b, 0x6000_0000u32);
    }

    // ── P5. CHARACTERISATION (current buggy behaviour): sf from Rd ONLY ───
    // PASSING today. The encoders derive `sf` exclusively from the destination
    // register; source widths are ignored. This pins that behaviour — when the
    // width-mixing defect is fixed this property starts failing, which is the
    // cue to delete it and rely on the `*_rejects_mixed_*` witnesses below.
    #[test]
    fn sf_rejected_on_width_mismatch(
        n in 0u32..=30, rd_is_x in any::<bool>(),
        rn_is_x in any::<bool>(), rm_is_x in any::<bool>(),
    ) {
        // Mixed x/w operand widths must be REJECTED (previously sf was
        // silently taken from Rd alone).
        let mk = |is_x: bool, n: u32| if is_x { xreg(n) } else { wreg(n) };
        prop_assume!(rd_is_x != rn_is_x || rn_is_x != rm_is_x);
        let ops3 = vec![mk(rd_is_x, n), mk(rn_is_x, n), mk(rm_is_x, n)];
        for &(_name, _opc, f) in THREE_OP {
            prop_assert!(f(&ops3).is_err(), "three-op mixed width must be rejected");
        }
        prop_assume!(rd_is_x != rm_is_x);
        let mvn_ops = vec![mk(rd_is_x, n), mk(rm_is_x, n)];
        prop_assert!(encode_mvn(&mvn_ops).is_err(), "mvn mixed width must be rejected");
    }

}

// ═══════════════════════════════════════════════════════════════════════════
//  REGRESSION PROPERTIES (former #[ignore]d witnesses, now active: the
//  encoders reject unknown shift kinds, mixed widths, and surplus operands).
// ═══════════════════════════════════════════════════════════════════════════
proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    // ── W1. NEW: encode_bic silently accepts mixed register widths ────────
    // BIC requires <Rd>,<Rn>,<Rm> to share one width (ARMv8 ARM §C4.1.115;
    // GAS/llvm-mc reject `bic x0,w1,x2` with "operand size mismatch"). The
    // encoder derives sf from Rd only, so it returns Ok — this witness FAILS.
    #[test]
    fn bic_rejects_mixed_register_widths(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
        mix in 0u32..=5u32,
    ) {
        let (d, n, m) = match mix {
            0 => (xreg(rd), wreg(rn), wreg(rm)),   // X dest, W sources
            1 => (wreg(rd), xreg(rn), xreg(rm)),   // W dest, X sources
            2 => (xreg(rd), xreg(rn), wreg(rm)),   // one W source
            3 => (xreg(rd), wreg(rn), xreg(rm)),
            4 => (wreg(rd), xreg(rn), wreg(rm)),
            _ => (wreg(rd), wreg(rn), xreg(rm)),
        };
        prop_assert!(encode_bic(&[d, n, m]).is_err(), "bic mixed width must be Err");
    }

    // ── W2. NEW: encode_bics silently accepts mixed register widths ───────
    #[test]
    fn bics_rejects_mixed_register_widths(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
        mix in 0u32..=5u32,
    ) {
        let (d, n, m) = match mix {
            0 => (xreg(rd), wreg(rn), wreg(rm)),
            1 => (wreg(rd), xreg(rn), xreg(rm)),
            2 => (xreg(rd), xreg(rn), wreg(rm)),
            3 => (xreg(rd), wreg(rn), xreg(rm)),
            4 => (wreg(rd), xreg(rn), wreg(rm)),
            _ => (wreg(rd), wreg(rn), xreg(rm)),
        };
        prop_assert!(encode_bics(&[d, n, m]).is_err(), "bics mixed width must be Err");
    }

    // ── W3. NEW: encode_mvn silently accepts mixed Rd/Rm widths ───────────
    // MVN <Rd>,<Rm> needs both operands to share width; `mvn x0,w1` is illegal
    // (GAS: "operand size mismatch"). sf is taken from Rd only → Ok. FAILS.
    #[test]
    fn mvn_rejects_mixed_register_widths(
        rd in 0u32..=30, rm in 0u32..=30, mix in 0u32..=1u32,
    ) {
        let ops = match mix {
            0 => vec![xreg(rd), wreg(rm)],   // X dest, W source
            _ => vec![wreg(rd), xreg(rm)],   // W dest, X source
        };
        prop_assert!(encode_mvn(&ops).is_err(), "mvn mixed width must be Err");
    }

    // ── W4. NEW: encode_orn coerces unknown shift kind to LSL ─────────────
    // Only lsl/lsr/asr/ror are defined; `orn x0,x1,x2,foo #1` must be Err.
    // The catch-all `_ => 0b00` arm returns Ok (encoded as LSL). FAILS.
    #[test]
    fn orn_rejects_unknown_shift_kind(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
        amount in 0u32..=63u32, bi in 0u32..=6u32,
    ) {
        let kind = BOGUS_SHIFTS[bi as usize];
        let ops = vec![xreg(rd), xreg(rn), xreg(rm), shift(kind, amount)];
        prop_assert!(encode_orn(&ops).is_err(), "orn unknown shift kind must be Err");
    }

    // ── W5. NEW: encode_bic coerces unknown shift kind to LSL ─────────────
    #[test]
    fn bic_rejects_unknown_shift_kind(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
        amount in 0u32..=63u32, bi in 0u32..=6u32,
    ) {
        let kind = BOGUS_SHIFTS[bi as usize];
        let ops = vec![xreg(rd), xreg(rn), xreg(rm), shift(kind, amount)];
        prop_assert!(encode_bic(&ops).is_err(), "bic unknown shift kind must be Err");
    }

    // ── W6. NEW: encode_bics coerces unknown shift kind to LSL ────────────
    #[test]
    fn bics_rejects_unknown_shift_kind(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
        amount in 0u32..=63u32, bi in 0u32..=6u32,
    ) {
        let kind = BOGUS_SHIFTS[bi as usize];
        let ops = vec![xreg(rd), xreg(rn), xreg(rm), shift(kind, amount)];
        prop_assert!(encode_bics(&ops).is_err(), "bics unknown shift kind must be Err");
    }

    // ── W7. NEW: encode_mvn coerces unknown shift kind to LSL ─────────────
    // (Distinct from the existing muddled report about W-register ror/asr; this
    //  is the genuine catch-all `_ => 0b00` path for *any* unknown kind.)
    #[test]
    fn mvn_rejects_unknown_shift_kind(
        rd in 0u32..=30, rm in 0u32..=30,
        amount in 0u32..=63u32, bi in 0u32..=6u32,
    ) {
        let kind = BOGUS_SHIFTS[bi as usize];
        let ops = vec![xreg(rd), xreg(rm), shift(kind, amount)];
        prop_assert!(encode_mvn(&ops).is_err(), "mvn unknown shift kind must be Err");
    }

    // ── W8. PARAMETRIC BREADTH: every logical-NOT encoder rejects mixed width ─
    // Surfaces ALL affected functions (new: bic/bics/mvn; known: orn/eon) in one
    // shrinkable failure. FAILS for the buggy encoders.
    #[test]
    fn all_logical_not_encoders_reject_mixed_width(
        n in 0u32..=30, mix in 0u32..=5u32,
    ) {
        let (d, n_, m) = match mix {
            0 => (xreg(n), wreg(n), wreg(n)),
            1 => (wreg(n), xreg(n), xreg(n)),
            2 => (xreg(n), xreg(n), wreg(n)),
            3 => (xreg(n), wreg(n), xreg(n)),
            4 => (wreg(n), xreg(n), wreg(n)),
            _ => (wreg(n), wreg(n), xreg(n)),
        };
        for &(name, _opc, f) in THREE_OP {
            prop_assert!(f(&[d.clone(), n_.clone(), m.clone()]).is_err(),
                "{} should reject mixed width", name);
        }
        // MVN takes only Rd and Rm; the three-operand mixing patterns above
        // do not always make THAT pair mixed (e.g. (x, w, x) gives MVN
        // (x, x)), so the rejection is only asserted for genuinely mixed
        // two-operand inputs.
        let d_is_x = matches!(mix, 0 | 2 | 3);
        let m_is_x = matches!(mix, 1 | 3 | 5);
        if d_is_x != m_is_x {
            prop_assert!(encode_mvn(&[d, m]).is_err(), "mvn should reject mixed width");
        }
    }

    // ── W9. PARAMETRIC BREADTH: every logical-NOT encoder rejects unknown shift ─
    // Surfaces ALL affected functions (new: orn/bic/bics/mvn; known: eon) in one
    // shrinkable failure. FAILS for the buggy encoders.
    #[test]
    fn all_logical_not_encoders_reject_unknown_shift_kind(
        rd in 0u32..=30, rn in 0u32..=30, rm in 0u32..=30,
        amount in 0u32..=63u32, bi in 0u32..=6u32,
    ) {
        let kind = BOGUS_SHIFTS[bi as usize];
        let ops3 = vec![xreg(rd), xreg(rn), xreg(rm), shift(kind, amount)];
        for &(name, _opc, f) in THREE_OP {
            prop_assert!(f(&ops3).is_err(), "{} should reject unknown shift kind", name);
        }
        let ops_mv = vec![xreg(rd), xreg(rm), shift(kind, amount)];
        prop_assert!(encode_mvn(&ops_mv).is_err(), "mvn should reject unknown shift kind");
    }
}
