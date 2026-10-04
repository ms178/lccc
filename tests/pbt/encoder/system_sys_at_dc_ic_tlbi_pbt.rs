//! Property-based tests for the SYS-family encoders in `system.rs`:
//!   * `encode_sys`  — generic `SYS #op1, Cn, Cm, #op2 [, Xt]`
//!   * `encode_at`   — address-translation barrier ops (s1e1r/s1e1w/s1e0r/s1e0w)
//!   * `encode_ic`   — instruction-cache maintenance (ialluis/iallu/ivau)
//!   * `encode_dc`   — data-cache maintenance (civac/cvac/cvap/cvau/ivac/zva)
//!   * `encode_tlbi` — TLB invalidation (vmalle1, vae1, ipas2e1, …)
//!
//! All are encoded as SYS-class instructions sharing the bit layout
//!
//!     1101 0101 0000 1 op1[18:16] CRn[15:12] CRm[11:8] op2[7:5] Rt[4:0]
//!
//! with the high opcode fixed (the per-instruction "base word" fixes the
//! op1/CRn/CRm/op2 nibbles). The encoders take either a raw comma-separated
//! operand string (sys/ic) or both a parsed `&[Operand]` and the raw string
//! (at/dc/tlbi); the register operand lives in bits[4:0] (Rt).
//!
//! # Reference oracle
//!
//! Every "known-word" anchor and every accept/reject verdict below was
//! cross-checked against `clang-14 --target=aarch64-linux-gnu` (LLVM-MC),
//! e.g.:
//!   `sys #0, c0, c0, #0`      = 0xD508_001F   (Rt defaults to XZR)
//!   `sys #3, c7, c14, #1, x0` = 0xD50B_7E20   (== DC CIVAC)
//!   `at  s1e1r, x5`           = 0xD508_7805
//!   `ic  iallu`               = 0xD508_751F   (Rt = XZR)
//!   `ic  ivau, x5`            = 0xD50B_7525
//!   `tlbi vmalle1`            = 0xD508_871F   (Rt = XZR)
//!   `tlbi vae1, x5`           = 0xD508_8725
//!   `dc  cvac, x5`            = 0xD50B_7A25
//!
//! # Findings surfaced
//!
//! The bit-packing is **correct** for valid operands: the high opcode is
//! constant, the Rt field round-trips, no-register forms correctly emit
//! Rt = XZR (31), and unknown operations are rejected.
//!
//! Several **validation bugs** are exposed as `#[ignore]`d witness properties
//! so the default `cargo test` stays green. Run them explicitly with
//! `cargo test --lib system_sys_at_dc_ic_tlbi -- --ignored`:
//!   * **SYS** — out-of-range `op1`/`CRn`/`CRm`/`op2` are silently masked
//!     (`& 7` / `& 0xF`) instead of being rejected. clang requires
//!     `op1,op2 ∈ [0,7]` and `CRn,CRm ∈ [0,15]`.
//!   * **SYS/AT/IC/TLBI/DC** — any FP/SIMD register name (`d0`, `s1`, `q5`,
//!     `v0`, …) is accepted as `Rt` via `parse_reg_num`; clang rejects these
//!     as "invalid operand for instruction".
//!   * **AT** — a missing register defaults to Rt = XZR instead of erroring;
//!     clang rejects with "specified at op requires a register".
//!   * **IC IALLUIS/IALLU** — a spurious register operand is accepted and
//!     overwrites the (mandatory) Rt = XZR field; clang rejects with
//!     "specified ic op does not use a register".
//!   * **IC IVAU** — a missing register defaults to Rt = XZR instead of
//!     erroring; clang rejects with "specified ic op requires a register".
//!   * **TLBI no-reg ops** (vmalle1, alle1, vmalls12e1, …) — a spurious
//!     register operand overwrites the mandatory Rt = XZR; clang rejects
//!     with "specified tlbi op does not use a register".
//!   * **TLBI reg ops** (vae1, ipas2e1, aside1is, …) — a missing register
//!     defaults to Rt = XZR instead of erroring.
//!   * **DC** — a missing register defaults to Rt = **0** (x0) instead of
//!     erroring; clang rejects with "specified dc op requires a register".
//!   * **DC** — variant matching uses `op.contains(...)`, so bogus names such
//!     as `xcvacx` are accepted (aliased to CVAC); clang rejects with
//!     "invalid operand for DC instruction".

use lccc::pbt_internals::Operand;
use lccc::pbt_internals::*;
use proptest::prelude::*;

// ── helpers ──────────────────────────────────────────────────────────────

/// Unwrap an encoder result, panicking if it is not `EncodeResult::Word`.
fn word(r: Result<EncodeResult, String>) -> u32 {
    match r {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {:?}", other),
    }
}

/// Build a 64-bit GP register operand `x<n>`.
fn xreg(n: u32) -> Operand {
    Operand::Reg(format!("x{}", n))
}

/// Rt field (bits[4:0]).
fn rt_of(w: u32) -> u32 {
    w & 0x1F
}

// A representative set of FP/SIMD register names that `parse_reg_num`
// accepts but that are illegal as a SYS-class Rt operand.
const FP_REGS: &[&str] = &["d0", "d31", "s5", "q7", "v0", "v31", "h3", "b1"];

// ── instruction tables (cross-checked against clang-14) ──────────────────

const AT_OPS: &[(&str, u32)] = &[
    ("s1e1r", 0xd508_7800),
    ("s1e1w", 0xd508_7820),
    ("s1e0r", 0xd508_7840),
    ("s1e0w", 0xd508_7860),
];

// IC ops that take NO register (Rt must be XZR = 31).
const IC_NO_REG_OPS: &[(&str, u32)] = &[("ialluis", 0xd508_711f), ("iallu", 0xd508_751f)];
// IC ops that REQUIRE a register.
const IC_REG_OPS: &[(&str, u32)] = &[("ivau", 0xd50b_7520)];

const DC_VARIANTS: &[(&str, u32)] = &[
    ("civac", 0xd50b_7e20),
    ("cvac", 0xd50b_7a20),
    ("cvap", 0xd50b_7c20),
    ("cvau", 0xd50b_7b20),
    ("ivac", 0xd508_7620),
    ("zva", 0xd50b_7420),
];

// TLBI ops that take NO register (Rt must be XZR = 31).
const TLBI_NO_REG_OPS: &[(&str, u32)] = &[
    ("vmalle1is", 0xd508_831f),
    ("vmalle1", 0xd508_871f),
    ("alle1is", 0xd50c_839f),
    ("alle1", 0xd50c_879f),
    ("alle2is", 0xd50c_831f),
    ("vmalls12e1is", 0xd50c_83df),
    ("vmalls12e1", 0xd50c_87df),
];
// TLBI ops that REQUIRE a register (base Rt field = 0).
const TLBI_REG_OPS: &[(&str, u32)] = &[
    ("vae1is", 0xd508_8320),
    ("vae1", 0xd508_8720),
    ("vae2is", 0xd50c_8320),
    ("vae2", 0xd50c_8720),
    ("vale1is", 0xd508_83a0),
    ("vale1", 0xd508_87a0),
    ("ipas2e1is", 0xd50c_8020),
    ("ipas2e1", 0xd50c_8420),
    ("ipas2le1is", 0xd50c_80a0),
    ("ipas2le1", 0xd50c_84a0),
    ("aside1is", 0xd508_8340),
];

// =========================================================================
// encode_sys — SYS #op1, Cn, Cm, #op2 [, Xt]
// =========================================================================
//
// word = 0xD508_0000 | (op1 & 7)<<16 | (CRn & 0xF)<<12 | (CRm & 0xF)<<8
//                      | (op2 & 7)<<5 | Rt
// Rt defaults to 31 (XZR) when the register is omitted.

proptest! {
    // S1. Fixed opcode: bits[31:19] are constant at 0xD508_0000 for every
    //     well-formed input, regardless of field values.
    #[test]
    fn sys_high_opcode_fixed(
        op1 in 0u32..=7, crn in 0u32..=15, crm in 0u32..=15,
        op2 in 0u32..=7, rt in 0u32..=31,
    ) {
        let raw = format!("#{}, c{}, c{}, #{}, x{}", op1, crn, crm, op2, rt);
        let w = word(encode_sys(&raw));
        prop_assert_eq!(w & 0xFFF8_0000, 0xD508_0000u32, "opcode bits[31:19]");
    }

    // S2. Field round-trip: within each field's legal range, every field
    //     round-trips out of the result at its declared position.
    #[test]
    fn sys_fields_roundtrip_in_valid_range(
        op1 in 0u32..=7, crn in 0u32..=15, crm in 0u32..=15,
        op2 in 0u32..=7, rt in 0u32..=31,
    ) {
        let raw = format!("#{}, c{}, c{}, #{}, x{}", op1, crn, crm, op2, rt);
        let w = word(encode_sys(&raw));
        prop_assert_eq!((w >> 16) & 0x7, op1, "op1 at [18:16]");
        prop_assert_eq!((w >> 12) & 0xF, crn, "CRn at [15:12]");
        prop_assert_eq!((w >> 8) & 0xF, crm, "CRm at [11:8]");
        prop_assert_eq!((w >> 5) & 0x7, op2, "op2 at [7:5]");
        prop_assert_eq!(rt_of(w), rt, "Rt at [4:0]");
    }

    // S3. Default register: with exactly four operands, Rt defaults to 31
    //     (XZR).
    #[test]
    fn sys_omitted_register_defaults_to_xzr(
        op1 in 0u32..=7, crn in 0u32..=15, crm in 0u32..=15, op2 in 0u32..=7,
    ) {
        let raw = format!("#{}, c{}, c{}, #{}", op1, crn, crm, op2);
        let w = word(encode_sys(&raw));
        prop_assert_eq!(rt_of(w), 31u32);
    }

    // S4. Injectivity over legal ranges: distinct (op1,CRn,CRm,op2,Rt)
    //     tuples yield distinct words.
    #[test]
    fn sys_distinct_valid_tuples_distinct_words(
        a1 in 0u32..=7, acn in 0u32..=15, acm in 0u32..=15, a2 in 0u32..=7, art in 0u32..=31,
        b1 in 0u32..=7, bcn in 0u32..=15, bcm in 0u32..=15, b2 in 0u32..=7, brt in 0u32..=31,
    ) {
        prop_assume!((a1, acn, acm, a2, art) != (b1, bcn, bcm, b2, brt));
        let wa = word(encode_sys(&format!("#{}, c{}, c{}, #{}, x{}", a1, acn, acm, a2, art)));
        let wb = word(encode_sys(&format!("#{}, c{}, c{}, #{}, x{}", b1, bcn, bcm, b2, brt)));
        prop_assert_ne!(wa, wb);
    }

    // S5. Error contract: malformed operand strings are rejected.
    #[test]
    fn sys_rejects_malformed_operands(kind in 0u8..5) {
        let raw = match kind {
            0 => "#1, c0, c0",            // < 4 operands
            1 => "#1, c0, c0, #0, #0",    // 5th part not a register
            2 => "foo, c0, c0, #0",       // non-numeric op1
            3 => "#1, c0, c0, bar",       // non-numeric op2
            _ => "#1, c0, c0, #0, xyz",   // unparseable register
        };
        prop_assert!(encode_sys(raw).is_err(), "expected Err for: {}", raw);
    }
}

#[test]
fn sys_known_words() {
    // clang-14 reference encodings.
    assert_eq!(word(encode_sys("#0, c0, c0, #0")), 0xD508_001Fu32); // Rt = XZR
    assert_eq!(word(encode_sys("#0, c0, c0, #0, x0")), 0xD508_0000u32);
    assert_eq!(word(encode_sys("#3, c7, c14, #1, x0")), 0xD50B_7E20u32); // == DC CIVAC
    // Uppercase CRn/CRm accepted (lowercased before the leading 'c' strip).
    assert_eq!(
        word(encode_sys("#0, C7, C10, #1, x5")),
        word(encode_sys("#0, c7, c10, #1, x5")),
    );
}

// ── SYS bug witnesses (default `cargo test` ignores these) ───────────────
proptest! {
    // W-S1. op1 must be in [0,7]; out-of-range values must be Err, not masked.
    #[ignore = "documented bug: encode_sys masks op1 with &7 instead of rejecting (clang: [0,7])"]
    #[test]
    fn witness_sys_rejects_out_of_range_op1(op1 in 8u32..=255) {
        let raw = format!("#{}, c0, c0, #0", op1);
        prop_assert!(encode_sys(&raw).is_err(), "out-of-range op1 must be Err: {}", raw);
    }

    // W-S2. CRn/CRm must be in [0,15].
    #[ignore = "documented bug: encode_sys masks CRn/CRm with &0xF instead of rejecting (clang: [0,15])"]
    #[test]
    fn witness_sys_rejects_out_of_range_crn_crm(
        crn in 16u32..=255, crm in 16u32..=255,
    ) {
        let raw = format!("#0, c{}, c{}, #0", crn, crm);
        prop_assert!(encode_sys(&raw).is_err(), "out-of-range CRn/CRm must be Err: {}", raw);
    }

    // W-S3. op2 must be in [0,7].
    #[ignore = "documented bug: encode_sys masks op2 with &7 instead of rejecting (clang: [0,7])"]
    #[test]
    fn witness_sys_rejects_out_of_range_op2(op2 in 8u32..=255) {
        let raw = format!("#0, c0, c0, #{}", op2);
        prop_assert!(encode_sys(&raw).is_err(), "out-of-range op2 must be Err: {}", raw);
    }

    // W-S4. FP/SIMD registers are illegal Rt operands.
    #[ignore = "documented bug: encode_sys accepts FP/SIMD register as Rt (clang: invalid operand)"]
    #[test]
    fn witness_sys_rejects_fp_register(idx in 0usize..FP_REGS.len()) {
        let reg = FP_REGS[idx];
        let raw = format!("#0, c0, c0, #0, {}", reg);
        prop_assert!(encode_sys(&raw).is_err(), "FP/SIMD register must be Err: {}", raw);
    }
}

// =========================================================================
// encode_at — AT <op>, Xt   (all four ops REQUIRE a register)
// =========================================================================
//
// word = (base & !0x1F) | Rt

proptest! {
    // A1. For every op, the base word is fixed and Rt occupies only bits[4:0].
    #[test]
    fn at_base_fixed_and_rt_isolated(op_idx in 0usize..AT_OPS.len(), rt in 0u32..=31) {
        let (op, base) = AT_OPS[op_idx];
        let w = word(encode_at(&[], &format!("{}, x{}", op, rt)));
        prop_assert_eq!(w & !0x1F, base, "base word (Rt cleared) for AT {}", op);
        prop_assert_eq!(rt_of(w), rt);
    }

    // A2. Rt round-trips into bits[4:0] for every op and every Rt.
    #[test]
    fn at_rt_roundtrips(op_idx in 0usize..AT_OPS.len(), rt in 0u32..=31) {
        let (op, _) = AT_OPS[op_idx];
        let w = word(encode_at(&[], &format!("{}, x{}", op, rt)));
        prop_assert_eq!(rt_of(w), rt);
    }

    // A3. Injectivity: distinct Rt yield distinct words for a fixed op.
    #[test]
    fn at_distinct_rt_distinct_words(
        op_idx in 0usize..AT_OPS.len(), a in 0u32..=31, b in 0u32..=31,
    ) {
        prop_assume!(a != b);
        let (op, _) = AT_OPS[op_idx];
        let wa = word(encode_at(&[], &format!("{}, x{}", op, a)));
        let wb = word(encode_at(&[], &format!("{}, x{}", op, b)));
        prop_assert_ne!(wa, wb);
    }

    // A4. Unknown operation is rejected.
    #[test]
    fn at_rejects_unknown_op(idx in 0u8..8) {
        let op = match idx {
            0 => "s1e2r", 1 => "s1e3r", 2 => "s12e1r", 3 => "s1e1rp",
            4 => "foo",   5 => "",      6 => "S1E1X",  _ => "s1e1",
        };
        prop_assert!(encode_at(&[], &format!("{}, x0", op)).is_err(), "unknown AT op: {:?}", op);
    }
}

#[test]
fn at_known_words() {
    assert_eq!(word(encode_at(&[], "s1e1r, x5")), 0xD508_7805u32);
    assert_eq!(word(encode_at(&[], "s1e1w, x5")), 0xD508_7825u32);
    assert_eq!(word(encode_at(&[], "s1e0r, x0")), 0xD508_7840u32);
    assert_eq!(word(encode_at(&[], "s1e0w, x9")), 0xD508_7869u32);
}

proptest! {
    // W-A1. AT ops REQUIRE a register; a missing register must be Err (the
    //       encoder currently defaults to Rt = XZR).
    #[ignore = "documented bug: encode_at defaults missing register to XZR (clang: requires a register)"]
    #[test]
    fn witness_at_rejects_missing_register(op_idx in 0usize..AT_OPS.len()) {
        let (op, _) = AT_OPS[op_idx];
        prop_assert!(encode_at(&[], op).is_err(), "missing register must be Err for AT {}", op);
    }

    // W-A2. FP/SIMD registers are illegal Rt operands.
    #[ignore = "documented bug: encode_at accepts FP/SIMD register as Rt (clang: invalid operand)"]
    #[test]
    fn witness_at_rejects_fp_register(
        op_idx in 0usize..AT_OPS.len(), fp_idx in 0usize..FP_REGS.len(),
    ) {
        let (op, _) = AT_OPS[op_idx];
        let reg = FP_REGS[fp_idx];
        let raw = format!("{}, {}", op, reg);
        prop_assert!(encode_at(&[], &raw).is_err(), "FP/SIMD register must be Err: {}", raw);
    }
}

// =========================================================================
// encode_ic — IC <op> [, Xt]
//   * IALLUIS / IALLU : no register, Rt must be XZR (31)
//   * IVAU            : requires Xt
// =========================================================================

proptest! {
    // I1. No-register ops (IALLUIS/IALLU) always emit Rt = 31 (XZR) and the
    //     canonical base word, when invoked with no register.
    #[test]
    fn ic_no_reg_ops_emit_xzr(op_idx in 0usize..IC_NO_REG_OPS.len()) {
        let (op, base) = IC_NO_REG_OPS[op_idx];
        let w = word(encode_ic(op));
        prop_assert_eq!(w, base, "IC {} with no register", op);
        prop_assert_eq!(rt_of(w), 31u32);
    }

    // I2. IVAU: Rt round-trips into bits[4:0] for every Rt.
    #[test]
    fn ic_ivau_rt_roundtrips(op_idx in 0usize..IC_REG_OPS.len(), rt in 0u32..=31) {
        let (op, _) = IC_REG_OPS[op_idx];
        let w = word(encode_ic(&format!("{}, x{}", op, rt)));
        prop_assert_eq!(rt_of(w), rt);
    }

    // I3. IVAU: base word is fixed; only bits[4:0] depend on the register.
    #[test]
    fn ic_ivau_base_fixed(op_idx in 0usize..IC_REG_OPS.len(), rt in 0u32..=31) {
        let (op, base) = IC_REG_OPS[op_idx];
        let w = word(encode_ic(&format!("{}, x{}", op, rt)));
        prop_assert_eq!(w & !0x1F, base);
    }

    // I4. Unknown operation is rejected.
    #[test]
    fn ic_rejects_unknown_op(op in "[a-z]{1,8}") {
        prop_assume!(!matches!(op.as_str(), "ialluis" | "iallu" | "ivau"));
        prop_assert!(encode_ic(&op).is_err(), "unknown IC op: {}", op);
    }
}

#[test]
fn ic_known_words() {
    assert_eq!(word(encode_ic("iallu")), 0xD508_751Fu32);
    assert_eq!(word(encode_ic("ialluis")), 0xD508_711Fu32);
    assert_eq!(word(encode_ic("ivau, x5")), 0xD50B_7525u32);
}

proptest! {
    // W-I1. IALLUIS/IALLU do not use a register; a spurious operand must be
    //       Err (the encoder currently overwrites the mandatory Rt = XZR).
    #[ignore = "documented bug: encode_ic accepts register on IALLUIS/IALLU (clang: does not use a register)"]
    #[test]
    fn witness_ic_no_reg_op_rejects_register(
        op_idx in 0usize..IC_NO_REG_OPS.len(), rt in 0u32..=31,
    ) {
        let (op, _) = IC_NO_REG_OPS[op_idx];
        let raw = format!("{}, x{}", op, rt);
        prop_assert!(encode_ic(&raw).is_err(), "no-register IC op must reject register: {}", raw);
    }

    // W-I2. IVAU requires a register; a missing register must be Err (the
    //       encoder currently defaults to Rt = XZR).
    #[ignore = "documented bug: encode_ic defaults missing IVAU register to XZR (clang: requires a register)"]
    #[test]
    fn witness_ic_ivau_rejects_missing_register(op_idx in 0usize..IC_REG_OPS.len()) {
        let (op, _) = IC_REG_OPS[op_idx];
        prop_assert!(encode_ic(op).is_err(), "IVAU missing register must be Err");
    }

    // W-I3. FP/SIMD registers are illegal Rt operands for IVAU.
    #[ignore = "documented bug: encode_ic accepts FP/SIMD register as Rt (clang: invalid operand)"]
    #[test]
    fn witness_ic_rejects_fp_register(
        op_idx in 0usize..IC_REG_OPS.len(), fp_idx in 0usize..FP_REGS.len(),
    ) {
        let (op, _) = IC_REG_OPS[op_idx];
        let reg = FP_REGS[fp_idx];
        let raw = format!("{}, {}", op, reg);
        prop_assert!(encode_ic(&raw).is_err(), "FP/SIMD register must be Err: {}", raw);
    }
}

// =========================================================================
// encode_dc — DC <variant>, Xt   (all variants REQUIRE a register)
// =========================================================================

proptest! {
    // D1. For every variant, the base word is fixed and Rt occupies only
    //     bits[4:0].
    #[test]
    fn dc_base_fixed_and_rt_isolated(v_idx in 0usize..DC_VARIANTS.len(), rt in 0u32..=31) {
        let (variant, base) = DC_VARIANTS[v_idx];
        let ops = vec![Operand::Symbol(variant.to_string()), xreg(rt)];
        let raw = format!("{}, x{}", variant, rt);
        let w = word(encode_dc(&ops, &raw));
        prop_assert_eq!(w & !0x1F, base, "base word for DC {}", variant);
        prop_assert_eq!(rt_of(w), rt);
    }

    // D2. Rt round-trips into bits[4:0] for every variant and every Rt.
    #[test]
    fn dc_rt_roundtrips(v_idx in 0usize..DC_VARIANTS.len(), rt in 0u32..=31) {
        let (variant, _) = DC_VARIANTS[v_idx];
        let ops = vec![Operand::Symbol(variant.to_string()), xreg(rt)];
        let w = word(encode_dc(&ops, &format!("{}, x{}", variant, rt)));
        prop_assert_eq!(rt_of(w), rt);
    }

    // D3. Injectivity: distinct Rt yield distinct words for a fixed variant.
    #[test]
    fn dc_distinct_rt_distinct_words(
        v_idx in 0usize..DC_VARIANTS.len(), a in 0u32..=31, b in 0u32..=31,
    ) {
        prop_assume!(a != b);
        let (variant, _) = DC_VARIANTS[v_idx];
        let wa = word(encode_dc(
            &[Operand::Symbol(variant.into()), xreg(a)],
            &format!("{}, x{}", variant, a),
        ));
        let wb = word(encode_dc(
            &[Operand::Symbol(variant.into()), xreg(b)],
            &format!("{}, x{}", variant, b),
        ));
        prop_assert_ne!(wa, wb);
    }

    // D4. Unknown variant is rejected. (Names that merely *contain* a real
    //      substring like "cvacxx" are a separate bug — see W-D3; filtered out
    //      here so this property exercises the clean reject path.)
    #[test]
    fn dc_rejects_unknown_variant(v in "[a-z]{2,6}") {
        // Skip anything that is, or substring-aliases, a real variant.
        let alias = ["civac", "cvac", "cvap", "cvau", "ivac", "zva"];
        prop_assume!(!DC_VARIANTS.iter().any(|(n, _)| *n == v));
        prop_assume!(!alias.iter().any(|s| v.contains(s)));
        let raw = format!("{}, x0", v);
        let ops = vec![Operand::Symbol(v.to_string()), xreg(0)];
        prop_assert!(encode_dc(&ops, &raw).is_err(), "unknown DC variant: {}", v);
    }
}

#[test]
fn dc_known_words() {
    // clang-14 reference encodings (cvap needs -march=armv8.4-a; its word is
    // the SYS #3,c7,c12,#1 form 0xD50B_7C20 | Rt).
    fn enc(variant: &str, rt: u32) -> u32 {
        word(encode_dc(
            &[Operand::Symbol(variant.into()), xreg(rt)],
            &format!("{}, x{}", variant, rt),
        ))
    }
    assert_eq!(enc("cvac", 5), 0xD50B_7A25u32);
    assert_eq!(enc("civac", 0), 0xD50B_7E20u32);
    assert_eq!(enc("cvau", 1), 0xD50B_7B21u32);
    assert_eq!(enc("ivac", 2), 0xD508_7622u32);
    assert_eq!(enc("zva", 3), 0xD50B_7423u32);
    assert_eq!(enc("cvap", 7), 0xD50B_7C27u32); // FEAT_DCCVAP
}

proptest! {
    // W-D1. DC variants REQUIRE a register; a missing register must be Err
    //       (the encoder currently defaults to Rt = 0, i.e. x0).
    #[ignore = "documented bug: encode_dc defaults missing register to Rt=0 (clang: requires a register)"]
    #[test]
    fn witness_dc_rejects_missing_register(v_idx in 0usize..DC_VARIANTS.len()) {
        let (variant, _) = DC_VARIANTS[v_idx];
        let ops = vec![Operand::Symbol(variant.to_string())];
        let res = encode_dc(&ops, variant);
        prop_assert!(res.is_err(), "missing register must be Err for DC {}", variant);
    }

    // W-D2. FP/SIMD registers are illegal Rt operands.
    #[ignore = "documented bug: encode_dc accepts FP/SIMD register as Rt (clang: invalid operand)"]
    #[test]
    fn witness_dc_rejects_fp_register(
        v_idx in 0usize..DC_VARIANTS.len(), fp_idx in 0usize..FP_REGS.len(),
    ) {
        let (variant, _) = DC_VARIANTS[v_idx];
        let reg = FP_REGS[fp_idx];
        let ops = vec![Operand::Symbol(variant.to_string()), Operand::Reg(reg.to_string())];
        let raw = format!("{}, {}", variant, reg);
        let res = encode_dc(&ops, &raw);
        prop_assert!(res.is_err(), "FP/SIMD register must be Err: {}", raw);
    }

    // W-D3. Variant matching uses `op.contains(...)`, so bogus variant names
    //       containing a real substring are silently accepted.
    #[ignore = "documented bug: encode_dc substring-matches variant names (clang: invalid operand for DC)"]
    #[test]
    fn witness_dc_rejects_bogus_substring_variant(v_idx in 0usize..DC_VARIANTS.len()) {
        let (variant, _) = DC_VARIANTS[v_idx];
        // e.g. "xcvacx", "xxcvacxx" — contains the real substring but is not it.
        let bogus = format!("x{}x", variant);
        let ops = vec![Operand::Symbol(bogus.clone()), xreg(0)];
        let raw = format!("{}, x0", bogus);
        let res = encode_dc(&ops, &raw);
        prop_assert!(res.is_err(), "bogus variant must be Err: {}", raw);
    }
}

// =========================================================================
// encode_tlbi — TLBI <op> [, Xt]
//   * no-reg ops (vmalle1, alle1, vmalls12e1, …) : Rt must be XZR (31)
//   * reg ops   (vae1, ipas2e1, aside1is, …)    : REQUIRE Xt
// =========================================================================

proptest! {
    // T1. No-register ops always emit the canonical base word with Rt = 31.
    #[test]
    fn tlbi_no_reg_ops_emit_xzr(op_idx in 0usize..TLBI_NO_REG_OPS.len()) {
        let (op, base) = TLBI_NO_REG_OPS[op_idx];
        let w = word(encode_tlbi(&[], op));
        prop_assert_eq!(w, base, "TLBI {} with no register", op);
        prop_assert_eq!(rt_of(w), 31u32);
    }

    // T2. Register-ops: base word fixed, Rt isolated to bits[4:0].
    #[test]
    fn tlbi_reg_ops_base_fixed_and_rt_isolated(
        op_idx in 0usize..TLBI_REG_OPS.len(), rt in 0u32..=31,
    ) {
        let (op, base) = TLBI_REG_OPS[op_idx];
        let w = word(encode_tlbi(&[], &format!("{}, x{}", op, rt)));
        prop_assert_eq!(w & !0x1F, base, "base word for TLBI {}", op);
        prop_assert_eq!(rt_of(w), rt);
    }

    // T3. Register-ops: Rt round-trips.
    #[test]
    fn tlbi_reg_ops_rt_roundtrips(op_idx in 0usize..TLBI_REG_OPS.len(), rt in 0u32..=31) {
        let (op, _) = TLBI_REG_OPS[op_idx];
        let w = word(encode_tlbi(&[], &format!("{}, x{}", op, rt)));
        prop_assert_eq!(rt_of(w), rt);
    }

    // T4. Register-ops: injectivity of Rt.
    #[test]
    fn tlbi_reg_ops_distinct_rt_distinct_words(
        op_idx in 0usize..TLBI_REG_OPS.len(), a in 0u32..=31, b in 0u32..=31,
    ) {
        prop_assume!(a != b);
        let (op, _) = TLBI_REG_OPS[op_idx];
        let wa = word(encode_tlbi(&[], &format!("{}, x{}", op, a)));
        let wb = word(encode_tlbi(&[], &format!("{}, x{}", op, b)));
        prop_assert_ne!(wa, wb);
    }

    // T5. Unknown operation is rejected.
    #[test]
    fn tlbi_rejects_unknown_op(op in "[a-z0-9]{1,10}") {
        let known = TLBI_NO_REG_OPS.iter().map(|(n, _)| *n)
            .chain(TLBI_REG_OPS.iter().map(|(n, _)| *n))
            .collect::<Vec<_>>();
        prop_assume!(!known.contains(&op.as_str()));
        prop_assert!(encode_tlbi(&[], &op).is_err(), "unknown TLBI op: {}", op);
    }
}

#[test]
fn tlbi_known_words() {
    // clang-14 reference encodings.
    assert_eq!(word(encode_tlbi(&[], "vmalle1")), 0xD508_871Fu32); // Rt = XZR
    assert_eq!(word(encode_tlbi(&[], "vmalle1is")), 0xD508_831Fu32);
    assert_eq!(word(encode_tlbi(&[], "vae1, x5")), 0xD508_8725u32);
    assert_eq!(word(encode_tlbi(&[], "ipas2e1, x2")), 0xD50C_8422u32);
    // Case-insensitive op name.
    assert_eq!(word(encode_tlbi(&[], "VMALLE1")), 0xD508_871Fu32);
}

proptest! {
    // W-T1. No-register ops do not use a register; a spurious operand must be
    //       Err (the encoder currently overwrites the mandatory Rt = XZR).
    #[ignore = "documented bug: encode_tlbi accepts register on no-reg ops (clang: does not use a register)"]
    #[test]
    fn witness_tlbi_no_reg_op_rejects_register(
        op_idx in 0usize..TLBI_NO_REG_OPS.len(), rt in 0u32..=31,
    ) {
        let (op, _) = TLBI_NO_REG_OPS[op_idx];
        let raw = format!("{}, x{}", op, rt);
        prop_assert!(encode_tlbi(&[], &raw).is_err(), "no-reg TLBI op must reject register: {}", raw);
    }

    // W-T2. Register-ops REQUIRE a register; a missing register must be Err
    //       (the encoder currently defaults to Rt = XZR).
    #[ignore = "documented bug: encode_tlbi defaults missing register to XZR (clang: requires a register)"]
    #[test]
    fn witness_tlbi_reg_op_rejects_missing_register(op_idx in 0usize..TLBI_REG_OPS.len()) {
        let (op, _) = TLBI_REG_OPS[op_idx];
        prop_assert!(encode_tlbi(&[], op).is_err(), "reg-op TLBI missing register must be Err");
    }

    // W-T3. FP/SIMD registers are illegal Rt operands.
    #[ignore = "documented bug: encode_tlbi accepts FP/SIMD register as Rt (clang: invalid operand)"]
    #[test]
    fn witness_tlbi_rejects_fp_register(
        op_idx in 0usize..TLBI_REG_OPS.len(), fp_idx in 0usize..FP_REGS.len(),
    ) {
        let (op, _) = TLBI_REG_OPS[op_idx];
        let reg = FP_REGS[fp_idx];
        let raw = format!("{}, {}", op, reg);
        prop_assert!(encode_tlbi(&[], &raw).is_err(), "FP/SIMD register must be Err: {}", raw);
    }
}
