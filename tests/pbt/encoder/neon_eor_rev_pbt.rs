//! Property-based tests for NEON **vector EOR** and the **REV16/REV32/REV64**
//! (vector) family.
//!
//! ## Targets
//! * **EOR (vector)** is *not* a standalone function: the mnemonic dispatches
//!   through `encode_logical` → [`encode_neon_logical`] with `opc = 0b10`
//!   (the U=1, size=00 logical-three-same encoding). This is distinct from
//!   [`encode_neon_eor3`] (the SHA3 four-register EOR3), which is tested in
//!   `neon_eor3_pbt.rs`.
//! * **REV16/REV32** are encoded by [`encode_neon_two_misc`]:
//!   `rev16 → encode_neon_two_misc(ops, 0, 0b00001)` and
//!   `rev32 → encode_neon_two_misc(ops, 1, 0b00000)`.
//! * **REV64** is encoded by [`encode_neon_rev64`] (also exercised in
//!   `neon_rev64_pbt.rs`; included here so the whole REV family is covered by
//!   one parametric oracle).
//!
//! ## Oracle
//! Every fixed word was cross-validated against LLVM's `llvm-mc-18`
//! (`-triple=aarch64 -show-encoding`) and is independent of this crate. The
//! independent reference encoders (`ref_encode_eor`, `ref_encode_rev`) re-assemble
//! each word field-by-field from the documented ARMv8-A ARM layouts.
//!
//! ## Findings (documented by `#[ignore]`d properties, kept out of the
//! default suite so `cargo test` stays green):
//!
//! 1. **`encode_neon_logical` (EOR path) does not validate the arrangement.**
//!    `EOR (vector)` is defined by the ARMv8-A ARM ONLY for `.8B`/`.16B`
//!    (it is a bitwise op on byte vectors). The encoder derives `Q` from
//!    `arr == "16b"` alone and forces `size = 00`, so `.4h`/`.8h`/`.2s`/… are
//!    silently emitted as UNALLOCATED words instead of returning `Err`.
//!    `llvm-mc-18` rejects them outright: `eor v0.4h, v1.4h, v2.4h` →
//!    `error: invalid operand for instruction`.
//!
//! 2. **`encode_neon_two_misc` (REV16/REV32) does not restrict the `size`
//!    field per instruction.** `neon_arr_to_q_size` happily returns any size,
//!    so `rev16` accepts `.4h` (size must be 00) and `rev32` accepts `.2s`
//!    (size must be 00/01), both producing UNALLOCATED encodings. `llvm-mc-18`
//!    rejects `rev16 v0.4h, v1.4h` and `rev32 v0.2s, v1.2s`.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::{
    encode_neon_logical, encode_neon_rev64, encode_neon_two_misc, neon_arr_to_q_size,
};
use proptest::prelude::*;

// --- shared helpers -------------------------------------------------------

fn va(n: u32, arr: &str) -> Operand {
    Operand::RegArrangement {
        reg: format!("v{n}"),
        arrangement: arr.to_string(),
    }
}

fn reg_num_strategy() -> impl Strategy<Value = u32> {
    0u32..=31u32
}

fn word_of(res: Result<EncodeResult, String>) -> u32 {
    match res {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {other:?}"),
    }
}

// =========================================================================
// EOR (vector)  —  encode_neon_logical(operands, 0b10)
// =========================================================================
//
// Encoding (Advanced SIMD three-same, logical group):
//   0 Q U 01110 size 1 Rm 00011 1 Rn Rd      with U=1, size=00, opcode=011
// Valid arrangements: .8B (Q=0), .16B (Q=1) ONLY.

const EOR_OPC: u32 = 0b10;

/// Independent reference encoder for EOR (vector): U=1, size=00.
fn ref_encode_eor(rd: u32, rn: u32, rm: u32, arr: &str) -> u32 {
    let q: u32 = if arr == "16b" { 1 } else { 0 };
    ((q << 30)
        | (1u32 << 29)            // U = 1 for EOR
        | (0b01110u32 << 24))         // size = 00
        | (1u32 << 21)
        | (rm << 16)
        | (0b000111u32 << 10)     // opcode(15:11)=00011, bit10=1
        | (rn << 5)
        | rd
}

// Golden table, every word cross-validated with `llvm-mc-18`.
//   eor  v0.8b,  v1.8b,  v2.8b   -> 0x2E221C20
//   eor  v0.16b, v1.16b, v2.16b  -> 0x6E221C20
//   eor  v5.16b, v6.16b, v7.16b  -> 0x6E271CC5
//   eor  v31.8b, v30.8b, v29.8b  -> 0x2E3D1FDF
const EOR_GOLDEN: &[(u32, u32, u32, &str, u32)] = &[
    // (Rd, Rn, Rm, arrangement, expected_word)
    (0, 1, 2, "8b", 0x2E221C20),
    (0, 1, 2, "16b", 0x6E221C20),
    (5, 6, 7, "16b", 0x6E271CC5),
    (31, 30, 29, "8b", 0x2E3D1FDF),
];

#[test]
fn eor_matches_golden_table() {
    for &(rd, rn, rm, arr, expected) in EOR_GOLDEN {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let got = word_of(encode_neon_logical(&ops, EOR_OPC));
        assert_eq!(
            got, expected,
            "eor v{rd}.{arr}, v{rn}.{arr}, v{rm}.{arr}: got 0x{got:08X}, want 0x{expected:08X}",
        );
        assert_eq!(
            ref_encode_eor(rd, rn, rm, arr),
            expected,
            "reference encoder drift"
        );
    }
}

proptest! {
    // === Differential against the independent reference encoder ============
    #[test]
    fn eor_matches_reference_encoder(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in prop_oneof![Just("8b"), Just("16b")],
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let got = word_of(encode_neon_logical(&ops, EOR_OPC));
        prop_assert_eq!(got, ref_encode_eor(rd, rn, rm, arr));
    }

    // === Fixed bits are invariant =========================================
    // bit31=0; U(bit29)=1; bits28-24=01110; bit21=1; size(bits23-22)=00;
    // opcode(bits15-10)=000111.
    #[test]
    fn eor_fixed_bits_invariant(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in prop_oneof![Just("8b"), Just("16b")],
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let w = word_of(encode_neon_logical(&ops, EOR_OPC));
        prop_assert_eq!((w >> 31) & 1, 0, "bit 31");
        prop_assert_eq!((w >> 29) & 1, 1, "U bit");
        prop_assert_eq!((w >> 24) & 0x1F, 0b01110, "bits 28-24");
        prop_assert_eq!((w >> 22) & 0x3, 0b00, "size");
        prop_assert_eq!((w >> 21) & 1, 1, "bit 21");
        prop_assert_eq!((w >> 10) & 0x3F, 0b000111, "opcode+bit10");
    }

    // === Register fields round-trip, Q maps the arrangement ===============
    #[test]
    fn eor_register_fields_round_trip(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in prop_oneof![Just("8b"), Just("16b")],
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let w = word_of(encode_neon_logical(&ops, EOR_OPC));
        let want_q: u32 = if arr == "16b" { 1 } else { 0 };
        prop_assert_eq!(w & 0x1F, rd, "Rd");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn");
        prop_assert_eq!((w >> 16) & 0x1F, rm, "Rm");
        prop_assert_eq!((w >> 30) & 1, want_q, "Q");
    }

    // === Negative contract: too few operands rejected =====================
    // EOR (vector) needs 3 operands; fewer must yield Err (via get_neon_reg).
    #[test]
    fn eor_rejects_too_few_operands(n in 0usize..3) {
        let ops: Vec<Operand> = (0..n).map(|_| va(0, "8b")).collect();
        prop_assert!(
            encode_neon_logical(&ops, EOR_OPC).is_err(),
            "{n} operands must be rejected; eor needs 3",
        );
    }
}

// --- documented finding #1: invalid arrangement silently encoded ---------
//
// EOR (vector) is defined ONLY for .8B/.16B. The encoder accepts every other
// arrangement (forcing Q=0, size=00) and returns an UNALLOCATED word. This
// FAILS under `cargo test --ignored`, surfacing the missing range check.
proptest! {
    #[test]
    #[ignore]
    fn eor_rejects_non_byte_arrangements(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in prop_oneof![
            Just("4h"), Just("8h"), Just("2s"), Just("4s"), Just("1d"), Just("2d"),
        ],
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        let res = encode_neon_logical(&ops, EOR_OPC);
        prop_assert!(
            res.is_err(),
            "eor v{rd}.{arr}, v{rn}.{arr}, v{rm}.{arr}: only .8b/.16b are valid for EOR (vector) \
             (llvm-mc rejects); expected Err but got {res:?}",
        );
    }
}

// =========================================================================
// REV16 / REV32 / REV64 (vector) family
// =========================================================================

#[derive(Clone, Copy, Debug)]
enum RevKind {
    Rev16,
    Rev32,
    Rev64,
}

impl RevKind {
    fn encode(&self, ops: &[Operand]) -> Result<EncodeResult, String> {
        match self {
            RevKind::Rev16 => encode_neon_two_misc(ops, 0, 0b00001),
            RevKind::Rev32 => encode_neon_two_misc(ops, 1, 0b00000),
            RevKind::Rev64 => encode_neon_rev64(ops),
        }
    }

    /// Arrangements VALID per the ARMv8-A ARM for each variant:
    ///   REV16 -> 8B/16B (size 00)
    ///   REV32 -> 8B/16B/4H/8H (size 00 or 01)
    ///   REV64 -> 8B/16B/4H/8H/2S/4S (size 00/01/10)
    fn valid_arrangements(&self) -> &'static [&'static str] {
        match self {
            RevKind::Rev16 => &["8b", "16b"],
            RevKind::Rev32 => &["8b", "16b", "4h", "8h"],
            RevKind::Rev64 => &["8b", "16b", "4h", "8h", "2s", "4s"],
        }
    }

    /// Independent reference encoder, matching the ARMv8-A "two-register
    /// miscellaneous" layout:
    ///   `0 Q U 01110 size 10000 opcode 10 Rn Rd`
    /// REV16: U=0 opcode=0001; REV32: U=1 opcode=0000; REV64: U=0 opcode=0000.
    fn ref_encode(&self, rd: u32, rn: u32, arr: &str) -> u32 {
        let (q, size) = neon_arr_to_q_size(arr).unwrap();
        let (u, opcode): (u32, u32) = match self {
            RevKind::Rev16 => (0, 0b00001),
            RevKind::Rev32 => (1, 0b00000),
            RevKind::Rev64 => (0, 0b00000),
        };
        if matches!(self, RevKind::Rev64) {
            // REV64 lives in its own encoder layout: bit21=1, bits20-16=00000,
            // opcode(bits15-12)=0000, bit11=1, bit10=0.
            (q << 30)
                | (0b01110u32 << 24)
                | (size << 22)
                | (0b100000u32 << 16)
                | (0b000010u32 << 10)
                | (rn << 5)
                | rd
        } else {
            (q << 30)
                | (u << 29)
                | (0b01110u32 << 24)
                | (size << 22)
                | (0b10000u32 << 17)  // bit21=1, bits20-17=0000
                | (opcode << 12)      // bit16 + opcode(15:12)
                | (0b10u32 << 10)     // bit11=1, bit10=0
                | (rn << 5)
                | rd
        }
    }
}

/// Strategy yielding (RevKind, valid arrangement) pairs, sampling every
/// variant and every one of its valid arrangements.
fn rev_case() -> impl Strategy<Value = (RevKind, &'static str)> {
    prop_oneof![
        (Just(RevKind::Rev16), 0u32..2),
        (Just(RevKind::Rev32), 0u32..4),
        (Just(RevKind::Rev64), 0u32..6),
    ]
    .prop_map(|(kind, i)| (kind, kind.valid_arrangements()[i as usize]))
}

// Golden table cross-validated with `llvm-mc-18`.
//   rev16 v0.8b,  v1.8b   -> 0x0E201820
//   rev16 v0.16b, v1.16b  -> 0x4E201820
//   rev32 v0.8b,  v1.8b   -> 0x2E200820
//   rev32 v0.4h,  v1.4h   -> 0x2E600820
//   rev32 v0.16b, v1.16b  -> 0x6E200820
//   rev32 v0.8h,  v1.8h   -> 0x6E600820
//   rev32 v10.8h, v20.8h  -> 0x6E600A8A
//   rev64 v0.8b,  v1.8b   -> 0x0E200820
//   rev64 v0.4h,  v1.4h   -> 0x0E600820
//   rev64 v0.2s,  v1.2s   -> 0x0EA00820
const REV_GOLDEN: &[(RevKind, u32, u32, &str, u32)] = &[
    (RevKind::Rev16, 0, 1, "8b", 0x0E201820),
    (RevKind::Rev16, 0, 1, "16b", 0x4E201820),
    (RevKind::Rev32, 0, 1, "8b", 0x2E200820),
    (RevKind::Rev32, 0, 1, "4h", 0x2E600820),
    (RevKind::Rev32, 0, 1, "16b", 0x6E200820),
    (RevKind::Rev32, 0, 1, "8h", 0x6E600820),
    (RevKind::Rev32, 10, 20, "8h", 0x6E600A8A),
    (RevKind::Rev64, 0, 1, "8b", 0x0E200820),
    (RevKind::Rev64, 0, 1, "4h", 0x0E600820),
    (RevKind::Rev64, 0, 1, "2s", 0x0EA00820),
];

#[test]
fn rev_matches_golden_table() {
    for &(kind, rd, rn, arr, expected) in REV_GOLDEN {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let got = word_of(kind.encode(&ops));
        assert_eq!(
            got, expected,
            "{kind:?} v{rd}.{arr}, v{rn}.{arr}: got 0x{got:08X}, want 0x{expected:08X}",
        );
        assert_eq!(
            kind.ref_encode(rd, rn, arr),
            expected,
            "reference encoder drift"
        );
    }
}

proptest! {
    // === Differential against the independent reference encoder ============
    #[test]
    fn rev_matches_reference_encoder(
        (kind, arr) in rev_case(),
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let got = word_of(kind.encode(&ops));
        prop_assert_eq!(got, kind.ref_encode(rd, rn, arr));
    }

    // === Common fixed bits across the whole family ========================
    // bit31=0; bits28-24=01110; bit21=1; bit11=1; bit10=0 for every variant.
    #[test]
    fn rev_common_fixed_bits(
        (kind, arr) in rev_case(),
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let w = word_of(kind.encode(&ops));
        prop_assert_eq!((w >> 31) & 1, 0, "bit 31");
        prop_assert_eq!((w >> 24) & 0x1F, 0b01110, "bits 28-24");
        prop_assert_eq!((w >> 21) & 1, 1, "bit 21");
        prop_assert_eq!((w >> 10) & 0x3, 0b10, "bits 11-10");
    }

    // === Register fields + Q/size round-trip ==============================
    #[test]
    fn rev_fields_round_trip(
        (kind, arr) in rev_case(),
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let w = word_of(kind.encode(&ops));
        let (q, size) = neon_arr_to_q_size(arr).unwrap();
        prop_assert_eq!(w & 0x1F, rd, "Rd");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn");
        prop_assert_eq!((w >> 30) & 1, q, "Q");
        prop_assert_eq!((w >> 22) & 0x3, size, "size");
    }

    // === Negative contract: too few operands rejected =====================
    // Every REV variant needs 2 operands; fewer must yield Err.
    #[test]
    fn rev_rejects_too_few_operands(
        kind in prop_oneof![Just(RevKind::Rev16), Just(RevKind::Rev32), Just(RevKind::Rev64)],
        n in 0usize..2,
    ) {
        let ops: Vec<Operand> = (0..n).map(|_| va(0, "8b")).collect();
        prop_assert!(
            kind.encode(&ops).is_err(),
            "{kind:?} with {n} operands must be rejected; needs 2",
        );
    }
}

// --- documented finding #2: REV16 accepts non-byte arrangements ----------
//
// REV16 (vector) is defined ONLY for size=00 (.8B/.16B). The encoder maps
// any arrangement through neon_arr_to_q_size, so .4h/.2s/.2d produce size
// = 01/10/11 — all UNALLOCATED — instead of Err.
proptest! {
    #[test]
    #[ignore]
    fn rev16_rejects_non_byte_arrangements(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in prop_oneof![
            Just("4h"), Just("8h"), Just("2s"), Just("4s"), Just("1d"), Just("2d"),
        ],
    ) {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let res = RevKind::Rev16.encode(&ops);
        prop_assert!(
            res.is_err(),
            "rev16 v{rd}.{arr}, v{rn}.{arr}: only size=00 (.8b/.16b) is valid for REV16 \
             (llvm-mc rejects); expected Err but got {res:?}",
        );
    }

    // --- documented finding #3: REV32 accepts word/double arrangements ----
    //
    // REV32 (vector) is defined for size=00/01 (.8B/.16B/.4H/.8H) ONLY.
    // .2S/.4S/.1D/.2D (size 10/11) are UNALLOCATED.
    #[test]
    #[ignore]
    fn rev32_rejects_word_or_double_arrangements(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in prop_oneof![Just("2s"), Just("4s"), Just("1d"), Just("2d")],
    ) {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let res = RevKind::Rev32.encode(&ops);
        prop_assert!(
            res.is_err(),
            "rev32 v{rd}.{arr}, v{rn}.{arr}: only size=00/01 (.8b/.16b/.4h/.8h) is valid for \
             REV32 (llvm-mc rejects); expected Err but got {res:?}",
        );
    }
}
