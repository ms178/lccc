//! Property-based tests for `encode_neon_float_cmp_zero`.
//!
//! `encode_neon_float_cmp_zero` packs the AArch64 floating-point
//! compare-to-zero family — `FCMEQ/FCMGE/FCMGT/FCMLE/FCMLT Vd.T, Vn.T, #0.0` —
//! in the "Advanced SIMD two-register miscellaneous" encoding group:
//!
//! ```text
//!   31 30 29 28-24 23-22 21-17  16-12   11-10 9-5 4-0
//!    0  Q  U  01110  size  10000  opcode  10   Rn  Rd
//! ```
//! `Q`/`sz` are derived from the arrangement `T` (2S→Q0,sz0; 4S→Q1,sz0;
//! 2D→Q1,sz1); `u_bit` and `opcode` are the per-mnemonic selectors; `size_hi`
//! becomes bit[23] of the `size` field.
//!
//! ## Oracle
//! No `llvm-mc` / `aarch64-linux-gnu-as` is available in this environment, so
//! absolute ARM-correctness of the per-mnemonic `opcode` *value* (set by the
//! dispatcher in `mod.rs`, not by this function) is NOT asserted here. This
//! function is a pure field-packer; its contract is "place these fields at these
//! documented bit positions". The oracle is therefore **field decomposition**:
//! each input field must land in its documented bit slot, the fixed bits must be
//! constant, and the output must be deterministic. An independent
//! `ref_encode_float_cmp_zero` re-assembles the documented layout field-by-field
//! for a differential cross-check that catches packing regressions.
//!
//! ## Finding — UNALLOCATED `size` field via unvalidated `size_hi`
//! Per the ARMv8-A ARM ("Advanced SIMD two-register miscellaneous",
//! FCMEQ/FCMGE/FCMGT/FCMLE/FCMLT (vector) #0.0), the `size` field bits[23:22]
//! is allocated ONLY for size=00 (single-precision, .2S/.4S) and size=01
//! (double-precision, .2D); bit[23] must be 0. The implementation never
//! validates `size_hi` and ORs it straight into bit[23]. Worse, the dispatcher
//! (`encoder/mod.rs:539-554`) actually passes `size_hi = 1` for `fcmgt` and
//! `fcmlt`, so e.g. `fcmgt v0.4s, v1.4s, #0.0` yields `size = 0b10` — an
//! UNALLOCATED encoding — instead of an error. See
//! `NEON_FLOAT_CMP_ZERO_BUG_REPORT.md`.
//!
//! The finding is pinned two ways below: a green *characterization* test
//! (`size_hi_nonzero_currently_emits_unallocated_size`) that records the
//! current buggy acceptance, and an `#[ignore]`d property
//! (`prop_rejects_unallocated_size_hi`) asserting the spec'd contract
//! (`Err`) — which will start passing once the bug is fixed.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_float_cmp_zero;
use proptest::prelude::*;

// --- helpers --------------------------------------------------------------

/// Build `Operand::RegArrangement { reg: "v{n}", arrangement }`.
fn va(n: u32, arr: &str) -> Operand {
    Operand::RegArrangement {
        reg: format!("v{n}"),
        arrangement: arr.to_string(),
    }
}

fn reg_num_strategy() -> impl Strategy<Value = u32> {
    0u32..=31u32
}

fn u_bit_strategy() -> impl Strategy<Value = u32> {
    prop_oneof![Just(0u32), Just(1u32)]
}

/// 5-bit opcode field (bits 16-12). Only the field *placement* is under test
/// here; the per-mnemonic opcode *value* is the dispatcher's responsibility.
fn opcode_strategy() -> impl Strategy<Value = u32> {
    0u32..=0x1Fu32
}

/// Arrangements architecturally VALID for floating-point compare-to-zero
/// (ARMv8-A ARM, FCMEQ/FCMGE/FCMGT/FCMLE/FCMLT (vector) #0.0): 2S, 4S, 2D.
fn valid_arrangement_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("2s"), Just("4s"), Just("2d"),]
}

/// Arrangements that are INVALID for the float compare-to-zero group: integer
/// widths (B/H), scalar `1d`, half-precision, and arbitrary garbage strings.
fn invalid_arrangement_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("8b".to_string()),
        Just("16b".to_string()),
        Just("4h".to_string()),
        Just("8h".to_string()),
        Just("1d".to_string()),
        Just("4s2".to_string()),
        Just("garbage".to_string()),
    ]
}

/// Documented (Q, sz) table for the valid float compare-to-zero arrangements.
fn qsz_of(arr: &str) -> (u32, u32) {
    match arr {
        "2s" => (0u32, 0u32),
        "4s" => (1u32, 0u32),
        "2d" => (1u32, 1u32),
        _ => unreachable!("invalid arrangement passed to qsz_of: {arr}"),
    }
}

/// Independent reference encoder: assembles the word field-by-field from the
/// documented ARMv8-A ARM layout. Only called with `size_hi == 0` (the sole
/// allocated value), so `size` stays within {00, 01}.
fn ref_encode_float_cmp_zero(rd: u32, rn: u32, arr: &str, u_bit: u32, opcode: u32) -> u32 {
    let (q, sz) = qsz_of(arr);
    let size = sz; // size_hi == 0 for the allocated case
    (q << 30)
        | ((u_bit & 1) << 29)
        | (0b01110u32 << 24)
        | (size << 22)
        | (0b10000u32 << 17) // bits 21-17
        | ((opcode & 0x1F) << 12) // opcode, bits 16-12
        | (0b10u32 << 10) // bits 11-10
        | (rn << 5)
        | rd
}

fn word_of(res: Result<EncodeResult, String>) -> u32 {
    match res {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {other:?}"),
    }
}

// field extractors
fn rd_of(w: u32) -> u32 {
    w & 0x1F
}
fn rn_of(w: u32) -> u32 {
    (w >> 5) & 0x1F
}
fn opcode_of(w: u32) -> u32 {
    (w >> 12) & 0x1F
}
fn size_of(w: u32) -> u32 {
    (w >> 22) & 0x3
}
fn u_of(w: u32) -> u32 {
    (w >> 29) & 1
}
fn q_of(w: u32) -> u32 {
    (w >> 30) & 1
}

// --- properties -----------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    // === Field decomposition: every input lands in its documented slot =====
    #[test]
    fn prop_float_cmp_zero_decomposes_into_documented_fields(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
        u_bit in u_bit_strategy(),
        opcode in opcode_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr)];
        // size_hi == 0: the only value ARMv8-A allocates for this group.
        let w = word_of(encode_neon_float_cmp_zero(&ops, u_bit, 0, opcode));

        let (q, sz) = qsz_of(arr);
        prop_assert_eq!(rd_of(w), rd);
        prop_assert_eq!(rn_of(w), rn);
        prop_assert_eq!(opcode_of(w), opcode);
        prop_assert_eq!(q_of(w), q);
        prop_assert_eq!(u_of(w), u_bit);
        prop_assert_eq!(size_of(w), sz);

        // fixed bits: bit31=0, bits[28:24]=01110, bits[21:17]=10000, bits[11:10]=10
        prop_assert_eq!(w >> 31, 0u32, "bit31 must be 0");
        prop_assert_eq!((w >> 24) & 0x1F, 0b01110u32, "bits[28:24] must be 01110");
        prop_assert_eq!((w >> 17) & 0x1F, 0b10000u32, "bits[21:17] must be 10000");
        prop_assert_eq!((w >> 10) & 0x3, 0b10u32, "bits[11:10] must be 10");
    }

    // === Differential: matches an independent field-by-field reassembly =====
    #[test]
    fn prop_float_cmp_zero_matches_reference(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
        u_bit in u_bit_strategy(),
        opcode in opcode_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let got = word_of(encode_neon_float_cmp_zero(&ops, u_bit, 0, opcode));
        let want = ref_encode_float_cmp_zero(rd, rn, arr, u_bit, opcode);
        prop_assert_eq!(got, want);
    }

    // === Determinism: identical inputs produce identical output ============
    #[test]
    fn prop_float_cmp_zero_is_deterministic(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
        u_bit in u_bit_strategy(),
        size_hi in 0u32..=1u32,
        opcode in opcode_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let w1 = word_of(encode_neon_float_cmp_zero(&ops, u_bit, size_hi, opcode));
        let w2 = word_of(encode_neon_float_cmp_zero(&ops, u_bit, size_hi, opcode));
        prop_assert_eq!(w1, w2);
    }

    // === Negative contract: unsupported arrangements return Err ============
    // The float compare-to-zero group is only defined for 2S/4S/2D; every
    // other arrangement must be rejected (and the function does reject them).
    #[test]
    fn prop_float_cmp_zero_rejects_unsupported_arrangements(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in invalid_arrangement_strategy(),
        u_bit in u_bit_strategy(),
        size_hi in 0u32..=1u32,
        opcode in opcode_strategy(),
    ) {
        let ops = vec![va(rd, &arr), va(rn, &arr)];
        let res = encode_neon_float_cmp_zero(&ops, u_bit, size_hi, opcode);
        prop_assert!(
            res.is_err(),
            "expected Err for unsupported arrangement {:?}, got {:?}",
            arr, res,
        );
    }

    // === FINDING: size_hi != 0 must be rejected as UNALLOCATED =============
    // ARMv8-A allocates size bits[23:22] only as 00/01 for this group, so
    // bit[23] (size_hi) must be 0. The function does NOT validate it and the
    // dispatcher passes size_hi=1 for fcmgt/fcmlt. This property asserts the
    // spec'd contract (Err); it FAILS in-band, demonstrating the bug.
    #[test]
    #[ignore = "documented bug: float compare-zero accepts unallocated size_hi bit"]
    fn prop_rejects_unallocated_size_hi(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
        u_bit in u_bit_strategy(),
        opcode in opcode_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let res = encode_neon_float_cmp_zero(&ops, u_bit, 1, opcode);
        prop_assert!(
            res.is_err(),
            "size_hi=1 yields unallocated size field (bit[23]=1) but got Ok: {:?}",
            res,
        );
    }
}

// --- characterization of the current bug (green today) --------------------

/// Pins the CURRENT buggy behavior so a future fix flips this red as a
/// reminder. `size_hi = 1` with a single-precision arrangement must place a 1
/// in bit[23] of the `size` field (size = 0b10), which is UNALLOCATED per the
/// ARMv8-A ARM for the float compare-to-zero group — yet it returns `Ok`.
/// `fcmgt`/`fcmlt` in `encoder/mod.rs` actually pass `size_hi = 1`.
#[test]
fn size_hi_nonzero_currently_emits_unallocated_size() {
    let ops = vec![va(0, "4s"), va(1, "4s")]; // fcmgt v0.4s, v1.4s, #0.0 path
    let w = word_of(encode_neon_float_cmp_zero(&ops, 0, 1, 0b01100));
    assert_eq!(size_of(w), 0b10, "size=0b10 is UNALLOCATED for this group");
    // Contrast: the same mnemonic with size_hi=0 (the allocated form).
    let w_ok = word_of(encode_neon_float_cmp_zero(&ops, 0, 0, 0b01100));
    assert_eq!(size_of(w_ok), 0b00);
    assert_eq!(w ^ w_ok, 1u32 << 23, "size_hi flips exactly bit[23]");
}

#[test]
fn accepts_all_three_valid_arrangements() {
    for &arr in &["2s", "4s", "2d"] {
        let ops = vec![va(3, arr), va(7, arr)];
        let res = encode_neon_float_cmp_zero(&ops, 0, 0, 0b01100);
        assert!(res.is_ok(), "arrangement {arr} should be accepted: {res:?}");
    }
}
