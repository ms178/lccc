//! Property-based tests for the AArch64 scalar FP "data-processing (1 source)"
//! encoders [`encode_fneg`], [`encode_fabs`], [`encode_fsqrt`] and the FP
//! precision-conversion encoder [`encode_fcvt_precision`].
//!
//! # Reference oracle (cross-checked against `llvm-mc --arch=aarch64`)
//!
//! FNEG / FABS / FSQRT share the "Floating-point data-processing (1 source)"
//! layout (ARM ARM §C5.6):
//!   `0 00 11110 ftype 1 opcode 10000 Rn Rd`
//!     [31]=0, [30:24]=0011110 (0x1E), [23:22]=ftype (00=S, 01=D),
//!     [21]=1 (fixed), [20:15]=opcode (6-bit), [14:10]=10000 (fixed),
//!     [9:5]=Rn (source), [4:0]=Rd (dest).
//!   opcodes: FABS=000001, FNEG=000010, FSQRT=000011.
//!
//! FCVT uses the same 1-source template with a fixed opcode prefix and a 2-bit
//! destination-precision selector `opc`:
//!   `0 00 11110 ftype 1 0001 opc 10000 Rn Rd`
//!     ftype = source precision (00=S, 01=D, 11=H), opc = dest precision.
//!
//! Worked examples (canonical assembler output, little-endian → u32):
//!   fneg  s0,s0 = 0x1E214000   fneg  d0,d0 = 0x1E614000
//!   fabs  s0,s0 = 0x1E20C000   fabs  d0,d0 = 0x1E60C000
//!   fsqrt s0,s0 = 0x1E21C000   fsqrt d0,d0 = 0x1E61C000
//!   fcvt  d0,s0 = 0x1E22C000   fcvt  s0,d0 = 0x1E624000
//!   fcvt  h0,s0 = 0x1E23C000   fcvt  s0,h0 = 0x1EE24000
//!   fcvt  d0,h0 = 0x1EE2C000   fcvt  h0,d0 = 0x1E63C000
//!
//! # Findings surfaced
//!
//! The bit-packing is **correct** for valid operands: every field lands at its
//! canonical ARMv8 bit position with no truncation, arity / immediate / GP-bank
//! / out-of-range register inputs are rejected, and the encodings match the
//! canonical assembler bit-for-bit (properties P1–P8, plus the concrete KATs).
//!
//! Two **validation bugs** are exposed as witness properties B1–B2. They are
//! marked `#[ignore]` so `cargo test` stays green; run them with
//! `cargo test --test pbt_arm_fp_scalar -- --ignored fneg_fabs_fsqrt_fcvt`. The encoders silently
//! accept operands the architecture treats as UNDEFINED:
//!   * B1 — FNEG/FABS/FSQRT derive `ftype` only from the destination prefix and
//!     validate neither operand's precision nor its bank, so mixed-precision
//!     (`fneg d0,s0`) and GP-bank (`fneg x0,x0`) operands are accepted;
//!   * B2 — FCVT must reject same-precision conversions (ARM ARM constraint
//!     `opc != ftype`); `fcvt s0,s0` / `d0,d0` / `h0,h0` are UNDEFINED but
//!     accepted.

use lccc::pbt_internals::Operand;
use lccc::pbt_internals::*;
use proptest::prelude::*;

// ── field extractors for the FP 1-source / FCVT layout ───────────────────
fn rd_of(w: u32) -> u32 {
    w & 0x1F
}
fn rn_of(w: u32) -> u32 {
    (w >> 5) & 0x1F
}
/// bits[14:10] fixed "10000" field.
fn fixed10_of(w: u32) -> u32 {
    (w >> 10) & 0x1F
}
/// bits[20:15] 6-bit opcode field (FNEG/FABS/FSQRT).
fn opcode6_of(w: u32) -> u32 {
    (w >> 15) & 0x3F
}
/// bits[16:15] 2-bit `opc` (FCVT destination precision).
fn fcvt_opc_of(w: u32) -> u32 {
    (w >> 15) & 0x3
}
/// bits[18:17] FCVT fixed opcode prefix (must be 0001).
fn fcvt_opc_prefix_of(w: u32) -> u32 {
    (w >> 17) & 0xF
}
fn ftype_of(w: u32) -> u32 {
    (w >> 22) & 0x3
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

/// Reference word for an FP 1-source op (FNEG/FABS/FSQRT), verified against
/// `llvm-mc` for every `opcode`.
fn unary_ref(opcode: u32, ftype: u32, rn: u32, rd: u32) -> u32 {
    0x1E000000u32 | (ftype << 22) | (1 << 21) | (opcode << 15) | (0b10000 << 10) | (rn << 5) | rd
}

/// Reference word for FCVT, verified against `llvm-mc` for all six legal
/// source/dest precision combinations.
fn fcvt_ref(ftype: u32, opc: u32, rn: u32, rd: u32) -> u32 {
    0x1E000000u32
        | (ftype << 22)
        | (1 << 21)
        | (0b0001 << 17)
        | (opc << 15)
        | (0b10000 << 10)
        | (rn << 5)
        | rd
}

/// Scalar FP precision prefix → ftype/opc code: S→00, D→01, H→11.
fn prec_code(prefix: &str) -> u32 {
    match prefix.chars().next().unwrap_or(' ') {
        's' => 0b00,
        'd' => 0b01,
        'h' => 0b11,
        _ => unreachable!("prec_code only called for s/d/h"),
    }
}

// The three 1-source ops under test, as (encoder, name, opcode) triples so the
// family properties generalise the layout invariants across FNEG/FABS/FSQRT.
type Enc = fn(&[Operand]) -> Result<EncodeResult, String>;
const UNARY_OPS: &[(Enc, &str, u32)] = &[
    (encode_fneg, "fneg", 0b000010),
    (encode_fabs, "fabs", 0b000001),
    (encode_fsqrt, "fsqrt", 0b000011),
];

// ── Concrete cross-checks against `llvm-mc --arch=aarch64` output ────────
#[test]
fn concrete_fneg_fabs_fsqrt_match_llvm_mc() {
    // Single-precision base words (Rn=Rd=0).
    assert_eq!(
        expect_word(encode_fneg(&reg2("s", 0, "s", 0))),
        0x1E214000,
        "FNEG S0,S0"
    );
    assert_eq!(
        expect_word(encode_fabs(&reg2("s", 0, "s", 0))),
        0x1E20C000,
        "FABS S0,S0"
    );
    assert_eq!(
        expect_word(encode_fsqrt(&reg2("s", 0, "s", 0))),
        0x1E21C000,
        "FSQRT S0,S0"
    );
    // Double-precision base words.
    assert_eq!(
        expect_word(encode_fneg(&reg2("d", 0, "d", 0))),
        0x1E614000,
        "FNEG D0,D0"
    );
    assert_eq!(
        expect_word(encode_fabs(&reg2("d", 0, "d", 0))),
        0x1E60C000,
        "FABS D0,D0"
    );
    assert_eq!(
        expect_word(encode_fsqrt(&reg2("d", 0, "d", 0))),
        0x1E61C000,
        "FSQRT D0,D0"
    );
}

#[test]
fn concrete_fcvt_match_llvm_mc() {
    // All six legal cross-precision conversions (S↔D, S↔H, D↔H).
    assert_eq!(
        expect_word(encode_fcvt_precision(&reg2("d", 0, "s", 0))),
        0x1E22C000,
        "FCVT D0,S0"
    );
    assert_eq!(
        expect_word(encode_fcvt_precision(&reg2("s", 0, "d", 0))),
        0x1E624000,
        "FCVT S0,D0"
    );
    assert_eq!(
        expect_word(encode_fcvt_precision(&reg2("h", 0, "s", 0))),
        0x1E23C000,
        "FCVT H0,S0"
    );
    assert_eq!(
        expect_word(encode_fcvt_precision(&reg2("s", 0, "h", 0))),
        0x1EE24000,
        "FCVT S0,H0"
    );
    assert_eq!(
        expect_word(encode_fcvt_precision(&reg2("d", 0, "h", 0))),
        0x1EE2C000,
        "FCVT D0,H0"
    );
    assert_eq!(
        expect_word(encode_fcvt_precision(&reg2("h", 0, "d", 0))),
        0x1E63C000,
        "FCVT H0,D0"
    );
}

/// Helper: build a 2-register operand list `(dst, src)`.
fn reg2(dst_p: &str, dst_n: u32, src_p: &str, src_n: u32) -> Vec<Operand> {
    vec![
        Operand::Reg(format!("{}{}", dst_p, dst_n)),
        Operand::Reg(format!("{}{}", src_p, src_n)),
    ]
}

proptest! {
    // ════════════════════════════════════════════════════════════════════════
    // FNEG / FABS / FSQRT (FP data-processing, 1 source)
    // ════════════════════════════════════════════════════════════════════════

    /// P1 — Reference / field-layout oracle (differential vs the `llvm-mc`
    /// template). For any 1-source op, homogeneous-precision FP operands place
    /// every field at its canonical ARMv8 bit position with no truncation, and
    /// the whole word equals the reference template.
    #[test]
    fn prop_unary_places_fields(
        op_idx in 0u32..3u32, rd in 0u32..32, rn in 0u32..32, dbl in any::<bool>(),
    ) {
        let (enc, _name, opcode) = UNARY_OPS[op_idx as usize];
        let p = if dbl { "d" } else { "s" };
        let ops = vec![
            Operand::Reg(format!("{}{}", p, rd)),
            Operand::Reg(format!("{}{}", p, rn)),
        ];
        let ftype = if dbl { 0b01u32 } else { 0b00u32 };
        let w = expect_word(enc(&ops));

        prop_assert_eq!(w, unary_ref(opcode, ftype, rn, rd));
        // Field-by-field extraction.
        prop_assert_eq!(w >> 24, 0x1Eu32);            // [31:24] = 0x1E
        prop_assert_eq!((w >> 21) & 1, 1u32);         // bit 21 fixed = 1
        prop_assert_eq!(opcode6_of(w), opcode);       // [20:15] = opcode
        prop_assert_eq!(fixed10_of(w), 0b10000u32);   // [14:10] = 10000
        prop_assert_eq!(ftype_of(w), ftype);          // [23:22] = precision
        prop_assert_eq!(sf_of(w), 0);                 // scalar FP, sf always 0
        prop_assert_eq!(rn_of(w), rn);                // [9:5] source round-trips
        prop_assert_eq!(rd_of(w), rd);                // [4:0] dest round-trips
    }

    /// P2 — `ftype` is derived solely from the destination prefix
    /// ('d'→01, 's'→00); sf (bit 31) is always 0 for scalar FP.
    #[test]
    fn prop_unary_ftype_sf_from_dest(
        op_idx in 0u32..3u32, rd in 0u32..32, rn in 0u32..32, dbl in any::<bool>(),
    ) {
        let (enc, _, _) = UNARY_OPS[op_idx as usize];
        let p = if dbl { "d" } else { "s" };
        let ops = vec![
            Operand::Reg(format!("{}{}", p, rd)),
            Operand::Reg(format!("{}{}", p, rn)),
        ];
        let w = expect_word(enc(&ops));
        prop_assert_eq!(ftype_of(w), if dbl { 0b01 } else { 0b00 });
        prop_assert_eq!(sf_of(w), 0);
    }

    /// P3 — Out-of-range FP register numbers (>= 32) MUST be rejected by
    /// `get_reg`/`parse_reg_num`, not silently masked into the 5-bit field.
    #[test]
    fn prop_unary_rejects_out_of_range_reg(
        op_idx in 0u32..3u32, n in 32u32..256u32, pos in 0u32..2u32,
    ) {
        let (enc, _, _) = UNARY_OPS[op_idx as usize];
        let mut names = vec!["d0".to_string(), "d0".to_string()];
        names[pos as usize] = format!("d{}", n);
        let ops: Vec<Operand> = names.into_iter().map(Operand::Reg).collect();
        prop_assert!(
            enc(&ops).is_err(),
            "register d{} must be rejected (5-bit field), not silently masked", n
        );
    }

    /// P4 — Determinism: identical operands ⇒ identical word.
    #[test]
    fn prop_unary_is_deterministic(
        op_idx in 0u32..3u32, rd in 0u32..32, rn in 0u32..32, dbl in any::<bool>(),
    ) {
        let (enc, _, _) = UNARY_OPS[op_idx as usize];
        let p = if dbl { "d" } else { "s" };
        let ops = vec![
            Operand::Reg(format!("{}{}", p, rd)),
            Operand::Reg(format!("{}{}", p, rn)),
        ];
        prop_assert_eq!(expect_word(enc(&ops)), expect_word(enc(&ops)));
    }

    // ════════════════════════════════════════════════════════════════════════
    // FCVT (FP precision conversion)
    // ════════════════════════════════════════════════════════════════════════

    /// P5 — Reference / field-layout oracle (differential vs `llvm-mc`). For
    /// every *legal* (distinct) source/dest precision pair in {S,D,H}, the
    /// encoder produces exactly the canonical FCVT word with every field in
    /// place. Same-precision pairs are UNDEFINED and are excluded here (see B2).
    #[test]
    fn prop_fcvt_precision_places_fields(
        src_pi in 0u32..3u32, dst_pi in 0u32..3u32, rd in 0u32..32, rn in 0u32..32,
    ) {
        prop_assume!(src_pi != dst_pi); // same precision is UNDEFINED → witness B2
        let prefs = ["s", "d", "h"];
        let dst_p = prefs[dst_pi as usize];
        let src_p = prefs[src_pi as usize];
        let ops = reg2(dst_p, rd, src_p, rn);
        let ftype = prec_code(src_p);
        let opc = prec_code(dst_p);
        let w = expect_word(encode_fcvt_precision(&ops));

        prop_assert_eq!(w, fcvt_ref(ftype, opc, rn, rd));
        // Field-by-field extraction.
        prop_assert_eq!(w >> 24, 0x1Eu32);                 // [31:24] = 0x1E
        prop_assert_eq!((w >> 21) & 1, 1u32);              // bit 21 fixed = 1
        prop_assert_eq!(fcvt_opc_prefix_of(w), 0b0001u32); // [18:17] = 0001
        prop_assert_eq!(fcvt_opc_of(w), opc);              // [16:15] = dest precision
        prop_assert_eq!(fixed10_of(w), 0b10000u32);        // [14:10] = 10000
        prop_assert_eq!(ftype_of(w), ftype);               // [23:22] = source precision
        prop_assert_eq!(sf_of(w), 0);                      // scalar FP, sf always 0
        prop_assert_eq!(rn_of(w), rn);                     // [9:5] source round-trips
        prop_assert_eq!(rd_of(w), rd);                     // [4:0] dest round-trips
    }

    /// P6 — `ftype` is derived solely from the source prefix and `opc` solely
    /// from the dest prefix (S→00, D→01, H→11), for every legal conversion.
    #[test]
    fn prop_fcvt_precision_derives_ftype_opc(
        src_pi in 0u32..3u32, dst_pi in 0u32..3u32, rd in 0u32..32, rn in 0u32..32,
    ) {
        prop_assume!(src_pi != dst_pi);
        let prefs = ["s", "d", "h"];
        let ops = reg2(prefs[dst_pi as usize], rd, prefs[src_pi as usize], rn);
        let w = expect_word(encode_fcvt_precision(&ops));
        prop_assert_eq!(ftype_of(w), prec_code(prefs[src_pi as usize]));
        prop_assert_eq!(fcvt_opc_of(w), prec_code(prefs[dst_pi as usize]));
        prop_assert_eq!(sf_of(w), 0);
    }

    /// P7 — Arity (< 2 operands) and non-register (immediate) operands MUST be
    /// rejected; FCVT requires two register operands.
    #[test]
    fn prop_fcvt_precision_rejects_arity_and_immediate(imm in any::<i64>()) {
        // Too few operands.
        prop_assert!(encode_fcvt_precision(&[]).is_err());
        prop_assert!(encode_fcvt_precision(&[Operand::Reg("s0".into())]).is_err());
        // Immediate operand in either slot.
        let imm_dst = vec![Operand::Imm(imm), Operand::Reg("s0".into())];
        prop_assert!(encode_fcvt_precision(&imm_dst).is_err());
        let imm_src = vec![Operand::Reg("d0".into()), Operand::Imm(imm)];
        prop_assert!(encode_fcvt_precision(&imm_src).is_err());
    }

    /// P8 — Out-of-range FP register numbers (>= 32) MUST be rejected, and any
    /// register prefix outside {s,d,h} (GP banks W/X and SIMD lanes Q/V/B) MUST
    /// be rejected — FCVT only converts between S/D/H scalar precisions.
    #[test]
    fn prop_fcvt_precision_rejects_out_of_range_and_nonfp_bank(
        n in 32u32..256u32, bad_pi in 0u32..5u32, pos in 0u32..2u32,
    ) {
        // Out-of-range register number in either slot.
        let bad_dst = vec![Operand::Reg(format!("s{}", n)), Operand::Reg("d0".into())];
        prop_assert!(
            encode_fcvt_precision(&bad_dst).is_err(),
            "register s{} must be rejected, not masked", n
        );
        let bad_src = vec![Operand::Reg("d0".into()), Operand::Reg(format!("s{}", n))];
        prop_assert!(
            encode_fcvt_precision(&bad_src).is_err(),
            "register s{} must be rejected, not masked", n
        );
        // Non-{s,d,h} prefix in either slot is invalid for FCVT.
        let bad_prefixes = ["x", "w", "q", "v", "b"];
        let bp = bad_prefixes[bad_pi as usize];
        let mut names = vec!["s0".to_string(), "d0".to_string()];
        names[pos as usize] = format!("{}0", bp);
        let ops: Vec<Operand> = names.into_iter().map(Operand::Reg).collect();
        prop_assert!(
            encode_fcvt_precision(&ops).is_err(),
            "register prefix '{}' is not a legal FCVT precision (s/d/h); got {:?}",
            bp, encode_fcvt_precision(&ops)
        );
    }

    // ── Bug witnesses (FAIL by design; #[ignore] keeps `cargo test` green) ──

    /// B1a — FINDING (FAILS): `encode_fneg` requires homogeneous-precision
    /// FP-register operands. Mixed precision (`fneg d0,s0`) and GP-bank
    /// operands (`fneg x0,x0`) must be rejected, but the encoder derives
    /// `ftype` only from the destination prefix and never validates either
    /// operand's precision or bank, so it silently accepts illegal operands.
    /// Confirmed UNDEFINED by `llvm-mc`.
    #[test]
    #[ignore]
    fn prop_fneg_rejects_mixed_precision_and_bank(n in 0u32..32) {
        let enc: Enc = encode_fneg;
        let mix = vec![
            Operand::Reg(format!("d{}", n)),
            Operand::Reg(format!("s{}", n)),
        ];
        prop_assert!(
            enc(&mix).is_err(),
            "fneg d{},s{} mixed precision must be rejected; got {:?}",
            n, n, enc(&mix)
        );
        let gp = vec![
            Operand::Reg(format!("x{}", n)),
            Operand::Reg(format!("x{}", n)),
        ];
        prop_assert!(
            enc(&gp).is_err(),
            "fneg x{},x{} GP-bank operands must be rejected; got {:?}",
            n, n, enc(&gp)
        );
    }

    /// B1b — FINDING (FAILS): `encode_fabs` — same defect class as B1a.
    /// `fabs d0,s0` / `fabs x0,x0` must be rejected but are accepted.
    #[test]
    #[ignore]
    fn prop_fabs_rejects_mixed_precision_and_bank(n in 0u32..32) {
        let enc: Enc = encode_fabs;
        let mix = vec![
            Operand::Reg(format!("d{}", n)),
            Operand::Reg(format!("s{}", n)),
        ];
        prop_assert!(
            enc(&mix).is_err(),
            "fabs d{},s{} mixed precision must be rejected; got {:?}",
            n, n, enc(&mix)
        );
        let gp = vec![
            Operand::Reg(format!("x{}", n)),
            Operand::Reg(format!("x{}", n)),
        ];
        prop_assert!(
            enc(&gp).is_err(),
            "fabs x{},x{} GP-bank operands must be rejected; got {:?}",
            n, n, enc(&gp)
        );
    }

    /// B1c — FINDING (FAILS): `encode_fsqrt` — same defect class as B1a.
    /// `fsqrt d0,s0` / `fsqrt x0,x0` must be rejected but are accepted.
    #[test]
    #[ignore]
    fn prop_fsqrt_rejects_mixed_precision_and_bank(n in 0u32..32) {
        let enc: Enc = encode_fsqrt;
        let mix = vec![
            Operand::Reg(format!("d{}", n)),
            Operand::Reg(format!("s{}", n)),
        ];
        prop_assert!(
            enc(&mix).is_err(),
            "fsqrt d{},s{} mixed precision must be rejected; got {:?}",
            n, n, enc(&mix)
        );
        let gp = vec![
            Operand::Reg(format!("x{}", n)),
            Operand::Reg(format!("x{}", n)),
        ];
        prop_assert!(
            enc(&gp).is_err(),
            "fsqrt x{},x{} GP-bank operands must be rejected; got {:?}",
            n, n, enc(&gp)
        );
    }

    /// B2 — FINDING (FAILS): FCVT must reject same-precision conversions. The
    /// ARM ARM constraint for FCVT is `opc != ftype`; `fcvt s0,s0`, `fcvt d0,d0`
    /// and `fcvt h0,h0` are all UNDEFINED (confirmed by `llvm-mc`: "invalid
    /// operand for instruction"). The encoder never checks this, so it emits a
    /// (mis)encoded word instead of returning `Err`. Run with
    /// `cargo test --test pbt_arm_fp_scalar -- --ignored fneg_fabs_fsqrt_fcvt`.
    #[test]
    #[ignore]
    fn prop_fcvt_precision_rejects_same_precision(
        pi in 0u32..3u32, rd in 0u32..32, rn in 0u32..32,
    ) {
        let prefs = ["s", "d", "h"];
        let p = prefs[pi as usize];
        let ops = reg2(p, rd, p, rn);
        prop_assert!(
            encode_fcvt_precision(&ops).is_err(),
            "fcvt {}{},{}{} is UNDEFINED (opc==ftype, same precision); got {:?}",
            p, rd, p, rn, encode_fcvt_precision(&ops)
        );
    }
}
