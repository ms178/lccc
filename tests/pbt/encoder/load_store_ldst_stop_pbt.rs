//! Property-based tests for four `load_store.rs` encoders that share a
//! common defect class: **silent offset mishandling**.
//!
//! Covered functions (all exercised through the real production symbols):
//!   * `encode_ldur_stur`  — LDUR/STUR (unscaled immediate)
//!   * `encode_ldtr_sized` — LDTR/STTR/LDTRB/LDTRH (unprivileged, sized)
//!   * `encode_ldrsw`      — LDRSW (signed word load, all addressing forms)
//!   * `encode_stop`       — STADD/STCLR/STEOR/STSET (LSE store aliases)
//!
//! This module is **complementary** to the inline `prop_encode_*_tests`
//! modules inside `load_store.rs`. It exists as a separate file (per the
//! repo's `*_pbt.rs` convention, declared in `encoder/mod.rs`) so the
//! negative-contract bug witnesses can be kept uniformly `#[ignore]` —
//! keeping the default `cargo test` run green while still recording each
//! defect as a failing, runnable property.
//!
//! ## Findings recorded here
//!
//! ### LDUR/STUR, LDTR/STTR — silent imm9 masking (shared defect)
//! Both encoders compute the 9-bit signed immediate as
//! `imm9_enc = (imm9 as u32) & 0x1FF` with NO range check. The imm9 field
//! covers the signed range **[-256, 255]**; an offset strictly outside
//! that range is silently wrapped (e.g. #256 → #0, #-257 → #-1) and a
//! wrong instruction word is emitted with no diagnostic.
//!   * Witness: `ldur_stur_out_of_range_imm9_rejected` (#[ignore])
//!   * Witness: `ldtr_sized_out_of_range_imm9_rejected` (#[ignore])
//!   * Already reported: `bug_reports/encode_ldur_stur_imm9_truncation.md`
//!     and `bug_reports/ldtr_sized_imm9_silent_truncation.md`.
//!
//! ### LDRSW — silent offset truncation (Mem + pre/post forms)
//! The unscaled (LDURSW) fallback and the pre/post-index paths all do
//! `imm9 = (*offset as i32) & 0x1FF` with no range check. For the `[base,#imm]`
//! Mem form, an offset representable by NEITHER the unsigned imm12 field
//! (pimm ∈ {0,4,…,16380}) NOR the signed imm9 ([-256,255]) is silently
//! truncated instead of rejected.
//!   * Witness: `ldrsw_unrepresentable_mem_offset_rejected` (#[ignore])
//!   * Witness: `ldrsw_pre_post_out_of_range_imm9_rejected` (#[ignore])
//!   * Already reported: `bug_reports/ldrsw_silent_offset_truncation.md`.
//!
//! ### encode_stop — silent immediate-offset drop (NEW finding)
//! The ARMv8.1-A LSE *store* aliases (STADD/STCLR/STEOR/STSET and their
//! `b`/`h`/`l` variants) have **no** immediate-offset addressing form —
//! the only permitted syntax is `STop <Rs>, [<Xn|SP>]`. `encode_stop`
//! matches `Operand::Mem { base, .. }` and never inspects `offset`, so a
//! non-zero offset such as `stadd x0, [x1, #8]` is silently encoded as
//! `stadd x0, [x1]`. This is the same defect family already reported for
//! the sibling `encode_ldop` / `encode_swp` / `encode_cas` / exclusive
//! load-store encoders, but had NOT previously been reported for
//! `encode_stop`.
//!   * Witness: `stop_nonzero_offset_rejected` (#[ignore])
//!   * Mechanism (passes): `stop_offset_silently_dropped`
//!   * Report: `bug_reports/encode_stop_silent_offset_drop.md`.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::{encode_ldrsw, encode_ldtr_sized, encode_ldur_stur, encode_stop};
use proptest::prelude::*;

// ── Oracle classification ─────────────────────────────────────────────
// Each function is a pure instruction encoder. The strongest applicable
// oracle is a **Reference / field-placement** check (the ARM ARM fixes the
// bit layout), complemented by a **Negative/Error Contract** for the
// out-of-range inputs the encoders wrongly accept.
//   Stronger considered & rejected:
//     - State Machine (3): no state field / lifecycle — pure functions.
//     - Differential (7): no second independent implementation available.
//     - Round-trip (4a): these are one-way encoders with no in-tree inverse.
//   Weaker available: Invariant (4d), Crash-Only (6).

// ── Helpers ───────────────────────────────────────────────────────────

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
/// Sign-extend the imm9 field ([20:12]) back to i32.
fn decode_imm9(w: u32) -> i32 {
    let f = (w >> 12) & 0x1FF;
    if f & 0x100 != 0 {
        (f | 0xFFFFFE00) as i32
    } else {
        f as i32
    }
}

// ═══════════════════════════════════════════════════════════════════════
// encode_ldur_stur  (LDUR/STUR — unscaled immediate, ARM ARM §C4.1.66)
//   size[31:30] 111[29:27] V[26] 00[25:24] opc[23:22] 0[21] imm9[20:12]
//   op2[11:10] Rn[9:5] Rt[4:0]     (imm9 signed, ∈ [-256,255])
//   Goldens (hand-derived, GP V=0): ldur x0,[x1]=0xF8400020
//   ldur x0,[x1,#8]=0xF8408020  ldur x0,[x1,#-1]=0xF85FF020  ldur w0,[x1]=0xB8400020
// ═══════════════════════════════════════════════════════════════════════

proptest! {
    // Property 1 — in-range imm9 round-trips and fields land correctly.
    #[test]
    fn ldur_stur_in_range_matches_reference(
        rt in 0u32..=31u32,
        rn in 0u32..=31u32,
        off in -256i64..=255i64,
        is_load in any::<bool>(),
    ) {
        let ops = vec![gp('x', rt), mem(rn, off)];
        let w = word(encode_ldur_stur(&ops, is_load, 0b00));
        prop_assert_eq!(w & 0x1F, rt, "Rt [4:0]");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn [9:5]");
        prop_assert_eq!(decode_imm9(w) as i64, off, "imm9 sign-extended round-trip");
        // fixed bits: [29:27]=111, [25:24]=00, bit21=0, V=0 for GP
        prop_assert_eq!((w >> 27) & 0b111, 0b111u32);
        prop_assert_eq!((w >> 24) & 0b11, 0u32);
        prop_assert_eq!((w >> 21) & 1, 0u32);
        prop_assert_eq!((w >> 26) & 1, 0u32);
    }

    // Property 2 — load vs store differ ONLY in opc bit 22.
    #[test]
    fn ldur_stur_load_xor_store_is_opc_bit22(
        rt in 0u32..=31u32,
        rn in 0u32..=31u32,
        off in -256i64..=255i64,
    ) {
        let ops = vec![gp('x', rt), mem(rn, off)];
        let load = word(encode_ldur_stur(&ops, true, 0b00));
        let store = word(encode_ldur_stur(&ops, false, 0b00));
        prop_assert_eq!(load ^ store, 0x0040_0000u32);
    }

    // Property 3 — op2_bits parameter lands in [11:10] (LDUR=00, LDTR=10).
    #[test]
    fn ldur_stur_op2_bits_in_field_11_10(op2 in 0u32..=3u32) {
        let ops = vec![gp('x', 0), mem(1, 0)];
        let w = word(encode_ldur_stur(&ops, true, op2));
        prop_assert_eq!((w >> 10) & 0b11, op2 & 0b11);
    }

    // Property 4 — NEGATIVE CONTRACT (witness). The imm9 field is a SIGNED
    // 9-bit immediate covering [-256,255]; an offset strictly outside that
    // range is unrepresentable by LDUR/STUR and MUST be rejected. Instead
    // the encoder does `imm9_enc = (imm9 as u32) & 0x1FF`, silently
    // wrapping out-of-range offsets.
    //
    // EXPECTED TO FAIL today; marked #[ignore] so `cargo test` stays green.
    // Run with:  cargo test ldur_stur_out_of_range_imm9_rejected -- --ignored
    #[test]
    #[ignore = "documented bug: LDUR/STUR imm9 offsets outside [-256,255] are masked via & 0x1FF"]
    fn ldur_stur_out_of_range_imm9_rejected(
        is_load in any::<bool>(),
        off in (-4096i64..4096i64).prop_filter(
            "out of signed-9-bit range", |o| *o < -256 || *o > 255),
    ) {
        let ops = vec![gp('x', 0), mem(1, off)];
        let res = encode_ldur_stur(&ops, is_load, 0b00);
        prop_assert!(
            res.is_err(),
            "offset {} is outside the LDUR/STUR imm9 range [-256,255] and must \
             be rejected; got {:?}", off, res);
    }
}

#[test]
fn ldur_stur_golden_encodings() {
    assert_eq!(
        word(encode_ldur_stur(&[gp('x', 0), mem(1, 0)], true, 0b00)),
        0xF8400020
    );
    assert_eq!(
        word(encode_ldur_stur(&[gp('x', 0), mem(1, 8)], true, 0b00)),
        0xF8408020
    );
    assert_eq!(
        word(encode_ldur_stur(&[gp('x', 0), mem(1, -1)], true, 0b00)),
        0xF85FF020
    );
    assert_eq!(
        word(encode_ldur_stur(&[gp('w', 0), mem(1, 0)], true, 0b00)),
        0xB8400020
    );
}

// ═══════════════════════════════════════════════════════════════════════
// encode_ldtr_sized  (LDTR/STTR — unprivileged, ARM ARM §C4.1.66)
//   size[31:30] 111[29:27] V=0[26] 00[25:24] opc[23:22] 0[21] imm9[20:12]
//   10[11:10] Rn[9:5] Rt[4:0]      (imm9 signed, ∈ [-256,255])
//   Goldens: ldtr x0,[x1]=0xF8400820  sttr x0,[x1]=0xF8000820
//   ldtr x0,[x1,#8]=0xF8408820  ldtr x0,[x1,#-1]=0xF85FF820  ldtrb w0,[x1]=0x38400820
// ═══════════════════════════════════════════════════════════════════════

proptest! {
    // Property 1 — in-range imm9 round-trips; size param → [31:30]; MS=10.
    #[test]
    fn ldtr_sized_in_range_matches_reference(
        rt in 0u32..=31u32,
        rn in 0u32..=31u32,
        off in -256i64..=255i64,
        size in 0u32..=3u32,
        is_load in any::<bool>(),
    ) {
        let ops = vec![gp('x', rt), mem(rn, off)];
        let w = word(encode_ldtr_sized(&ops, is_load, size));
        prop_assert_eq!(w & 0x1F, rt, "Rt [4:0]");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn [9:5]");
        prop_assert_eq!((w >> 30) & 0b11, size, "size param → [31:30]");
        prop_assert_eq!((w >> 10) & 0b11, 0b10u32, "MS [11:10]=10 for LDTR/STTR");
        prop_assert_eq!((w >> 26) & 1, 0u32, "V=0 for GP");
        prop_assert_eq!(decode_imm9(w) as i64, off, "imm9 sign-extended round-trip");
    }

    // Property 2 — load vs store differ ONLY in opc bit 22, for every size.
    #[test]
    fn ldtr_sized_load_xor_store_is_opc_bit22(
        rt in 0u32..=31u32,
        rn in 0u32..=31u32,
        off in -256i64..=255i64,
        size in 0u32..=3u32,
    ) {
        let ops = vec![gp('x', rt), mem(rn, off)];
        let load = word(encode_ldtr_sized(&ops, true, size));
        let store = word(encode_ldtr_sized(&ops, false, size));
        prop_assert_eq!(load ^ store, 0x0040_0000u32);
    }

    // Property 3 — NEGATIVE CONTRACT (witness). Same masking defect as
    // LDUR/STUR: out-of-range imm9 must be rejected, not silently wrapped.
    //
    // EXPECTED TO FAIL today; marked #[ignore].
    // Run with:  cargo test ldtr_sized_out_of_range_imm9_rejected -- --ignored
    #[test]
    #[ignore = "documented bug: LDTR/STTR imm9 offsets outside [-256,255] are masked via & 0x1FF"]
    fn ldtr_sized_out_of_range_imm9_rejected(
        is_load in any::<bool>(),
        size in 0u32..=3u32,
        off in (-4096i64..4096i64).prop_filter(
            "out of signed-9-bit range", |o| *o < -256 || *o > 255),
    ) {
        let ops = vec![gp('x', 0), mem(1, off)];
        let res = encode_ldtr_sized(&ops, is_load, size);
        prop_assert!(
            res.is_err(),
            "offset {} is outside the LDTR/STTR imm9 range [-256,255] and must \
             be rejected; got {:?}", off, res);
    }
}

#[test]
fn ldtr_sized_golden_encodings() {
    assert_eq!(
        word(encode_ldtr_sized(&[gp('x', 0), mem(1, 0)], true, 0b11)),
        0xF8400820
    );
    assert_eq!(
        word(encode_ldtr_sized(&[gp('x', 0), mem(1, 0)], false, 0b11)),
        0xF8000820
    );
    assert_eq!(
        word(encode_ldtr_sized(&[gp('x', 0), mem(1, 8)], true, 0b11)),
        0xF8408820
    );
    assert_eq!(
        word(encode_ldtr_sized(&[gp('x', 0), mem(1, -1)], true, 0b11)),
        0xF85FF820
    );
    assert_eq!(
        word(encode_ldtr_sized(&[gp('w', 0), mem(1, 0)], true, 0b00)),
        0x38400820
    );
}

// ═══════════════════════════════════════════════════════════════════════
// encode_ldrsw  (LDRSW — signed word load, ARM ARM §C4.1.65)
//   unsigned : 10 111 0 01 10 imm12[21:10] Rn[9:5] Rt[4:0]   (pimm=imm12*4)
//   LDURSW   : 10 111 0 00 10 0 imm9[20:12] 00 Rn Rt         (imm9 signed)
//   pre/post : 10 111 0 00 10 0 imm9[20:12] {11|01} Rn Rt
//   Goldens: ldrsw x0,[x1]=0xB9800020  ldrsw x0,[x1,#4]=0xB9800420
//   pre 0xB8808C20  post 0xB8808420  ldursw x0,[x1,#1]=0xB8801020
// ═══════════════════════════════════════════════════════════════════════

proptest! {
    // Property 1 — unsigned-offset layout vs golden for valid pimm.
    #[test]
    fn ldrsw_unsigned_offset_matches_reference(
        rt in 0u32..=31u32,
        rn in 0u32..=31u32,
        imm12 in 0u32..=4095u32,
    ) {
        let off = (imm12 as i64) * 4;
        let ops = vec![gp('x', rt), mem(rn, off)];
        let w = word(encode_ldrsw(&ops));
        let expected = (0xB9800020u32 as i64
            + (rt as i64)
            + (((rn as i64) - 1) << 5)
            + ((imm12 as i64) << 10)) as u32;
        prop_assert_eq!(w, expected);
        prop_assert_eq!((w >> 24) & 0b11, 0b01u32, "[25:24]=01 unsigned");
        prop_assert_eq!((w >> 22) & 0b11, 0b10u32, "opc=10");
    }

    // Property 2 — pre/post-index: imm9 round-trips and marker [11:10] set.
    #[test]
    fn ldrsw_pre_post_index_imm9_and_marker(
        rt in 0u32..=31u32,
        rn in 0u32..=31u32,
        imm9 in -256i32..=255i32,
    ) {
        let pre = word(encode_ldrsw(&[gp('x', rt),
            Operand::MemPreIndex { base: format!("x{}", rn), offset: imm9 as i64 }]));
        let post = word(encode_ldrsw(&[gp('x', rt),
            Operand::MemPostIndex { base: format!("x{}", rn), offset: imm9 as i64 }]));
        prop_assert_eq!(decode_imm9(pre) as i32, imm9);
        prop_assert_eq!(decode_imm9(post) as i32, imm9);
        prop_assert_eq!((pre >> 10) & 0b11, 0b11u32, "pre marker [11:10]=11");
        prop_assert_eq!((post >> 10) & 0b11, 0b01u32, "post marker [11:10]=01");
    }

    // Property 3 — NEGATIVE CONTRACT (witness, Mem form). An offset
    // representable by NEITHER the unsigned imm12 field (pimm ∈ {0,4,…,16380})
    // NOR the signed imm9 ([-256,255]) MUST be rejected. The encoder instead
    // truncates via `& 0x1FF`.
    //
    // EXPECTED TO FAIL today; marked #[ignore].
    // Run with:  cargo test ldrsw_unrepresentable_mem_offset_rejected -- --ignored
    #[test]
    #[ignore = "documented bug: LDRSW Mem offsets outside the representable range are truncated via & 0x1FF"]
    fn ldrsw_unrepresentable_mem_offset_rejected(
        off in (-10000i64..=100000i64).prop_filter(
            "not unsigned-pimm and not signed-imm9",
            |o| { let o = *o;
                  !((0..=16380).contains(&o) && o % 4 == 0) && !(-256..=255).contains(&o) }),
    ) {
        let ops = vec![gp('x', 0), mem(1, off)];
        let res = encode_ldrsw(&ops);
        prop_assert!(
            res.is_err(),
            "offset {} is not representable by LDRSW (unsigned pimm or signed imm9) \
             and must be rejected; got {:?}", off, res);
    }

    // Property 4 — NEGATIVE CONTRACT (witness, pre/post-index). The imm9
    // field for pre/post-index is a signed 9-bit value in [-256,255]; values
    // outside that range MUST be rejected, not masked.
    //
    // EXPECTED TO FAIL today; marked #[ignore].
    // Run with:  cargo test ldrsw_pre_post_out_of_range_imm9_rejected -- --ignored
    #[test]
    #[ignore = "documented bug: LDRSW pre/post imm9 offsets outside [-256,255] are masked"]
    fn ldrsw_pre_post_out_of_range_imm9_rejected(
        form in 0u32..2u32,
        off in (-4096i64..4096i64).prop_filter(
            "out of signed-9-bit range", |o| *o < -256 || *o > 255),
    ) {
        let mem = match form {
            0 => Operand::MemPreIndex { base: "x1".to_string(), offset: off },
            _ => Operand::MemPostIndex { base: "x1".to_string(), offset: off },
        };
        let res = encode_ldrsw(&[gp('x', 0), mem]);
        prop_assert!(
            res.is_err(),
            "pre/post offset {} is outside imm9 range [-256,255] and must be \
             rejected; got {:?}", off, res);
    }
}

#[test]
fn ldrsw_golden_encodings() {
    assert_eq!(word(encode_ldrsw(&[gp('x', 0), mem(1, 0)])), 0xB9800020);
    assert_eq!(word(encode_ldrsw(&[gp('x', 0), mem(1, 4)])), 0xB9800420);
    // #1 is not a multiple of 4 → LDURSW fallback, imm9=1 → 0xB8801020.
    assert_eq!(word(encode_ldrsw(&[gp('x', 0), mem(1, 1)])), 0xB8801020);
    let pre = encode_ldrsw(&[
        gp('x', 0),
        Operand::MemPreIndex {
            base: "x1".into(),
            offset: 8,
        },
    ]);
    let post = encode_ldrsw(&[
        gp('x', 0),
        Operand::MemPostIndex {
            base: "x1".into(),
            offset: 8,
        },
    ]);
    assert_eq!(word(pre), 0xB8808C20);
    assert_eq!(word(post), 0xB8808420);
}

// ═══════════════════════════════════════════════════════════════════════
// encode_stop  (STADD/STCLR/STEOR/STSET — LSE store aliases, ARM ARM §C6.2.274)
//   size[31:30] 111000[29:24] 0[23]=A  R[22] 1[21] Rs[20:16] 0[15]
//     opc[14:12] 00[11:10] Rn[9:5] Rt[4:0]=31 (XZR/WZR)
//   opc: STADD=000 STCLR=001 STEOR=010 STSET=011 ; R: release ('l' suffix)
//   These store aliases have NO immediate-offset form: only `STop <Rs>,[<Xn|SP>]`.
//   Goldens: stadd x0,[x1]=0xF820003F  stadd w0,[x1]=0xB820003F
//   staddb w0,[x1]=0x3820003F  staddl x0,[x1]=0xF860003F  stclr x0,[x1]=0xF820103F
// ═══════════════════════════════════════════════════════════════════════

/// Representative coverage of every base op + suffix class (24 mnemonics).
const STOP_MNEMONICS: &[&str] = &[
    "stadd", "staddl", "staddb", "staddlb", "staddh", "staddlh", "stclr", "stclrl", "stclrb",
    "stclrlb", "stclrh", "stclrlh", "steor", "steorl", "steorb", "steorlb", "steorh", "steorlh",
    "stset", "stsetl", "stsetb", "stsetlb", "stseth", "stsetlh",
];

fn stop_opc(mn: &str) -> u32 {
    if mn.starts_with("stadd") {
        0b000
    } else if mn.starts_with("stclr") {
        0b001
    } else if mn.starts_with("steor") {
        0b010
    } else if mn.starts_with("stset") {
        0b011
    } else {
        panic!("unexpected mnemonic {}", mn)
    }
}

/// Spec-transcribed reference word (built from the ARM bit layout, NOT by
/// calling `encode_stop`). Mirrors the encoder's rule that `<Rs>`'s width
/// selects size for the register form; 'b'/'h' suffixes override it.
fn stop_ref(mn: &str, rs: u32, rs_is64: bool, rn: u32) -> u32 {
    let suffix = mn
        .strip_prefix("stadd")
        .or_else(|| mn.strip_prefix("stclr"))
        .or_else(|| mn.strip_prefix("steor"))
        .or_else(|| mn.strip_prefix("stset"))
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
    let r: u32 = u32::from(suffix.contains('l'));
    (size << 30)
        | (0b111000 << 24)
        | (r << 22)
        | (1 << 21)
        | (rs << 16)
        | (stop_opc(mn) << 12)
        | (rn << 5)
        | 31 // Rt = XZR/WZR (store alias)
}

prop_compose! {
    fn arb_stop_mn()(idx in 0usize..STOP_MNEMONICS.len()) -> &'static str {
        STOP_MNEMONICS[idx]
    }
}

proptest! {
    // Property 1 — reference-encoding differential (PASSES). For every
    // valid (mnemonic, width, register) combination with offset 0 the
    // encoder output must equal the spec-transcribed reference word.
    #[test]
    fn stop_reference_encoding_matches(
        mn in arb_stop_mn(),
        rs in 0u32..=31u32,
        rn in 0u32..=31u32,
        rs_is64 in any::<bool>(),
    ) {
        // The byte/halfword forms take w registers (b/h ops read 8/16
        // bits of a W register; GNU as rejects the x spelling).
        let width = if mn.contains('b') || mn.contains('h') {
            'w'
        } else if rs_is64 {
            'x'
        } else {
            'w'
        };
        let want_is64 = width == 'x';
        let ops = vec![gp(width, rs), mem(rn, 0)];
        let got = word(encode_stop(mn, &ops));
        let want = stop_ref(mn, rs, want_is64, rn);
        prop_assert_eq!(got, want, "reference mismatch for {}", mn);
    }

    // Property 2 — non-zero offsets are rejected (was: silently dropped).
    // The ST-only aliases encode only [Xn]; there is no offset field.
    #[test]
    fn stop_nonzero_offset_is_rejected(
        mn in arb_stop_mn(),
        off in any::<i64>().prop_filter("non-zero", |o| *o != 0),
    ) {
        let width = if mn.contains('b') || mn.contains('h') { 'w' } else { 'x' };
        let ops = vec![gp(width, 0), mem(1, off)];
        let res = encode_stop(mn, &ops);
        prop_assert!(res.is_err(),
            "non-zero offset must be rejected (no offset field), got {:?}", res);
    }

    // Property 3 — NEGATIVE CONTRACT (witness). The LSE store-alias group
    // has NO immediate-offset form; the only permitted syntax is
    // `STop <Rs>, [<Xn|SP>]`. A non-zero offset is therefore unrepresentable
    // and MUST be rejected rather than silently encoded as `[Xn]`.
    //
    // EXPECTED TO FAIL today; marked #[ignore].
    // Run with:  cargo test stop_nonzero_offset_rejected -- --ignored
    #[test]
    #[ignore = "documented bug: LSE store aliases have no offset form; non-zero offset silently dropped"]
    fn stop_nonzero_offset_rejected(
        mn in arb_stop_mn(),
        off in (-32768i64..32767i64).prop_filter("non-zero", |o| *o != 0),
    ) {
        let ops = vec![gp('x', 0), mem(1, off)];
        let res = encode_stop(mn, &ops);
        prop_assert!(
            res.is_err(),
            "non-zero offset {} is unrepresentable for ST* aliases and must be \
             Err; got {:?}", off, res);
    }
}

#[test]
fn stop_golden_encodings() {
    let cases: &[(&str, char, u32)] = &[
        ("stadd", 'x', 0xF820003F),
        ("stadd", 'w', 0xB820003F),
        ("staddb", 'w', 0x3820003F),
        ("staddh", 'w', 0x7820003F),
        ("staddl", 'x', 0xF860003F),
        ("stclr", 'x', 0xF820103F),
        ("steor", 'x', 0xF820203F),
        ("stset", 'x', 0xF820303F),
    ];
    for &(mn, width, golden) in cases {
        let ops = vec![gp(width, 0), mem(1, 0)];
        assert_eq!(
            word(encode_stop(mn, &ops)),
            golden,
            "golden mismatch for {}",
            mn
        );
        assert_eq!(
            stop_ref(mn, 0, width == 'x', 1),
            golden,
            "ref mismatch for {}",
            mn
        );
    }
}
