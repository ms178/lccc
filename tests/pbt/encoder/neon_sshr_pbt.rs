//! Property-based tests for `encode_neon_sshr`.
//!
//! `encode_neon_sshr` encodes the AArch64 NEON `SSHR` (Signed Shift Right)
//! immediate instruction — `SSHR Vd.T, Vn.T, #shift` — in the
//! "Advanced SIMD shift by immediate" encoding group:
//!
//! ```text
//!   31 30 29 28-23   22-16       15-10    9-5  4-0
//!    0  Q  0  011110  immh:immb  000001    Rn   Rd
//! ```
//! with U = 0 (bit 29) and `immh:immb = (2*esize - shift)`.
//!
//! The architecturally-valid shift range is `1 <= shift <= esize`
//! (ARMv8-A ARM, "SSHR (vector)"; verified with llvm-mc-14: shifts 0 and 9 for
//! an 8-bit element both fail with "immediate must be an integer in range
//! [1, 8]").
//!
//! Oracle: golden words come from `llvm-mc-14 -show-encoding` and are
//! independent of this crate. `ref_encode_sshr` re-assembles the word from
//! scratch using the documented formula (`immh:immb = 2*esize - shift`, no
//! masking), so it deliberately does NOT mirror the implementation.
//!
//! Finding (documented by the ignored tests): `encode_neon_sshr` performs NO
//! shift-range validation. shift==0 and `esize < shift <= 2*esize` are
//! silently accepted and emit words with immh == 0b0000 (architecturally
//! UNDEFINED); shifts beyond 2*esize and negative immediates additionally
//! PANIC in debug builds. It is the signed sibling of `encode_neon_ushr`
//! (identical structure, identical defect class). See SSHR_ENCODER_BUG_REPORT.md.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::{encode_neon_sshr, encode_neon_ushr, neon_arr_to_q_size};
use proptest::prelude::*;

// --- helpers --------------------------------------------------------------

fn va(n: u32, arr: &str) -> Operand {
    Operand::RegArrangement {
        reg: format!("v{}", n),
        arrangement: arr.to_string(),
    }
}

fn imm_op(v: i64) -> Operand {
    Operand::Imm(v)
}

fn reg_num_strategy() -> impl Strategy<Value = u32> {
    0u32..=31u32
}

/// Element size (bits) for an arrangement, per ARMv8-A ARM.
fn esize_of(arr: &str) -> u32 {
    match arr {
        "8b" | "16b" => 8,
        "4h" | "8h" => 16,
        "2s" | "4s" => 32,
        "2d" => 64,
        _ => 0,
    }
}

/// Arrangements architecturally VALID for SSHR (ARMv8-A ARM, SSHR (vector)).
fn valid_arrangement_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![
        Just("8b"),
        Just("16b"),
        Just("4h"),
        Just("8h"),
        Just("2s"),
        Just("4s"),
        Just("2d"),
    ]
}

/// Valid input: register pair + arrangement + shift in the legal range
/// `1 <= shift <= esize`.
fn valid_input_strategy() -> impl Strategy<Value = (u32, u32, &'static str, u32)> {
    (
        reg_num_strategy(),
        reg_num_strategy(),
        prop_oneof![
            (Just("8b"), 1u32..=8),
            (Just("16b"), 1u32..=8),
            (Just("4h"), 1u32..=16),
            (Just("8h"), 1u32..=16),
            (Just("2s"), 1u32..=32),
            (Just("4s"), 1u32..=32),
            (Just("2d"), 1u32..=64),
        ],
    )
        .prop_map(|(rd, rn, (arr, shift))| (rd, rn, arr, shift))
}

/// Independent reference encoder. Re-assembles the word from the documented
/// ARMv8-A layout using `immh:immb = 2*esize - shift` (NO masking), for
/// `1 <= shift <= esize`. Cross-checked against the llvm-mc golden table.
fn ref_encode_sshr(rd: u32, rn: u32, arr: &str, shift: u32) -> u32 {
    let (q, _size) = neon_arr_to_q_size(arr).unwrap();
    let esize = esize_of(arr);
    let immh_immb = 2 * esize - shift;
    (q << 30) | (0b011110u32 << 23) | (immh_immb << 16) | (0b000001u32 << 10) | (rn << 5) | rd
}

fn word_of(res: Result<EncodeResult, String>) -> u32 {
    match res {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {:?}", other),
    }
}

// --- golden table (absolute oracle, from `llvm-mc -show-encoding`) --------

/// (Rd, Rn, arrangement, shift, expected_word). Every word was emitted by
/// `llvm-mc-14 -assemble -arch=aarch64 -mattr=+neon -show-encoding` and is
/// therefore independent of this crate. The little-endian byte stream
/// `[b0,b1,b2,b3]` was folded into `word = b0 | b1<<8 | b2<<16 | b3<<24`.
const GOLDEN: &[(u32, u32, &str, u32, u32)] = &[
    (0, 1, "8b", 1, 0x0F0F0420),    // sshr v0.8b, v1.8b, #1
    (0, 1, "8b", 8, 0x0F080420),    // sshr v0.8b, v1.8b, #8   (shift == esize)
    (5, 6, "16b", 4, 0x4F0C04C5),   // sshr v5.16b, v6.16b, #4 (Q=1)
    (0, 1, "4h", 1, 0x0F1F0420),    // sshr v0.4h, v1.4h, #1
    (0, 1, "4s", 1, 0x4F3F0420),    // sshr v0.4s, v1.4s, #1
    (0, 1, "4s", 32, 0x4F200420),   // sshr v0.4s, v1.4s, #32 (shift == esize)
    (0, 1, "2d", 1, 0x4F7F0420),    // sshr v0.2d, v1.2d, #1
    (31, 30, "2d", 64, 0x4F4007DF), // sshr v31.2d, v30.2d, #64 (shift == esize)
];

#[test]
fn sshr_golden_table() {
    for &(rd, rn, arr, shift, expected) in GOLDEN {
        let ops = vec![va(rd, arr), va(rn, arr), imm_op(shift as i64)];
        let got = word_of(encode_neon_sshr(&ops));
        assert_eq!(
            got, expected,
            "sshr v{}.{} v{}.{} shift {} mismatch",
            rd, arr, rn, arr, shift,
        );
        assert_eq!(
            ref_encode_sshr(rd, rn, arr, shift),
            expected,
            "reference encoder drift for {} shift {}",
            arr,
            shift,
        );
    }
}

// --- properties (all PASS today) -----------------------------------------

proptest! {
    // === Differential oracle vs. independent reference encoder ============
    #[test]
    fn sshr_matches_reference_encoder(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
        shift in 1u32..64u32,
    ) {
        prop_assume!(shift <= esize_of(arr));
        let ops = vec![va(rd, arr), va(rn, arr), imm_op(shift as i64)];
        let got = word_of(encode_neon_sshr(&ops));
        let want = ref_encode_sshr(rd, rn, arr, shift);
        prop_assert_eq!(got, want, "SSHR word mismatch for {} shift {}", arr, shift);
    }

    // === Differential vs. USHR sibling: differ ONLY in bit 29 (U) ========
    // SSHR and USHR share the same encoding, except U=0 for SSHR and U=1 for
    // USHR. For identical operands the XOR of the two words must be exactly
    // 1<<29.
    #[test]
    fn sshr_differs_from_ushr_only_in_u_bit(input in valid_input_strategy()) {
        let (rd, rn, arr, shift) = input;
        let ops = vec![va(rd, arr), va(rn, arr), imm_op(shift as i64)];
        let sshr = word_of(encode_neon_sshr(&ops));
        let ushr = word_of(encode_neon_ushr(&ops));
        prop_assert_eq!(sshr ^ ushr, 1u32 << 29,
            "SSHR {:#x} and USHR {:#x} for {} shift {} must differ only in bit 29",
            sshr, ushr, arr, shift);
    }

    // === Fixed architectural bits are constant for valid shifts ===========
    #[test]
    fn sshr_fixed_bits_are_constant(input in valid_input_strategy()) {
        let (rd, rn, arr, shift) = input;
        let ops = vec![va(rd, arr), va(rn, arr), imm_op(shift as i64)];
        let w = word_of(encode_neon_sshr(&ops));

        prop_assert_eq!((w >> 31) & 1, 0u32, "bit 31 must be 0");
        prop_assert_eq!((w >> 29) & 1, 0u32, "U bit must be 0 for SSHR");
        prop_assert_eq!((w >> 23) & 0x3F, 0b011110u32, "bits 28-23 must be 011110");
        prop_assert_eq!((w >> 10) & 0x3F, 0b000001u32, "opcode+1 bits 15-10 must be 000001");
    }

    // === Field placement + immh:immb + Q map correctly ====================
    #[test]
    fn sshr_fields_and_immh_immb_place_correctly(input in valid_input_strategy()) {
        let (rd, rn, arr, shift) = input;
        let ops = vec![va(rd, arr), va(rn, arr), imm_op(shift as i64)];
        let w = word_of(encode_neon_sshr(&ops));
        let (q, _size) = neon_arr_to_q_size(arr).unwrap();
        let esize = esize_of(arr);

        prop_assert_eq!(w & 0x1F, rd, "Rd field for {}", arr);
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field for {}", arr);
        prop_assert_eq!((w >> 30) & 1, q, "Q bit for {}", arr);
        // immh:immb (bits 22-16) must equal 2*esize - shift, and the high
        // immh nibble must be non-zero (else the encoding is UNDEFINED).
        let immh_immb = (w >> 16) & 0x7F;
        prop_assert_eq!(immh_immb, 2 * esize - shift, "immh:immb for {} shift {}", arr, shift);
        prop_assert_ne!((immh_immb >> 3) & 0xF, 0u32, "immh must be non-zero for {}", arr);
    }

    // === Negative contract: malformed input is rejected ==================
    #[test]
    fn sshr_rejects_invalid_input(
        arr in "[a-z0-9]{1,4}".prop_filter("unknown arrangement", |s| {
            !matches!(s.as_str(), "8b"|"16b"|"4h"|"8h"|"2s"|"4s"|"1d"|"2d")
        }),
    ) {
        let ops = vec![va(0, arr.as_str()), va(1, arr.as_str()), imm_op(1)];
        prop_assert!(encode_neon_sshr(&ops).is_err(),
            "unsupported arrangement {:?} must be rejected", arr);

        // .1d is not a valid SSHR arrangement -> Err (currently correctly rejected)
        let ops1d = vec![va(0, "1d"), va(1, "1d"), imm_op(1)];
        prop_assert!(encode_neon_sshr(&ops1d).is_err(), "1d is not a valid SSHR arrangement");

        // too few operands -> Err
        prop_assert!(encode_neon_sshr(&[va(0, "8b"), va(1, "8b")]).is_err(),
            "sshr requires 3 operands (got 2)");
        prop_assert!(encode_neon_sshr(&[va(0, "8b")]).is_err(),
            "sshr requires 3 operands (got 1)");

        // third operand must be an immediate -> Err
        let bad = vec![va(0, "8b"), va(1, "8b"), va(2, "8b")];
        prop_assert!(encode_neon_sshr(&bad).is_err(), "third operand must be an immediate");
    }
}

// --- bug witnesses (FAIL today; ignored so default cargo test is green) ---

proptest! {
    // BUG: shift is never range-checked. The ARMv8-A ARM (and llvm-mc) only
    // accept `1 <= shift <= esize`. The implementation silently accepts
    // shift==0 and `esize < shift <= 2*esize`, emitting words whose immh
    // nibble is 0b0000 (architecturally UNDEFINED). Shifts beyond 2*esize and
    // negative immediates additionally PANIC (see the test below).
    //
    // Run: cargo test -- --ignored sshr_rejects_out_of_range_shift
    #[test]
    #[ignore]
    fn sshr_rejects_out_of_range_shift(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
        extra in 1u32..=64u32,
    ) {
        let esize = esize_of(arr);
        // Out-of-range, panic-free shift in [esize+1, 2*esize]:
        let shift_b = esize + 1 + (extra % esize);
        for &shift in &[0u32, shift_b] {
            let ops = vec![va(rd, arr), va(rn, arr), imm_op(shift as i64)];
            let res = encode_neon_sshr(&ops);
            prop_assert!(res.is_err(),
                "sshr {} shift {} is out of range [1, {}] but was accepted as {:?}",
                arr, shift, esize, res.ok());
        }
    }
}

// BUG: oversized (`> 2*esize`) and negative immediate shifts must be
// rejected, but `encode_neon_sshr` computes `(2*esize - shift)` with no
// bounds check. In debug builds `shift > 2*esize` underflows and a negative
// immediate (cast to `u32::MAX` by `get_imm(..)? as u32`) also underflows,
// panicking the whole assembler; in release the subtraction silently wraps.
//
// This is a FAILING proptest witness: it asserts the encoder returns `Err`,
// which it does not (panic in debug, `Ok(garbage)` in release). `#[ignore]`d
// so the default suite stays green. See SSHR bug reports under pbt-out/.
//
// Run: cargo test -- --ignored neon_sshr_pbt::sshr_returns_err_on_overflow_shift
proptest! {
    #[test]
    #[ignore]
    fn sshr_returns_err_on_overflow_shift(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
        extra in 1u32..=128u32,
        neg in 1u32..=64u32,
    ) {
        let esize = esize_of(arr);

        // Case A: positive shift strictly greater than 2*esize -> underflow / wrap.
        let big = 2 * esize + extra;
        let ops_a = vec![va(rd, arr), va(rn, arr), imm_op(big as i64)];
        let res_a = encode_neon_sshr(&ops_a);
        prop_assert!(res_a.is_err(),
            "sshr {} shift {} (> 2*esize {}) should be rejected, got {:?}",
            arr, big, 2 * esize, res_a.ok());

        // Case B: negative immediate -> `as u32` = u32::MAX -> underflow / wrap.
        let ops_b = vec![va(rd, arr), va(rn, arr), imm_op(-(neg as i64))];
        let res_b = encode_neon_sshr(&ops_b);
        prop_assert!(res_b.is_err(),
            "sshr {} negative shift -{} should be rejected, got {:?}",
            arr, neg, res_b.ok());
    }
}
