//! Property-based tests for `encode_neon_float_three_same`.
//!
//! `encode_neon_float_three_same` emits the AArch64 **"Advanced SIMD
//! Floating-point three same"** encoding group, parameterised by `u_bit`,
//! `size_hi`, and `opcode`:
//!
//! ```text
//!   31 30 29 28-24 23-22 21 20-16 15-11 10 9-5 4-0
//!    0  Q  U  01110  size   1   Rm   opcode 1  Rn  Rd
//! ```
//! where `size = (size_hi << 1) | sz` and `(Q, sz)` is fixed by the
//! destination arrangement: `2s→(0,0)`, `4s→(1,0)`, `2d→(1,1)`.
//! Call sites (`fadd`, `fsub`, `fmul`, `fdiv`, `fcmeq`, `facgt`, `fmax`, ...)
//! always pass `u_bit ∈ {0,1}`, `size_hi ∈ {0,1}`, `opcode ∈ 0..32`.
//!
//! ## Oracle
//! The reference word is re-assembled field-by-field from the layout above,
//! independently of the implementation. It is additionally pinned to an
//! absolute golden table produced by LLVM's assembler (`clang --target=aarch64`)
//! — e.g. `fadd v0.4s,v1.4s,v2.4s = 0x4e22d420`, `fsub ...4s = 0x4ea2d420`
//! (differing only in bit 23 = `size_hi`), `facgt v0.2d,... = 0x6ee2ec20`.
//! Register numbers are bounded to 0–31 by `parse_reg_num`, so `v32`+ is
//! rejected before reaching this function.
//!
//! ## Finding (documented by the `#[ignore]`d property `rejects_out_of_range_params`)
//! `u_bit`, `size_hi`, and `opcode` are *fixed-width* fields (1/1/5 bits).
//! The ARM ARM gives no wrapping semantics for them, so an out-of-range value
//! must be rejected with `Err`. The implementation instead OR-shifts the raw
//! value into the word with **no masking or range check**, silently corrupting
//! adjacent constant bits (e.g. `u_bit=2` flips bit 30, destroying Q;
//! `size_hi=2` flips bit 24, destroying the `01110` constant group;
//! `opcode=32` flips bit 16, corrupting Rm). See
//! `NEON_FLOAT_THREE_SAME_BUG_REPORT.md`.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_float_three_same;
use proptest::prelude::*;

// --- helpers --------------------------------------------------------------

/// Build a NEON vector register operand `v{n}.<arr>`.
fn reg_arr(n: u32, arr: &str) -> Operand {
    Operand::RegArrangement {
        reg: format!("v{n}"),
        arrangement: arr.to_string(),
    }
}

/// In-range vector register number 0..=31 (what `parse_reg_num` accepts).
fn reg_num_strategy() -> impl Strategy<Value = u32> {
    0u32..=31u32
}

/// The only arrangements this encoder accepts for the FP three-same group.
fn arr_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("2s"), Just("4s"), Just("2d")]
}

/// Arrangements that must be REJECTED: valid NEON integer/double-half forms
/// that are wrong for FP three-same, plus outright invalid specifiers.
fn bad_arr_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("8b".to_string()),
        Just("16b".to_string()),
        Just("4h".to_string()),
        Just("8h".to_string()),
        Just("1d".to_string()),
        Just("8s".to_string()),
        Just("4d".to_string()),
        Just("2b".to_string()),
        Just("1q".to_string()),
        Just("3x".to_string()),
    ]
}

/// Valid field widths per the ARM ARM "FP three same" layout.
fn u_bit_strategy() -> impl Strategy<Value = u32> {
    Just(0u32).prop_union(Just(1u32))
}
fn size_hi_strategy() -> impl Strategy<Value = u32> {
    Just(0u32).prop_union(Just(1u32))
}
fn opcode_strategy() -> impl Strategy<Value = u32> {
    0u32..=31u32
}

/// `(Q, sz)` for an arrangement, exactly as the implementation maps them.
fn arr_qs(arr: &str) -> (u32, u32) {
    match arr {
        "2s" => (0, 0),
        "4s" => (1, 0),
        "2d" => (1, 1),
        _ => unreachable!("reference only fed valid arrangements"),
    }
}

/// Independent reference encoder built straight from the documented bit layout.
fn ref_encode(rd: u32, rn: u32, rm: u32, u_bit: u32, size_hi: u32, opcode: u32, arr: &str) -> u32 {
    let (q, sz) = arr_qs(arr);
    let size = ((size_hi & 1) << 1) | sz;
    let mut w = 0u32;
    w |= q << 30; // bit 30 = Q
    w |= (u_bit & 0x1) << 29; // bit 29 = U
    w |= 0b01110u32 << 24; // bits 28-24
    w |= (size & 0x3) << 22; // bits 23-22 = size
    w |= 1u32 << 21; // bit 21
    w |= (rm & 0x1F) << 16; // bits 20-16 = Rm
    w |= (opcode & 0x1F) << 11; // bits 15-11 = opcode
    w |= 1u32 << 10; // bit 10
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

// --- golden table (absolute oracle, cross-checked against LLVM clang) -----

type GoldenRow = (u32, u32, u32, u32, u32, u32, &'static str, u32);

/// Produced by `clang --target=aarch64` for the named instructions; fields are
/// (Rd, Rn, Rm, u_bit, size_hi, opcode, arrangement, expected_word).
const GOLDEN: &[GoldenRow] = &[
    (0, 1, 2, 0, 0, 0b11010, "4s", 0x4e22d420), // fadd v0.4s,v1.4s,v2.4s
    (0, 1, 2, 0, 1, 0b11010, "4s", 0x4ea2d420), // fsub v0.4s,v1.4s,v2.4s (bit23=size_hi)
    (0, 1, 2, 1, 0, 0b11011, "4s", 0x6e22dc20), // fmul v0.4s,v1.4s,v2.4s
    (0, 1, 2, 1, 0, 0b11111, "2d", 0x6e62fc20), // fdiv v0.2d,v1.2d,v2.2d
    (0, 1, 2, 0, 0, 0b11100, "4s", 0x4e22e420), // fcmeq v0.4s,v1.4s,v2.4s
    (0, 1, 2, 1, 1, 0b11101, "2d", 0x6ee2ec20), // facgt v0.2d,v1.2d,v2.2d
    (0, 1, 2, 0, 0, 0b11110, "2s", 0x0e22f420), // fmax v0.2s,v1.2s,v2.2s
    (0, 1, 2, 0, 1, 0b11110, "4s", 0x4ea2f420), // fmin v0.4s,v1.4s,v2.4s
];

#[test]
fn matches_golden_table() {
    for &(rd, rn, rm, u_bit, size_hi, opcode, arr, expected) in GOLDEN {
        let ops = vec![reg_arr(rd, arr), reg_arr(rn, arr), reg_arr(rm, arr)];
        let got = word_of(encode_neon_float_three_same(&ops, u_bit, size_hi, opcode));
        assert_eq!(
            got, expected,
            "fp-3same {arr} (rd={rd},rn={rn},rm={rm},u={u_bit},sh={size_hi},opc=0b{opcode:b}): \
             got 0x{got:08X}, want 0x{expected:08X}",
        );
        // The reference encoder must agree with the golden values too.
        assert_eq!(
            ref_encode(rd, rn, rm, u_bit, size_hi, opcode, arr),
            expected,
            "reference encoder drift",
        );
    }
}

// --- properties -----------------------------------------------------------

proptest! {
    // === Oracle: reference encoder (differential) =========================
    // For every in-range register triple and valid field value, the
    // implementation must equal the independently-assembled reference word.
    #[test]
    fn matches_reference_encoder(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        u_bit in u_bit_strategy(),
        size_hi in size_hi_strategy(),
        opcode in opcode_strategy(),
        arr in arr_strategy(),
    ) {
        let ops = vec![reg_arr(rd, arr), reg_arr(rn, arr), reg_arr(rm, arr)];
        let got = word_of(encode_neon_float_three_same(&ops, u_bit, size_hi, opcode));
        let want = ref_encode(rd, rn, rm, u_bit, size_hi, opcode, arr);
        prop_assert_eq!(got, want);
    }

    // === Field placement: Rd/Rn/Rm/U/size/opcode round-trip ===============
    // Each variable field must round-trip exactly for in-range inputs — no
    // silent truncation of register numbers or field bits. The arrangement
    // alone fixes Q (bit 30) and the low size bit (bit 22, sz); size_hi fixes
    // bit 23.
    #[test]
    fn fields_round_trip(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        u_bit in u_bit_strategy(),
        size_hi in size_hi_strategy(),
        opcode in opcode_strategy(),
        arr in arr_strategy(),
    ) {
        let ops = vec![reg_arr(rd, arr), reg_arr(rn, arr), reg_arr(rm, arr)];
        let w = word_of(encode_neon_float_three_same(&ops, u_bit, size_hi, opcode));

        let (q, sz) = arr_qs(arr);
        let size = (size_hi << 1) | sz;

        prop_assert_eq!(w & 0x1F, rd, "Rd field");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field");
        prop_assert_eq!((w >> 16) & 0x1F, rm, "Rm field");
        prop_assert_eq!((w >> 29) & 0x1, u_bit, "U bit");
        prop_assert_eq!((w >> 22) & 0x3, size, "size field (size_hi<<1 | sz)");
        prop_assert_eq!((w >> 30) & 0x1, q, "Q bit (from arrangement)");
        prop_assert_eq!((w >> 11) & 0x1F, opcode, "opcode field");
    }

    // === Arrangement contract: only 2s/4s/2d are accepted =================
    // Every other arrangement specifier must yield Err. Also covers the
    // arity contract: fewer than 3 operands must yield Err.
    #[test]
    fn rejects_invalid_arrangement_and_arity(
        bad_arr in bad_arr_strategy(),
        n in 0u32..2u32, // operand counts 0,1,2 (insufficient)
    ) {
        // (a) unsupported arrangement on the destination operand.
        let ops_bad_arr = vec![
            reg_arr(0, &bad_arr),
            reg_arr(1, &bad_arr),
            reg_arr(2, &bad_arr),
        ];
        prop_assert!(
            encode_neon_float_three_same(&ops_bad_arr, 0, 0, 0b11010).is_err(),
            "arrangement {bad_arr} must be rejected for fp three-same",
        );
        // (b) fewer than 3 operands.
        let short: Vec<Operand> = (0..n).map(|i| reg_arr(i, "4s")).collect();
        prop_assert!(
            encode_neon_float_three_same(&short, 0, 0, 0b11010).is_err(),
            "{n} operands must be rejected (need >= 3)",
        );
        // (c) non-register operand kind.
        let ops_bad_kind = vec![Operand::Imm(0), Operand::Imm(1), Operand::Imm(2)];
        prop_assert!(
            encode_neon_float_three_same(&ops_bad_kind, 0, 0, 0b11010).is_err(),
            "non-RegArrangement operands must be rejected",
        );
    }

    // === Fixed-bits invariant =============================================
    // The architecturally-constant bits never change for any valid input:
    // bit 31 = 0, bits 28-24 = 01110, bit 21 = 1, bit 10 = 1.
    #[test]
    fn fixed_bits_are_constant(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        u_bit in u_bit_strategy(),
        size_hi in size_hi_strategy(),
        opcode in opcode_strategy(),
        arr in arr_strategy(),
    ) {
        let ops = vec![reg_arr(rd, arr), reg_arr(rn, arr), reg_arr(rm, arr)];
        let w = word_of(encode_neon_float_three_same(&ops, u_bit, size_hi, opcode));

        prop_assert_eq!((w >> 31) & 0x1, 0, "bit 31 must be 0");
        prop_assert_eq!((w >> 24) & 0x1F, 0b01110, "bits 28-24 must be 01110");
        prop_assert_eq!((w >> 21) & 0x1, 1, "bit 21 must be 1");
        prop_assert_eq!((w >> 10) & 0x1, 1, "bit 10 must be 1");
    }

    // === Finding: out-of-range U/size_hi/opcode are NOT rejected ==========
    // Per the ARM ARM these are fixed-width fields (1/1/5 bits) with no
    // wrapping semantics, so an out-of-range value MUST yield `Err`. The
    // current implementation silently OR-shifts it in, corrupting adjacent
    // constant bits. `#[ignore]`d because the impl fails the contract; run
    // with `--ignored rejects_out_of_range_params`.
    #[test]
    #[ignore]
    fn rejects_out_of_range_params(
        big_u in 2u32..16u32,
        big_size_hi in 2u32..16u32,
        big_opcode in 32u32..256u32,
    ) {
        let ops = vec![reg_arr(0, "4s"), reg_arr(1, "4s"), reg_arr(2, "4s")];

        {
            let u = big_u;
            let res = encode_neon_float_three_same(&ops, u, 0, 0b11010);
            prop_assert!(
                res.is_err(),
                "u_bit={} is out of the 1-bit field (spec: bit 29); expected Err, got {:?}",
                u, res,
            );
        }
        {
            let sh = big_size_hi;
            let res = encode_neon_float_three_same(&ops, 0, sh, 0b11010);
            prop_assert!(
                res.is_err(),
                "size_hi={} is out of the 1-bit field (spec: feeds size bit 23); expected Err, got {:?}",
                sh, res,
            );
        }
        {
            let o = big_opcode;
            let res = encode_neon_float_three_same(&ops, 0, 0, o);
            prop_assert!(
                res.is_err(),
                "opcode={} is out of the 5-bit field (spec: bits 15-11); expected Err, got {:?}",
                o, res,
            );
        }
    }
}
