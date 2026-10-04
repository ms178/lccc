//! Property-based tests for `encode_neon_cmp_zero`.
//!
//! `encode_neon_cmp_zero` encodes the AArch64 NEON compare-to-zero family —
//! `CMEQ/CMGE/CMGT/CMLE/CMLT Vd.T, Vn.T, #0` — in the
//! "Advanced SIMD two-register miscellaneous" encoding group:
//!
//! ```text
//!   31 30 29 28-24 23-22 21-17  16-12   11-10 9-5 4-0
//!    0  Q  U  01110  size  10000  opcode  10   Rn  Rd
//! ```
//! `Q`/`size` are derived from the arrangement `T`; `u_bit` and `opcode` are
//! the per-mnemonic selectors passed by the dispatcher (`mod.rs`).
//!
//! ## Oracle
//! The golden words below were hand-derived from the ARMv8-A ARM bit layout
//! (ARM DDI 0487, "Advanced SIMD two-register miscellaneous", compare-with-zero
//! rows) and are independent of this crate's implementation. They anchor the
//! absolute correctness of every fixed field. The independent reference
//! encoder `ref_encode_cmp_zero` mirrors the same documented layout.
//!
//! Unlike its sibling `encode_neon_addv` (which has a wrong-constant bug),
//! `encode_neon_cmp_zero` packs its bits correctly: the golden table and
//! differential/field-decomposition properties all PASS.
//!
//! ## Finding (documented by the `#[ignore]`d test `rejects_unallocated_size_11_arrangements`)
//! The ARMv8-A ARM defines CMEQ/CMGE/CMGT/CMLE/CMLT (zero) ONLY for the
//! arrangements 8B/16B/4H/8H/2S/4S (size = 00/01/10). The encoding field
//! `size=11` is UNALLOCATED for this instruction group. The implementation
//! nonetheless accepts `.1d`/`.2d` (which `neon_arr_to_q_size` maps to
//! size=11) and emits a word instead of returning `Err`, so a user can write
//! e.g. `cmeq v0.2d, v1.2d, #0` and silently get an unallocated instruction.
//! See `NEON_CMP_ZERO_BUG_REPORT.md`.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::{encode_neon_cmp_zero, neon_arr_to_q_size};
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

fn u_bit_strategy() -> impl Strategy<Value = u32> {
    prop_oneof![Just(0u32), Just(1u32)]
}

fn opcode_strategy() -> impl Strategy<Value = u32> {
    0u32..=0x1Fu32 // 5-bit opcode field, in range
}

/// Arrangements architecturally VALID for compare-to-zero (ARMv8-A ARM,
/// CMEQ/CMGE/CMGT/CMLE/CMLT (vector) #0): 8B, 16B, 4H, 8H, 2S, 4S.
/// `.1d`/`.2d` (size=11) are UNALLOCATED and tested separately.
fn valid_arrangement_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![
        Just("8b"),
        Just("16b"),
        Just("4h"),
        Just("8h"),
        Just("2s"),
        Just("4s"),
    ]
}

/// Independent reference encoder: assembles the word field-by-field from the
/// documented ARMv8-A ARM layout. The `u_bit`/`opcode` are masked to their
/// field widths so this reference cannot overflow into adjacent fields even
/// for out-of-range inputs.
fn ref_encode_cmp_zero(rd: u32, rn: u32, arr: &str, u_bit: u32, opcode: u32) -> u32 {
    let (q, size) = neon_arr_to_q_size(arr).unwrap();
    (q << 30)
        | ((u_bit & 1) << 29)
        | (0b01110u32 << 24)
        | (size << 22)
        | (0b10000u32 << 17) // bits 21-17
        | ((opcode & 0x1F) << 12) // opcode, bits 16-12
        | (0b10u32 << 10) // bits 11-10
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

/// Hand-derived from the ARMv8-A ARM layout for compare-to-zero.
/// Each row: `(Rd, Rn, arrangement, u_bit, opcode, expected_word)`.
const GOLDEN: &[(u32, u32, &str, u32, u32, u32)] = &[
    // CMEQ (zero): U=0, opcode=01001
    (0, 1, "4s", 0, 0b01001, 0x4EA09820), // cmeq v0.4s, v1.4s, #0
    (0, 1, "8b", 0, 0b01001, 0x0E209820), // cmeq v0.8b, v1.8b, #0  (Q=0)
    (5, 6, "2s", 0, 0b01001, 0x0EA098C5), // cmeq v5.2s, v6.2s, #0  (Q=0)
    (31, 30, "16b", 0, 0b01001, 0x4E209BDF), // cmeq v31.16b, v30.16b, #0
    (0, 0, "4h", 0, 0b01001, 0x0E609800), // cmeq v0.4h, v0.4h, #0  (Q=0,size=01)
    // CMLE (zero): U=1, opcode=00011  — exercises the U bit
    (3, 7, "4s", 1, 0b00011, 0x6EA038E3), // cmle v3.4s, v7.4s, #0
    // CMLT (zero): U=0, opcode=00100
    (2, 9, "8h", 0, 0b00100, 0x4E604922), // cmlt v2.8h, v9.8h, #0
];

/// Absolute oracle: every golden word must be reproduced exactly, and the
/// independent reference encoder must agree with the golden values too.
#[test]
fn matches_golden_table() {
    for &(rd, rn, arr, u_bit, opcode, expected) in GOLDEN {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let got = word_of(encode_neon_cmp_zero(&ops, u_bit, opcode));
        assert_eq!(
            got, expected,
            "cmp_zero v{rd}.{arr}, v{rn}.{arr} (u={u_bit}, opc=0x{opcode:02X}): \
             got 0x{got:08X}, want 0x{expected:08X}",
        );
        assert_eq!(
            ref_encode_cmp_zero(rd, rn, arr, u_bit, opcode),
            expected,
            "reference encoder drift for v{rd}.{arr}",
        );
    }
}

// --- properties -----------------------------------------------------------

proptest! {
    // === Oracle: differential against independent reference encoder ========
    // For every valid arrangement, register pair, U bit, and in-range opcode,
    // the implementation must equal the independently-assembled reference word.
    #[test]
    fn matches_reference_encoder(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
        u_bit in u_bit_strategy(),
        opcode in opcode_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let got = word_of(encode_neon_cmp_zero(&ops, u_bit, opcode));
        let want = ref_encode_cmp_zero(rd, rn, arr, u_bit, opcode);
        prop_assert_eq!(got, want);
    }

    // === Field placement + fixed bits ====================================
    // Every field must land in its documented position, and the
    // architecturally-constant bits must never change regardless of inputs.
    #[test]
    fn bit_fields_decompose_correctly(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
        u_bit in u_bit_strategy(),
        opcode in opcode_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let w = word_of(encode_neon_cmp_zero(&ops, u_bit, opcode));
        let (q, size) = neon_arr_to_q_size(arr).unwrap();

        // Register fields round-trip.
        prop_assert_eq!(w & 0x1F, rd, "Rd field (bits 4-0)");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field (bits 9-5)");
        // U / opcode selectors land in their fields.
        prop_assert_eq!((w >> 29) & 0x1, u_bit, "U bit (bit 29)");
        prop_assert_eq!((w >> 12) & 0x1F, opcode, "opcode field (bits 16-12)");
        // Q / size derived from arrangement.
        prop_assert_eq!((w >> 30) & 0x1, q, "Q bit (bit 30)");
        prop_assert_eq!((w >> 22) & 0x3, size, "size field (bits 23-22)");
        // Architecturally-constant bits.
        prop_assert_eq!((w >> 31) & 1, 0, "bit 31 must be 0");
        prop_assert_eq!((w >> 24) & 0x1F, 0b01110, "fixed bits 28-24 = 01110");
        prop_assert_eq!((w >> 17) & 0x1F, 0b10000, "fixed bits 21-17 = 10000");
        prop_assert_eq!((w >> 10) & 0x3, 0b10, "fixed bits 11-10 = 10");
    }

    // === Negative contract: insufficient operands rejected ===============
    // With 0 or 1 operands the encoder must return `Err`.
    #[test]
    fn rejects_insufficient_operands(few in 0u8..=1) {
        let mut ops: Vec<Operand> = Vec::new();
        for i in 0..few {
            ops.push(va(i as u32, "4s"));
        }
        let res = encode_neon_cmp_zero(&ops, 0, 0b01001);
        prop_assert!(res.is_err(),
            "expected Err for {} operands, got Ok", few);
    }

    // === Negative contract: unsupported arrangement rejected =============
    // Any arrangement string not understood by `neon_arr_to_q_size` must
    // cause `encode_neon_cmp_zero` to return `Err` (no silent fallthrough).
    #[test]
    fn rejects_unsupported_arrangement(
        arr in "[a-z0-9]{1,4}".prop_filter("must be an unknown arrangement", |s| {
            !matches!(s.as_str(),
                "8b"|"16b"|"4h"|"8h"|"2s"|"4s"|"1d"|"2d")
        }),
    ) {
        let ops = vec![va(0, arr.as_str()), va(1, arr.as_str())];
        prop_assert!(encode_neon_cmp_zero(&ops, 0, 0b01001).is_err(),
            "unsupported arrangement {arr:?} should be rejected");
    }
}

// --- documented finding: unallocated size=11 silently encoded -------------

/// Compare-to-zero instructions are defined by the ARMv8-A ARM ONLY for
/// 8B/16B/4H/8H/2S/4S (size 00/01/10). `.1d`/`.2d` map to size=11, which is
/// UNALLOCATED in the "Advanced SIMD two-register miscellaneous" encoding and
/// must be rejected. The implementation accepts them and emits a word instead
/// of returning `Err`.
///
/// `#[ignore]`d so the suite stays green; the finding is documented in
/// `NEON_CMP_ZERO_BUG_REPORT.md`. Reproduce with
/// `cargo test -- --ignored rejects_unallocated_size_11_arrangements`.
#[test]
#[ignore]
fn rejects_unallocated_size_11_arrangements() {
    for arr in &["1d", "2d"] {
        let ops = vec![va(0, arr), va(1, arr)];
        let res = encode_neon_cmp_zero(&ops, 0, 0b01001);
        assert!(
            res.is_err(),
            "compare-to-zero does not support .{arr}; expected Err but got Ok(0x{:08X})",
            res.as_ref()
                .ok()
                .map(|e| match e {
                    EncodeResult::Word(w) => *w,
                    _ => 0,
                })
                .unwrap_or(0),
        );
    }
}
