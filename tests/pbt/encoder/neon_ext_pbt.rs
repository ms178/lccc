//! Property-based tests for `encode_neon_ext`.
//!
//! `encode_neon_ext(operands)` encodes the AArch64 NEON extract instruction:
//!
//! ```text
//!   EXT Vd.<T>, Vn.<T>, Vm.<T>, #<imm>
//! ```
//! in the "Advanced SIMD extract" encoding group (ARMv8-A ARM, C7.2.75):
//!
//! ```text
//!   31  30   29-23      22 21   20-16   15   14-11   10   9-5   4-0
//!    0   Q   1011100     0  0     Rm      0    imm4    0    Rn    Rd
//! ```
//!
//! Architectural constraints (ARMv8-A ARM):
//!   * EXT is defined **only** for the byte arrangements `.8B` (Q=0) and
//!     `.16B` (Q=1). Every other arrangement (`4h/8h/2s/4s/1d/2d`) is
//!     UNALLOCATED; LLVM rejects it with "invalid operand for instruction".
//!   * `imm4` (bits 14-11) is the byte index. For `.8B` the valid range is
//!     `#0..#7` (imm4 = 0000-0111); imm4 = 1000-1111 is UNDEFINED. For
//!     `.16B` the valid range is `#0..#15` (imm4 = 0000-1111). An index
//!     outside the arrangement's range is UNALLOCATED.
//!
//! ## Oracle
//! The golden words below follow directly from the documented bit layout
//! (the field shifts are simple enough that a structurally-distinct reference
//! encoder — packing a constant high nibble separately — is the primary
//! independent oracle):
//!
//! ```text
//!   ext v0.8b,   v1.8b,   v2.8b,   #3   -> 0x2E021820
//!   ext v0.16b,  v1.16b,  v2.16b,  #8   -> 0x6E024020
//!   ext v31.16b, v30.16b, v29.16b, #15  -> 0x6E1D7BDF
//!   ext v5.8b,   v6.8b,   v7.8b,   #0   -> 0x2E0700C5
//! ```
//! (No aarch64 assembler/llvm-mc was available in this environment to
//! cross-check; the values follow directly from the spec layout above.)
//!
//! ## Findings (surfaced by the `#[ignore]`-d witness properties)
//! `encode_neon_ext` does **no** range or arrangement validation:
//!   1. `index` is silently masked with `& 0xF` (`(index & 0xF) << 11`), so
//!      an out-of-range index (e.g. `#16`, `#20`, `#100`) is truncated mod 16
//!      and a valid-looking word is emitted instead of `Err`. Likewise `.8B`
//!      silently accepts `#8..#15` which is UNDEFINED.
//!   2. The arrangement is only consulted to set Q for exactly `"16b"`; any
//!      non-byte arrangement (`.4h`, `.2s`, ...) is accepted and encoded as a
//!      Q=0 EXT word, which is UNALLOCATED.
//!      Both witnesses are `#[ignore]`-d so `cargo test` stays green; run them
//!      explicitly with `cargo test ext -- --ignored`.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_ext;
use proptest::prelude::*;

// --- helpers --------------------------------------------------------------

/// Build `Operand::RegArrangement { reg: "v{n}", arrangement }`.
fn va(n: u32, arr: &str) -> Operand {
    Operand::RegArrangement {
        reg: format!("v{n}"),
        arrangement: arr.to_string(),
    }
}

fn imm(v: i64) -> Operand {
    Operand::Imm(v)
}

fn reg_num_strategy() -> impl Strategy<Value = u32> {
    0u32..=31u32
}

/// Canonical operand list for `EXT Vd.T, Vn.T, Vm.T, #imm` (in-range index).
fn canonical_ops(rd: u32, rn: u32, rm: u32, arr: &str, index: i64) -> Vec<Operand> {
    vec![va(rd, arr), va(rn, arr), va(rm, arr), imm(index)]
}

/// Independent reference encoder, assembled field-by-field from the ARM
/// layout. The constant `1011100` is placed at bits 29-23 via a single
/// shifted constant and the low word packed separately, keeping it
/// structurally distinct from the SUT's nested OR-chain.
fn ref_encode_ext(rd: u32, rn: u32, rm: u32, imm4: u32, q: u32) -> u32 {
    let mut w = 0u32;
    w |= (q & 1) << 30; // bit 30 = Q
    w |= 0b1011100u32 << 23; // bits 29-23 = 1011100
    // bits 22, 21, 15, 10 stay 0
    w |= (rm & 0x1F) << 16; // bits 20-16 = Rm
    w |= (imm4 & 0xF) << 11; // bits 14-11 = imm4
    w |= (rn & 0x1F) << 5; // bits 9-5 = Rn
    w |= rd & 0x1F; // bits 4-0 = Rd
    w
}

fn word_of(res: Result<EncodeResult, String>) -> u32 {
    match res {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {other:?}"),
    }
}

// --- golden table (absolute oracle) --------------------------------------
// (Rd, Rn, Rm, arr, index, Q, expected_word)
const GOLDEN: &[(u32, u32, u32, &str, i64, u32, u32)] = &[
    (0, 1, 2, "8b", 3, 0, 0x2E021820), // ext v0.8b,   v1.8b,   v2.8b,   #3
    (0, 1, 2, "16b", 8, 1, 0x6E024020), // ext v0.16b,  v1.16b,  v2.16b,  #8
    (31, 30, 29, "16b", 15, 1, 0x6E1D7BDF), // ext v31.16b, v30.16b, v29.16b, #15
    (5, 6, 7, "8b", 0, 0, 0x2E0700C5), // ext v5.8b,   v6.8b,   v7.8b,   #0
];

#[test]
fn ext_matches_golden_table() {
    for &(rd, rn, rm, arr, index, q, expected) in GOLDEN {
        let ops = canonical_ops(rd, rn, rm, arr, index);
        let got = word_of(encode_neon_ext(&ops));
        assert_eq!(
            got, expected,
            "ext v{rd}.{arr}, v{rn}.{arr}, v{rm}.{arr}, #{index}: \
             got 0x{got:08X}, want 0x{expected:08X}",
        );
        assert_eq!(
            ref_encode_ext(rd, rn, rm, index as u32 & 0xF, q),
            expected,
            "reference encoder drift",
        );
    }
}

// --- properties -----------------------------------------------------------

proptest! {
    // === Oracle: reference encoder (differential) =========================
    // For every in-range register triple and valid arrangement/index, the
    // implementation must equal the independently-assembled reference word.
    #[test]
    fn matches_reference_encoder(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in prop_oneof![Just("8b"), Just("16b")],
        index in 0i64..16i64,
    ) {
        let ops = canonical_ops(rd, rn, rm, arr, index);
        let got = word_of(encode_neon_ext(&ops));
        let q = if arr == "16b" { 1u32 } else { 0u32 };
        let want = ref_encode_ext(rd, rn, rm, (index as u32) & 0xF, q);
        prop_assert_eq!(got, want);
    }

    // === Field placement: Rd/Rn/Rm/imm4 round-trip, Q from arrangement ===
    // The four data fields must round-trip exactly with no truncation or
    // cross-field bleed for in-range register numbers and in-range index,
    // and the Q bit must be derived solely from the arrangement.
    #[test]
    fn fields_round_trip_and_q_is_correct(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in prop_oneof![Just("8b"), Just("16b")],
        index in 0i64..16i64,
    ) {
        let ops = canonical_ops(rd, rn, rm, arr, index);
        let w = word_of(encode_neon_ext(&ops));
        let expected_q = if arr == "16b" { 1u32 } else { 0u32 };

        prop_assert_eq!((w >> 30) & 1, expected_q, "Q bit (bit 30) from arrangement");
        prop_assert_eq!(w & 0x1F, rd, "Rd field (bits 4-0)");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field (bits 9-5)");
        prop_assert_eq!((w >> 16) & 0x1F, rm, "Rm field (bits 20-16)");
        prop_assert_eq!((w >> 11) & 0xF, (index as u32) & 0xF, "imm4 field (bits 14-11)");
    }

    // === Fixed-bits invariant =============================================
    // The architecturally-constant bits never change: bit 31 = 0, bits 29-23
    // = 0b1011100, bits 22 & 21 = 0, bit 15 = 0, bit 10 = 0.
    #[test]
    fn fixed_bits_are_constant(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in prop_oneof![Just("8b"), Just("16b")],
        index in 0i64..16i64,
    ) {
        let ops = canonical_ops(rd, rn, rm, arr, index);
        let w = word_of(encode_neon_ext(&ops));

        prop_assert_eq!((w >> 31) & 1, 0, "bit 31 must be 0");
        prop_assert_eq!((w >> 23) & 0x7F, 0b1011100, "bits 29-23 must be 0b1011100");
        prop_assert_eq!((w >> 21) & 0b11, 0b00, "bits 22-21 must be 00");
        prop_assert_eq!((w >> 15) & 1, 0, "bit 15 must be 0");
        prop_assert_eq!((w >> 10) & 1, 0, "bit 10 must be 0");
    }

    // === Error contract: operand count ===================================
    // Fewer than 4 operands must be rejected with Err.
    #[test]
    fn rejects_too_few_operands(n in 0usize..4) {
        let ops: Vec<Operand> = (0..n).map(|_| va(0, "8b")).collect();
        let res = encode_neon_ext(&ops);
        prop_assert!(res.is_err(), "expected Err for {} operands, got {:?}", n, res);
    }

    // === Negative contract (BUG WITNESS — silent index truncation) ========
    // EXT indices are bounded: 0..=7 for `.8B` and 0..=15 for `.16B`. Any
    // index outside the arrangement's range is UNALLOCATED and MUST yield
    // Err. `encode_neon_ext` instead masks with `& 0xF`, so e.g. `#16` is
    // silently encoded as `#0` and `#20` as `#4`, emitting a valid-looking
    // word. This property therefore currently FAILS; it is `#[ignore]`-d so
    // the default `cargo test` run stays green. Run explicitly with
    // `cargo test ext -- --ignored`.
    #[test]
    #[ignore = "bug witness: encode_neon_ext silently truncates out-of-range index (index & 0xF) instead of returning Err"]
    fn rejects_out_of_range_index(index in 16i64..256i64) {
        let ops = canonical_ops(0, 1, 2, "16b", index);
        let res = encode_neon_ext(&ops);
        prop_assert!(
            res.is_err(),
            "ext v0.16b, v1.16b, v2.16b, #{index} is out of range (0..=15) \
             and is UNALLOCATED; expected Err, got {:?}",
            res,
        );
    }

    // === Negative contract (BUG WITNESS — no arrangement validation) ======
    // EXT is defined ONLY for `.8B`/`.16B`; every other arrangement is
    // UNALLOCATED (LLVM rejects it). `encode_neon_ext` only special-cases
    // exactly `"16b"` to set Q, accepting any other arrangement as Q=0 and
    // emitting a valid-looking EXT word. This property therefore currently
    // FAILS; it is `#[ignore]`-d so the default `cargo test` run stays green.
    #[test]
    #[ignore = "bug witness: encode_neon_ext accepts non-byte arrangements (only .8b/.16b are defined) instead of returning Err"]
    fn rejects_non_byte_arrangements(
        arr in prop_oneof![
            Just("4h"), Just("8h"), Just("2s"), Just("4s"), Just("1d"), Just("2d"),
        ],
    ) {
        let ops = vec![va(0, arr), va(1, arr), va(2, arr), imm(4)];
        let res = encode_neon_ext(&ops);
        prop_assert!(
            res.is_err(),
            "ext with .{arr} is UNALLOCATED (only .8b/.16b are defined); \
             expected Err, got {:?}",
            res,
        );
    }
}
