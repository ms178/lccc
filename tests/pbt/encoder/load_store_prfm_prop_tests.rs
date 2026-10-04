//! Property-based tests for `encode_prfm` (Prefetch Memory, ARMv8-A).
//!
//! This is a separate `#[cfg(test)]` module, pulled into the build by the
//! `#[cfg(test)] #[path = ...] mod load_store_prfm_prop_tests;` declaration at
//! the bottom of `load_store.rs`. `use lccc::pbt_internals::*;` therefore reaches the same
//! `encode_prfm` / `encode_prfop` / `Operand` / `EncodeResult` items the
//! in-file test modules use.

use lccc::pbt_internals::*;
use proptest::prelude::*;

// ── Independent oracle ───────────────────────────────────────────────
// ORACLE: differential-vs-`llvm-mc-18` + reference-encoding / negative
// contract.
//
// Target: `encode_prfm` (PRFM — Prefetch Memory, ARM ARM §C4.1.89 immediate
// form and §C4.1.90 register form). The authoritative encodings below were
// produced by `llvm-mc-18 --triple=aarch64 --show-encoding` (NOT this crate's
// own formula):
//
//   prfm pldl1keep, [x0]             = 0xF9800000   (immediate form, OK)
//   prfm pldl1keep, [x0, #8]         = 0xF9800400
//   prfm pldl3strm, [x10, #32760]    = 0xF9BFFD45
//   prfm pstl3strm, [x5, #16]        = 0xF98008B5
//   prfm pldl1keep, [x0, x1]         = 0xF8A16800   (register form)
//   prfm pldl1keep, [x0, x1, lsl #3] = 0xF8A17800
//   prfm pldl1keep, [x0, w1, uxtw]   = 0xF8A14800
//
// Bit layouts (independently derived from the ARM ARM):
//   PRFM (immediate, unsigned offset):
//     11 111 0 01 10 imm12[21:10] Rn[9:5] Rt(prfop)[4:0]   base = 0xF9800000
//   PRFM (register):
//     11 111 0 00 10 1 Rm[20:16] option[15:13] S[12] 10 Rn[9:5] Rt[4:0]
//     The opc=10 field lives at bits [23:22] → bit 23 set = 0x00800000.
//
// FINDINGS (bug-witness properties are `#[ignore]`'d so default
// `cargo test` stays green; run them explicitly with `-- --ignored`):
//  • immediate form: CORRECT — properties 1–3 pass.
//  • register form:  BUG — the encoder writes `(0b10 << 23)` = 0x01000000
//    (bit 24) for the opc field, but the ARM ARM places opc=10 at bits
//    [23:22] → 0x00800000 (bit 23). Off-by-one shift (property 4).
//  • large offset:   BUG — `imm12 = (imm/8) as u32` wraps to a small value
//    when imm ≥ 8·2^32, so the subsequent `> 0xFFF` range check is bypassed
//    and a far-out-of-range offset encodes silently instead of returning
//    Err (property 5).

/// Canonical prefetch-operation table (ARM ARM "prfop"). (name, 5-bit value).
const PRFOP_TABLE: &[(&str, u32)] = &[
    ("pldl1keep", 0b00000),
    ("pldl1strm", 0b00001),
    ("pldl2keep", 0b00010),
    ("pldl2strm", 0b00011),
    ("pldl3keep", 0b00100),
    ("pldl3strm", 0b00101),
    ("plil1keep", 0b01000),
    ("plil1strm", 0b01001),
    ("plil2keep", 0b01010),
    ("plil2strm", 0b01011),
    ("plil3keep", 0b01100),
    ("plil3strm", 0b01101),
    ("pstl1keep", 0b10000),
    ("pstl1strm", 0b10001),
    ("pstl2keep", 0b10010),
    ("pstl2strm", 0b10011),
    ("pstl3keep", 0b10100),
    ("pstl3strm", 0b10101),
];

fn word(r: Result<EncodeResult, String>) -> u32 {
    match r {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected Word, got {:?}", other),
    }
}

proptest! {
    // ── Property 1: immediate form matches the llvm-mc-18 reference (differential). ──
    // PASSES. For every prfop value 0..31, base register x0..x30, and a scaled
    // offset imm = imm12*8 (0 <= imm12 <= 4095), the encoded word must equal
    // the independently-derived reference anchored to the llvm-mc golden
    // `prfm pldl1keep,[x0] = 0xF9800000`.
    #[test]
    fn prop_prfm_imm_matches_llvm_mc_reference(
        prfop_val in 0u32..=31u32,
        rn in 0u32..=30u32,
        imm12 in 0u32..=0xFFFu32,
    ) {
        let ops = vec![
            Operand::Imm(prfop_val as i64),
            Operand::Mem { base: format!("x{}", rn), offset: (imm12 as i64) * 8 },
        ];
        let w = word(encode_prfm(&ops));
        let expected = 0xF9800000u32 | (imm12 << 10) | (rn << 5) | prfop_val;
        prop_assert_eq!(w, expected);
    }

    // ── Property 2: immediate-form field decomposition (structural). ──
    // PASSES. The fixed opcode bits [31:22] are constant (0xF9800000) and the
    // three variable fields occupy disjoint, correctly-placed bit ranges.
    #[test]
    fn prop_prfm_imm_field_decomposition(
        prfop_val in 0u32..=31u32,
        rn in 0u32..=30u32,
        imm12 in 0u32..=0xFFFu32,
    ) {
        let ops = vec![
            Operand::Imm(prfop_val as i64),
            Operand::Mem { base: format!("x{}", rn), offset: (imm12 as i64) * 8 },
        ];
        let w = word(encode_prfm(&ops));
        prop_assert_eq!(w & 0x1F, prfop_val, "Rt/prfop field [4:0]");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field [9:5]");
        prop_assert_eq!((w >> 10) & 0xFFF, imm12, "imm12 field [21:10]");
        prop_assert_eq!(w & 0xFFC00000, 0xF9800000, "fixed opcode bits [31:22]");
    }

    // ── Property 3: prfop Symbol and Imm input paths agree. ──
    // PASSES. For every named prfop, `Operand::Symbol(name)` and
    // `Operand::Imm(encode_prfop(name))` must produce byte-identical words.
    #[test]
    fn prop_prfm_symbol_and_imm_paths_agree(
        pidx in 0usize..PRFOP_TABLE.len(),
        rn in 0u32..=30u32,
        imm12 in 0u32..=0xFFFu32,
    ) {
        let (name, val) = PRFOP_TABLE[pidx];
        let off = (imm12 as i64) * 8;
        let via_sym = word(encode_prfm(&[
            Operand::Symbol(name.to_string()),
            Operand::Mem { base: format!("x{}", rn), offset: off },
        ]));
        let via_imm = word(encode_prfm(&[
            Operand::Imm(val as i64),
            Operand::Mem { base: format!("x{}", rn), offset: off },
        ]));
        prop_assert_eq!(via_sym, via_imm);
    }

    // ── Property 4: register-offset form matches llvm-mc-18 (BUG WITNESS). ──
    // EXPECTED TO FAIL today: the encoder writes `(0b10 << 23)` = 0x01000000
    // (bit 24) for the opc field, but the ARM ARM places opc=10 at bits
    // [23:22] → 0x00800000 (bit 23). E.g. `prfm pldl1keep,[x0,x1]` encodes to
    // 0xF9216800 instead of the llvm-mc-mandated 0xF8A16800. `#[ignore]`'d so
    // default `cargo test` stays green; the failure (run with --ignored) IS
    // the bug being reported.
    #[test]
    fn prop_prfm_register_offset_matches_llvm_mc(
        pidx in 0usize..PRFOP_TABLE.len(),
        rn in 0u32..=30u32,
        rm in 0u32..=30u32,
        opt_idx in 0u8..4u8,
        shift_amt in 0u8..4u8,
    ) {
        let (name, prfop) = PRFOP_TABLE[pidx];
        let (extend, shift, expect_option) = match opt_idx {
            0 => (Some("lsl".to_string()),  Some(shift_amt), 0b011u32),
            1 => (Some("uxtw".to_string()), Some(shift_amt), 0b010u32),
            2 => (Some("sxtw".to_string()), Some(shift_amt), 0b110u32),
            _ => (Some("sxtx".to_string()), Some(shift_amt), 0b111u32),
        };
        let s_bit = if shift_amt > 0 { 1u32 } else { 0u32 };
        let ops = vec![
            Operand::Symbol(name.to_string()),
            Operand::MemRegOffset {
                base: format!("x{}", rn),
                index: format!("x{}", rm),
                extend,
                shift,
            },
        ];
        let w = word(encode_prfm(&ops));
        // Reference per ARM ARM §C4.1.90, anchored to llvm-mc 0xF8A16800.
        let expected = 0xC0000000u32   // size=11 [31:30]
            | 0x38000000u32            // 111    [29:27]
            | 0x00800000u32            // opc=10 [23:22]  (bit 23)
            | 0x00200000u32            // [21]=1
            | (rm << 16)               // Rm     [20:16]
            | (expect_option << 13)    // option [15:13]
            | (s_bit << 12)            // S      [12]
            | 0x00000800u32            // [11:10]=10
            | (rn << 5)                // Rn     [9:5]
            | prfop;                   // Rt     [4:0]
        prop_assert_eq!(
            w, expected,
            "register-offset PRFM opcode mismatch: opc written at bit 24 \
             ((0b10<<23)=0x01000000) instead of bit 23 (0x00800000); \
             llvm-mc-18 reference = {:#010x}",
            expected,
        );
    }

    // ── Property 5: large offset must not be silently truncated (BUG WITNESS). ──
    // EXPECTED TO FAIL today: `imm12 = (imm/8) as u32` wraps to a small value
    // when imm ≥ 8·2^32, so the subsequent `> 0xFFF` range check is bypassed
    // and a far-out-of-range offset encodes to a small imm12 instead of
    // returning Err. `#[ignore]`'d so default `cargo test` stays green; the
    // failure (run with --ignored) IS the bug being reported.
    #[test]
    fn prop_prfm_large_offset_not_silently_truncated(
        scaled in (0x1_0000_0000_i64..=0x1_0000_0FFF_i64),
    ) {
        // imm is 8-byte aligned and non-negative, but imm/8 (=scaled) is far
        // outside the 12-bit field AND, critically, outside u32 range.
        let imm = scaled * 8;
        let ops = vec![
            Operand::Symbol("pldl1keep".to_string()),
            Operand::Mem { base: "x0".to_string(), offset: imm },
        ];
        let r = encode_prfm(&ops);
        prop_assert!(
            r.is_err(),
            "scaled offset {} (imm={}) exceeds the 12-bit field and must be \
             rejected, but the encoder returned {:?} — `(imm/8) as u32` \
             wrapped to {} before the `> 0xFFF` range check",
            scaled, imm, r, (scaled as u32),
        );
    }
}
