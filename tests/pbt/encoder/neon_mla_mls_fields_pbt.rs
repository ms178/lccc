//! Property-based tests for FIXED-FIELD correctness of `encode_neon_mla` and
//! `encode_neon_mls`, focused on **register-field placement** and
//! **opcode / u-bit encoding** — plus the ADDITIONAL bug found: mismatched
//! arrangement operands are silently accepted (the Vn/Vm arrangement specifiers
//! are ignored).
//!
//! ## Oracle
//! Differential against `llvm-mc-18 --triple=aarch64 --show-encoding`, an
//! authoritative assembler independent of this crate. The golden words below
//! are the big-endian `u32` reconstructed from llvm-mc's little-endian
//! `[b0,b1,b2,b3]` encoding bytes: `b3<<24 | b2<<16 | b1<<8 | b0`.
//!
//! ## Verified field facts (all PROVEN by the golden table + properties)
//! ```text
//!   31 30 29 28-24 23-22 21 20-16 15-11 10 9-5 4-0
//!    0  Q  U  01110  size  1   Rm   10010  1  Rn  Rd
//! ```
//!   * MLA: U(bit29)=0, opcode[15:11]=10010
//!   * MLS: U(bit29)=1, opcode[15:11]=10010  — MLA and MLS **share** opcode
//!     `10010`; they differ **only** in the U bit. (The sibling `MUL` has
//!     opcode `10011`, U=0 — a common source of confusion.)
//!   * Rd -> [4:0], Rn -> [9:5], Rm -> [20:16]  (no swap, no truncation)
//!
//!   llvm-mc-18 reference bytes:
//!     `mul v0.4s, v1.4s, v2.4s` -> [20,9c,a2,4e] = 0x4EA29C20  (opcode 10011)
//!     `mla v0.4s, v1.4s, v2.4s` -> [20,94,a2,4e] = 0x4EA29420  (opcode 10010, U=0)
//!     `mls v0.4s, v1.4s, v2.4s` -> [20,94,a2,6e] = 0x6EA29420  (opcode 10010, U=1)
//!
//!   => register placement, opcode, and u-bit are all **correct** for valid
//!      inputs. The early hypothesis that MLS reused the wrong opcode was
//!      disproved by the reference assembler — this suite locks that in so a
//!      future regression in any of these fields is caught.
//!
//! ## ADDITIONAL BUG (beyond the known unallocated-doubleword defect)
//! `encode_neon_mla` / `encode_neon_mls` discard the Vn/Vm arrangement
//! specifiers (bound to `_`), so mismatched-arrangement operands such as
//! `mla v0.4s, v1.8b, v2.2s` are silently accepted and coerced to the
//! destination arrangement, whereas `llvm-mc` rejects them: "invalid operand
//! for instruction". Witnessed by the `#[ignore]`d tests
//! `mla_rejects_mismatched_arrangement` / `mls_rejects_mismatched_arrangement`.
//! See `pbt-out/bug_reports/`.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::{encode_neon_mla, encode_neon_mls, neon_arr_to_q_size};
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

/// Arrangements architecturally VALID for MLA/MLS (size 00/01/10).
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

/// Correct, llvm-mc-18-verified reference encoder for the three-same multiply
/// group. `u_bit` = 0 for MLA, 1 for MLS; opcode is always `10010`.
fn ref_encode(rd: u32, rn: u32, rm: u32, arr: &str, u_bit: u32) -> u32 {
    let (q, size) = neon_arr_to_q_size(arr).unwrap();
    (q << 30)
        | (u_bit << 29)
        | (0b01110u32 << 24)
        | (size << 22)
        | (1u32 << 21)
        | (rm << 16)
        | (0b10010u32 << 11) // opcode bits[15:11]
        | (1u32 << 10) // fixed '1'
        | (rn << 5)
        | rd
}

fn word_of(res: Result<EncodeResult, String>) -> u32 {
    match res {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {other:?}"),
    }
}

// --- golden tables (absolute oracle, llvm-mc-18 verified) -----------------

const MLA_GOLDEN: &[(u32, u32, u32, &str, u32)] = &[
    // (Rd, Rn, Rm, arrangement, expected_word)
    (0, 1, 2, "4s", 0x4EA29420),     // mla v0.4s, v1.4s, v2.4s
    (0, 1, 2, "2s", 0x0EA29420),     // mla v0.2s, v1.2s, v2.2s  (Q=0)
    (5, 6, 7, "8h", 0x4E6794C5),     // mla v5.8h, v6.8h, v7.8h
    (31, 30, 29, "16b", 0x4E3D97DF), // mla v31.16b, v30.16b, v29.16b
    (0, 0, 0, "8b", 0x0E209400),     // mla v0.8b, v0.8b, v0.8b  (Q=0,size=00)
    (10, 20, 30, "4h", 0x0E7E968A),  // mla v10.4h, v20.4h, v30.4h (Q=0)
];

const MLS_GOLDEN: &[(u32, u32, u32, &str, u32)] = &[
    (0, 1, 2, "4s", 0x6EA29420),     // mls v0.4s, v1.4s, v2.4s
    (0, 1, 2, "2s", 0x2EA29420),     // mls v0.2s, v1.2s, v2.2s  (Q=0)
    (5, 6, 7, "8h", 0x6E6794C5),     // mls v5.8h, v6.8h, v7.8h
    (31, 30, 29, "16b", 0x6E3D97DF), // mls v31.16b, v30.16b, v29.16b
    (0, 0, 0, "8b", 0x2E209400),     // mls v0.8b, v0.8b, v0.8b  (Q=0,size=00)
    (10, 20, 30, "4h", 0x2E7E968A),  // mls v10.4h, v20.4h, v30.4h (Q=0)
];

#[test]
fn mla_matches_llvm_mc_golden() {
    for &(rd, rn, rm, arr, want) in MLA_GOLDEN {
        let got = word_of(encode_neon_mla(&[va(rd, arr), va(rn, arr), va(rm, arr)]));
        assert_eq!(
            got, want,
            "mla v{rd}.{arr}, v{rn}.{arr}, v{rm}.{arr}: got 0x{got:08X}, want 0x{want:08X}",
        );
        assert_eq!(
            ref_encode(rd, rn, rm, arr, 0),
            want,
            "reference encoder drift"
        );
    }
}

#[test]
fn mls_matches_llvm_mc_golden() {
    for &(rd, rn, rm, arr, want) in MLS_GOLDEN {
        let got = word_of(encode_neon_mls(&[va(rd, arr), va(rn, arr), va(rm, arr)]));
        assert_eq!(
            got, want,
            "mls v{rd}.{arr}, v{rn}.{arr}, v{rm}.{arr}: got 0x{got:08X}, want 0x{want:08X}",
        );
        assert_eq!(
            ref_encode(rd, rn, rm, arr, 1),
            want,
            "reference encoder drift"
        );
    }
}

// --- properties: register placement, opcode, u-bit (all PASS) -------------

proptest! {
    // === Oracle: differential vs llvm-mc-18-derived reference =============
    // For every valid arrangement and register triple, the implementation
    // must equal the independently-assembled (llvm-mc-verified) word. This
    // simultaneously proves Rd/Rn/Rm placement, the opcode (10010), and the
    // u-bit are all correct across the entire valid domain.
    #[test]
    fn mla_matches_reference_encoder(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        prop_assert_eq!(word_of(encode_neon_mla(&ops)), ref_encode(rd, rn, rm, arr, 0));
    }

    #[test]
    fn mls_matches_reference_encoder(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr), va(rm, arr)];
        prop_assert_eq!(word_of(encode_neon_mls(&ops)), ref_encode(rd, rn, rm, arr, 1));
    }

    // === Register-field placement: exact round-trip ======================
    // The 5-bit Rd/Rn/Rm fields must round-trip with NO swap and NO silent
    // truncation for in-range inputs; Q/size must match the arrangement.
    #[test]
    fn mla_register_fields_round_trip(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let w = word_of(encode_neon_mla(&[va(rd, arr), va(rn, arr), va(rm, arr)]));
        let (q, size) = neon_arr_to_q_size(arr).unwrap();
        prop_assert_eq!(w & 0x1F, rd, "Rd field");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field");
        prop_assert_eq!((w >> 16) & 0x1F, rm, "Rm field");
        prop_assert_eq!((w >> 30) & 0x1, q, "Q bit");
        prop_assert_eq!((w >> 22) & 0x3, size, "size field");
    }

    #[test]
    fn mls_register_fields_round_trip(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let w = word_of(encode_neon_mls(&[va(rd, arr), va(rn, arr), va(rm, arr)]));
        let (q, size) = neon_arr_to_q_size(arr).unwrap();
        prop_assert_eq!(w & 0x1F, rd, "Rd field");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field");
        prop_assert_eq!((w >> 16) & 0x1F, rm, "Rm field");
        prop_assert_eq!((w >> 30) & 0x1, q, "Q bit");
        prop_assert_eq!((w >> 22) & 0x3, size, "size field");
    }

    // === Fixed opcode + u-bit invariant ==================================
    // MLA: U=0, opcode=10010.  MLS: U=1, opcode=10010.  (MUL is the 10011 one.)
    // Bit 31=0, bits 28-24=01110, bit 21=1, bit 10=1 for both.
    #[test]
    fn mla_opcode_and_u_bit_correct(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let w = word_of(encode_neon_mla(&[va(rd, arr), va(rn, arr), va(rm, arr)]));
        prop_assert_eq!((w >> 31) & 1, 0u32, "bit 31");
        prop_assert_eq!((w >> 29) & 1, 0u32, "U bit must be 0 for MLA");
        prop_assert_eq!((w >> 24) & 0x1F, 0b01110u32, "bits 28-24");
        prop_assert_eq!((w >> 21) & 1, 1u32, "bit 21");
        prop_assert_eq!((w >> 11) & 0x1F, 0b10010u32, "opcode bits 15-11");
        prop_assert_eq!((w >> 10) & 1, 1u32, "bit 10");
    }

    #[test]
    fn mls_opcode_and_u_bit_correct(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        arr in valid_arrangement_strategy(),
    ) {
        let w = word_of(encode_neon_mls(&[va(rd, arr), va(rn, arr), va(rm, arr)]));
        prop_assert_eq!((w >> 31) & 1, 0u32, "bit 31");
        prop_assert_eq!((w >> 29) & 1, 1u32, "U bit must be 1 for MLS");
        prop_assert_eq!((w >> 24) & 0x1F, 0b01110u32, "bits 28-24");
        prop_assert_eq!((w >> 21) & 1, 1u32, "bit 21");
        prop_assert_eq!((w >> 11) & 0x1F, 0b10010u32, "opcode bits 15-11");
        prop_assert_eq!((w >> 10) & 1, 1u32, "bit 10");
    }
}

// --- ADDITIONAL BUG witness: mismatched-arrangement operands (#[ignore]) --
// Both encoders bind the Vn/Vm arrangements to `_` and ignore them, so any
// mismatch among the three arrangement specifiers is silently coerced to the
// destination arrangement. llvm-mc-18 rejects these with "invalid operand for
// instruction". The correct contract is to return `Err`.
//
// Reproduce (FAILS today, passes once the bug is fixed):
//   cargo test --lib mla_rejects_mismatched_arrangement -- --ignored
//   cargo test --lib mls_rejects_mismatched_arrangement -- --ignored

#[test]
#[ignore = "documented bug: mla ignores Vn/Vm arrangements, silently accepts mismatched operands"]
fn mla_rejects_mismatched_arrangement() {
    // Dest .4s but Vn/Vm carry different arrangements -> must be Err.
    let ops = vec![va(0, "4s"), va(1, "8b"), va(2, "2s")];
    assert!(
        encode_neon_mla(&ops).is_err(),
        "MLA requires all three arrangements to match; \
         `mla v0.4s, v1.8b, v2.2s` should be rejected (llvm-mc: invalid operand)"
    );
}

#[test]
#[ignore = "documented bug: mls ignores Vn/Vm arrangements, silently accepts mismatched operands"]
fn mls_rejects_mismatched_arrangement() {
    let ops = vec![va(0, "4s"), va(1, "8b"), va(2, "2s")];
    assert!(
        encode_neon_mls(&ops).is_err(),
        "MLS requires all three arrangements to match; \
         `mls v0.4s, v1.8b, v2.2s` should be rejected (llvm-mc: invalid operand)"
    );
}
