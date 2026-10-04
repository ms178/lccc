//! Property-based tests for `encode_neon_ushr`
//! (the AArch64 "Advanced SIMD unsigned shift right by immediate" encoder,
//! `0 Q 1 0 11110 immh:immb 000001 Rn Rd`).
//!
//! Encoding (ARMv8 ARM, "Advanced SIMD shift by immediate"):
//!   `0 Q 1 0 1 1 1 1 0 immh immb 0 0 0 0 0 1 Rn Rd`
//!    31 30 29 28-23 22-19 18-16 15----10 9-5 4-0
//!   bit[29] = U: 1 = USHR (unsigned) — hardcoded here, this function only
//!   emits USHR.  bits[15:10] = 000001 (the USHR/SSHR opcode).
//!
//! Element size / shift decode (architectural):
//!   `esize = 8 << HighestSetBit(immh)`
//!   `shift = (esize * 2) - UInt(immh:immb)`   =>   immh:immb = esize*2 - shift
//!   with `immh != 0` (immh == 0 is UNALLOCATED for this group).
//!   Valid `shift` range is therefore `[1, esize]`.
//!
//! Findings surfaced by this file (kept as `#[ignore]`d bug witnesses so the
//! default `cargo test` stays green):
//!  * `prop_out_of_range_shifts_must_be_rejected` — the encoder performs NO
//!    range check on `shift`: `shift == 0` emits `immh == 0000` (UNALLOCATED)
//!    and `shift > esize` silently re-encodes as a *different* element size.
//!    Per the ARM ARM both MUST be rejected. See BUG_REPORT_neon_ushr.md.
//!  * `prop_overflowing_shifts_must_not_panic` — for shifts larger than
//!    `2 * esize` the per-size subtraction (`16 - shift`, `32 - shift`, ...)
//!    underflows and **panics** in debug builds instead of returning `Err`.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_ushr;
use proptest::prelude::*;
use std::panic::{AssertUnwindSafe, catch_unwind};

// (arrangement, element_bits, Q-bit) — every architecturally valid USHR form.
const ARRAYS: &[(&str, u32, u32)] = &[
    ("8b", 8, 0),
    ("16b", 8, 1),
    ("4h", 16, 0),
    ("8h", 16, 1),
    ("2s", 32, 0),
    ("4s", 32, 1),
    ("2d", 64, 1),
];

fn shift_ops(rd: u32, arr: &str, rn: u32, shift: i64) -> Vec<Operand> {
    vec![
        Operand::RegArrangement {
            reg: format!("v{}", rd),
            arrangement: arr.to_string(),
        },
        Operand::RegArrangement {
            reg: format!("v{}", rn),
            arrangement: arr.to_string(),
        },
        Operand::Imm(shift),
    ]
}

/// Run the encoder and unwrap a `Word` result (fail the test on anything else).
fn encode_word(ops: &[Operand]) -> Result<u32, String> {
    match encode_neon_ushr(ops) {
        Ok(EncodeResult::Word(w)) => Ok(w),
        Ok(other) => Err(format!("unexpected non-Word result: {:?}", other)),
        Err(e) => Err(e),
    }
}

/// Architecturally-defined USHR word for a *valid* shift (esize*2 - shift,
/// masked into the full 7-bit immh:immb field — high bits are zero for the
/// smaller element sizes, so this matches the per-size masks used inside the
/// encoder for every shift in `[1, esize]`).
fn arm_ushr_word(q: u32, esize: u32, shift: u32, rn: u32, rd: u32) -> u32 {
    let immh_immb = (esize * 2).wrapping_sub(shift) & 0x7F;
    (q << 30) | (1 << 29) | (0b011110 << 23) | (immh_immb << 16) | (0b000001 << 10) | (rn << 5) | rd
}

proptest! {
    // 1. Reference / differential oracle.
    //    For every valid shift in [1, esize], the emitted word MUST equal the
    //    architecturally-defined USHR word. This jointly validates the
    //    immh:immb field placement and every fixed bit
    //    (bit31=0, U(bit29)=1, class[28:23]=011110, opc[15:10]=000001) across
    //    all four element sizes and both vector widths.
    #[test]
    fn prop_matches_arm_reference(
        rd in 0u32..32u32, rn in 0u32..32u32, s in 1u32..64u32
    ) {
        for &(arr, esize, q) in ARRAYS {
            let shift = ((s - 1) % esize) + 1; // valid shift in [1, esize]
            let got = encode_word(&shift_ops(rd, arr, rn, shift as i64))
                .expect("a valid shift must encode");
            let want = arm_ushr_word(q, esize, shift, rn, rd);
            prop_assert_eq!(got, want, "USHR word mismatch for {} shift {}", arr, shift);
        }
    }

    // 2. Algebraic: operand fields + every fixed bit map 1:1 to the source
    //    operands for every arrangement + valid shift.
    #[test]
    fn prop_register_q_and_fixed_bits_preserved(
        rd in 0u32..32u32, rn in 0u32..32u32, s in 1u32..64u32
    ) {
        for &(arr, esize, q) in ARRAYS {
            let shift = ((s - 1) % esize) + 1;
            let w = encode_word(&shift_ops(rd, arr, rn, shift as i64))
                .expect("valid shift must encode");
            // Register / width fields.
            prop_assert_eq!(w & 0x1F, rd & 0x1F, "Rd for {}", arr);
            prop_assert_eq!((w >> 5) & 0x1F, rn & 0x1F, "Rn for {}", arr);
            prop_assert_eq!((w >> 30) & 1, q, "Q for {}", arr);
            // Fixed bits that define the USHR instruction.
            prop_assert_eq!(w >> 31, 0, "bit31 must be 0");
            prop_assert_eq!((w >> 29) & 1, 1, "U(bit29) must be 1 (unsigned USHR)");
            prop_assert_eq!((w >> 23) & 0b111111, 0b011110, "class[28:23] for {}", arr);
            prop_assert_eq!((w >> 10) & 0x3F, 0b000001, "opc[15:10] for {}", arr);
        }
    }

    // 3. Round-trip on the immh:immb field (bits[22:16]).
    //    The encoded field MUST equal esize*2 - shift, and the shift MUST be
    //    fully reconstructable from the word.
    #[test]
    fn prop_shift_round_trips_through_immh_immb(
        rd in 0u32..32u32, s in 1u32..64u32
    ) {
        for &(arr, esize, _) in ARRAYS {
            let shift = ((s - 1) % esize) + 1;
            let w = encode_word(&shift_ops(rd, arr, 0, shift as i64))
                .expect("valid shift must encode");
            let field = (w >> 16) & 0x7F;
            prop_assert_eq!(field, esize * 2 - shift, "immh:immb for {} shift {}", arr, shift);
            prop_assert_eq!(esize * 2 - field, shift, "shift round-trip for {}", arr);
            // immh (top nibble of the field) must be non-zero — it is the
            // element-size discriminator, so 0000 would be UNALLOCATED.
            prop_assert!((field >> 3) != 0, "immh must be non-zero for {}", arr);
        }
    }

    // 4. BUG WITNESS (negative / error contract) — #[ignore].
    //    Per the ARM ARM, USHR requires `1 <= shift <= esize`; `shift == 0`
    //    forces `immh == 0000` (UNALLOCATED) and `shift > esize` is undefined
    //    (it silently re-encodes as a *different* element size). Both MUST be
    //    rejected. The encoder does NO range check and returns `Ok`, so this
    //    property FAILS on the current code — hence `#[ignore]`.
    //    (These two shift values never underflow the subtraction, so the only
    //    observable defect is the erroneous `Ok`.)
    #[test]
    #[ignore]
    fn prop_out_of_range_shifts_must_be_rejected(rd in 0u32..32u32, rn in 0u32..32u32) {
        for &(arr, esize, _) in ARRAYS {
            // shift == 0 is UNALLOCATED (immh == 0).
            prop_assert!(
                encode_word(&shift_ops(rd, arr, rn, 0)).is_err(),
                "shift=0 must be rejected for {} (immh would be 0)", arr
            );
            // shift just above the maximum for this element size.
            let too_big = (esize + 1) as i64;
            prop_assert!(
                encode_word(&shift_ops(rd, arr, rn, too_big)).is_err(),
                "shift={} (> esize={}) must be rejected for {}", too_big, esize, arr
            );
        }
    }

    // 5. BUG WITNESS (robustness / panic) — #[ignore].
    //    For shifts larger than `2 * esize` the per-size subtraction
    //    (`16 - shift`, `32 - shift`, `64 - shift`, `128 - shift`) underflows
    //    and the encoder PANICS in debug builds instead of returning `Err`.
    //    An encoder must never panic on bad user input — it must reject it.
    #[test]
    #[ignore]
    fn prop_overflowing_shifts_must_not_panic(rd in 0u32..32u32, rn in 0u32..32u32) {
        for &(arr, esize, _) in ARRAYS {
            for &shift in &[(2 * esize + 1), (4 * esize)] {
                let ops = shift_ops(rd, arr, rn, shift as i64);
                let outcome = catch_unwind(AssertUnwindSafe(|| encode_word(&ops)));
                match outcome {
                    Ok(Err(_)) => { /* correctly rejected */ }
                    Ok(Ok(w)) => prop_assert!(
                        false,
                        "shift={} (> 2*esize={}) for {} must be Err, got Ok(0x{:08x})",
                        shift, esize, arr, w
                    ),
                    Err(_) => prop_assert!(
                        false,
                        "shift={} (> 2*esize={}) for {} PANICKED (subtraction underflow)",
                        shift, esize, arr
                    ),
                }
            }
        }
    }
}
