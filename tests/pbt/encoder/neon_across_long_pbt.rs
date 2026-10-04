//! Property-based tests for `encode_neon_across_long`.
//!
//! `encode_neon_across_long` encodes the AArch64 NEON "add long across vector"
//! reduction instructions `SADDLV`/`UADDLV` —
//! `SADDLV <Vd>, <Vn>.<T>` / `UADDLV <Vd>, <Vn>.<T>` — in the
//! "Advanced SIMD across lanes" encoding group:
//!
//! ```text
//!   31 30 29 28-24 23-22 21-17  16-12   11-10 9-5 4-0
//!    0  Q  U  01110  size  11000  opcode  10   Rn  Rd
//! ```
//! with `opcode = 00011`. `U` selects signed (`0`) vs unsigned (`1`);
//! `Q`/`size` come from the source arrangement `<T>`.
//!
//! ## Oracle
//! The golden words are hand-derived from the ARMv8-A ARM bit layout
//! (ARM DDI 0487, "Advanced SIMD across lanes", SADDLV/UADDLV rows:
//! opcode=00011, U=0/1) and are independent of this crate's implementation.
//! The independent reference encoder `ref_encode_across_long` assembles the
//! word field-by-field from that layout. Note `encode_neon_across_long`'s
//! bit arithmetic is byte-for-byte identical to its tested sibling
//! `encode_neon_across` (see `neon_across_pbt.rs`), so for architecturally
//! valid inputs every emitted word is correct.
//!
//! ## Finding (documented by the `#[ignore]`d test `across_long_rejects_unallocated_arrangements`)
//! The implementation does **not** validate the `size` field. For the
//! SADDLV/UADDLV class the ARMv8-A ARM explicitly defines
//! `size == 0b11` as **UNALLOCATED** (the shared "Advanced SIMD across lanes"
//! decode falls through to UNALLOCATED for that case). The only arrangements
//! that map to `size == 0b11` are `.1d` and `.2d`. The encoder accepts them
//! and emits a word instead of returning `Err`, producing a silently
//! unallocated encoding. Example: `uaddlv d0, v1.2d` yields `Ok(0x6EF03820)`
//! instead of an error.
//! See `ACROSS_LONG_ENCODER_BUG_REPORT.md`.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::{encode_neon_across_long, neon_arr_to_q_size};
use proptest::prelude::*;

// --- helpers --------------------------------------------------------------

/// Build a scalar destination register operand `Vd` with number `n`
/// (e.g. `d0`, `s5`, `h10` — the destination of an SADDLV/UADDLV reduction).
fn dest(prefix: &str, n: u32) -> Operand {
    Operand::Reg(format!("{prefix}{n}"))
}

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
    0u32..=1u32 // signed (SADDLV) vs unsigned (UADDLV)
}

/// Arrangements architecturally VALID for SADDLV/UADDLV
/// (ARMv8-A ARM, "SADDLV (vector)" / "UADDLV (vector)"):
/// 8B, 16B, 4H, 8H, 4S. These cover size=0b00, 0b01, 0b10 only.
fn valid_arrangement_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("8b"), Just("16b"), Just("4h"), Just("8h"), Just("4s")]
}

/// The SADDLV/UADDLV opcode (bits 16-12) is fixed at 0b00011.
const OPCODE: u32 = 0b00011;

/// Independent reference encoder: assembles the word field-by-field from the
/// documented ARMv8-A ARM layout (does NOT call the implementation under test).
fn ref_encode_across_long(rd: u32, rn: u32, u: u32, opcode: u32, arr: &str) -> u32 {
    let (q, size) = neon_arr_to_q_size(arr).unwrap();
    (q << 30)
        | (u << 29)
        | (0b01110u32 << 24)
        | (size << 22)
        | (0b11000u32 << 17) // bits 21-17
        | (opcode << 12) // opcode, bits 16-12
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

/// Hand-derived from the ARMv8-A ARM layout for SADDLV/UADDLV (opcode=00011).
/// (rd, rn, arrangement, u, expected_word)
const GOLDEN: &[(u32, u32, &str, u32, u32)] = &[
    (0, 1, "4s", 0, 0x4EB03820),    // saddlv d0, v1.4s
    (0, 1, "4h", 0, 0x0E703820),    // saddlv s0, v1.4h  (Q=0)
    (0, 1, "4s", 1, 0x6EB03820),    // uaddlv d0, v1.4s
    (0, 1, "8b", 0, 0x0E303820),    // saddlv h0, v1.8b  (Q=0,size=00)
    (5, 6, "8h", 1, 0x6E7038C5),    // uaddlv s5, v6.8h
    (10, 20, "16b", 0, 0x4E303A8A), // saddlv h10, v20.16b
];

#[test]
fn across_long_matches_golden_table() {
    for &(rd, rn, arr, u, expected) in GOLDEN {
        let dest_prefix = match arr {
            "4s" => "d",
            "4h" | "8h" => "s",
            _ => "h",
        };
        let ops = vec![dest(dest_prefix, rd), va(rn, arr)];
        let got = word_of(encode_neon_across_long(&ops, u, OPCODE));
        assert_eq!(
            got, expected,
            "saddlv/uaddlv rd={rd}, v{rn}.{arr}, u={u}: got 0x{got:08X}, want 0x{expected:08X}",
        );
        // Cross-check the reference encoder against the golden values too.
        assert_eq!(
            ref_encode_across_long(rd, rn, u, OPCODE, arr),
            expected,
            "reference encoder drift"
        );
    }
}

// --- properties -----------------------------------------------------------

proptest! {
    // === Oracle: differential against independent reference encoder =========
    // For every valid SADDLV/UADDLV arrangement, U bit, and register pair,
    // the implementation must equal the independently-assembled reference word.
    #[test]
    fn across_long_matches_reference_encoder(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
        u in u_bit_strategy(),
    ) {
        let ops = vec![dest("d", rd), va(rn, arr)];
        let got = word_of(encode_neon_across_long(&ops, u, OPCODE));
        let want = ref_encode_across_long(rd, rn, u, OPCODE, arr);
        prop_assert_eq!(got, want);
    }

    // === Fixed-bits invariant ==============================================
    // The architecturally-constant bits must never change: bit31=0,
    // bits28-24=01110, bits21-17=11000, opcode(bits16-12)=00011,
    // bits11-10=10.
    #[test]
    fn across_long_fixed_bits_are_constant(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
        u in u_bit_strategy(),
    ) {
        let ops = vec![dest("d", rd), va(rn, arr)];
        let w = word_of(encode_neon_across_long(&ops, u, OPCODE));

        prop_assert_eq!((w >> 31) & 1, 0, "bit 31 must be 0");
        prop_assert_eq!((w >> 24) & 0x1F, 0b01110, "bits 28-24");
        prop_assert_eq!((w >> 17) & 0x1F, 0b11000, "bits 21-17");
        prop_assert_eq!((w >> 12) & 0x1F, 0b00011, "opcode bits 16-12");
        prop_assert_eq!((w >> 10) & 0x3, 0b10, "bits 11-10");
    }

    // === Field placement: Rd/Rn round-trip + Q/size/U mapping ==============
    #[test]
    fn across_long_fields_round_trip_and_map_arrangement(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
        u in u_bit_strategy(),
    ) {
        let ops = vec![dest("d", rd), va(rn, arr)];
        let w = word_of(encode_neon_across_long(&ops, u, OPCODE));
        let (q, size) = neon_arr_to_q_size(arr).unwrap();

        prop_assert_eq!(w & 0x1F, rd, "Rd field");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field");
        prop_assert_eq!((w >> 30) & 0x1, q, "Q bit");
        prop_assert_eq!((w >> 29) & 0x1, u, "U bit");
        prop_assert_eq!((w >> 22) & 0x3, size, "size field");
    }

    // === Determinism: identical inputs produce identical words =============
    #[test]
    fn across_long_is_deterministic(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
        u in u_bit_strategy(),
    ) {
        let ops = vec![dest("d", rd), va(rn, arr)];
        let a = word_of(encode_neon_across_long(&ops, u, OPCODE));
        let b = word_of(encode_neon_across_long(&ops, u, OPCODE));
        prop_assert_eq!(a, b);
    }

    // === Destination accepts both Operand forms equivalently ===============
    // The destination match arm accepts both `Operand::Reg` (scalar Hd/Sd/Dd)
    // and `Operand::RegArrangement`. Both must yield the same register number
    // and therefore the same encoded word.
    #[test]
    fn across_long_dest_reg_and_arrangement_equivalent(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
        u in u_bit_strategy(),
    ) {
        let ops_reg = vec![dest("d", rd), va(rn, arr)];
        let ops_arr = vec![va(rd, "4s"), va(rn, arr)]; // dest as RegArrangement
        let a = word_of(encode_neon_across_long(&ops_reg, u, OPCODE));
        let b = word_of(encode_neon_across_long(&ops_arr, u, OPCODE));
        prop_assert_eq!(a, b);
    }

    // === Negative contract: unsupported arrangement strings rejected ======
    // Any arrangement not understood by `neon_arr_to_q_size` must cause
    // `encode_neon_across_long` to return `Err`.
    #[test]
    fn across_long_rejects_unknown_arrangement(
        arr in "[a-z0-9]{1,4}".prop_filter("must be an unknown arrangement", |s| {
            !matches!(s.as_str(), "8b"|"16b"|"4h"|"8h"|"2s"|"4s"|"1d"|"2d")
        }),
    ) {
        let ops = vec![dest("d", 0), va(1, arr.as_str())];
        prop_assert!(encode_neon_across_long(&ops, 0, OPCODE).is_err(),
            "unknown arrangement {arr:?} should be rejected");
    }
}

// --- documented finding: unallocated (size=11) arrangements silently encoded
//
// This is a genuine PBT witness: proptest reports `Falsifiable` with a shrunk
// counterexample when run with `--ignored`. It is `#[ignore]`d so the default
// suite stays green; the write-up lives in
// the transplanted campaign notes.

proptest! {
    // === Negative contract: UNALLOCATED arrangements (size=0b11) must Err ===
    // SADDLV/UADDLV is defined by the ARMv8-A ARM ONLY for arrangements whose
    // `size` field is 0b00 (8B/16B), 0b01 (4H/8H), or 0b10 (4S). The shared
    // "Advanced SIMD across lanes" decode marks `size == 0b11` as UNALLOCATED,
    // and the only arrangements mapping to `size == 0b11` are `.1d` and `.2d`.
    // The implementation accepts them and returns Ok(...) instead of Err.
    //
    // Reproduce: cargo test --test pbt_arm_neon_a across_long_rejects_unallocated_size -- --ignored
    #[test]
    #[ignore]
    fn across_long_rejects_unallocated_size(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        u in u_bit_strategy(),
        arr in prop_oneof![Just("1d"), Just("2d")],
    ) {
        let ops = vec![dest("d", rd), va(rn, arr)];
        let res = encode_neon_across_long(&ops, u, OPCODE);
        prop_assert!(
            res.is_err(),
            "SADDLV/UADDLV (opcode={OPCODE}) does not support .{arr} \
             (size=0b11, UNALLOCATED); expected Err but got {res:?}",
        );
    }
}
