//! Property-based tests for `encode_neon_shrn` (NEON `SHRN`/`RSHRN`/`SHRN2`/`RSHRN2`
//! shift-right-narrow-immediate encoder).
//!
//! Encoding (ARMv8-A ARM, "Advanced SIMD shift by immediate", narrowing):
//!   `0 Q 0 01111 0 immh immb 100001 Rn Rd`   (SHRN/RSHRN: opcode 100001/100011)
//!    31 30 29 28-24 23 22-19 18-16 15-10 9-5 4-0
//!
//! `immh:immb = src_bits - shift` (7 bits). The mnemonic indicates the source
//! element size; the destination is half as wide. Valid source arrangements are
//! `.8h` (16-bit, dest .8b/.16b), `.4s` (32-bit, dest .4h/.8h), `.2d` (64-bit,
//! dest .2s/.4s). For each, `shift` must lie in `1..=src_bits/2`; outside that it
//! would decode to a different element size (or the reserved `immh == 0000`).
//!
//! Golden words cross-checked by hand against the ARM ARM:
//!   `shrn  v0.8b,  v1.8h, #1`  => `0x0F0F8420`
//!   `shrn2 v0.16b, v1.8h, #8`  => `0x4F088420`
//!   `shrn  v5.4h,  v6.4s, #4`  => `0x0F1C84C5`
//!   `shrn  v2.2s,  v3.2d, #16` => `0x0F308462`
//!   `rshrn v0.8b,  v1.8h, #1`  => `0x0F0F8C20`
//!
//! Findings:
//!  * The bit layout produced for every *valid* input matches an independent
//!    reference model (all correctness properties PASS).
//!  * `prop_shrn_truncates_huge_immediate` is a NEGATIVE-CONTRACT witness: the
//!    immediate is cast `i64 -> u32` *before* the range check, so an out-of-range
//!    value whose low 32 bits fall in `[1, half_bits]` is silently encoded as the
//!    truncated shift instead of being rejected. Marked `#[ignore]` so the
//!    default `cargo test` stays green.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_shrn;
use proptest::prelude::*;

// --- helpers --------------------------------------------------------------

/// Build a `SHRN Vd, Vn.<src>, #shift` operand slice. The destination
/// arrangement is read-then-discarded by the encoder (Q comes from `is_high`),
/// so its value does not affect the encoding.
fn shrn_ops(rd: u32, rn: u32, src_arr: &str, shift: i64) -> Vec<Operand> {
    vec![
        Operand::RegArrangement {
            reg: format!("v{rd}"),
            arrangement: "8b".to_string(),
        },
        Operand::RegArrangement {
            reg: format!("v{rn}"),
            arrangement: src_arr.to_string(),
        },
        Operand::Imm(shift),
    ]
}

/// The dispatch opcode for the non-rounding (SHRN) form.
const OPC_SHRN: u32 = 0b100001;
/// The dispatch opcode for the rounding (RSHRN) form.
const OPC_RSHRN: u32 = 0b100011;

/// Source-bit width for a supported SHRN arrangement.
fn src_bits(src_arr: &str) -> u32 {
    match src_arr {
        "8h" => 16,
        "4s" => 32,
        "2d" => 64,
        _ => unreachable!("unsupported src arrangement {src_arr}"),
    }
}

/// Independent reference encoder. Field grouping/shifts are deliberately written
/// differently from the implementation (`0 01111 0` as `0b01111 << 24` and an
/// explicit `immh`/`immb` split) so it is a genuine second implementation.
fn ref_shrn_word(rd: u32, rn: u32, src_arr: &str, shift: u32, opcode: u32, is_high: bool) -> u32 {
    let sbits = src_bits(src_arr);
    let immhb = sbits - shift; // 7-bit immh:immb
    let immh = (immhb >> 3) & 0xF; // bits 22..19
    let immb = immhb & 0x7; // bits 18..16
    let q: u32 = u32::from(is_high);
    // 0(31) Q(30) 0(29) 01111(28..24) 0(23) immh(22..19) immb(18..16) opcode(15..10) Rn(9..5) Rd(4..0)
    (q << 30) | (0b01111 << 24) | (immh << 19) | (immb << 16) | (opcode << 10) | (rn << 5) | rd
}

fn src_arr_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("8h"), Just("4s"), Just("2d")]
}

fn opcode_strategy() -> impl Strategy<Value = u32> {
    prop_oneof![Just(OPC_SHRN), Just(OPC_RSHRN)]
}

// --- golden constants -----------------------------------------------------

#[test]
fn golden_shrn_8h_shift1() {
    let ops = shrn_ops(0, 1, "8h", 1);
    let EncodeResult::Word(w) = encode_neon_shrn(&ops, OPC_SHRN, false).unwrap() else {
        panic!("expected Word");
    };
    assert_eq!(w, 0x0F0F8420);
}

#[test]
fn golden_shrn2_8h_shift8() {
    let ops = shrn_ops(0, 1, "8h", 8);
    let EncodeResult::Word(w) = encode_neon_shrn(&ops, OPC_SHRN, true).unwrap() else {
        panic!("expected Word");
    };
    assert_eq!(w, 0x4F088420);
}

#[test]
fn golden_shrn_4s_shift4() {
    let ops = shrn_ops(5, 6, "4s", 4);
    let EncodeResult::Word(w) = encode_neon_shrn(&ops, OPC_SHRN, false).unwrap() else {
        panic!("expected Word");
    };
    assert_eq!(w, 0x0F1C84C5);
}

#[test]
fn golden_shrn_2d_shift16() {
    let ops = shrn_ops(2, 3, "2d", 16);
    let EncodeResult::Word(w) = encode_neon_shrn(&ops, OPC_SHRN, false).unwrap() else {
        panic!("expected Word");
    };
    assert_eq!(w, 0x0F308462);
}

#[test]
fn golden_rshrn_rounding_opcode() {
    let ops = shrn_ops(0, 1, "8h", 1);
    let EncodeResult::Word(w) = encode_neon_shrn(&ops, OPC_RSHRN, false).unwrap() else {
        panic!("expected Word");
    };
    // RSHRN differs from SHRN only in opcode bit 11.
    assert_eq!(w, 0x0F0F8420 | (1 << 11));
    assert_eq!(w, 0x0F0F8C20);
}

// --- property 1: differential vs independent reference model ---------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// For every valid (arrangement, shift, register, form, high/low) the
    /// encoder's word must equal an independently written reference encoder.
    #[test]
    fn prop_shrn_matches_reference(
        src in src_arr_strategy(),
        rd in 0u32..=31,
        rn in 0u32..=31,
        shift_adj in 0u32.., // combined with arrangement to pick a valid shift
        opcode in opcode_strategy(),
        is_high in prop::bool::ANY,
    ) {
        let half = src_bits(src) / 2;
        let shift = (shift_adj % half) + 1; // 1..=half
        let ops = shrn_ops(rd, rn, src, shift as i64);
        let EncodeResult::Word(w) = encode_neon_shrn(&ops, opcode, is_high).unwrap() else {
            return Err(TestCaseError::fail("expected EncodeResult::Word"));
        };
        let expected = ref_shrn_word(rd, rn, src, shift, opcode, is_high);
        prop_assert_eq!(w, expected);
    }
}

// --- property 2: field decomposition --------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Every fixed/derived field of the encoded word must reflect the inputs:
    ///   bits 4..0   = Rd
    ///   bits 9..5   = Rn
    ///   bit  30     = Q == is_high
    ///   bits 31,29,28,23 = 0
    ///   bits 28..24 = 01111
    ///   immh:immb decodes back to shift = src_bits - immh:immb
    ///   bits 15..10 = opcode exactly
    #[test]
    fn prop_shrn_field_layout(
        src in src_arr_strategy(),
        rd in 0u32..=31,
        rn in 0u32..=31,
        shift_adj in 0u32..,
        opcode in opcode_strategy(),
        is_high in prop::bool::ANY,
    ) {
        let half = src_bits(src) / 2;
        let shift = (shift_adj % half) + 1;
        let ops = shrn_ops(rd, rn, src, shift as i64);
        let EncodeResult::Word(w) = encode_neon_shrn(&ops, opcode, is_high).unwrap() else {
            return Err(TestCaseError::fail("expected EncodeResult::Word"));
        };

        prop_assert_eq!(w & 0x1F, rd, "Rd field");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field");
        prop_assert_eq!((w >> 30) & 1, u32::from(is_high), "Q bit");
        prop_assert_eq!((w >> 31) & 1, 0, "bit31 must be 0");
        prop_assert_eq!((w >> 29) & 1, 0, "bit29 (U) must be 0");
        prop_assert_eq!((w >> 28) & 1, 0, "bit28 must be 0");
        prop_assert_eq!((w >> 23) & 1, 0, "bit23 must be 0");
        prop_assert_eq!((w >> 24) & 0x1F, 0b01111, "bits28..24 = 01111");
        prop_assert_eq!((w >> 10) & 0x3F, opcode & 0x3F, "opcode field");

        let immh = (w >> 19) & 0xF;
        let immb = (w >> 16) & 0x7;
        let immhb = (immh << 3) | immb;
        prop_assert_eq!(src_bits(src) - immhb, shift, "shift round-trips via immh:immb");
    }
}

// --- property 3: no UNALLOCATED encoding (immh != 0000) -------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// For every accepted encoding the immh field must be non-zero (a zero immh
    /// is the reserved/unallocated encoding in the shift-by-immediate class).
    #[test]
    fn prop_shrn_immh_never_zero(
        src in src_arr_strategy(),
        rd in 0u32..=31,
        rn in 0u32..=31,
        shift_adj in 0u32..,
        opcode in opcode_strategy(),
        is_high in prop::bool::ANY,
    ) {
        let half = src_bits(src) / 2;
        let shift = (shift_adj % half) + 1;
        let ops = shrn_ops(rd, rn, src, shift as i64);
        let EncodeResult::Word(w) = encode_neon_shrn(&ops, opcode, is_high).unwrap() else {
            return Err(TestCaseError::fail("expected EncodeResult::Word"));
        };
        let immh = (w >> 19) & 0xF;
        prop_assert_ne!(immh, 0, "immh must never be 0000 (unallocated)");
    }
}

// --- property 4: negative contract (out-of-spec inputs are rejected) ------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// `shift == 0` is out of range for every source arrangement and must Err.
    #[test]
    fn prop_shrn_rejects_shift_zero(src in src_arr_strategy(), rd in 0u32..=31, rn in 0u32..=31) {
        let ops = shrn_ops(rd, rn, src, 0);
        prop_assert!(encode_neon_shrn(&ops, OPC_SHRN, false).is_err(),
                     "shift 0 should be rejected");
    }

    /// `shift == half_bits + 1` is one past the maximum and must Err.
    #[test]
    fn prop_shrn_rejects_shift_too_large(src in src_arr_strategy(), rd in 0u32..=31, rn in 0u32..=31) {
        let over = (src_bits(src) / 2) + 1;
        let ops = shrn_ops(rd, rn, src, over as i64);
        prop_assert!(encode_neon_shrn(&ops, OPC_SHRN, false).is_err(),
                     "shift past half-width should be rejected");
    }

    /// Unsupported source arrangements must Err.
    #[test]
    fn prop_shrn_rejects_bad_arrangement(bad in "(8b|16b|4h|2s|1d|2d2|4s_bad|8h_bad|foo)") {
        let ops = shrn_ops(0, 1, &bad, 1);
        prop_assert!(encode_neon_shrn(&ops, OPC_SHRN, false).is_err(),
                     "arrangement {bad:?} should be rejected");
    }
}

#[test]
fn rejects_too_few_operands() {
    let ops = vec![
        Operand::RegArrangement {
            reg: "v0".into(),
            arrangement: "8b".into(),
        },
        Operand::RegArrangement {
            reg: "v1".into(),
            arrangement: "8h".into(),
        },
        // missing #shift
    ];
    assert!(encode_neon_shrn(&ops, OPC_SHRN, false).is_err());
    let ops = vec![Operand::RegArrangement {
        reg: "v0".into(),
        arrangement: "8b".into(),
    }];
    assert!(encode_neon_shrn(&ops, OPC_SHRN, false).is_err());
}

#[test]
fn rejects_non_immediate_shift() {
    let ops = vec![
        Operand::RegArrangement {
            reg: "v0".into(),
            arrangement: "8b".into(),
        },
        Operand::RegArrangement {
            reg: "v1".into(),
            arrangement: "8h".into(),
        },
        Operand::Reg("x2".into()),
    ];
    assert!(encode_neon_shrn(&ops, OPC_SHRN, false).is_err());
}

// --- property 5: NEGATIVE-CONTRACT witness (truncation bug) ---------------

/// `get_imm(..)? as u32` truncates the `i64` *before* the range check, so an
/// out-of-range immediate whose low 32 bits land inside `[1, half_bits]` is
/// silently encoded as that truncated shift instead of being rejected.
///
/// `0x1_0000_0001` (= 4294967297) wraps to `1` as `u32`, so a `.8h` operand is
/// encoded identically to `#1`. Per the ARM ARM the shift must be a small
/// integer in `[1, 32]`; this value must be rejected.
///
/// Marked `#[ignore]`: documents the finding without breaking `cargo test`.
#[test]
#[ignore = "negative-contract witness: huge immediate silently truncated into range"]
fn prop_shrn_truncates_huge_immediate() {
    let huge: i64 = 0x1_0000_0001; // truncates to shift 1
    let ops = shrn_ops(0, 1, "8h", huge);
    let res = encode_neon_shrn(&ops, OPC_SHRN, false);
    assert!(
        res.is_err(),
        "out-of-range immediate {huge:#x} must be rejected, got {res:?}"
    );

    // A second form: a value one past the true max (9 for .8h) wrapped around
    // the 32-bit boundary so it lands back inside the range.
    let wrap_to_two: i64 = 0x1_0000_0002; // truncates to shift 2
    let ops = shrn_ops(0, 1, "8h", wrap_to_two);
    let res = encode_neon_shrn(&ops, OPC_SHRN, false);
    assert!(
        res.is_err(),
        "out-of-range immediate {wrap_to_two:#x} must be rejected, got {res:?}"
    );
}
