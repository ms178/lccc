//! Property-based tests for `encode_neon_across`.
//!
//! `encode_neon_across` is the shared backend that encodes the AArch64
//! "Advanced SIMD across lanes" reduction instructions —
//! `UMAXV / UMINV / SMAXV / SMINV Vd.T, Vn.T` (and is also the correct
//! reference shape used by `ADDV`/`SADDLV`/`UADDLV`) — in the group:
//!
//! ```text
//!   31 30 29 28-24 23-22 21-17  16-12   11-10 9-5 4-0
//!    0  Q  U  01110  size  11000  opcode  10   Rn  Rd
//! ```
//!
//! The ARMv8-A ARM ("Advanced SIMD across lanes") assigns these opcodes:
//!
//! | instr  | U | opcode (bits 16-12) |
//! |--------|---|----------------------|
//! | SMAXV  | 0 | 0 1 0 1 0 |
//! | SMINV  | 0 | 0 1 0 1 1 |
//! | UMAXV  | 1 | 0 1 0 1 0 |
//! | UMINV  | 1 | 0 1 0 1 1 |
//! | ADDV   | 0 | 1 1 0 1 1 |
//! | SADDLV | 0 | 0 0 0 1 0 |
//! | UADDLV | 1 | 0 0 0 1 0 |
//!
//! ## Oracle
//! The golden words are hand-derived from the ARMv8-A ARM bit layout and are
//! INDEPENDENT of this crate's implementation. `ref_encode_across` mirrors the
//! documented field placement (which, for `encode_neon_across`, is correct —
//! unlike its buggy sibling `encode_neon_addv`). An external `llvm-mc`/gas was
//! not available in this environment, so the golden table is the absolute
//! anchor.
//!
//! ## Finding (documented by the `#[ignore]`d test `across_rejects_unallocated_arrangements`)
//! Every across-lanes instruction is defined by the ARMv8-A ARM ONLY for
//! arrangements whose `size` field is 0b00, 0b01, or 0b10 — i.e. `.8b/.16b`,
//! `.4h/.8h`, `.2s/.4s`. The `size = 0b11` arrangements `.1d` and `.2d` are
//! UNALLOCATED (UNDEFINED) for the whole across-lanes group. `encode_neon_across`
//! accepts them silently (via `neon_arr_to_q_size`, which returns `Ok` for
//! `1d`/`2d`) and emits a word instead of returning `Err`.
//!
//! A second, related defect lives in the *dispatch* table (`mod.rs`), not in
//! `encode_neon_across` itself: `sminv`/`uminv` are dispatched with
//! `opcode = 0b11010` instead of the architecturally-correct `0b01011`. The
//! function faithfully encodes whatever opcode it is handed, so that is a
//! caller bug — noted here for completeness.
//! See `NEON_ACROSS_BUG_REPORT.md`.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::{encode_neon_across, neon_arr_to_q_size};
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
    0u32..=1u32
}

/// Architecturally-valid 5-bit opcodes for the across-lanes group.
fn opcode_strategy() -> impl Strategy<Value = u32> {
    prop_oneof![
        Just(0b01010u32), // SMAXV/UMAXV
        Just(0b01011u32), // SMINV/UMINV
        Just(0b11011u32), // ADDV
        Just(0b00010u32), // SADDLV/UADDLV
        (0u32..=0x1Fu32), // arbitrary in-range 5-bit opcode (field placement)
    ]
}

/// Arrangements architecturally VALID for across-lanes reductions
/// (ARMv8-A ARM): size ∈ {0b00, 0b01, 0b10}. `.1d`/`.2d` (size=0b11) are NOT.
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
/// documented ARMv8-A ARM layout. Mirrors the (correct) bit placement of
/// `encode_neon_across`.
fn ref_encode_across(rd: u32, rn: u32, arr: &str, u_bit: u32, opcode: u32) -> u32 {
    let (q, size) = neon_arr_to_q_size(arr).unwrap();
    (q << 30)
        | (u_bit << 29)
        | (0b01110u32 << 24)
        | (size << 22)
        | (0b11000u32 << 17) // bits 21-17
        | (opcode << 12)     // bits 16-12
        | (0b10u32 << 10)    // bits 11-10
        | (rn << 5)
        | rd
}

fn word_of(res: Result<EncodeResult, String>) -> u32 {
    match res {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {other:?}"),
    }
}

// --- golden table (absolute oracle, hand-derived from the ARMv8-A ARM) ----

/// Each entry uses the CORRECT opcode per the ARMv8-A ARM. These confirm the
/// function emits the right word for every real instruction it serves.
/// (Note: the live dispatch for sminv/uminv passes a *wrong* opcode; these
/// golden values document the correct behavior independent of that caller bug.)
const GOLDEN: &[(u32, u32, u32, u32, &str, u32)] = &[
    // (u_bit, opcode, Rd, Rn, arrangement, expected_word)
    // UMAXV (U=1, opc=01010)
    (1, 0b01010, 0, 1, "4s", 0x6EB0A820), // umaxv v0.4s, v1.4s
    (1, 0b01010, 2, 3, "8b", 0x2E30A862), // umaxv v2.8b, v3.8b
    // SMAXV (U=0, opc=01010)
    (0, 0b01010, 0, 1, "4s", 0x4EB0A820), // smaxv v0.4s, v1.4s
    (0, 0b01010, 5, 6, "8h", 0x4E70A8C5), // smaxv v5.8h, v6.8h
    // UMINV (U=1, opc=01011)
    (1, 0b01011, 0, 1, "4s", 0x6EB0B820), // uminv v0.4s, v1.4s
    // SMINV (U=0, opc=01011)
    (0, 0b01011, 0, 1, "4s", 0x4EB0B820), // sminv v0.4s, v1.4s
    // ADDV (U=0, opc=11011) — matches the independent neon_addv_pbt.rs oracle
    (0, 0b11011, 0, 1, "4s", 0x4EB1B820), // addv v0.4s, v1.4s
];

#[test]
fn across_matches_golden_table() {
    for &(u_bit, opcode, rd, rn, arr, expected) in GOLDEN {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let got = word_of(encode_neon_across(&ops, u_bit, opcode));
        assert_eq!(
            got, expected,
            "u_bit={u_bit} opc=0b{opcode:05b} v{rd}.{arr}, v{rn}.{arr}: \
             got 0x{got:08X}, want 0x{expected:08X}",
        );
        // Cross-check the reference encoder against the golden values too.
        assert_eq!(
            ref_encode_across(rd, rn, arr, u_bit, opcode),
            expected,
            "reference encoder drift for v{rd}.{arr}"
        );
    }
}

// --- properties -----------------------------------------------------------

proptest! {
    // === Oracle: differential against independent reference encoder ========
    // For every valid arrangement, register pair, U bit, and in-range opcode,
    // the implementation must equal the independently-assembled word. PASSES:
    // `encode_neon_across` places every field in the correct bit window.
    #[test]
    fn across_matches_reference_encoder(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
        u_bit in u_bit_strategy(),
        opcode in opcode_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let got = word_of(encode_neon_across(&ops, u_bit, opcode));
        let want = ref_encode_across(rd, rn, arr, u_bit, opcode);
        prop_assert_eq!(got, want);
    }

    // === Field placement: every field round-trips / maps arrangement ======
    // Extract each field from the emitted word and confirm it equals the
    // intended input. Confirms no overlap between adjacent fields and that
    // the Q/size bits reflect the arrangement.
    #[test]
    fn fields_round_trip_and_map_arrangement(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
        u_bit in u_bit_strategy(),
        opcode in opcode_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let w = word_of(encode_neon_across(&ops, u_bit, opcode));
        let (q, size) = neon_arr_to_q_size(arr).unwrap();

        prop_assert_eq!((w >> 31) & 1, 0, "bit 31 must be 0");
        prop_assert_eq!((w >> 30) & 1, q, "Q bit");
        prop_assert_eq!((w >> 29) & 1, u_bit, "U bit");
        prop_assert_eq!((w >> 24) & 0x1F, 0b01110, "bits 28-24 = 01110");
        prop_assert_eq!((w >> 22) & 0x3, size, "size bits");
        prop_assert_eq!((w >> 17) & 0x1F, 0b11000, "bits 21-17 = 11000");
        prop_assert_eq!((w >> 12) & 0x1F, opcode, "opcode bits 16-12");
        prop_assert_eq!((w >> 10) & 0x3, 0b10, "bits 11-10 = 10");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field");
        prop_assert_eq!(w & 0x1F, rd, "Rd field");
    }

    // === Field independence: a delta in one field changes only its window ==
    // Holding all else equal, incrementing Rd by d (no carry) changes exactly
    // bits 4-0; incrementing Rn changes exactly bits 9-5; flipping U changes
    // exactly bit 29; incrementing the opcode changes exactly bits 16-12.
    // This proves fields are cleanly separated with no aliasing/bleed-through.
    #[test]
    fn fields_are_independent(
        rd in 0u32..30u32,        // leave room for +1 without carry-out
        rn in 0u32..30u32,
        opcode in 0u32..0x1Eu32,  // 5-bit, room for +1
        arr in valid_arrangement_strategy(),
    ) {
        let base_ops = vec![va(rd, arr), va(rn, arr)];
        let base = word_of(encode_neon_across(&base_ops, 0, opcode));

        // Rd delta -> only bits 4-0 change.
        let w_rd = word_of(encode_neon_across(&[va(rd + 1, arr), va(rn, arr)], 0, opcode));
        prop_assert_eq!(w_rd - base, 1);
        prop_assert_eq!((w_rd ^ base) & !0x1F, 0, "Rd delta leaked beyond bits 4-0");

        // Rn delta -> only bits 9-5 change (delta = 1<<5 = 0x20).
        let w_rn = word_of(encode_neon_across(&[va(rd, arr), va(rn + 1, arr)], 0, opcode));
        prop_assert_eq!(w_rn - base, 1u32 << 5);
        prop_assert_eq!((w_rn ^ base) & !(0x1F << 5), 0, "Rn delta leaked beyond bits 9-5");

        // U flip -> only bit 29 changes.
        let w_u = word_of(encode_neon_across(&base_ops, 1, opcode));
        prop_assert_eq!(w_u ^ base, 1u32 << 29, "U flip must toggle only bit 29");

        // opcode delta -> only bits 16-12 change (delta = 1<<12).
        let w_op = word_of(encode_neon_across(&base_ops, 0, opcode + 1));
        prop_assert_eq!(w_op - base, 1u32 << 12);
        prop_assert_eq!((w_op ^ base) & !(0x1F << 12), 0, "opcode delta leaked beyond bits 16-12");
    }

    // === Determinism: identical inputs yield identical words ===============
    #[test]
    fn encoding_is_deterministic(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
        u_bit in u_bit_strategy(),
        opcode in opcode_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let a = word_of(encode_neon_across(&ops, u_bit, opcode));
        let b = word_of(encode_neon_across(&ops, u_bit, opcode));
        prop_assert_eq!(a, b);
    }

    // === Negative contract: too few operands rejected (passing) ===========
    // The function must return Err when fewer than two operands are supplied.
    #[test]
    fn rejects_too_few_operands(
        n in 0usize..2,
    ) {
        let ops: Vec<Operand> = (0..n).map(|i| va(i as u32, "4s")).collect();
        prop_assert!(encode_neon_across(&ops, 1, 0b01010).is_err(),
            "{n} operands must be rejected");
    }

    // === Negative contract: unknown arrangement rejected (passing) ========
    // Any arrangement string not understood by `neon_arr_to_q_size` must cause
    // `Err` (no silent fallthrough).
    #[test]
    fn rejects_unknown_arrangement(
        arr in "[a-z0-9]{1,4}".prop_filter("must be an unknown arrangement", |s| {
            !matches!(s.as_str(),
                "8b"|"16b"|"4h"|"8h"|"2s"|"4s"|"1d"|"2d")
        }),
    ) {
        let ops = vec![va(0, arr.as_str()), va(1, arr.as_str())];
        prop_assert!(encode_neon_across(&ops, 1, 0b01010).is_err(),
            "unknown arrangement {arr:?} should be rejected");
    }
}

// --- documented finding: unallocated arrangements silently encoded --------

/// Across-lanes instructions are defined by the ARMv8-A ARM ONLY for
/// 8B/16B/4H/8H/2S/4S (size ∈ {0b00, 0b01, 0b10}). `.1d`/`.2d` (size=0b11)
/// are UNALLOCATED for the ENTIRE across-lanes group (UMAXV/UMINV/SMAXV/SMINV/
/// ADDV/SADDLV/UADDLV) and must be rejected. The implementation accepts them
/// and emits a word instead of returning `Err`.
///
/// `#[ignore]`d because it documents a genuine gap (not yet fixed).
/// Run with `cargo test -- --ignored across_rejects_unallocated_arrangements`.
#[test]
#[ignore]
fn across_rejects_unallocated_arrangements() {
    for arr in &["1d", "2d"] {
        let ops = vec![va(0, arr), va(1, arr)];
        let res = encode_neon_across(&ops, 1, 0b01010);
        let emitted = res
            .as_ref()
            .ok()
            .map(|e| match e {
                EncodeResult::Word(w) => *w,
                _ => 0,
            })
            .unwrap_or(0);
        assert!(
            res.is_err(),
            "across-lanes reductions do not support .{arr} (size=0b11 is UNALLOCATED); \
             expected Err but got Ok(0x{emitted:08X})",
        );
    }
}

// --- documented finding: no opcode-allocation validation ------------------

/// The Advanced SIMD across-lanes group allocates ONLY these opcode field
/// (bits 16-12) values (ARMv8-A ARM): 00010 (SADDLV/UADDLV), 01010 (MAXV),
/// 01011 (MINV), 11011 (ADDV). Every other 5-bit opcode is UNALLOCATED for
/// this group. `encode_neon_across` performs NO such allocation check — it
/// places whatever `opcode` it is handed into bits 16-12 — so it will emit
/// UNALLOCATED encodings for arbitrary caller inputs. (This is exactly what
/// enables the live `sminv`/`uminv` dispatch to silently produce UNALLOCATED
/// words with opcode 0b11010 — see `sminv-uminv-dispatch-wrong-opcode.md`.)
///
/// `#[ignore]`d because it documents a genuine gap (not yet fixed).
/// Run with `cargo test -- --ignored across_rejects_unallocated_opcodes`.
#[test]
#[ignore]
fn across_rejects_unallocated_opcodes() {
    const ALLOCATED: &[u32] = &[0b00010, 0b01010, 0b01011, 0b11011];
    for opcode in 0u32..=0x1F {
        if ALLOCATED.contains(&opcode) {
            continue;
        }
        for &u_bit in &[0u32, 1] {
            let ops = vec![va(0, "4s"), va(1, "4s")];
            let res = encode_neon_across(&ops, u_bit, opcode);
            let emitted = res
                .as_ref()
                .ok()
                .map(|e| match e {
                    EncodeResult::Word(w) => *w,
                    _ => 0,
                })
                .unwrap_or(0);
            assert!(
                res.is_err(),
                "across-lanes opcode=0b{opcode:05b} (U={u_bit}) is UNALLOCATED; \
                 expected Err but got Ok(0x{emitted:08X})",
            );
        }
    }
}
