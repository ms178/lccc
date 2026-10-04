//! Property-based tests for `encode_neon_scalar_three_same`.
//!
//! `encode_neon_scalar_three_same` emits the AArch64 **"Scalar three same"**
//! encoding group, parameterised by `u_bit`, `opcode`, and `size`:
//!
//! ```text
//!   31 30 29 28-24 23-22 21 20-16 15-11 10 9-5 4-0
//!    0  1  U  11110  size  1   Rm   opcode 1  Rn  Rd
//! ```
//! (ARM DDI 0487, "Scalar Three Same".) The sole call sites are
//! `add Dd,Dn,Dm` → (u=0, opcode=10000, size=11) and
//! `sub Dd,Dn,Dm` → (u=1, opcode=10000, size=11).
//!
//! ## Oracle
//! The reference word is re-assembled field-by-field from the layout above,
//! independently of the implementation. The absolute correctness of every
//! constant field is additionally pinned by a hand-derived golden table
//! (`add d0,d1,d2 = 0x5EE28420`, `sub d0,d1,d2 = 0x7EE28420`, differing only
//! in the U bit at position 29 — exactly as the ARM ARM specifies). Register
//! numbers are bounded to 0–31 by `parse_reg_num`, so `d32`+ is rejected.
//!
//! ## Finding (documented by the `#[ignore]`d property `rejects_out_of_range_params`)
//! `U`, `size`, and `opcode` are *fixed-width* fields (1/2/5 bits). The ARM
//! ARM gives no wrapping semantics for them, so any out-of-range value must be
//! rejected with `Err`. The implementation instead OR-shifts the raw value
//! into the word with **no masking or range check**, silently corrupting the
//! adjacent constant bits (e.g. `u_bit=2` flips bit 30, destroying the `01`
//! scalar marker; `size=4` flips bit 24; `opcode=32` flips bit 16 in Rm).
//! See `NEON_SCALAR_THREE_SAME_BUG_REPORT.md`.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_scalar_three_same;
use proptest::prelude::*;

// --- helpers --------------------------------------------------------------

/// Build a scalar D-register operand `d{n}` (the only form this encoder reads).
fn reg(n: u32) -> Operand {
    Operand::Reg(format!("d{n}"))
}

/// In-range scalar register number 0..=31 (what `parse_reg_num` accepts).
fn reg_num_strategy() -> impl Strategy<Value = u32> {
    0u32..=31u32
}

/// Valid field widths per the ARM ARM "Scalar three same" layout.
fn u_bit_strategy() -> impl Strategy<Value = u32> {
    Just(0u32).prop_union(Just(1u32))
}
fn size_strategy() -> impl Strategy<Value = u32> {
    0u32..=3u32
}
fn opcode_strategy() -> impl Strategy<Value = u32> {
    0u32..=31u32
}

/// Independent reference encoder built straight from the documented bit layout.
fn ref_encode(rd: u32, rn: u32, rm: u32, u_bit: u32, opcode: u32, size: u32) -> u32 {
    let mut w = 0u32;
    w |= 0b01u32 << 30; // bits 31-30 = 01 (scalar marker)
    w |= (u_bit & 0x1) << 29; // bit 29 = U
    w |= 0b11110u32 << 24; // bits 28-24
    w |= (size & 0x3) << 22; // bits 23-22
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

// --- golden table (absolute oracle) ---------------------------------------

/// Hand-derived from the ARMv8-A "Scalar three same" layout:
/// ADD → U=0, opcode=10000; SUB → U=1, opcode=10000; both use size=11 (64-bit D).
const GOLDEN: &[(u32, u32, u32, u32, u32, u32, u32)] = &[
    // (Rd, Rn, Rm, u_bit, opcode, size, expected_word)
    (0, 1, 2, 0, 0b10000, 0b11, 0x5EE28420), // add d0, d1, d2
    (0, 1, 2, 1, 0b10000, 0b11, 0x7EE28420), // sub d0, d1, d2  (U bit set)
    (5, 6, 7, 0, 0b10000, 0b11, 0x5EE784C5), // add d5, d6, d7
    (31, 30, 29, 1, 0b10000, 0b11, 0x7EFD87DF), // sub d31, d30, d29
    (0, 0, 0, 0, 0b10000, 0b11, 0x5EE08400), // add d0, d0, d0
    (0, 1, 2, 0, 0b10001, 0b10, 0x5EA28C20), // generic 3-same: size=10, opc=10001
];

#[test]
fn matches_golden_table() {
    for &(rd, rn, rm, u_bit, opcode, size, expected) in GOLDEN {
        let ops = vec![reg(rd), reg(rn), reg(rm)];
        let got = word_of(encode_neon_scalar_three_same(&ops, u_bit, opcode, size));
        assert_eq!(
            got, expected,
            "scalar-3same (rd={rd},rn={rn},rm={rm},u={u_bit},opc=0b{opcode:b},size=0b{size:b}): \
             got 0x{got:08X}, want 0x{expected:08X}",
        );
        // The reference encoder must agree with the golden values too.
        assert_eq!(
            ref_encode(rd, rn, rm, u_bit, opcode, size),
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
        opcode in opcode_strategy(),
        size in size_strategy(),
    ) {
        let ops = vec![reg(rd), reg(rn), reg(rm)];
        let got = word_of(encode_neon_scalar_three_same(&ops, u_bit, opcode, size));
        let want = ref_encode(rd, rn, rm, u_bit, opcode, size);
        prop_assert_eq!(got, want);
    }

    // === Field placement: Rd/Rn/Rm/U/size/opcode round-trip ===============
    // Each variable field must round-trip exactly for in-range inputs — no
    // silent truncation of register numbers or field bits.
    #[test]
    fn fields_round_trip(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        u_bit in u_bit_strategy(),
        opcode in opcode_strategy(),
        size in size_strategy(),
    ) {
        let ops = vec![reg(rd), reg(rn), reg(rm)];
        let w = word_of(encode_neon_scalar_three_same(&ops, u_bit, opcode, size));

        prop_assert_eq!((w) & 0x1F, rd, "Rd field");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field");
        prop_assert_eq!((w >> 16) & 0x1F, rm, "Rm field");
        prop_assert_eq!((w >> 29) & 0x1, u_bit, "U bit");
        prop_assert_eq!((w >> 22) & 0x3, size, "size field");
        prop_assert_eq!((w >> 11) & 0x1F, opcode, "opcode field");
    }

    // === Fixed-bits invariant =============================================
    // The architecturally-constant bits never change for any valid input:
    // bits 31-30 = 01, bits 28-24 = 11110, bit 21 = 1, bit 10 = 1.
    #[test]
    fn fixed_bits_are_constant(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        u_bit in u_bit_strategy(),
        opcode in opcode_strategy(),
        size in size_strategy(),
    ) {
        let ops = vec![reg(rd), reg(rn), reg(rm)];
        let w = word_of(encode_neon_scalar_three_same(&ops, u_bit, opcode, size));

        prop_assert_eq!((w >> 30) & 0x3, 0b01, "bits 31-30 must be 01 (scalar)");
        prop_assert_eq!((w >> 24) & 0x1F, 0b11110, "bits 28-24");
        prop_assert_eq!((w >> 21) & 0x1, 1, "bit 21 must be 1");
        prop_assert_eq!((w >> 10) & 0x1, 1, "bit 10 must be 1");
    }

    // === Negative contract: arity / operand type / register range =========
    // (a) fewer than 3 operands → Err; (b) a non-Reg operand → Err;
    // (c) an out-of-range register (d32+) that `parse_reg_num` rejects → Err.
    #[test]
    fn rejects_malformed_operands(
        n in 0u32..2u32, // operand counts 0,1,2 (insufficient)
        bad_reg in 32u32..200u32, // beyond the 5-bit register field
    ) {
        // (a) insufficient operand count
        let short: Vec<Operand> = (0..n).map(reg).collect();
        prop_assert!(
            encode_neon_scalar_three_same(&short, 0, 0b10000, 0b11).is_err(),
            "{n} operands must be rejected (need >= 3)",
        );
        // (b) wrong operand kind
        let ops_bad_kind = vec![
            Operand::Imm(7),
            Operand::Imm(8),
            Operand::Imm(9),
        ];
        prop_assert!(
            encode_neon_scalar_three_same(&ops_bad_kind, 0, 0b10000, 0b11).is_err(),
            "non-Reg operands must be rejected",
        );
        // (c) out-of-range register number (parse_reg_num returns None for >=32)
        let ops_bad_reg = vec![reg(bad_reg), reg(1), reg(2)];
        prop_assert!(
            encode_neon_scalar_three_same(&ops_bad_reg, 0, 0b10000, 0b11).is_err(),
            "register d{bad_reg} (>= 32) must be rejected",
        );
    }

    // === Finding: out-of-range U/size/opcode are NOT rejected ==============
    // Per the ARM ARM these are fixed-width fields (1/2/5 bits) with no
    // wrapping semantics, so an out-of-range value MUST yield `Err`. The
    // current implementation silently OR-shifts it in, corrupting adjacent
    // constant bits. This property is `#[ignore]`d because the impl fails the
    // contract; run with `--ignored rejects_out_of_range_params`.
    #[test]
    #[ignore]
    fn rejects_out_of_range_params(
        big_u in 2u32..16u32,
        big_size in 4u32..16u32,
        big_opcode in 32u32..256u32,
    ) {
        let ops = vec![reg(0), reg(1), reg(2)];

        {
            let u = big_u;
            let res = encode_neon_scalar_three_same(&ops, u, 0b10000, 0b11);
            prop_assert!(
                res.is_err(),
                "U={} is out of the 1-bit field (spec: bit 29); expected Err, got {:?}",
                u, res,
            );
        }
        {
            let s = big_size;
            let res = encode_neon_scalar_three_same(&ops, 0, 0b10000, s);
            prop_assert!(
                res.is_err(),
                "size={} is out of the 2-bit field (spec: bits 23-22); expected Err, got {:?}",
                s, res,
            );
        }
        {
            let o = big_opcode;
            let res = encode_neon_scalar_three_same(&ops, 0, o, 0b11);
            prop_assert!(
                res.is_err(),
                "opcode={} is out of the 5-bit field (spec: bits 15-11); expected Err, got {:?}",
                o, res,
            );
        }
    }
}
