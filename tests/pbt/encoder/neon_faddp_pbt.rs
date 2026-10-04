//! Property-based tests for `encode_neon_faddp`.
//!
//! `encode_neon_faddp` encodes the AArch64 NEON `FADDP` (floating-point
//! pairwise add) instruction in two forms:
//!
//! ### Vector form — `FADDP Vd.T, Vn.T, Vm.T`
//! "Advanced SIMD three same" encoding group (ARMv8-A ARM, U=1, opcode=11010):
//! ```text
//!   31 30 29 28-24 23 22  21 20-16 15-10  9-5  4-0
//!    0  Q  1  01110  0 sz   1   Rm  110101  Rn   Rd
//! ```
//! `Q` and `sz` are derived from `T`: `.2s`→(Q=0,sz=0), `.4s`→(Q=1,sz=0),
//! `.2d`→(Q=1,sz=1). The byte/halfword/integer arrangements are
//! architecturally unallocated for `FADDP` and must be rejected.
//!
//! ### Scalar form — `FADDP Sd, Vn.2S` / `FADDP Dd, Vn.2D`
//! "Advanced SIMD scalar pairwise" encoding group:
//! ```text
//!   31-30 29 28 27-23  22  21-17  16-12 11-10 9-5 4-0
//!     01   1  1  11110  sz  11000  01101   10   Rn  Rd
//! ```
//!
//! ## Oracle
//! The vector golden words below were hand-derived from the ARMv8-A ARM
//! three-same bit layout (independent of this crate) and anchor the absolute
//! correctness of every fixed field. A field-by-field reference encoder (which
//! re-derives `Q`/`sz` from the arrangement rather than calling
//! `neon_arr_to_q_size`) cross-checks the implementation differentially.
//!
//! ## Finding (documented by the `#[ignore]`d test `scalar_missing_bit24`)
//! The scalar form emits an **incorrect high constant**: it places `11110` at
//! bits 28-24 plus an explicit `0` at bit 23 (`0x7E` top byte), but the
//! "Advanced SIMD scalar pairwise" template `0 1 U 1 11110 sz …` requires a `1`
//! at bit 28 *and* `11110` spanning bits 27-23, i.e. **bit 24 must be 1**
//! (`0x7F` top byte). The implementation is therefore off by `0x01000000`.
//! See `FADDP_SCALAR_BIT24_BUG_REPORT.md`.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_faddp;
use proptest::prelude::*;

// --- helpers --------------------------------------------------------------

/// `Operand::RegArrangement { reg: "v{n}", arrangement }`.
fn va(n: u32, arr: &str) -> Operand {
    Operand::RegArrangement {
        reg: format!("v{n}"),
        arrangement: arr.to_string(),
    }
}

/// `Operand::Reg("s{n}")` or `"d{n}"` for the scalar destination.
fn reg(name: &str) -> Operand {
    Operand::Reg(name.to_string())
}

fn reg_num_strategy() -> impl Strategy<Value = u32> {
    0u32..=31u32
}

/// Architecturally valid vector arrangements for FADDP (single/double float).
fn vec_arrangement_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("2s"), Just("4s"), Just("2d")]
}

/// Valid scalar source arrangements.
fn scalar_arrangement_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("2s"), Just("2d")]
}

/// Independent reference encoder for the VECTOR form. It re-derives Q/sz from
/// the arrangement directly (not via `neon_arr_to_q_size`), so a bug shared
/// with that helper would still be caught by the absolute golden table.
fn ref_encode_faddp_vec(rd: u32, rn: u32, rm: u32, arr: &str) -> u32 {
    let (q, sz) = match arr {
        "2s" => (0u32, 0u32),
        "4s" => (1, 0),
        "2d" => (1, 1),
        _ => unreachable!("invalid arrangement in reference encoder"),
    };
    (q << 30)
        | (1 << 29)
        | (0b01110 << 24)
        | (sz << 22)
        | (1 << 21)
        | (rm << 16)
        | (0b110101 << 10)
        | (rn << 5)
        | rd
}

fn word_of(res: Result<EncodeResult, String>) -> u32 {
    match res {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {other:?}"),
    }
}

// --- golden table (absolute oracle) — vector form -------------------------

/// Hand-derived from the ARMv8-A ARM three-same layout for FADDP (vector),
/// U=1, opcode=11010.
const GOLDEN_VEC: &[(u32, u32, u32, &str, u32)] = &[
    // (Rd, Rn, Rm, arrangement, expected_word)
    (0, 1, 2, "4s", 0x6E22D420),    // faddp v0.4s, v1.4s, v2.4s
    (0, 1, 2, "2s", 0x2E22D420),    // faddp v0.2s, v1.2s, v2.2s  (Q=0)
    (0, 1, 2, "2d", 0x6E62D420),    // faddp v0.2d, v1.2d, v2.2d  (sz=1)
    (5, 6, 7, "4s", 0x6E27D4C5),    // faddp v5.4s, v6.4s, v7.4s
    (31, 30, 29, "2d", 0x6E7DD7DF), // faddp v31.2d, v30.2d, v29.2d
    (10, 20, 30, "2s", 0x2E3ED68A), // faddp v10.2s, v20.2s, v30.2s (Q=0)
];

#[test]
fn faddp_vector_matches_golden_table() {
    for &(rd, rn, rm, arr, expected) in GOLDEN_VEC {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let got = word_of(encode_neon_faddp(&ops));
        assert_eq!(
            got, expected,
            "faddp v{rd}.{arr}, v{rn}.{arr}, v{rm}.{arr}: got 0x{got:08X}, want 0x{expected:08X}",
        );
        // Cross-check the reference encoder against the golden values too.
        assert_eq!(
            ref_encode_faddp_vec(rd, rn, rm, arr),
            expected,
            "reference encoder drift"
        );
    }
}

// --- properties -----------------------------------------------------------

proptest! {
    // === Oracle: reference encoder (differential) — vector form ===========
    // For every valid arrangement and register triple, the implementation
    // must equal the independently-assembled reference word.
    #[test]
    fn vector_matches_reference_encoder(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in vec_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let got = word_of(encode_neon_faddp(&ops));
        let want = ref_encode_faddp_vec(rd, rn, rm, arr);
        prop_assert_eq!(got, want);
    }

    // === Field placement + fixed bits — vector form =======================
    // Rd/Rn/Rm round-trip exactly; Q/sz map from the arrangement; and the
    // architecturally-constant bits never change for valid inputs.
    #[test]
    fn vector_fields_and_fixed_bits(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in vec_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let w = word_of(encode_neon_faddp(&ops));
        let (q, sz) = match arr { "2s"=>(0,0),"4s"=>(1,0),"2d"=>(1,1), _=>unreachable!() };

        prop_assert_eq!(w & 0x1F, rd, "Rd field");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field");
        prop_assert_eq!((w >> 16) & 0x1F, rm, "Rm field");
        prop_assert_eq!((w >> 30) & 0x1, q, "Q bit");
        prop_assert_eq!((w >> 22) & 0x1, sz, "sz bit (bit 22)");

        // Fixed fields: bit31=0, U(bit29)=1, bits28-24=01110, bit23=0,
        // bit21=1, opcode+1 (bits15-10)=110101.
        prop_assert_eq!((w >> 31) & 1, 0, "bit 31 must be 0");
        prop_assert_eq!((w >> 29) & 1, 1, "U bit must be 1 for FADDP");
        prop_assert_eq!((w >> 24) & 0x1F, 0b01110, "bits 28-24");
        prop_assert_eq!((w >> 23) & 1, 0, "bit 23 must be 0 (sz is 1-bit)");
        prop_assert_eq!((w >> 21) & 1, 1, "bit 21 must be 1");
        prop_assert_eq!((w >> 10) & 0x3F, 0b110101, "opcode bits 15-10");
    }

    // === Negative contract: vector rejects unallocated arrangements =======
    // FADDP (vector) is defined only for .2s/.4s/.2d. Any other arrangement
    // (incl. byte/halfword/integer and the scalar ".1d") must return Err.
    #[test]
    fn vector_rejects_unallocated_arrangement(
        arr in "[a-z0-9]{1,4}".prop_filter("must be an unsupported arrangement", |s| {
            !matches!(s.as_str(), "2s"|"4s"|"2d")
        }),
    ) {
        let ops = vec![va(0, arr.as_str()), va(1, arr.as_str()), va(2, arr.as_str())];
        prop_assert!(encode_neon_faddp(&ops).is_err(),
            "FADDP vector does not support .{arr:?}; expected Err");
    }

    // === Scalar form: sz mapping + register round-trip + code's layout ====
    // The scalar form places sz at bit 22 and Rn/Rd at their standard fields.
    // This characterizes the implementation's *current* (bit-24-buggy) layout;
    // see the `scalar_missing_bit24` ignored test for the spec deviation.
    #[test]
    fn scalar_fields_sz_and_reg_roundtrip(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in scalar_arrangement_strategy(),
    ) {
        let dest = if arr == "2d" { format!("d{rd}") } else { format!("s{rd}") };
        let ops = vec![reg(&dest), va(rn, arr)];
        let w = word_of(encode_neon_faddp(&ops));
        let sz = if arr == "2d" { 1u32 } else { 0u32 };

        prop_assert_eq!(w & 0x1F, rd, "Rd field");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field");
        prop_assert_eq!((w >> 22) & 0x1, sz, "sz bit");
        // Code's current fixed middle (bits 21-10) and top byte (bits 31-24).
        prop_assert_eq!((w >> 10) & 0xFFF, 0xC36, "scalar opcode/middle bits 21-10");
        prop_assert_eq!((w >> 24) & 0xFF, 0x7E, "scalar top byte (code's current value)");
    }

    // === Negative contract: scalar rejects unsupported source =============
    #[test]
    fn scalar_rejects_unsupported_arrangement(
        arr in "[a-z0-9]{1,4}".prop_filter("must be an unsupported arrangement", |s| {
            !matches!(s.as_str(), "2s"|"2d")
        }),
    ) {
        let ops = vec![reg("s0"), va(1, arr.as_str())];
        prop_assert!(encode_neon_faddp(&ops).is_err(),
            "FADDP scalar does not support source .{arr:?}; expected Err");
    }
}

// --- documented findings --------------------------------------------------

/// Too few operands (0 or 1) must be rejected by both dispatch branches.
#[test]
fn rejects_too_few_operands() {
    assert!(
        encode_neon_faddp(&[]).is_err(),
        "0 operands should be rejected"
    );
    assert!(
        encode_neon_faddp(&[va(0, "2s")]).is_err(),
        "1 operand should be rejected"
    );
    assert!(
        encode_neon_faddp(&[reg("s0")]).is_err(),
        "1 scalar operand should be rejected"
    );
}

/// Scalar form: non-`Operand::Reg` destination must be rejected.
#[test]
fn scalar_rejects_non_reg_dest() {
    // A RegArrangement destination is not a valid scalar FADDP destination.
    let ops = vec![va(0, "s"), va(1, "2s")];
    assert!(
        encode_neon_faddp(&ops).is_err(),
        "scalar FADDP expects a plain register destination (Sd/Dd)"
    );
}

/// **FINDING — scalar bit-24 encoding bug.**
///
/// The "Advanced SIMD scalar pairwise" template is `0 1 U 1 11110 sz …`:
/// a `1` at bit 28, then `11110` spanning bits 27-23 — so **bit 24 must be 1**,
/// giving a top byte of `0x7F`. The implementation instead emits `0x7E`
/// (bit 24 = 0): it placed `11110` at bits 28-24 and an extra `0` at bit 23.
/// Every scalar FADDP word is therefore `0x01000000` too small.
///
/// This test is `#[ignore]`d because the current implementation does NOT meet
/// the spec contract. Run with:
///   `cargo test -- --ignored scalar_missing_bit24`
/// See `FADDP_SCALAR_BIT24_BUG_REPORT.md`.
#[test]
#[ignore]
fn scalar_missing_bit24() {
    for &(rd, rn, arr) in &[
        (0u32, 1u32, "2s"),
        (0, 1, "2d"),
        (5, 6, "2s"),
        (31, 30, "2d"),
    ] {
        let dest = if arr == "2d" {
            format!("d{rd}")
        } else {
            format!("s{rd}")
        };
        let ops = vec![reg(&dest), va(rn, arr)];
        let res = encode_neon_faddp(&ops);
        let w = match res {
            Ok(EncodeResult::Word(w)) => w,
            other => panic!("expected Word, got {other:?}"),
        };
        assert_eq!(
            (w >> 24) & 0xFF,
            0x7F,
            "scalar FADDP {dest}, v{rn}.{arr}: top byte is 0x{:02X}, \
             spec requires 0x7F (bit 24 set); full word 0x{w:08X}",
            (w >> 24) & 0xFF,
        );
    }
}
