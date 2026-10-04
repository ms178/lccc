//! Property-based tests for `encode_neon_scalar_two_misc`.
//!
//! `encode_neon_scalar_two_misc` emits the AArch64 **"Scalar two-register
//! miscellaneous"** encoding group, parameterised by `u_bit` and `opcode`:
//!
//! ```text
//!   31 30 29 28-24 23-22  21  20-17 16-12 11-10 9-5 4-0
//!    0  1  U  11110  size   1   0000  opcode  1 0  Rn  Rd
//! ```
//! (ARM DDI 0487, "Scalar Two-register miscellaneous".) The call sites are the
//! scalar forms of `sqabs` → (u=0, opcode=0b00111) and `sqneg` → (u=0,
//! opcode=0b01000) in `encoder/mod.rs`.
//!
//! The `size` field is **not** a free parameter: it is derived from the
//! destination register's letter prefix — `b`→00, `h`→01, `s`→10, `d`→11.
//!
//! ## Oracle
//! The reference word is re-assembled field-by-field from the layout above,
//! independently of the implementation. Absolute correctness is additionally
//! pinned by hand-derived golden values (`sqabs d0,d1 = 0x5EE07820`,
//! `sqneg d31,d30 = 0x5EE08BDF`). Register numbers are bounded 0–31 by
//! `parse_reg_num`, so `d32`+ is rejected.
//!
//! ## Finding (documented by the `#[ignore]`d property `rejects_out_of_range_params`)
//! `U` and `opcode` are *fixed-width* fields (1 bit at [29], 5 bits at
//! [16:12]). The ARM ARM gives no wrapping semantics, so an out-of-range value
//! MUST be rejected with `Err`. The implementation OR-shifts the raw value
//! into the word with **no masking or range check**, silently corrupting the
//! adjacent constant bits (e.g. `u_bit=2` flips bit 30, destroying the `01`
//! scalar marker; `opcode=32` flips bit 17, breaking the fixed `10000` pattern
//! at [21:17]).
//!
//! A secondary (non-blocking) observation: the encoder validates neither that
//! `Rn` shares `Rd`'s width nor that scalar `sqabs`/`sqneg` forbid the B
//! (`size=00`) variant — `sqabs d0, s1` and `sqabs b0, b1` both encode without
//! error. These are left as documented quirks of the generic encoder.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_scalar_two_misc;
use proptest::prelude::*;

// --- helpers --------------------------------------------------------------

/// Build a scalar register operand by letter prefix + number.
fn reg(prefix: &str, n: u32) -> Operand {
    Operand::Reg(format!("{prefix}{n}"))
}

/// In-range scalar register number 0..=31 (what `parse_reg_num` accepts).
fn reg_num_strategy() -> impl Strategy<Value = u32> {
    0u32..=31u32
}

fn u_bit_strategy() -> impl Strategy<Value = u32> {
    Just(0u32).prop_union(Just(1u32))
}
fn opcode_strategy() -> impl Strategy<Value = u32> {
    0u32..=31u32
}

/// The four register-letter prefixes that map to a `size` field, with the
/// expected 2-bit size value.
const PREFIX_SIZE: &[(char, u32)] = &[('b', 0b00), ('h', 0b01), ('s', 0b10), ('d', 0b11)];

/// Independent reference encoder built straight from the documented bit layout.
fn ref_encode(rd: u32, rn: u32, size: u32, u_bit: u32, opcode: u32) -> u32 {
    let mut w = 0u32;
    w |= 0b01u32 << 30; // bits 31-30 = 01 (scalar marker)
    w |= (u_bit & 0x1) << 29; // bit 29 = U
    w |= 0b11110u32 << 24; // bits 28-24
    w |= (size & 0x3) << 22; // bits 23-22 = size
    w |= 0b10000u32 << 17; // bit 21 = 1, bits 20-17 = 0000
    w |= (opcode & 0x1F) << 12; // bits 16-12 = opcode
    w |= 0b10u32 << 10; // bits 11-10 = 10
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

/// Hand-derived from the ARMv8-A "Scalar two-register miscellaneous" layout.
///   sqabs: U=0, opcode=0b00111 ; sqneg: U=0, opcode=0b01000.
const GOLDEN: &[(char, u32, u32, u32, u32, u32, u32)] = &[
    // (Rd-prefix, Rd, Rn, u_bit, opcode, size, expected_word)
    ('d', 0, 1, 0, 0b00111, 0b11, 0x5EE07820), // sqabs d0, d1
    ('d', 31, 30, 0, 0b01000, 0b11, 0x5EE08BDF), // sqneg d31, d30  (U=0, opc=01000)
    ('s', 0, 1, 0, 0b00111, 0b10, 0x5EA07820), // sqabs s0, s1   (size=10)
    ('h', 2, 3, 0, 0b00111, 0b01, 0x5E607862), // sqabs h2, h3   (size=01)
];

#[test]
fn matches_golden_table() {
    for &(pfx, rd, rn, u_bit, opcode, size, expected) in GOLDEN {
        let ops = vec![reg(&pfx.to_string(), rd), reg(&pfx.to_string(), rn)];
        let got = word_of(encode_neon_scalar_two_misc(&ops, u_bit, opcode));
        assert_eq!(
            got, expected,
            "scalar-2misc ({}{rd},{}{rn},u={u_bit},opc=0b{opcode:b}): \
             got 0x{got:08X}, want 0x{expected:08X}",
            pfx, pfx,
        );
        assert_eq!(
            ref_encode(rd, rn, size, u_bit, opcode),
            expected,
            "reference encoder drift",
        );
    }
}

// --- properties -----------------------------------------------------------

proptest! {
    // === Oracle: reference encoder (differential) =========================
    // For every valid register pair, prefix, u_bit, and opcode, the
    // implementation must equal the independently-assembled reference word.
    #[test]
    fn matches_reference_encoder(
        prefix_idx in 0usize..PREFIX_SIZE.len(),
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        u_bit in u_bit_strategy(),
        opcode in opcode_strategy(),
    ) {
        let (pfx, size) = PREFIX_SIZE[prefix_idx];
        let pfx = pfx.to_string();
        let ops = vec![reg(&pfx, rd), reg(&pfx, rn)];
        let got = word_of(encode_neon_scalar_two_misc(&ops, u_bit, opcode));
        let want = ref_encode(rd, rn, size, u_bit, opcode);
        prop_assert_eq!(got, want);
    }

    // === Field placement: Rd/Rn/U/opcode round-trip =======================
    // Each variable field must round-trip exactly for in-range inputs — no
    // silent truncation of register numbers or field bits.
    #[test]
    fn fields_round_trip(
        prefix_idx in 0usize..PREFIX_SIZE.len(),
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        u_bit in u_bit_strategy(),
        opcode in opcode_strategy(),
    ) {
        let (pfx, size) = PREFIX_SIZE[prefix_idx];
        let pfx = pfx.to_string();
        let ops = vec![reg(&pfx, rd), reg(&pfx, rn)];
        let w = word_of(encode_neon_scalar_two_misc(&ops, u_bit, opcode));

        prop_assert_eq!(w & 0x1F, rd, "Rd field");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field");
        prop_assert_eq!((w >> 29) & 0x1, u_bit, "U bit");
        prop_assert_eq!((w >> 12) & 0x1F, opcode, "opcode field");
        prop_assert_eq!((w >> 22) & 0x3, size, "size field (from prefix)");
    }

    // === Fixed-bits invariant =============================================
    // The architecturally-constant bits never change for any valid input:
    // bits 31-30 = 01, bits 28-24 = 11110, bit 21 = 1, bits 20-17 = 0,
    // bits 11-10 = 10.
    #[test]
    fn fixed_bits_are_constant(
        prefix_idx in 0usize..PREFIX_SIZE.len(),
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        u_bit in u_bit_strategy(),
        opcode in opcode_strategy(),
    ) {
        let (pfx, _) = PREFIX_SIZE[prefix_idx];
        let pfx = pfx.to_string();
        let ops = vec![reg(&pfx, rd), reg(&pfx, rn)];
        let w = word_of(encode_neon_scalar_two_misc(&ops, u_bit, opcode));

        prop_assert_eq!((w >> 30) & 0x3, 0b01, "bits 31-30 must be 01 (scalar)");
        prop_assert_eq!((w >> 24) & 0x1F, 0b11110, "bits 28-24");
        prop_assert_eq!((w >> 17) & 0x1F, 0b10000, "bits 21-17 must be 10000");
        prop_assert_eq!((w >> 10) & 0x3, 0b10, "bits 11-10 must be 10");
    }

    // === Negative contract: arity / operand type / register range =========
    // (a) fewer than 2 operands → Err; (b) a non-Reg operand → Err;
    // (c) an out-of-range register (d32+) that `parse_reg_num` rejects → Err;
    // (d) an unsupported register-letter prefix (x/w/v/q) → Err.
    #[test]
    fn rejects_malformed_operands(
        n in 0u32..=1u32, // operand counts 0,1 (insufficient)
        bad_reg in 32u32..200u32, // beyond the 5-bit register field
        bad_pfx_idx in 0usize..4usize,
    ) {
        // (a) insufficient operand count
        let short: Vec<Operand> = (0..n).map(|_| reg("d", 0)).collect();
        prop_assert!(
            encode_neon_scalar_two_misc(&short, 0, 0b00111).is_err(),
            "{n} operands must be rejected (need >= 2)",
        );
        // (b) wrong operand kind
        let ops_bad_kind = vec![Operand::Imm(7), Operand::Imm(8)];
        prop_assert!(
            encode_neon_scalar_two_misc(&ops_bad_kind, 0, 0b00111).is_err(),
            "non-Reg operands must be rejected",
        );
        // (c) out-of-range register number (parse_reg_num returns None for >=32)
        let ops_bad_reg = vec![reg("d", bad_reg), reg("d", 1)];
        prop_assert!(
            encode_neon_scalar_two_misc(&ops_bad_reg, 0, 0b00111).is_err(),
            "register d{bad_reg} (>= 32) must be rejected",
        );
        // (d) unsupported prefix: x/w/v/q parse as registers but have no size
        let bad_pfx = ["x", "w", "v", "q"][bad_pfx_idx];
        let ops_bad_pfx = vec![reg(bad_pfx, 0), reg(bad_pfx, 1)];
        prop_assert!(
            encode_neon_scalar_two_misc(&ops_bad_pfx, 0, 0b00111).is_err(),
            "unsupported register prefix '{bad_pfx}' must be rejected",
        );
    }

    // === Finding: out-of-range U/opcode are NOT rejected ==================
    // Per the ARM ARM these are fixed-width fields (1/5 bits) with no
    // wrapping semantics, so an out-of-range value MUST yield `Err`. The
    // current implementation silently OR-shifts it in, corrupting adjacent
    // constant bits. This property is `#[ignore]`d because the impl fails the
    // contract; run with `--ignored rejects_out_of_range_params`.
    #[test]
    #[ignore]
    fn rejects_out_of_range_params(
        big_u in 2u32..16u32,
        big_opcode in 32u32..1024u32,
    ) {
        let ops = vec![reg("d", 0), reg("d", 1)];

        let res_u = encode_neon_scalar_two_misc(&ops, big_u, 0b00111);
        prop_assert!(
            res_u.is_err(),
            "U={big_u} is out of the 1-bit field (spec: bit 29); expected Err, got {:?}",
            res_u,
        );

        let res_o = encode_neon_scalar_two_misc(&ops, 0, big_opcode);
        prop_assert!(
            res_o.is_err(),
            "opcode={big_opcode} is out of the 5-bit field (spec: bits 16-12); expected Err, got {:?}",
            res_o,
        );
    }
}
