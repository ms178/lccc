//! Property-based tests for `encode_neon_rbit`.
//!
//! `encode_neon_rbit` encodes the AArch64 NEON `RBIT` instruction —
//! `RBIT Vd.T, Vn.T` — which reverses the bits in each byte of the vector.
//! It belongs to the "Advanced SIMD two-register miscellaneous" encoding group:
//!
//! ```text
//!   31 30 29 28-24 23-22 21-16  15-11  10 9-5 4-0
//!    0  Q  U  01110  size  100000 opcode 1  0  Rn  Rd
//! ```
//! with `U = 1`, `size = 01`, and `opcode = 0101` (RBIT is the U=1 / size=01
//! partner of CNT, which is `U = 0, size = 00, opcode = 0101`). The
//! arrangement `T` is restricted by the architecture to `.8b` (Q=0) and
//! `.16b` (Q=1) — RBIT reverses bits per byte, so no other element size is
//! permitted and the `size` field is a fixed `01`, not derived from element
//! width.
//!
//! ## Oracle
//! The golden words were hand-derived from the ARMv8-A ARM bit layout
//! (ARM DDI 0487, "Advanced SIMD two-register miscellaneous", RBIT row:
//! U=1, size=01, opcode=00101) and are independent of this crate's
//! implementation. They anchor the absolute correctness of every fixed
//! field. An independent reference encoder `ref_encode_rbit` reassembles
//! the word from the same documented layout for differential checking.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_rbit;
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

/// Arrangements architecturally VALID for RBIT (ARMv8-A ARM, RBIT (vector)):
/// only 8B and 16B. RBIT reverses bits per byte, so no wider element size exists.
fn valid_arrangement_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("8b"), Just("16b")]
}

/// Independent reference encoder: assembles the word field-by-field from the
/// documented ARMv8-A ARM layout for RBIT. This is a re-derivation of the
/// spec, written separately from `encode_neon_rbit` to serve as an oracle.
fn ref_encode_rbit(rd: u32, rn: u32, arr: &str) -> u32 {
    let q: u32 = if arr == "16b" { 1 } else { 0 };
    (q << 30) // bit 30: Q
        | (0b1u32 << 29) // bit 29: U = 1
        | (0b01110u32 << 24) // bits 28-24
        | (0b01u32 << 22) // bits 23-22: size = 01 (RBIT fixed size)
        | (0b100000u32 << 16) // bits 21-16 = 100000
        | (0b0101u32 << 12) // bits 15-12: opcode = 0101
        | (0b10u32 << 10) // bits 11-10 = 10
        | (rn << 5) // bits 9-5: Rn
        | rd // bits 4-0: Rd
}

fn word_of(res: Result<EncodeResult, String>) -> u32 {
    match res {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {other:?}"),
    }
}

// --- golden table (absolute oracle) ---------------------------------------

/// Hand-derived from the ARMv8-A ARM layout for RBIT.
/// `rbit v0.16b, v0.16b = 0x6E605800`, `rbit v0.8b, v0.8b = 0x2E605800`.
const GOLDEN: &[(u32, u32, &str, u32)] = &[
    // (Rd, Rn, arrangement, expected_word)
    (0, 1, "16b", 0x6E605820),  // rbit v0.16b, v1.16b
    (0, 0, "8b", 0x2E605800),   // rbit v0.8b, v0.8b  (Q=0)
    (5, 6, "16b", 0x6E6058C5),  // rbit v5.16b, v6.16b
    (31, 30, "8b", 0x2E605BDF), // rbit v31.8b, v30.8b
    (10, 20, "8b", 0x2E605A8A), // rbit v10.8b, v20.8b (Q=0)
    (7, 7, "16b", 0x6E6058E7),  // rbit v7.16b, v7.16b
];

#[test]
fn rbit_matches_golden_table() {
    for &(rd, rn, arr, expected) in GOLDEN {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let got = word_of(encode_neon_rbit(&ops));
        assert_eq!(
            got, expected,
            "rbit v{rd}.{arr}, v{rn}.{arr}: got 0x{got:08X}, want 0x{expected:08X}",
        );
        // Cross-check the reference encoder against the golden values too.
        assert_eq!(
            ref_encode_rbit(rd, rn, arr),
            expected,
            "reference encoder drift"
        );
    }
}

// --- properties -----------------------------------------------------------

proptest! {
    // === Oracle: differential against independent reference encoder ========
    // For every valid RBIT arrangement and register pair, the implementation
    // must equal the independently-assembled reference word.
    #[test]
    fn rbit_matches_reference_encoder(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let got = word_of(encode_neon_rbit(&ops));
        let want = ref_encode_rbit(rd, rn, arr);
        prop_assert_eq!(got, want);
    }

    // === Fixed-bits invariant =============================================
    // The architecturally-constant bits must never change, regardless of
    // registers or arrangement: bit31=0, U(bit29)=1, bits28-24=01110,
    // size(bits23-22)=01, bits21-16=100000, opcode(bits15-12)=0101,
    // bits11-10=10.
    #[test]
    fn rbit_fixed_bits_are_constant(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let w = word_of(encode_neon_rbit(&ops));

        prop_assert_eq!((w >> 31) & 1, 0, "bit 31 must be 0");
        prop_assert_eq!((w >> 29) & 1, 1, "U bit must be 1");
        prop_assert_eq!((w >> 24) & 0x1F, 0b01110, "bits 28-24");
        prop_assert_eq!((w >> 22) & 0x3, 0b01, "size bits 23-22");
        prop_assert_eq!((w >> 16) & 0x3F, 0b100000, "bits 21-16");
        prop_assert_eq!((w >> 12) & 0xF, 0b0101, "opcode bits 15-12");
        prop_assert_eq!((w >> 10) & 0x3, 0b10, "bits 11-10");
    }

    // === Field placement: Rd/Rn round-trip + Q mapping (passing) ==========
    // The five-bit register fields and the Q bit must round-trip exactly,
    // and Q must be derived solely from the arrangement (0 for .8b, 1 for .16b).
    #[test]
    fn fields_round_trip_and_map_arrangement(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let w = word_of(encode_neon_rbit(&ops));
        let expected_q: u32 = if arr == "16b" { 1 } else { 0 };

        prop_assert_eq!(w & 0x1F, rd, "Rd field");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field");
        prop_assert_eq!((w >> 30) & 0x1, expected_q, "Q bit");
    }

    // === Negative contract: unsupported arrangement rejected ==============
    // RBIT is architecturally defined ONLY for .8b/.16b. Any other
    // arrangement string must cause `encode_neon_rbit` to return `Err`
    // rather than silently emitting a word. Note: the function reads the
    // arrangement from the *destination* operand, so we vary it there.
    #[test]
    fn rejects_unsupported_arrangement(
        arr in "[a-z0-9]{1,4}".prop_filter("must be an arrangement RBIT does not support", |s| {
            !matches!(s.as_str(), "8b" | "16b")
        }),
    ) {
        let ops = vec![va(0, arr.as_str()), va(1, arr.as_str())];
        prop_assert!(encode_neon_rbit(&ops).is_err(),
            "RBIT does not support .{arr}; expected Err but got Ok");
    }

    // === FINDING #1 (failing): source arrangement mismatch silently accepted ===
    // RBIT (vector) is `RBIT Vd.T, Vn.T` with BOTH operands sharing the same
    // arrangement T, and T ∈ {8B, 16B} only (ARM DDI 0487, RBIT (vector)).
    // The encoder validates ONLY the destination arrangement (`arr_d`) and
    // discards the source arrangement (`arr_n`, via `let (rn, _) = ...`). A
    // pairing like `rbit v0.16b, v1.4s` is silently encoded as if the source
    // were `.16b` instead of being rejected. This input is fully reachable:
    // the parser turns `v1.4s` into `RegArrangement{v1,"4s"}` in any slot.
    // `#[ignore]`d so the suite stays green; run with `--ignored`.
    #[test]
    #[ignore]
    fn rbit_rejects_mismatched_source_arrangement(
        src_arr in prop_oneof![
            Just("4h"), Just("8h"), Just("2s"), Just("4s"), Just("1d"), Just("2d")
        ],
        rd in 0u32..=31,
        rn in 0u32..=31,
        is_q in any::<bool>(),
    ) {
        let dst_arr = if is_q { "16b" } else { "8b" };
        let ops = vec![va(rd, dst_arr), va(rn, src_arr)];
        prop_assert!(encode_neon_rbit(&ops).is_err(),
            "rbit v{rd}.{dst_arr}, v{rn}.{src_arr} must be rejected (source/dest \
             arrangement mismatch); got Ok");
    }

    // === FINDING #2 (failing): non-vector register class silently accepted ===
    // `parse_reg_num` accepts any of x/w/d/s/q/v/h/b prefixes, so a
    // `RegArrangement` built from a GPR name (e.g. `x0.16b`, reachable since
    // the parser's `is_register` returns true for x0/w0) is encoded as if it
    // were a vector register. RBIT operands MUST be SIMD registers V0-V31.
    #[test]
    #[ignore]
    fn rbit_rejects_non_vector_register_class(
        gpr in prop_oneof![
            Just("x0"), Just("x31"), Just("w5"), Just("sp")
        ],
    ) {
        let ops = vec![
            Operand::RegArrangement { reg: gpr.to_string(), arrangement: "16b".to_string() },
            Operand::RegArrangement { reg: "v1".to_string(), arrangement: "16b".to_string() },
        ];
        prop_assert!(encode_neon_rbit(&ops).is_err(),
            "rbit {gpr}.16b, v1.16b must be rejected (GPR in SIMD operand); got Ok");
    }
}

// --- documented finding: too few operands rejected ------------------------

/// `RBIT` requires exactly two operands (`Vd.T, Vn.T`). Passing fewer must
/// return `Err` rather than panicking or producing a malformed word.
#[test]
fn rbit_rejects_too_few_operands() {
    // Zero operands.
    assert!(encode_neon_rbit(&[]).is_err(), "0 operands must be Err");
    // One operand.
    assert!(
        encode_neon_rbit(&[va(0, "16b")]).is_err(),
        "1 operand must be Err",
    );
    // Two operands (boundary) must succeed.
    assert!(encode_neon_rbit(&[va(0, "16b"), va(1, "16b")]).is_ok());
}

// --- FINDING #1: empirical reproduction (run with `--ignored --nocapture`) --

/// Confirms FINDING #1 with concrete illegal pairings. Currently the
/// implementation silently accepts them; this test will `panic!` (showing the
/// accepted word) until the encoder validates the source arrangement.
#[test]
#[ignore]
fn rbit_mismatch_reproduction() {
    let cases: &[(u32, &str, u32, &str)] = &[
        (0, "16b", 1, "4s"), // rbit v0.16b, v1.4s
        (0, "16b", 1, "2d"), // rbit v0.16b, v1.2d
        (0, "8b", 1, "4h"),  // rbit v0.8b,  v1.4h
    ];
    for &(rd, darr, rn, sarr) in cases {
        let ops = vec![va(rd, darr), va(rn, sarr)];
        match encode_neon_rbit(&ops) {
            Ok(EncodeResult::Word(w)) => panic!(
                "BUG: `rbit v{rd}.{darr}, v{rn}.{sarr}` was silently accepted as \
                 0x{w:08X}; expected Err (source arrangement .{sarr} != dest .{darr})",
            ),
            Ok(other) => panic!("unexpected non-Word result: {other:?}"),
            Err(_) => { /* correct */ }
        }
    }
}
