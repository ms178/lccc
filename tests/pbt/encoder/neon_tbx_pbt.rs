//! Property-based tests for `encode_neon_tbx`.
//!
//! `encode_neon_tbx` encodes the AArch64 NEON `TBX` instruction —
//! `TBX Vd.Ta, { Vn.16b ... }, Vm.Ta` — a table-vector lookup with insert
//! (out-of-range indices leave the destination lane untouched), in the
//! "Advanced SIMD table lookup" encoding group:
//!
//! ```text
//!   31 30 29 28-24 23-21  20-16  15  14-13  12  11-10  9-5  4-0
//!    0  Q  0  01110  000    Rm    0   len    op   00    Rn   Rd
//! ```
//! with `op = 1` for TBX (vs `op = 0` for TBL). `Q` reflects the
//! destination arrangement `Ta` (`.8b` → Q=0, `.16b` → Q=1);
//! `len = (number of table registers) - 1` occupies bits 14-13.
//!
//! ## Oracle
//! The golden words were derived field-by-field from the same
//! llvm-mc-18-confirmed layout as the sibling `neon_tbl_pbt` suite, with
//! the single distinguishing bit `op` (bit 12) set to 1. The independent
//! reference encoder `ref_encode_tbx` re-assembles the word field-by-field
//! and is cross-checked against every golden value below.
//!
//! ## Findings (documented by the `#[ignore]`d witnesses at the bottom)
//!
//! **Finding A — out-of-range table size silently wraps.** `TBX` accepts
//! exactly 1–4 table registers. The implementation computes
//! `len = (num_regs - 1) & 0x3`, so a 5-register (or larger) list wraps
//! modulo 4 instead of being rejected. A 5-register list encodes identically
//! to a 1-register list (`len` wraps 4 → 0). `llvm-mc` rejects these:
//!
//! ```text
//!   $ echo 'tbx v0.16b, {v1.16b-v5.16b}, v6.16b' | llvm-mc-18 -assemble -triple=aarch64
//!   <stdin>:1:21: error: invalid number of vectors
//! ```
//!
//! **Finding B — non-byte destination arrangements silently map to Q=0.**
//! `TBX` is defined ONLY for `.8b`/`.16b` destinations. The encoder derives Q
//! from `arr_d == "16b"` and treats EVERY other arrangement (`.4h`, `.2s`,
//! `.1d`, ...) as Q=0, emitting a bogus word instead of `Err`.
//!
//! **Finding C — empty register list panics.** With an empty
//! `Operand::RegList`, the encoder indexes `regs[0]` and panics (index out
//! of bounds) instead of returning `Err`.
//!
//! The five passing properties confirm that, for valid inputs, every field
//! (including the TBX `op=1` bit) is encoded correctly; the three
//! `#[ignore]`d properties surface the missing validation.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_tbx;
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

/// Table sizes architecturally VALID for TBX: 1–4 registers.
fn table_size_strategy() -> impl Strategy<Value = u32> {
    1u32..=4u32
}

/// Arrangements architecturally VALID for the TBX destination/index: 8B, 16B.
fn valid_arrangement_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("8b"), Just("16b"),]
}

/// Independent reference encoder: assembles the word field-by-field from the
/// llvm-mc-confirmed "Advanced SIMD table lookup" layout for TBX (op=1).
/// `len` lives at bits 14-13; the op-bit (12) is 1 for TBX.
fn ref_encode_tbx(rd: u32, rn: u32, rm: u32, num_regs: u32, arr_d: &str) -> u32 {
    let q: u32 = if arr_d == "16b" { 1 } else { 0 };
    let len = num_regs - 1; // 1..=4 -> 0..=3, no masking
    (q << 30)
        | (0b001110u32 << 24) // bits 29-24 (bit 29 = 0)
        | (rm << 16)          // bits 20-16
        | (len << 13)         // bits 14-13
        | (1u32 << 12)        // bits 12: op = 1 for TBX
        | (rn << 5)           // bits 9-5
        | rd // bits 4-0
}

fn word_of(res: Result<EncodeResult, String>) -> u32 {
    match res {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {other:?}"),
    }
}

// --- golden table (absolute oracle) ---------------------------------------
//
// Each expected word is the TBX layout (op bit 12 set) assembled
// field-by-field; it equals the corresponding TBL word | 0x1000.
//   tbx v0.16b, {v1.16b}, v2.16b       -> 0x4E021020
//   tbx v0.16b, {v1.16b, v2.16b}, v3   -> 0x4E031020
//   tbx v0.16b, {v1.16b-v3.16b}, v4    -> 0x4E045020
//   tbx v0.16b, {v1.16b-v4.16b}, v5    -> 0x4E057020
//   tbx v7.16b, {v3.16b}, v9.16b       -> 0x4E091067
//   tbx v31.16b, {v30,v31.16b}, v29    -> 0x4E1D33DF
//   tbx v0.8b,  {v1.16b}, v2.8b        -> 0x0E021020 (Q=0)
//   tbx v0.8b,  {v1.16b-v4.16b}, v5    -> 0x0E057020 (Q=0)
const GOLDEN: &[(u32, u32, u32, u32, &str, u32)] = &[
    // (rd, rn(table base), rm(index), num_regs, arr_d, expected_word)
    (0, 1, 2, 1, "16b", 0x4E021020),
    (0, 1, 3, 2, "16b", 0x4E033020),
    (0, 1, 4, 3, "16b", 0x4E045020),
    (0, 1, 5, 4, "16b", 0x4E057020),
    (7, 3, 9, 1, "16b", 0x4E091067),
    (31, 30, 29, 2, "16b", 0x4E1D33DF),
    (0, 1, 2, 1, "8b", 0x0E021020),
    (0, 1, 5, 4, "8b", 0x0E057020),
];

#[test]
fn tbx_matches_golden_table() {
    for &(rd, rn, rm, num_regs, arr, expected) in GOLDEN {
        let ops = vec![va(rd, arr), table_list(rn, num_regs, "16b"), va(rm, arr)];
        let got = word_of(encode_neon_tbx(&ops));
        assert_eq!(
            got, expected,
            "tbx v{rd}.{arr}, {{v{rn}...+{num_regs}}}, v{rm}.{arr}: \
             got 0x{got:08X}, want 0x{expected:08X}",
        );
        // Cross-check the reference encoder against the golden values too.
        assert_eq!(
            ref_encode_tbx(rd, rn, rm, num_regs, arr),
            expected,
            "reference encoder drift",
        );
    }
}

// --- properties -----------------------------------------------------------

proptest! {
    // === Oracle: differential against independent reference encoder ========
    // For every valid table size, arrangement, and register triple, the
    // implementation must equal the independently-assembled reference word
    // (which itself matches the golden table).
    #[test]
    fn tbx_matches_reference_encoder(
        rd in reg_num_strategy(),
        rn in table_base_strategy(),
        rm in reg_num_strategy(),
        num_regs in table_size_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), table_list(rn, num_regs, "16b"), va(rm, arr)];
        let got = word_of(encode_neon_tbx(&ops));
        let want = ref_encode_tbx(rd, rn, rm, num_regs, arr);
        prop_assert_eq!(got, want);
    }

    // === Fixed-bits invariant =============================================
    // The architecturally-constant bits must never change: bit31=0,
    // bit29=0, bits28-24=01110, bits23-21=000, bit15=0, op-bit12=1 (TBX),
    // bits11-10=00.
    #[test]
    fn tbx_fixed_bits_are_constant(
        rd in reg_num_strategy(),
        rn in table_base_strategy(),
        rm in reg_num_strategy(),
        num_regs in table_size_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), table_list(rn, num_regs, "16b"), va(rm, arr)];
        let w = word_of(encode_neon_tbx(&ops));

        prop_assert_eq!((w >> 31) & 1, 0, "bit 31 must be 0");
        prop_assert_eq!((w >> 24) & 0x3F, 0b001110, "bits 29-24 (bit29=0, 28-24=01110)");
        prop_assert_eq!((w >> 21) & 0x7, 0b000, "bits 23-21 must be 000");
        prop_assert_eq!((w >> 15) & 1, 0, "bit 15 must be 0");
        prop_assert_eq!((w >> 12) & 1, 1, "op bit (12) must be 1 for TBX");
        prop_assert_eq!((w >> 10) & 0x3, 0b00, "bits 11-10 must be 00");
    }

    // === Field placement: len at bits 14-13 ===============================
    // The `len` field = (num_regs - 1) and lives at bits 14-13. Therefore
    // each additional table register (1->2->3->4) must add exactly
    // 1<<13 (0x2000) to the word, holding rd/rn/rm/arr fixed (and the op
    // bit 12 stays 1 throughout).
    #[test]
    fn tbx_len_field_advances_one_register_at_bit_13(
        rd in reg_num_strategy(),
        rn in table_base_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let mk = |n: u32| {
            let ops = vec![va(rd, arr), table_list(rn, n, "16b"), va(rm, arr)];
            word_of(encode_neon_tbx(&ops))
        };
        let w1 = mk(1);
        let w2 = mk(2);
        let w3 = mk(3);
        let w4 = mk(4);
        prop_assert_eq!(w2 - w1, 1u32 << 13, "1->2 regs must add 1<<13");
        prop_assert_eq!(w3 - w2, 1u32 << 13, "2->3 regs must add 1<<13");
        prop_assert_eq!(w4 - w3, 1u32 << 13, "3->4 regs must add 1<<13");
        // And bits 14-13 must equal (num_regs - 1) exactly, op bit 12 stays 1.
        for (n, w) in [(1u32, w1), (2, w2), (3, w3), (4, w4)] {
            prop_assert_eq!((w >> 13) & 0x3, n - 1, "len bits 14-13 == num_regs-1");
            prop_assert_eq!((w >> 12) & 1, 1, "op bit must remain 1 for TBX");
        }
    }

    // === Register field round-trip + Q mapping ============================
    // Rd (bits 4-0), Rn (bits 9-5), Rm (bits 20-16) round-trip, and Q (bit30)
    // is 1 iff the arrangement is .16b.
    #[test]
    fn tbx_register_fields_round_trip_and_q_maps_arrangement(
        rd in reg_num_strategy(),
        rn in table_base_strategy(),
        rm in reg_num_strategy(),
        num_regs in table_size_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), table_list(rn, num_regs, "16b"), va(rm, arr)];
        let w = word_of(encode_neon_tbx(&ops));
        let q = if arr == "16b" { 1u32 } else { 0 };

        prop_assert_eq!(w & 0x1F, rd, "Rd field (bits 4-0)");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field (bits 9-5) — table base");
        prop_assert_eq!((w >> 16) & 0x1F, rm, "Rm field (bits 20-16) — index");
        prop_assert_eq!((w >> 30) & 0x1, q, "Q bit from destination arrangement");
    }

    // === Negative contract: too few operands rejected =====================
    #[test]
    fn tbx_rejects_too_few_operands(n in 0usize..3) {
        let ops: Vec<Operand> = (0..n).map(|_| va(0, "16b")).collect();
        prop_assert!(encode_neon_tbx(&ops).is_err(),
            "{n} operands should be rejected; tbx needs 3");
    }

    // === Negative contract: non-RegList second operand rejected ===========
    #[test]
    fn tbx_rejects_non_reglist_table(whatever in 0u32..32) {
        let ops = vec![va(0, "16b"), va(whatever, "16b"), va(2, "16b")];
        prop_assert!(encode_neon_tbx(&ops).is_err(),
            "second operand must be a register list");
    }
}

// --- documented finding A: out-of-range table size silently wraps ---------
//
// `TBX` accepts exactly 1–4 table registers. The implementation computes
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
    fn tbx_rejects_table_larger_than_four_regs(
        rd in reg_num_strategy(),
        // keep every table register in v0..=v31 even at 8 entries, so the
        // ONLY reason encode_neon_tbx could Err is the size, not a bad reg.
        rn in 0u32..=23u32,
        rm in reg_num_strategy(),
        // 1-4 registers are VALID; only 5-8 are architecturally illegal.
        extra in 5u32..=8u32,
    ) {
        let ops = vec![va(rd, "16b"), table_list(rn, extra, "16b"), va(rm, "16b")];
        let res = encode_neon_tbx(&ops);
        prop_assert!(
            res.is_err(),
            "tbx with {extra} table registers: TBX allows only 1-4 (llvm-mc rejects); \
             expected Err but got {res:?}",
        );
    }
}

// --- documented finding B: non-byte destination arrangements -> Q=0 -------
//
// `TBX` is defined ONLY for `.8b`/`.16b` destinations. The encoder derives Q
// from `arr_d == "16b"` and maps every other arrangement (`.4h`, `.2s`, ...)
// to Q=0, emitting a bogus word instead of `Err`.
proptest! {
    #[test]
    #[ignore]
    fn tbx_rejects_non_byte_arrangements(
        rd in reg_num_strategy(),
        arr in prop_oneof![
            Just("4h"), Just("8h"), Just("2s"), Just("4s"), Just("1d"), Just("2d"),
        ],
    ) {
        let ops = vec![va(rd, arr), table_list(1, 1, "16b"), va(2, arr)];
        let res = encode_neon_tbx(&ops);
        prop_assert!(
            res.is_err(),
            "tbx v{rd}.{arr}, ...: TBX is defined only for .8b/.16b destinations; \
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
fn tbx_rejects_empty_register_list() {
    let ops = vec![va(0, "16b"), Operand::RegList(vec![]), va(2, "16b")];
    let res = encode_neon_tbx(&ops);
    assert!(
        res.is_err(),
        "empty table register list should return Err, not panic; got {res:?}",
    );
}
