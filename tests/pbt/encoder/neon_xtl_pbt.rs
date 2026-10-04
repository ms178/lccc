//! Property-based tests for `encode_neon_xtl`
//! (the AArch64 "Advanced SIMD extract *long*" alias encoder: UXTL/SXTL,
//! which are USHLL/SSHLL with `shift == 0`).
//!
//! Encoding (ARMv8 ARM, "Advanced SIMD shift by amount", long group, shift #0):
//!   `0 Q U 0 1 1 1 1 0 immh 000 1 0 1 0 0 1 Rn Rd`
//!    31 30 29 28-23 22-19 18-16 15-10 9-5 4-0
//!   where opcode = 101001 (fixed for this group),
//!         immb    = 000   (forced because UXTL/SXTL imply shift #0),
//!         Q       = `is_high` (the "...2" / high-half variants),
//!         U       = `u_bit`   (0 = signed SXTL/SSHLL, 1 = unsigned UXTL/USHLL).
//!
//! immh selects the SOURCE (narrow) element width:
//!   .8b/.16b -> immh = 0001   (8-bit source)
//!   .4h/.8h  -> immh = 0010   (16-bit source)
//!   .2s/.4s  -> immh = 0100   (32-bit source)
//! immh = 0000 / 0011 / 01x1 / 1xxx are UNALLOCATED for this alias.
//!
//! Range validation status:
//!   * Register numbers v0..=v31 ARE validated (via `parse_reg_num`, which
//!     returns `None` for v32+); out-of-range registers are rejected.
//!   * Source arrangements ARE whitelisted; unsupported ones are rejected.
//!
//! FINDING (witness test, `#[ignore]`d): unlike the register/arrangement
//! validation, the encoder performs **no cross-check** that `is_high` is
//! consistent with the source arrangement's half. A real assembler rejects
//! `uxtl v0.8h, v1.16b` (low/`uxtl` variant with a wide-half `.16b` source)
//! and `uxtl2 v0.8h, v1.8b` (high/`uxtl2` variant with a narrow-half `.8b`
//! source); this encoder silently encodes them. See
//! `witness_high_low_consistency_unchecked`.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_xtl;
use proptest::prelude::*;

// ── helpers ──────────────────────────────────────────────────────────────

/// Build a `RegArrangement` operand like `v3.8b`.
fn arr_operand(reg_num: u32, arr: &str) -> Operand {
    Operand::RegArrangement {
        reg: format!("v{}", reg_num),
        arrangement: arr.to_string(),
    }
}

/// Canonical immh value the ARM ARM assigns to each supported source width.
/// `None` for unsupported arrangements.
fn canonical_immh(arr: &str) -> Option<u32> {
    match arr {
        "8b" | "16b" => Some(0b0001),
        "4h" | "8h" => Some(0b0010),
        "2s" | "4s" => Some(0b0100),
        _ => None,
    }
}

/// The six supported source arrangements.
fn src_arr_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![
        Just("8b"),
        Just("16b"),
        Just("4h"),
        Just("8h"),
        Just("2s"),
        Just("4s"),
    ]
}

/// U bit is semantically a single bit (signed vs unsigned); only 0/1 are legal.
fn u_bit_strategy() -> impl Strategy<Value = u32> {
    prop_oneof![Just(0u32), Just(1u32),]
}

/// Encode a well-formed UXTL/SXTL operand pair and unwrap the resulting word.
fn encode_word(rd: u32, rn: u32, arr: &str, u_bit: u32, is_high: bool) -> u32 {
    let ops = vec![arr_operand(rd, arr), arr_operand(rn, arr)];
    match encode_neon_xtl(&ops, u_bit, is_high).expect("valid input must encode") {
        EncodeResult::Word(w) => w,
        other => panic!("expected EncodeResult::Word, got {:?}", other),
    }
}

// ── properties ───────────────────────────────────────────────────────────

proptest! {
    /// 1. Full structural match: the encoded word equals an independently
    ///    reassembled ARM ARM word AND every fixed/reserved field has the
    ///    spec-mandated value (so a wrong constant would still be caught even
    ///    if the OR expression were accidentally copied).
    #[test]
    fn prop_xtl_encoding_matches_reference(
        rd in 0u32..=31,
        rn in 0u32..=31,
        arr in src_arr_strategy(),
        u_bit in u_bit_strategy(),
        is_high in any::<bool>(),
    ) {
        let word = encode_word(rd, rn, arr, u_bit, is_high);
        let q = u32::from(is_high);
        let immh = canonical_immh(arr).unwrap();

        // Independent reassembly:
        //  31=0 | 30=Q | 29=U | 28-23=011110 | 22-19=immh | 18-16=000(immb)
        //  | 15-10=101001 | 9-5=Rn | 4-0=Rd
        let expected: u32 = (q << 30)
            | (u_bit << 29)
            | (0b011110u32 << 23)
            | (immh << 19)
            | (0b101001u32 << 10)
            | (rn << 5)
            | rd;

        prop_assert_eq!(word, expected);

        // Field-by-field sanity (independent of the expression above):
        prop_assert_eq!((word >> 31) & 1, 0u32, "bit 31 must be 0");
        prop_assert_eq!((word >> 30) & 1, q, "Q == is_high");
        prop_assert_eq!((word >> 29) & 1, u_bit, "U == u_bit");
        prop_assert_eq!((word >> 23) & 0x3F, 0b011110u32, "bits 28-23 fixed");
        prop_assert_eq!((word >> 19) & 0xF, immh, "immh field");
        prop_assert_eq!((word >> 16) & 0x7, 0u32, "immb must be 0 (shift #0)");
        prop_assert_eq!((word >> 10) & 0x3F, 0b101001u32, "opcode bits 15-10 fixed");
        prop_assert_eq!((word >> 5) & 0x1F, rn, "Rn field");
        prop_assert_eq!(word & 0x1F, rd, "Rd field");
    }

    /// 2. Field isolation: each independent input touches only its own bits.
    #[test]
    fn prop_field_isolation(
        rd in 0u32..=31,
        rn in 0u32..=31,
        arr in src_arr_strategy(),
        u_bit in u_bit_strategy(),
        is_high in any::<bool>(),
    ) {
        let base = encode_word(rd, rn, arr, u_bit, is_high);

        // Toggling is_high must flip ONLY bit 30.
        let flip_q = encode_word(rd, rn, arr, u_bit, !is_high);
        prop_assert_eq!(base ^ flip_q, 1u32 << 30, "is_high must affect only bit 30");

        // Toggling u_bit must flip ONLY bit 29.
        let flip_u = encode_word(rd, rn, arr, 1 - u_bit, is_high);
        prop_assert_eq!(base ^ flip_u, 1u32 << 29, "u_bit must affect only bit 29");

        // Changing Rd must affect ONLY bits 4-0, by exactly the Rd delta.
        let rd2 = (rd + 1) % 32;
        let chg_rd = encode_word(rd2, rn, arr, u_bit, is_high);
        let diff_rd = base ^ chg_rd;
        prop_assert_eq!(diff_rd & !0x1Fu32, 0u32, "Rd change must not touch bits >=5");
        prop_assert_eq!(diff_rd & 0x1F, rd ^ rd2);

        // Changing Rn must affect ONLY bits 9-5, by exactly the Rn delta.
        let rn2 = (rn + 1) % 32;
        let chg_rn = encode_word(rd, rn2, arr, u_bit, is_high);
        let diff_rn = base ^ chg_rn;
        prop_assert_eq!(diff_rn >> 10, 0u32, "Rn change must not touch bits >=10");
        prop_assert_eq!((diff_rn >> 5) & 0x1F, rn ^ rn2);
    }

    /// 3. Source-width determinism: each arrangement maps to its canonical
    ///    immh, and two arrangements of *different* element widths must yield
    ///    different words (immh differs -> word differs).
    #[test]
    fn prop_width_determines_immh_and_distinct_word(
        rd in 0u32..=31,
        rn in 0u32..=31,
        arr1 in src_arr_strategy(),
        arr2 in src_arr_strategy(),
        u_bit in u_bit_strategy(),
        is_high in any::<bool>(),
    ) {
        let immh1 = canonical_immh(arr1).unwrap();
        let immh2 = canonical_immh(arr2).unwrap();
        let w1 = encode_word(rd, rn, arr1, u_bit, is_high);
        let w2 = encode_word(rd, rn, arr2, u_bit, is_high);

        prop_assert_eq!((w1 >> 19) & 0xF, immh1);
        prop_assert_eq!((w2 >> 19) & 0xF, immh2);

        if immh1 != immh2 {
            prop_assert_ne!(w1, w2, "distinct source widths must produce distinct words");
        }
    }

    /// 4. Determinism: identical inputs produce identical words.
    #[test]
    fn prop_deterministic(
        rd in 0u32..=31,
        rn in 0u32..=31,
        arr in src_arr_strategy(),
        u_bit in u_bit_strategy(),
        is_high in any::<bool>(),
    ) {
        let a = encode_word(rd, rn, arr, u_bit, is_high);
        let b = encode_word(rd, rn, arr, u_bit, is_high);
        prop_assert_eq!(a, b);
    }

    /// 5. Negative contract: unsupported source arrangements are rejected.
    #[test]
    fn prop_rejects_unsupported_arrangement(
        bad_arr in prop::sample::select(vec![
            "2d", "1d", "1q", "16h", "2h", "16s", "8s", "1b", "9b", "", "foo", "4b",
        ]),
        u_bit in u_bit_strategy(),
        is_high in any::<bool>(),
    ) {
        let ops = vec![arr_operand(0, "8b"), arr_operand(1, bad_arr)];
        let res = encode_neon_xtl(&ops, u_bit, is_high);
        prop_assert!(
            res.is_err(),
            "unsupported source arrangement {:?} must be rejected, got {:?}",
            bad_arr,
            res
        );
    }
}

// ── plain negative-contract tests ────────────────────────────────────────

#[test]
fn reject_too_few_operands() {
    assert!(
        encode_neon_xtl(&[], 0, false).is_err(),
        "0 operands must error"
    );
    assert!(
        encode_neon_xtl(&[arr_operand(0, "8b")], 0, false).is_err(),
        "1 operand must error"
    );
}

#[test]
fn reject_out_of_range_register() {
    // v0..=v31 are valid; v32+ must be rejected (parse_reg_num returns None,
    // so the field must NOT be silently masked into 5 bits).
    let bad_rd = vec![arr_operand(32, "8h"), arr_operand(1, "8b")];
    assert!(
        encode_neon_xtl(&bad_rd, 0, false).is_err(),
        "out-of-range Rd (v32) must be rejected, not masked"
    );
    let bad_rn = vec![arr_operand(0, "8h"), arr_operand(40, "8b")];
    assert!(
        encode_neon_xtl(&bad_rn, 0, false).is_err(),
        "out-of-range Rn (v40) must be rejected, not masked"
    );
}

// ── bug witness (kept #[ignore]'d so default `cargo test` stays green) ────

/// FINDING: `encode_neon_xtl` does not validate that `is_high` matches the
/// source arrangement's half. A real assembler (GNU `as`, LLVM `llvm-mc`)
/// rejects both of these because the source half must agree with the
/// uxtl/uxtl2 (low/high) variant:
///   * `uxtl  v0.8h, v1.16b`  — is_high=false but a WIDE-half `.16b` source
///   * `uxtl2 v0.8h, v1.8b`   — is_high=true  but a NARROW-half `.8b` source
///     This encoder silently accepts them and emits a (valid but textually wrong)
///     USHLL/SSHLL word. When the missing cross-check is added, these assertions
///     pass and the `#[ignore]` can be removed.
#[test]
#[ignore = "BUG: is_high not cross-validated against source arrangement half"]
fn witness_high_low_consistency_unchecked() {
    // uxtl (low variant, Q=0) with a wide-half .16b source: should be Err.
    let ops_low_wide = vec![arr_operand(0, "8h"), arr_operand(1, "16b")];
    let res_low = encode_neon_xtl(&ops_low_wide, 0, false);
    assert!(
        res_low.is_err(),
        "uxtl (is_high=false) with a .16b wide-half source must be rejected, got {:?}",
        res_low
    );

    // uxtl2 (high variant, Q=1) with a narrow-half .8b source: should be Err.
    let ops_high_narrow = vec![arr_operand(0, "8h"), arr_operand(1, "8b")];
    let res_high = encode_neon_xtl(&ops_high_narrow, 0, true);
    assert!(
        res_high.is_err(),
        "uxtl2 (is_high=true) with a .8b narrow-half source must be rejected, got {:?}",
        res_high
    );
}
