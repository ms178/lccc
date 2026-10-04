//! Property-based tests for `encode_ldop`.
//!
//! `encode_ldop` encodes the ARMv8.1-A LSE atomic memory operations
//! LDADD / LDCLR / LDEOR / LDSET and their acquire (`a`) / release (`l`) /
//! byte (`b`) / halfword (`h`) variants (ARM ARM §C6.2.100 LDADD et seq.).
//!
//! ```text
//!   31-30  29-24   23  22  21  20-16  15  14-12  11-10  9-5  4-0
//!    size  111000   A   R   1   Rs    0   opc    00    Rn   Rt
//! ```
//!   size: 00=byte(*b) 01=half(*h) 10=W-regs 11=X-regs
//!   opc:  LDADD=000 LDCLR=001 LDEOR=010 LDSET=011
//!
//! This module is **complementary** to the inline `prop_encode_ldop_tests`
//! inside `load_store.rs` (which covers field placement, opc mapping, the
//! width / acquire-release differentials, and golden encodings). Here we
//! focus on two NEGATIVE-CONTRACT findings the inline suite does not
//! exercise:
//!
//! ## Finding 1 — silent immediate-offset drop
//! The LSE atomic memory-op group has **no** immediate-offset addressing
//! form. The ARM ARM permits only `LDADD <Rs>, <Rt>, [<Xn|SP>]` — the
//! effective address is exactly the base register. The encoder matches
//! `Operand::Mem { base, .. }` and never inspects `offset`, so a
//! non-zero offset such as `ldadd x0, x1, [x2, #8]` is silently encoded
//! as `ldadd x0, x1, [x2]`. This is the same defect family already
//! reported for `encode_ldxr_stxr` / `encode_ldxp_stxp` in this file.
//! Witness: `prop_nonzero_offset_rejected` (#[ignore]); mechanism
//! documented by `prop_offset_is_silently_dropped` (passes).
//!
//! ## Finding 2 — mismatched Rs/Rt width silently accepted
//! For the register (non-`b`/`h`) form the ARM ARM requires `<Rs>` and
//! `<Rt>` to have the *same* width. The encoder derives `size` solely
//! from `Rs` (operands[0]) and never validates `<Rt>`'s width, so
//! `ldadd w0, x1, [x2]` is silently encoded as a 32-bit op (size=10).
//! Witness: `prop_mismatched_width_rejected` (#[ignore]); mechanism
//! documented by `prop_mismatched_width_size_follows_rs` (passes).

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_ldop;
use proptest::prelude::*;

// ── Independent oracle ─────────────────────────────────────────────────
// The reference word is assembled field-by-field from the ARMv8.1-A ARM
// bit layout above, NOT by calling `encode_ldop`. opc is taken from a
// spec-transcribed lookup table; size/A/R are decoded from the mnemonic
// suffix by the same rules the assembler documents.

/// Register-form (no `b`/`h` suffix) variants — used by the width tests.
const REG_FORM_MNEMONICS: &[&str] = &[
    "ldadd", "ldadda", "ldaddl", "ldaddal", "ldclr", "ldclra", "ldclrl", "ldclral", "ldeor",
    "ldeora", "ldeorl", "ldeoral", "ldset", "ldseta", "ldsetl", "ldsetal",
];

/// Representative coverage of every base op and suffix class.
const ALL_MNEMONICS: &[&str] = &[
    "ldadd", "ldadda", "ldaddl", "ldaddal", "ldaddb", "ldaddab", "ldaddlb", "ldaddalb", "ldaddh",
    "ldaddah", "ldaddlh", "ldaddalh", "ldclr", "ldclra", "ldclrl", "ldclral", "ldclrb", "ldclrh",
    "ldeor", "ldeora", "ldeorl", "ldeoral", "ldeorb", "ldeorh", "ldset", "ldseta", "ldsetl",
    "ldsetal", "ldsetb", "ldseth",
];

fn opc_for(mn: &str) -> u32 {
    if mn.starts_with("ldadd") {
        0b000
    } else if mn.starts_with("ldclr") {
        0b001
    } else if mn.starts_with("ldeor") {
        0b010
    } else if mn.starts_with("ldset") {
        0b011
    } else {
        panic!("unexpected mnemonic {}", mn)
    }
}

/// Spec-transcribed reference encoder. `rs_is64` drives `size` for the
/// register form (mirroring that `<Rs>`'s width selects the operation
/// size); byte/half suffixes override it.
fn ref_encode(mn: &str, rs: u32, rs_is64: bool, rt: u32, rn: u32) -> u32 {
    let suffix = mn
        .strip_prefix("ldadd")
        .or_else(|| mn.strip_prefix("ldclr"))
        .or_else(|| mn.strip_prefix("ldeor"))
        .or_else(|| mn.strip_prefix("ldset"))
        .unwrap_or("");
    let size: u32 = if suffix.contains('b') {
        0b00
    } else if suffix.contains('h') {
        0b01
    } else if rs_is64 {
        0b11
    } else {
        0b10
    };
    let a: u32 = u32::from(suffix.contains('a'));
    let r: u32 = u32::from(suffix.contains('l'));
    (size << 30)
        | (0b111000 << 24)
        | (a << 23)
        | (r << 22)
        | (1 << 21)
        | (rs << 16)
        | (opc_for(mn) << 12)
        | (rn << 5)
        | rt
}

// --- helpers --------------------------------------------------------------

fn gp(width: char, n: u32) -> Operand {
    // Register 31 is spelled xzr/wzr (the canonical spelling; "x31" is not).
    Operand::Reg(if n == 31 {
        format!("{}zr", width)
    } else {
        format!("{}{}", width, n)
    })
}
fn mem(base: u32, off: i64) -> Operand {
    Operand::Mem {
        base: format!("x{}", base),
        offset: off,
    }
}
fn word(r: Result<EncodeResult, String>) -> u32 {
    match r {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected Word, got {:?}", other),
    }
}

prop_compose! {
    fn arb_mn(all: bool)(idx in 0usize..ALL_MNEMONICS.len()) -> &'static str {
        let _ = all;
        ALL_MNEMONICS[idx]
    }
}
prop_compose! {
    fn arb_reg_form_mn()(idx in 0usize..REG_FORM_MNEMONICS.len()) -> &'static str {
        REG_FORM_MNEMONICS[idx]
    }
}

proptest! {
    // ── Property 1: reference-encoding differential (PASSES). ──
    // For every valid (mnemonic, width, register-number) combination the
    // encoder output must equal the spec-transcribed reference word. This
    // anchors the core encoding as correct and cross-checks the hand-derived
    // goldens (ldadd x0,x1,[x2] = 0xF8200041, ldadd w0,w1,[x2] = 0xB8200041,
    // ldset x0,x1,[x2] = 0xF8203041).
    #[test]
    fn prop_reference_encoding_matches(
        mn in arb_mn(true),
        rs in 0u32..=31u32,
        rt in 0u32..=31u32,
        rn in 0u32..=31u32,
        rs_is64 in any::<bool>(),
    ) {
        // Valid input: rs and rt share width, offset == 0. The byte and
        // halfword forms always take w registers (the b/h ops read 8/16
        // bits of a W register; GNU as rejects the x spelling).
        let width = if mn.contains('b') || mn.contains('h') {
            'w'
        } else if rs_is64 {
            'x'
        } else {
            'w'
        };
        let ops = vec![gp(width, rs), gp(width, rt), mem(rn, 0)];
        let got = word(encode_ldop(mn, &ops));
        let want = ref_encode(mn, rs, rs_is64, rt, rn);
        prop_assert_eq!(got, want, "reference mismatch for {}", mn);
    }

    // ── Property 2: non-zero offsets are rejected (was: silently dropped).
    // The LSE atomic memory-op group has no immediate-offset form, so an
    // offset is unrepresentable and must be an error. (Finding 1, fixed:
    // the old encoder never inspected `Mem::offset`.)
    #[test]
    fn prop_nonzero_offset_is_rejected(
        mn in arb_reg_form_mn(),
        off in any::<i64>().prop_filter("non-zero", |o| *o != 0),
    ) {
        let ops = vec![gp('x', 0), gp('x', 1), mem(2, off)];
        prop_assert!(
            encode_ldop(mn, &ops).is_err(),
            "non-zero offset must be rejected, got {:?}",
            encode_ldop(mn, &ops)
        );
    }

    // ── Property 3: NEGATIVE CONTRACT — non-zero offset must be Err. ──
    // The LSE atomic memory-op group has no immediate-offset form: the only
    // permitted syntax is `LDop <Rs>, <Rt>, [<Xn|SP>]`. A non-zero offset is
    // therefore unrepresentable and MUST be rejected rather than silently
    // encoded as `[Xn]`.
    //
    // EXPECTED TO FAIL today: the encoder drops the offset and returns Ok.
    // This failure IS Finding 1. Marked #[ignore] so `cargo test` stays green.
    // ACTIVATED: the encoder now rejects non-zero offsets (was a documented
    // bug; fixed together with the CAS/SWP family).
    #[test]
    fn prop_nonzero_offset_rejected(
        mn in arb_mn(true),
        off in (-32768i64..32767i64).prop_filter("non-zero", |o| *o != 0),
    ) {
        let ops = vec![gp('x', 0), gp('x', 1), mem(2, off)];
        let res = encode_ldop(mn, &ops);
        prop_assert!(
            res.is_err(),
            "non-zero offset {} is unrepresentable for LSE atomics and must be Err; got {:?}",
            off, res
        );
    }

    // ── Property 4: mismatched width is rejected (was: size followed Rs). ──
    // The ARM ARM requires <Rs> and <Rt> to have the same width for the
    // register form; the old encoder derived `size` solely from <Rs> and
    // never validated <Rt>, silently encoding `ldadd w0, x1, [x2]` as a
    // 32-bit op. (Finding 2, fixed.)
    #[test]
    fn prop_mismatched_width_is_rejected(
        mn in arb_reg_form_mn(),
        rs in 0u32..=31u32,
        rt in 0u32..=31u32,
        rn in 0u32..=31u32,
        rs_is64 in any::<bool>(),
    ) {
        let (rs_w, rt_w) = if rs_is64 { ('x', 'w') } else { ('w', 'x') };
        let ops = vec![gp(rs_w, rs), gp(rt_w, rt), mem(rn, 0)];
        prop_assert!(
            encode_ldop(mn, &ops).is_err(),
            "mismatched Rs/Rt widths must be rejected, got {:?}",
            encode_ldop(mn, &ops)
        );
    }
}

// Deterministic anchor: hand-derived golden words (cross-checks the
// reference oracle against the ARMv8.1-A ARM encodings).
#[test]
fn golden_encodings_match_reference() {
    let cases: &[(&str, char, u32)] = &[
        ("ldadd", 'x', 0xF8200041),
        ("ldadd", 'w', 0xB8200041),
        ("ldaddb", 'w', 0x38200041),
        ("ldaddh", 'w', 0x78200041),
        ("ldclr", 'x', 0xF8201041),
        ("ldeor", 'x', 0xF8202041),
        ("ldset", 'x', 0xF8203041),
        ("ldadda", 'x', 0xF8A00041),
        ("ldaddl", 'x', 0xF8600041),
        ("ldaddal", 'x', 0xF8E00041),
    ];
    for &(mn, width, golden) in cases {
        let ops = vec![gp(width, 0), gp(width, 1), mem(2, 0)];
        let w = word(encode_ldop(mn, &ops));
        assert_eq!(
            w, golden,
            "golden mismatch for {} {}0,{}1,[x2]",
            mn, width, width
        );
        // Reference oracle must agree with both the encoder and the golden.
        assert_eq!(
            ref_encode(mn, 0, width == 'x', 1, 2),
            golden,
            "reference mismatch for {}",
            mn
        );
    }
}
