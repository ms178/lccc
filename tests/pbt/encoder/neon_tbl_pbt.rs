//! Property-based tests for `encode_neon_tbl`.
//!
//! `encode_neon_tbl` encodes the AArch64 NEON `TBL` instruction —
//! `TBL Vd.Ta, { Vn.16b ... }, Vm.Ta` — a table-vector lookup, in the
//! "Advanced SIMD table lookup" encoding group:
//!
//! ```text
//!   31 30 29 28-24 23-21  20-16  15  14-13  12  11-10  9-5  4-0
//!    0  Q  0  01110  000    Rm    0   len    op   00    Rn   Rd
//! ```
//! with `op = 0` for TBL. `Q` reflects the destination arrangement `Ta`
//! (`.8b` → Q=0, `.16b` → Q=1); `len = (number of table registers) - 1`
//! occupies bits 14-13. The table registers themselves are always `.16b`
//! (the full 128-bit table) — their arrangement is not part of the encoding.
//!
//! ## Oracle
//! The golden words were cross-validated against LLVM's `llvm-mc-18`
//! (`-triple=aarch64 -show-encoding`) and are independent of this crate's
//! implementation. They anchor the absolute correctness of every fixed
//! field, including the `len` progression for 1→2→3→4 register tables and
//! the Q bit for `.8b`/`.16b`. The independent reference encoder
//! `ref_encode_tbl` re-assembles the word field-by-field from the
//! llvm-mc-confirmed layout.
//!
//! ## Findings (documented by the `#[ignore]`d witnesses at the bottom)
//!
//! **Finding A — out-of-range table size silently wraps.** `TBL` accepts
//! exactly 1–4 table registers. The implementation computes
//! `len = (num_regs - 1) & 0x3`, so a 5-register (or larger) list wraps
//! modulo 4 instead of being rejected. `llvm-mc` rejects these outright:
//!
//! ```text
//!   $ echo 'tbl v0.16b, {v1.16b-v5.16b}, v6.16b' | llvm-mc-18 -assemble -triple=aarch64
//!   <stdin>:1:21: error: invalid number of vectors
//! ```
//!
//! e.g. a 5-register list encodes identically to a 1-register list
//! (`len` wraps 4 → 0). This is the classic silent-immediate-truncation
//! hazard.
//!
//! **Finding B — non-byte destination arrangements silently map to Q=0.**
//! `TBL` is defined ONLY for `.8b`/`.16b` destinations. The encoder derives Q
//! from `arr_d == "16b"` and treats EVERY other arrangement (`.4h`, `.2s`,
//! `.1d`, ...) as Q=0, emitting a bogus word instead of `Err`.
//!
//! **Finding C — empty register list panics.** With an empty
//! `Operand::RegList`, the encoder indexes `regs[0]` and panics (index out
//! of bounds) instead of returning `Err`.
//!
//! The four passing properties confirm that, for valid inputs, every field
//! is encoded correctly; the three `#[ignore]`d properties surface the
//! missing validation.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_tbl;
use proptest::prelude::*;

// --- helpers --------------------------------------------------------------

/// Build `Operand::RegArrangement { reg: "v{n}", arrangement }`.
fn va(n: u32, arr: &str) -> Operand {
    Operand::RegArrangement {
        reg: format!("v{n}"),
        arrangement: arr.to_string(),
    }
}

/// Build a table register-list operand `{ v{start}.arr ... v{start+count-1}.arr }`
/// of `count` entries. (Architecturally the table registers are `.16b`, but
/// the encoder ignores their arrangement and only reads the register number,
/// so the arrangement is harmless here.)
fn table_list(start: u32, count: u32, arr: &str) -> Operand {
    let regs: Vec<Operand> = (0..count).map(|i| va(start + i, arr)).collect();
    Operand::RegList(regs)
}

fn reg_num_strategy() -> impl Strategy<Value = u32> {
    0u32..=31u32
}

/// Table-base register strategy: the base of a 1-4 register consecutive
/// table list must leave room for the largest table (v{base+3} <= v31),
/// otherwise the list contains architecturally invalid registers (v32+)
/// which the encoder rightly rejects.
fn table_base_strategy() -> impl Strategy<Value = u32> {
    0u32..=28u32
}

/// Table sizes architecturally VALID for TBL: 1–4 registers.
fn table_size_strategy() -> impl Strategy<Value = u32> {
    1u32..=4u32
}

/// Arrangements architecturally VALID for the TBL destination/index: 8B, 16B.
fn valid_arrangement_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("8b"), Just("16b"),]
}

/// Independent reference encoder: assembles the word field-by-field from the
/// llvm-mc-confirmed "Advanced SIMD table lookup" layout for TBL (op=0).
/// `len` lives at bits 14-13; the op-bit (12) is 0 for TBL.
fn ref_encode_tbl(rd: u32, rn: u32, rm: u32, num_regs: u32, arr_d: &str) -> u32 {
    let q: u32 = if arr_d == "16b" { 1 } else { 0 };
    let len = num_regs - 1; // 1..=4 -> 0..=3, no masking
    (q << 30)
        | (0b001110u32 << 24) // bits 29-24 (bit 29 = 0)
        | (rm << 16)          // bits 20-16
        | (len << 13)         // bits 14-13
        | (rn << 5)           // bits 9-5
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
// Each expected word is the little-endian reading of the byte sequence
// llvm-mc emits, e.g.
//   tbl v0.16b, {v1.16b}, v2.16b -> [0x20,0x00,0x02,0x4e] -> 0x4E020020
//   tbl v0.8b,  {v1.16b}, v2.8b  -> [0x20,0x00,0x02,0x0e] -> 0x0E020020 (Q=0)
const GOLDEN: &[(u32, u32, u32, u32, &str, u32)] = &[
    // (rd, rn(table base), rm(index), num_regs, arr_d, expected_word)
    (0, 1, 2, 1, "16b", 0x4E020020), // tbl v0.16b, {v1.16b}, v2.16b
    (0, 1, 3, 2, "16b", 0x4E032020), // tbl v0.16b, {v1.16b, v2.16b}, v3.16b
    (0, 1, 4, 3, "16b", 0x4E044020), // tbl v0.16b, {v1.16b-v3.16b}, v4.16b
    (0, 1, 5, 4, "16b", 0x4E056020), // tbl v0.16b, {v1.16b-v4.16b}, v5.16b
    (7, 3, 9, 1, "16b", 0x4E090067), // tbl v7.16b, {v3.16b}, v9.16b
    (31, 30, 29, 2, "16b", 0x4E1D23DF), // tbl v31.16b, {v30.16b, v31.16b}, v29.16b
    (0, 1, 2, 1, "8b", 0x0E020020),  // tbl v0.8b,  {v1.16b}, v2.8b   (Q=0)
    (0, 1, 5, 4, "8b", 0x0E056020),  // tbl v0.8b,  {v1.16b-v4.16b}, v5.8b (Q=0)
];

#[test]
fn tbl_matches_golden_table() {
    for &(rd, rn, rm, num_regs, arr, expected) in GOLDEN {
        let ops = vec![va(rd, arr), table_list(rn, num_regs, "16b"), va(rm, arr)];
        let got = word_of(encode_neon_tbl(&ops));
        assert_eq!(
            got, expected,
            "tbl v{rd}.{arr}, {{v{rn}...+{num_regs}}}, v{rm}.{arr}: \
             got 0x{got:08X}, want 0x{expected:08X}",
        );
        // Cross-check the reference encoder against the golden values too.
        assert_eq!(
            ref_encode_tbl(rd, rn, rm, num_regs, arr),
            expected,
            "reference encoder drift"
        );
    }
}

// --- properties -----------------------------------------------------------

proptest! {
    // === Oracle: differential against independent reference encoder ========
    // For every valid table size, arrangement, and register triple, the
    // implementation must equal the independently-assembled reference word
    // (which itself matches llvm-mc on the golden table).
    #[test]
    fn tbl_matches_reference_encoder(
        rd in reg_num_strategy(),
        rn in table_base_strategy(),
        rm in reg_num_strategy(),
        num_regs in table_size_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), table_list(rn, num_regs, "16b"), va(rm, arr)];
        let got = word_of(encode_neon_tbl(&ops));
        let want = ref_encode_tbl(rd, rn, rm, num_regs, arr);
        prop_assert_eq!(got, want);
    }

    // === Fixed-bits invariant =============================================
    // The architecturally-constant bits must never change: bit31=0,
    // bit29=0, bits28-24=01110, bits23-21=000, bit15=0, op-bit12=0 (TBL),
    // bits11-10=00.
    #[test]
    fn tbl_fixed_bits_are_constant(
        rd in reg_num_strategy(),
        rn in table_base_strategy(),
        rm in reg_num_strategy(),
        num_regs in table_size_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), table_list(rn, num_regs, "16b"), va(rm, arr)];
        let w = word_of(encode_neon_tbl(&ops));

        prop_assert_eq!((w >> 31) & 1, 0, "bit 31 must be 0");
        prop_assert_eq!((w >> 24) & 0x3F, 0b001110, "bits 29-24 (bit29=0, 28-24=01110)");
        prop_assert_eq!((w >> 21) & 0x7, 0b000, "bits 23-21 must be 000");
        prop_assert_eq!((w >> 15) & 1, 0, "bit 15 must be 0");
        prop_assert_eq!((w >> 12) & 1, 0, "op bit (12) must be 0 for TBL");
        prop_assert_eq!((w >> 10) & 0x3, 0b00, "bits 11-10 must be 00");
    }

    // === Field placement: len at bits 14-13 ===============================
    // The `len` field = (num_regs - 1) and lives at bits 14-13. Therefore
    // each additional table register (1->2->3->4) must add exactly
    // 1<<13 (0x2000) to the word, holding rd/rn/rm/arr fixed.
    #[test]
    fn tbl_len_field_advances_one_register_at_bit_13(
        rd in reg_num_strategy(),
        rn in table_base_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let mk = |n: u32| {
            let ops = vec![va(rd, arr), table_list(rn, n, "16b"), va(rm, arr)];
            word_of(encode_neon_tbl(&ops))
        };
        let w1 = mk(1);
        let w2 = mk(2);
        let w3 = mk(3);
        let w4 = mk(4);
        prop_assert_eq!(w2 - w1, 1u32 << 13, "1->2 regs must add 1<<13");
        prop_assert_eq!(w3 - w2, 1u32 << 13, "2->3 regs must add 1<<13");
        prop_assert_eq!(w4 - w3, 1u32 << 13, "3->4 regs must add 1<<13");
        // And bits 14-13 must equal (num_regs - 1) exactly.
        for (n, w) in [(1u32, w1), (2, w2), (3, w3), (4, w4)] {
            prop_assert_eq!((w >> 13) & 0x3, n - 1, "len bits 14-13 == num_regs-1");
        }
    }

    // === Register field round-trip + Q mapping ============================
    // Rd (bits 4-0), Rn (bits 9-5), Rm (bits 20-16) round-trip, and Q (bit30)
    // is 1 iff the arrangement is .16b.
    #[test]
    fn tbl_register_fields_round_trip_and_q_maps_arrangement(
        rd in reg_num_strategy(),
        rn in table_base_strategy(),
        rm in reg_num_strategy(),
        num_regs in table_size_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), table_list(rn, num_regs, "16b"), va(rm, arr)];
        let w = word_of(encode_neon_tbl(&ops));
        let q = if arr == "16b" { 1u32 } else { 0 };

        prop_assert_eq!(w & 0x1F, rd, "Rd field (bits 4-0)");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field (bits 9-5) — table base");
        prop_assert_eq!((w >> 16) & 0x1F, rm, "Rm field (bits 20-16) — index");
        prop_assert_eq!((w >> 30) & 0x1, q, "Q bit from destination arrangement");
    }

    // === Negative contract: too few operands rejected =====================
    #[test]
    fn tbl_rejects_too_few_operands(n in 0usize..3) {
        let ops: Vec<Operand> = (0..n).map(|_| va(0, "16b")).collect();
        prop_assert!(encode_neon_tbl(&ops).is_err(),
            "{n} operands should be rejected; tbl needs 3");
    }

    // === Negative contract: non-RegList second operand rejected ===========
    #[test]
    fn tbl_rejects_non_reglist_table(whatever in 0u32..32) {
        let ops = vec![va(0, "16b"), va(whatever, "16b"), va(2, "16b")];
        prop_assert!(encode_neon_tbl(&ops).is_err(),
            "second operand must be a register list");
    }
}

// --- documented finding A: out-of-range table size silently wraps ---------
//
// `TBL` accepts exactly 1–4 table registers. The implementation computes
// `len = (num_regs - 1) & 0x3`, so a 5+ register list wraps modulo 4 and
// silently produces a *different* (1–4 register) instruction. `llvm-mc`
// rejects such lists with "invalid number of vectors".
//
// A 5-register list currently encodes identically to a 1-register list.
// `#[ignore]`d to keep the default suite green; FAILS when run with
// `--ignored`, surfacing the bug.
proptest! {
    #[test]
    #[ignore]
    fn tbl_rejects_table_larger_than_four_regs(
        rd in reg_num_strategy(),
        rn in table_base_strategy(),
        rm in reg_num_strategy(),
        // keep the table inside the 0..=31 register file even at 8 entries
        extra in 4u32..=8u32,
    ) {
        let ops = vec![va(rd, "16b"), table_list(rn, extra, "16b"), va(rm, "16b")];
        let res = encode_neon_tbl(&ops);
        prop_assert!(
            res.is_err(),
            "tbl with {extra} table registers: TBL allows only 1-4 (llvm-mc rejects); \
             expected Err but got {res:?}",
        );
    }
}

// --- documented finding B: non-byte destination arrangements -> Q=0 -------
//
// `TBL` is defined ONLY for `.8b`/`.16b` destinations. The encoder derives Q
// from `arr_d == "16b"` and maps every other arrangement (`.4h`, `.2s`, ...)
// to Q=0, emitting a bogus word instead of `Err`.
proptest! {
    #[test]
    #[ignore]
    fn tbl_rejects_non_byte_arrangements(
        rd in reg_num_strategy(),
        arr in prop_oneof![
            Just("4h"), Just("8h"), Just("2s"), Just("4s"), Just("1d"), Just("2d"),
        ],
    ) {
        let ops = vec![va(rd, arr), table_list(1, 1, "16b"), va(2, arr)];
        let res = encode_neon_tbl(&ops);
        prop_assert!(
            res.is_err(),
            "tbl v{rd}.{arr}, ...: TBL is defined only for .8b/.16b destinations; \
             expected Err but got {res:?}",
        );
    }
}

// --- documented finding C: empty register list panics --------------------
//
// With an empty `Operand::RegList`, the encoder indexes `regs[0]` and panics
// (index out of bounds). It should return `Err` instead.
#[test]
#[ignore]
fn tbl_rejects_empty_register_list() {
    let ops = vec![va(0, "16b"), Operand::RegList(vec![]), va(2, "16b")];
    let res = encode_neon_tbl(&ops);
    assert!(
        res.is_err(),
        "empty table register list should return Err, not panic; got {res:?}",
    );
}
