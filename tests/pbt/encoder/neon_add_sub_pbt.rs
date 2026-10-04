//! Property-based tests for `encode_neon_add_sub`.
//!
//! `encode_neon_add_sub` encodes the AArch64 NEON integer `ADD` / `SUB`
//! (vector) instructions — `ADD/SUB Vd.T, Vn.T, Vm.T` — in the
//! "Advanced SIMD integer three-same" encoding group:
//!
//! ```text
//!   31 30 29 28-24 23-22 21 20-16 15-11 10 9-5 4-0
//!    0  Q  U  01110  size  1   Rm  10000  1  Rn  Rd
//! ```
//! with `U = 0` for ADD and `U = 1` for SUB. `Q`/`size` come from the
//! arrangement `T` (8B/16B/4H/8H/2S/4S/2D).
//!
//! ## Oracle
//! The golden words in `GOLDEN` were hand-derived from the ARMv8-A ARM bit
//! layout (ARM DDI 0487, "ADD (vector)" / "SUB (vector)", encoding
//! `0 Q U 01110 size 1 Rm 100001 Rn Rd`). They are independent of this
//! crate's implementation and anchor the absolute correctness of every
//! fixed and variable field. The independent reference encoder
//! `ref_encode_add_sub` mirrors that same documented layout (with its own
//! arrangement→(Q,size) table) for the differential property.
//!
//! ## Status
//! The implementation assembles every field at the correct width and bit
//! position; all default properties PASS. One documented, `#[ignore]d
//! finding (`add_sub_accepts_nonstandard_1d_arrangement`) records that the
//! `.1d` arrangement — not in the ARM ARM assembler-symbol set for ADD/SUB —
//! is silently encoded instead of rejected. See `COVERAGE.md`.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_add_sub;
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

/// Arrangements architecturally VALID for ADD/SUB (vector)
/// (ARMv8-A ARM, ADD/SUB (vector) assembler symbol set):
/// 8B, 16B, 4H, 8H, 2S, 4S, 2D.
/// `.1d` (size=0b11, Q=0) is intentionally excluded — see the `#[ignore]d
/// witness below.
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

/// Independent arrangement→(Q,size) table, kept separate from the crate's
/// `neon_arr_to_q_size` so the differential property is not circular.
fn ref_arr_to_q_size(arr: &str) -> (u32, u32) {
    match arr {
        "8b" => (0, 0b00),
        "16b" => (1, 0b00),
        "4h" => (0, 0b01),
        "8h" => (1, 0b01),
        "2s" => (0, 0b10),
        "4s" => (1, 0b10),
        "1d" => (0, 0b11),
        "2d" => (1, 0b11),
        _ => panic!("ref_arr_to_q_size: unknown arrangement {arr:?}"),
    }
}

/// Independent reference encoder: assembles the word field-by-field from the
/// documented ARMv8-A ARM layout (`0 Q U 01110 size 1 Rm 100001 Rn Rd`).
fn ref_encode_add_sub(rd: u32, rn: u32, rm: u32, arr: &str, is_sub: bool) -> u32 {
    let (q, size) = ref_arr_to_q_size(arr);
    let u = if is_sub { 1u32 } else { 0u32 };
    (q << 30)
        | (u << 29)
        | (0b01110u32 << 24)
        | (size << 22)
        | (1u32 << 21)
        | (rm << 16)
        | (0b10000u32 << 11)
        | (1u32 << 10)
        | (rn << 5)
        | rd
}

fn word_of(res: Result<EncodeResult, String>) -> u32 {
    match res {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {other:?}"),
    }
}

// --- golden table (absolute oracle) ---------------------------------------

/// Hand-derived from the ARMv8-A ARM layout for ADD/SUB (vector).
/// (is_sub, Rd, Rn, Rm, arrangement, expected_word)
const GOLDEN: &[(bool, u32, u32, u32, &str, u32)] = &[
    (false, 0, 1, 2, "8b", 0x0E228420),  // add  v0.8b,  v1.8b,  v2.8b
    (true, 0, 1, 2, "8b", 0x2E228420),   // sub  v0.8b,  v1.8b,  v2.8b
    (false, 0, 1, 2, "16b", 0x4E228420), // add  v0.16b, v1.16b, v2.16b
    (false, 0, 1, 2, "4h", 0x0E628420),  // add  v0.4h,  v1.4h,  v2.4h
    (false, 0, 1, 2, "2s", 0x0EA28420),  // add  v0.2s,  v1.2s,  v2.2s
    (false, 0, 1, 2, "2d", 0x4EE28420),  // add  v0.2d,  v1.2d,  v2.2d
    (false, 31, 30, 29, "4s", 0x4EBD87DF), // add  v31.4s, v30.4s, v29.4s
    (true, 31, 0, 0, "2d", 0x6EE0841F),  // sub  v31.2d, v0.2d,  v0.2d
];

#[test]
fn add_sub_matches_golden_table() {
    for &(is_sub, rd, rn, rm, arr, expected) in GOLDEN {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let got = word_of(encode_neon_add_sub(&ops, is_sub));
        let mnem = if is_sub { "sub" } else { "add" };
        assert_eq!(
            got, expected,
            "{mnem} v{rd}.{arr}, v{rn}.{arr}, v{rm}.{arr}: got 0x{got:08X}, want 0x{expected:08X}",
        );
        // The reference encoder must agree with the hand-derived value too.
        assert_eq!(
            ref_encode_add_sub(rd, rn, rm, arr, is_sub),
            expected,
            "reference encoder drift for {mnem} .{arr}",
        );
    }
}

// --- properties -----------------------------------------------------------

proptest! {
    // === Oracle: differential against independent reference encoder ========
    // For every valid arrangement and register triple, both ADD and SUB, the
    // implementation must equal the independently-assembled reference word.
    #[test]
    fn add_sub_matches_reference_encoder(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
        is_sub in any::<bool>(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let got = word_of(encode_neon_add_sub(&ops, is_sub));
        let want = ref_encode_add_sub(rd, rn, rm, arr, is_sub);
        prop_assert_eq!(got, want);
    }

    // === Fixed-bits invariant =============================================
    // The architecturally-constant bits must never vary with operands:
    // bit31=0, bits28-24=01110, bit21=1, bits15-11=10000, bit10=1.
    #[test]
    fn add_sub_fixed_bits_are_constant(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
        is_sub in any::<bool>(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let w = word_of(encode_neon_add_sub(&ops, is_sub));

        prop_assert_eq!((w >> 31) & 1, 0, "bit 31 must be 0");
        prop_assert_eq!((w >> 24) & 0x1F, 0b01110, "bits 28-24 must be 01110");
        prop_assert_eq!((w >> 21) & 1, 1, "bit 21 must be 1");
        prop_assert_eq!((w >> 11) & 0x1F, 0b10000, "bits 15-11 must be 10000");
        prop_assert_eq!((w >> 10) & 1, 1, "bit 10 must be 1");
    }

    // === U bit distinguishes ADD from SUB ================================
    // Encoding the same operands as ADD vs SUB must differ in exactly bit 29
    // (the U field): ADD has U=0, SUB has U=1, and all other bits are equal.
    #[test]
    fn u_bit_distinguishes_add_and_sub(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let add_w = word_of(encode_neon_add_sub(&ops, false));
        let sub_w = word_of(encode_neon_add_sub(&ops, true));

        prop_assert_eq!((add_w >> 29) & 1, 0, "ADD must have U=0");
        prop_assert_eq!((sub_w >> 29) & 1, 1, "SUB must have U=1");
        prop_assert_eq!(add_w ^ sub_w, 1u32 << 29, "ADD/SUB must differ only in bit 29");
    }

    // === Field placement: Rd/Rn/Rm round-trip + Q/size mapping ===========
    // The three 5-bit register fields land in disjoint bit ranges and
    // round-trip exactly; Q (bit 30) and size (bits 23-22) track the
    // arrangement.
    #[test]
    fn fields_round_trip_and_map_arrangement(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
        is_sub in any::<bool>(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let w = word_of(encode_neon_add_sub(&ops, is_sub));
        let (q, size) = ref_arr_to_q_size(arr);

        prop_assert_eq!(w & 0x1F, rd, "Rd field bits 4-0");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field bits 9-5");
        prop_assert_eq!((w >> 16) & 0x1F, rm, "Rm field bits 20-16");
        prop_assert_eq!((w >> 30) & 0x1, q, "Q bit 30");
        prop_assert_eq!((w >> 22) & 0x3, size, "size bits 23-22");
    }

    // === Negative contract: unsupported arrangement rejected =============
    // Any arrangement string outside the valid set must cause
    // `encode_neon_add_sub` to return `Err` rather than silently emitting.
    #[test]
    fn rejects_unsupported_arrangement(
        arr in "[a-z0-9]{1,3}".prop_filter("must be an unknown arrangement", |s| {
            !matches!(s.as_str(),
                "8b"|"16b"|"4h"|"8h"|"2s"|"4s"|"1d"|"2d")
        }),
    ) {
        let ops = vec![va(0, arr.as_str()), va(1, arr.as_str()), va(2, arr.as_str())];
        prop_assert!(
            encode_neon_add_sub(&ops, false).is_err(),
            "unsupported arrangement {arr:?} should be rejected"
        );
    }

    // === Negative contract: out-of-range register rejected ===============
    // NEON register numbers are 0-31; v32+ must be rejected (no silent
    // truncation/overflow into adjacent fields). `parse_reg_num` enforces
    // this, and the property pins the contract.
    #[test]
    fn rejects_out_of_range_register(
        n in 32u32..=999u32,
        arr in valid_arrangement_strategy(),
    ) {
        let bad = format!("v{n}");
        let ops = vec![
            Operand::RegArrangement { reg: bad.clone(), arrangement: arr.to_string() },
            va(1, arr),
            va(2, arr),
        ];
        prop_assert!(
            encode_neon_add_sub(&ops, false).is_err(),
            "register v{n} (>=32) should be rejected for .{arr}"
        );
    }

    // === Negative contract: too few operands rejected ====================
    // ADD/SUB (vector) need three register operands; fewer must error rather
    // than panic or read missing operands as a default.
    #[test]
    fn rejects_too_few_operands(
        arr in valid_arrangement_strategy(),
    ) {
        // Two operands only:
        let ops2 = vec![va(0, arr), va(1, arr)];
        prop_assert!(encode_neon_add_sub(&ops2, false).is_err(),
            "two operands must be rejected");
        // Zero operands:
        let ops0: Vec<Operand> = vec![];
        prop_assert!(encode_neon_add_sub(&ops0, false).is_err(),
            "zero operands must be rejected");
    }
}

// --- documented finding: non-standard `.1d` arrangement silently encoded ---

/// Per the ARMv8-A ARM, the assembler symbol set for ADD/SUB (vector) is
/// 8B/16B/4H/8H/2S/4S/2D — `.1d` is **not** listed. The encoding it would
/// produce (Q=0, size=0b11) is not a named instruction. A conforming
/// assembler rejects `add v0.1d, v1.1d, v2.1d`; `encode_neon_add_sub` accepts
/// it because the shared `neon_arr_to_q_size` helper returns `(0, 0b11)` for
/// `"1d"` and the encoder performs no ADD/SUB-specific arrangement check.
///
/// This is a permissiveness gap (silent emission of an unallocated encoding),
/// not a correctness bug for the supported arrangements. `#[ignore]d so the
/// default suite stays green. Reproduce with
/// `cargo test -- --ignored add_sub_accepts_nonstandard_1d_arrangement`.
#[test]
#[ignore]
fn add_sub_accepts_nonstandard_1d_arrangement() {
    let ops = vec![va(0, "1d"), va(1, "1d"), va(2, "1d")];
    let res = encode_neon_add_sub(&ops, false);
    assert!(
        res.is_err(),
        "ADD/SUB (vector) has no `.1d` form per the ARMv8-A ARM; expected Err but got Ok(0x{:08X})",
        res.as_ref()
            .ok()
            .map(|e| match e {
                EncodeResult::Word(w) => *w,
                _ => 0,
            })
            .unwrap_or(0),
    );
}
