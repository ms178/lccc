//! Property-based tests for `encode_neon_zip_uzp`.
//!
//! `encode_neon_zip_uzp` encodes the AArch64 NEON permute family —
//! `UZP1/UZP2/ZIP1/ZIP2/TRN1/TRN2 Vd.T, Vn.T, Vm.T` — in the
//! "Advanced SIMD three same" encoding group, permute sub-class:
//!
//! ```text
//!   31 30 29 28-24 23-22 21  20-16 15 14-12  11-10 9-5 4-0
//!    0  Q  0  01110  size  0  Rm    0  opcode  10   Rn  Rd
//! ```
//! `Q`/`size` are derived from the arrangement `T`; `op_bits` is the 3-bit
//! `opcode` selector (UZP1=001, TRN1=010, ZIP1=011, UZP2=101, TRN2=110,
//! ZIP2=111). The boolean `is_zip` parameter is documented below.
//!
//! ## Oracle
//! The golden words were cross-validated against LLVM's `llvm-mc-18`
//! (`-triple=aarch64 -show-encoding`) and are independent of this crate's
//! implementation; they anchor the absolute correctness of every fixed field
//! for all six mnemonics (including the `.2d` / size=11 case). The
//! independent reference encoder `ref_encode_zip_uzp` re-assembles the word
//! field-by-field from the documented ARMv8-A ARM layout.
//!
//! ## Finding A — dead parameter (passing property `is_zip_does_not_affect_encoding`)
//! The `is_zip: bool` parameter is entirely unused (its binding is
//! `_is_zip`). ZIP/UZP/TRN are differentiated solely by `op_bits`; the
//! `is_zip` flag never influences the emitted word. This is harmless (every
//! caller in `mod.rs` passes `false`) but the parameter is dead weight in
//! the public API. The property documents it as a stable, intentional no-op.
//!
//! ## Finding B (documented by the `#[ignore]`d test `rejects_unallocated_1d_arrangement`)
//! The permute instructions are defined by the ARMv8-A ARM ONLY for the
//! arrangements 8B/16B/4H/8H/2S/4S/**2D**. The `.1d` arrangement encodes to
//! `size=11, Q=0`, which is UNALLOCATED (UNDEFINED): `llvm-mc` rejects it:
//!
//! ```text
//!   $ echo 'uzp1 v0.1d, v1.1d, v2.1d' | llvm-mc-18 -assemble -triple=aarch64
//!   <stdin>:1:6: error: invalid operand for instruction
//! ```
//!
//! (`.2d`, i.e. `size=11, Q=1`, IS valid and is covered by the golden table.)
//! `encode_neon_zip_uzp` delegates arrangement parsing to
//! `neon_arr_to_q_size`, which maps `1d`→(Q=0,size=11). The encoder then
//! emits an UNALLOCATED instruction word instead of returning `Err`. The
//! passing properties confirm every field is encoded correctly for the valid
//! arrangements; the failing `#[ignore]`d property documents the missing
//! range check. See `NEON_ZIP_UZP_BUG_REPORT.md`.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::{encode_neon_zip_uzp, neon_arr_to_q_size};
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

/// The six architecturally-valid permute `op_bits` values.
fn op_bits_strategy() -> impl Strategy<Value = u32> {
    prop_oneof![
        Just(0b001u32), // UZP1
        Just(0b010u32), // TRN1
        Just(0b011u32), // ZIP1
        Just(0b101u32), // UZP2
        Just(0b110u32), // TRN2
        Just(0b111u32), // ZIP2
    ]
}

/// Arrangements architecturally VALID for UZP/ZIP/TRN (ARMv8-A ARM, permute
/// three-same): 8B, 16B, 4H, 8H, 2S, 4S, 2D. `.1d` (size=11, Q=0) is
/// UNALLOCATED and tested separately; it is deliberately EXCLUDED here.
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

/// Independent reference encoder: assembles the word field-by-field from the
/// documented ARMv8-A ARM "three same, permute" layout. Does NOT share any
/// buggy path; `op_bits` is masked to 3 bits so an out-of-range selector
/// cannot bleed into bit 15.
fn ref_encode_zip_uzp(rd: u32, rn: u32, rm: u32, arr: &str, op_bits: u32) -> u32 {
    let (q, size) = neon_arr_to_q_size(arr).unwrap();
    (q << 30)
        | (0b01110u32 << 24) // bits 28-24; bit 29 (U) = 0
        | (size << 22)       // bits 23-22
        | (rm << 16)         // bits 20-16; bit 21 = 0
        | ((op_bits & 0x7) << 12) // bits 14-12; bit 15 = 0
        | (0b10u32 << 10)    // bits 11-10
        | (rn << 5)          // bits 9-5
        | rd // bits 4-0
}

fn word_of(res: Result<EncodeResult, String>) -> u32 {
    match res {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {other:?}"),
    }
}

// --- golden table (absolute oracle, cross-validated with llvm-mc-18) ------
//
// Each expected word is the big-endian reading of the little-endian byte
// sequence llvm-mc emits, e.g.
//   uzp1 v0.4s, v1.4s, v2.4s -> [0x20,0x18,0x82,0x4e] -> 0x4E821820
// `op_bits` is the mnemonic selector (UZP1=001, TRN1=010, ZIP1=011,
// UZP2=101, TRN2=110, ZIP2=111).
const GOLDEN: &[(u32, u32, u32, &str, u32, u32)] = &[
    // (Rd, Rn, Rm, arrangement, op_bits, expected_word)
    (0, 1, 2, "4s", 0b001, 0x4E821820), // uzp1 v0.4s,  v1.4s,  v2.4s   (Q=1,size=10)
    (0, 1, 2, "8b", 0b011, 0x0E023820), // zip1 v0.8b,  v1.8b,  v2.8b   (Q=0,size=00)
    (5, 6, 7, "2d", 0b101, 0x4EC758C5), // uzp2 v5.2d,  v6.2d,  v7.2d   (Q=1,size=11)
    (3, 4, 5, "4h", 0b010, 0x0E452883), // trn1 v3.4h,  v4.4h,  v5.4h   (Q=0,size=01)
    (31, 30, 29, "16b", 0b111, 0x4E1D7BDF), // zip2 v31.16b,v30.16b,v29.16b (Q=1,size=00)
    (0, 1, 2, "2s", 0b110, 0x0E826820), // trn2 v0.2s,  v1.2s,  v2.2s   (Q=0,size=10)
    (0, 1, 2, "2d", 0b001, 0x4EC21820), // uzp1 v0.2d,  v1.2d,  v2.2d   (Q=1,size=11)
];

#[test]
fn zip_uzp_matches_golden_table() {
    for &(rd, rn, rm, arr, op_bits, expected) in GOLDEN {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let got = word_of(encode_neon_zip_uzp(&ops, op_bits, false));
        assert_eq!(
            got, expected,
            "permute v{rd}.{arr}, v{rn}.{arr}, v{rm}.{arr} (op_bits=0b{op_bits:03b}): \
             got 0x{got:08X}, want 0x{expected:08X}",
        );
        // Cross-check the reference encoder against the golden values too.
        assert_eq!(
            ref_encode_zip_uzp(rd, rn, rm, arr, op_bits),
            expected,
            "reference encoder drift",
        );
    }
}

// --- properties -----------------------------------------------------------

proptest! {
    // === Oracle: differential against independent reference encoder ========
    // For every valid arrangement, register triple, and valid op_bits, the
    // implementation must equal the independently-assembled reference word
    // (which itself matches llvm-mc-18 on the golden table).
    #[test]
    fn zip_uzp_matches_reference_encoder(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
        op_bits in op_bits_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let got = word_of(encode_neon_zip_uzp(&ops, op_bits, false));
        let want = ref_encode_zip_uzp(rd, rn, rm, arr, op_bits);
        prop_assert_eq!(got, want);
    }

    // === Field placement + fixed bits ====================================
    // Every field must land in its documented position, and the
    // architecturally-constant bits must never change regardless of inputs.
    #[test]
    fn bit_fields_decompose_correctly(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
        op_bits in op_bits_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let w = word_of(encode_neon_zip_uzp(&ops, op_bits, false));
        let (q, size) = neon_arr_to_q_size(arr).unwrap();

        // Register fields round-trip.
        prop_assert_eq!(w & 0x1F, rd, "Rd field (bits 4-0)");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field (bits 9-5)");
        prop_assert_eq!((w >> 16) & 0x1F, rm, "Rm field (bits 20-16)");
        // op_bits selector lands in its field.
        prop_assert_eq!((w >> 12) & 0x7, op_bits, "opcode field (bits 14-12)");
        // Q / size derived from arrangement.
        prop_assert_eq!((w >> 30) & 0x1, q, "Q bit (bit 30)");
        prop_assert_eq!((w >> 22) & 0x3, size, "size field (bits 23-22)");
        // Architecturally-constant bits.
        prop_assert_eq!((w >> 31) & 1, 0, "bit 31 must be 0");
        prop_assert_eq!((w >> 29) & 1, 0, "U bit (bit 29) must be 0");
        prop_assert_eq!((w >> 24) & 0x1F, 0b01110, "fixed bits 28-24 = 01110");
        prop_assert_eq!((w >> 21) & 1, 0, "bit 21 must be 0");
        prop_assert_eq!((w >> 15) & 1, 0, "bit 15 must be 0");
        prop_assert_eq!((w >> 10) & 0x3, 0b10, "fixed bits 11-10 = 10");
    }

    // === Finding A: dead `is_zip` parameter is a no-op ===================
    // Toggling `is_zip` must not change the encoded word (the parameter is
    // unused). This documents the dead parameter as a stable behavior.
    #[test]
    fn is_zip_does_not_affect_encoding(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
        op_bits in op_bits_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let with_true = encode_neon_zip_uzp(&ops, op_bits, true);
        let with_false = encode_neon_zip_uzp(&ops, op_bits, false);
        // `is_zip` is a dead parameter; both results must be byte-identical.
        // Compare the Ok-wrapped words (valid arrangements always succeed).
        let t = word_of(with_true);
        let f = word_of(with_false);
        prop_assert_eq!(t, f,
            "is_zip flag must not affect the encoding (it is unused), yet words differ");
    }

    // === Negative contract: insufficient operands rejected ===============
    // With 0, 1, or 2 operands the encoder must return `Err`.
    #[test]
    fn rejects_insufficient_operands(few in 0u8..=2) {
        let mut ops: Vec<Operand> = Vec::new();
        for i in 0..few {
            ops.push(va(i as u32, "4s"));
        }
        let res = encode_neon_zip_uzp(&ops, 0b001, false);
        prop_assert!(res.is_err(),
            "expected Err for {few} operands; permute needs 3, got Ok");
    }

    // === Negative contract: unsupported arrangement rejected =============
    // Any arrangement string not understood by `neon_arr_to_q_size` must
    // cause `encode_neon_zip_uzp` to return `Err` (no silent fallthrough).
    #[test]
    fn rejects_unsupported_arrangement(
        arr in "[a-z0-9]{1,4}".prop_filter("must be an unknown arrangement", |s| {
            !matches!(s.as_str(),
                "8b"|"16b"|"4h"|"8h"|"2s"|"4s"|"1d"|"2d")
        }),
    ) {
        let ops = vec![va(0, arr.as_str()), va(1, arr.as_str()), va(2, arr.as_str())];
        prop_assert!(encode_neon_zip_uzp(&ops, 0b001, false).is_err(),
            "unsupported arrangement {arr:?} should be rejected");
    }
}

// --- documented finding: unallocated .1d silently encoded -----------------
//
// The permute instructions (UZP/ZIP/TRN) are defined by the ARMv8-A ARM ONLY
// for 8B/16B/4H/8H/2S/4S/2D. The `.1d` arrangement (size=11, Q=0) is
// UNALLOCATED and `llvm-mc` rejects it (`.2d`, size=11 Q=1, IS valid).
// The implementation accepts `.1d` and emits a word instead of returning
// `Err`.
//
// `#[ignore]`d to keep the default suite green, following this module's
// established convention (see `neon_rev64_pbt.rs`). It FAILS when run with
// `--ignored`, surfacing the bug. See `NEON_ZIP_UZP_BUG_REPORT.md`.
proptest! {
    #[test]
    #[ignore]
    fn rejects_unallocated_1d_arrangement(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        op_bits in op_bits_strategy(),
    ) {
        let ops = vec![va(rd, "1d"), va(rn, "1d"), va(rm, "1d")];
        let res = encode_neon_zip_uzp(&ops, op_bits, false);
        prop_assert!(
            res.is_err(),
            "permute v{rd}.1d, v{rn}.1d, v{rm}.1d (op_bits=0b{op_bits:03b}): \
             size=11,Q=0 is UNDEFINED for UZP/ZIP/TRN (llvm-mc rejects .1d); \
             expected Err but got {res:?}",
        );
    }
}
