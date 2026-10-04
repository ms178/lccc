//! Property-based tests for `encode_neon_shift_imm`
//! (the AArch64 "Advanced SIMD shift by immediate" encoder for the right
//! shifts: USHR / SSHR, `0 Q U 0 11110 immh:immb 000001 Rn Rd`).
//!
//! Encoding (ARMv8 ARM, "Advanced SIMD shift by immediate"):
//!   `0 Q U 0 1 1 1 1 0 immh immb 0 0 0 0 0 1 Rn Rd`
//!    31 30 29 28-23 22-19 18-16 15----10 9-5 4-0
//!   bit[29] = U: 1 = USHR (unsigned), 0 = SSHR (signed).
//!   bits[15:10] = 000001 (the USHR/SSHR opcode).
//!
//! Element size / shift decode (architectural):
//!   `esize = 8 << HighestSetBit(immh)`
//!   `shift = (esize * 2) - UInt(immh:immb)`   =>   immh:immb = esize*2 - shift
//!   with `immh != 0` (immh == 0 is UNALLOCATED for this group).
//!   Valid `shift` range is therefore `[1, esize]`.
//!
//! Findings surfaced by this file (both kept as `#[ignore]`d bug witnesses so
//! the default `cargo test` stays green):
//!  * `prop_out_of_range_shifts_must_be_rejected` — the encoder performs NO
//!    range check on `shift`: `shift == 0` emits `immh == 0` (a reserved /
//!    UNALLOCATED encoding) and `shift > esize` silently re-encodes as a
//!    *different* element size. Per the ARM ARM both MUST be rejected.
//!  * `prop_is_unsigned_must_select_u_bit` — the `_is_unsigned` parameter is
//!    *dead*: the U bit (29) is hardcoded to `1` (USHR) regardless of the
//!    argument, so this function can never emit SSHR (U=0). Both findings are
//!    promoted to BUG_REPORT.md.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_shift_imm;
use proptest::prelude::*;

// (arrangement, element_bits, Q-bit)
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
fn encode_word(ops: &[Operand], is_unsigned: bool) -> Result<u32, String> {
    match encode_neon_shift_imm(ops, is_unsigned) {
        Ok(EncodeResult::Word(w)) => Ok(w),
        Ok(other) => Err(format!("unexpected non-Word result: {:?}", other)),
        Err(e) => Err(e),
    }
}

/// Architecturally-defined USHR word for a valid shift, U=1.
fn arm_ushr_word(q: u32, esize: u32, shift: u32, rn: u32, rd: u32) -> u32 {
    let immh_immb = (esize * 2).wrapping_sub(shift) & 0x7F; // full 7-bit field
    (q << 30) | (1 << 29) | (0b011110 << 23) | (immh_immb << 16) | (0b000001 << 10) | (rn << 5) | rd
}

proptest! {
    // 1. Reference / differential oracle.
    //    For every valid shift in [1, esize], the emitted word MUST equal the
    //    architecturally-defined USHR word. This jointly validates immh:immb
    //    field placement and every fixed bit (class[28:23]=011110, opc[15:10]=000001,
    //    U=1, bit31=0) across all four element sizes and both vector widths.
    #[test]
    fn prop_matches_arm_reference(
        rd in 0u32..32u32, rn in 0u32..32u32, s in 1u32..64u32
    ) {
        for &(arr, esize, q) in ARRAYS {
            let shift = ((s - 1) % esize) + 1; // valid shift in [1, esize]
            let got = encode_word(&shift_ops(rd, arr, rn, shift as i64), true)
                .expect("a valid shift must encode");
            let want = arm_ushr_word(q, esize, shift, rn, rd);
            prop_assert_eq!(got, want, "USHR word mismatch for {} shift {}", arr, shift);
        }
    }

    // 2. Algebraic: Rd / Rn / Q fields map 1:1 to the source operands for
    //    every arrangement + valid shift.
    #[test]
    fn prop_register_and_q_fields_preserved(
        rd in 0u32..32u32, rn in 0u32..32u32, s in 1u32..64u32
    ) {
        for &(arr, esize, q) in ARRAYS {
            let shift = ((s - 1) % esize) + 1;
            let w = encode_word(&shift_ops(rd, arr, rn, shift as i64), true)
                .expect("valid shift must encode");
            prop_assert_eq!(w & 0x1F, rd & 0x1F, "Rd for {}", arr);
            prop_assert_eq!((w >> 5) & 0x1F, rn & 0x1F, "Rn for {}", arr);
            prop_assert_eq!((w >> 30) & 1, q, "Q for {}", arr);
        }
    }

    // 3. Differential / round-trip on the immh:immb field (bits[22:16]).
    //    The encoded field MUST equal esize*2 - shift, and the shift MUST be
    //    fully reconstructable from the word.
    #[test]
    fn prop_shift_round_trips_through_immh_immb(
        rd in 0u32..32u32, s in 1u32..64u32
    ) {
        for &(arr, esize, _) in ARRAYS {
            let shift = ((s - 1) % esize) + 1;
            let w = encode_word(&shift_ops(rd, arr, 0, shift as i64), true)
                .expect("valid shift must encode");
            let field = (w >> 16) & 0x7F;
            prop_assert_eq!(field, esize * 2 - shift, "immh:immb for {} shift {}", arr, shift);
            prop_assert_eq!(esize * 2 - field, shift, "shift round-trip for {}", arr);
        }
    }

    // 4. BUG WITNESS (negative / error contract) — #[ignore].
    //    Per the ARM ARM, USHR requires `1 <= shift <= esize`; `shift == 0`
    //    forces `immh == 0000` (UNALLOCATED) and `shift > esize` is undefined.
    //    Both MUST therefore be rejected. The encoder does NO range check and
    //    silently returns Ok, so this property FAILS on the current code —
    //    hence `#[ignore]`. See BUG_REPORT.md.
    #[test]
    #[ignore]
    fn prop_out_of_range_shifts_must_be_rejected(rd in 0u32..32u32, rn in 0u32..32u32) {
        for &(arr, esize, _) in ARRAYS {
            // shift == 0 is UNALLOCATED.
            prop_assert!(
                encode_word(&shift_ops(rd, arr, rn, 0), true).is_err(),
                "shift=0 must be rejected for {} (immh would be 0)", arr
            );
            // shift just above the maximum for this element size.
            let too_big = (esize + 1) as i64;
            prop_assert!(
                encode_word(&shift_ops(rd, arr, rn, too_big), true).is_err(),
                "shift={} (> esize={}) must be rejected for {}", too_big, esize, arr
            );
        }
    }

    // 5. BUG WITNESS (dead-parameter contract) — #[ignore].
    //    `_is_unsigned == false` must select U(bit29)=0 (SSHR) and therefore
    //    produce a word DIFFERENT from the unsigned (USHR) path. The
    //    implementation ignores the parameter and hardcodes U=1, so this
    //    property FAILS on the current code — hence `#[ignore]`.
    #[test]
    #[ignore]
    fn prop_is_unsigned_must_select_u_bit(
        rd in 0u32..32u32, rn in 0u32..32u32, s in 1u32..64u32
    ) {
        for &(arr, esize, _) in ARRAYS {
            let shift = ((s - 1) % esize) + 1;
            let ops = shift_ops(rd, arr, rn, shift as i64);
            let w_signed = encode_word(&ops, false).expect("signed path must encode");
            // U bit must be 0 for the signed (SSHR) path.
            prop_assert_eq!((w_signed >> 29) & 1, 0u32, "U must be 0 for SSHR ({})", arr);
            // And it must differ from the unsigned word.
            let w_unsigned = encode_word(&ops, true).expect("unsigned path must encode");
            prop_assert_ne!(w_signed, w_unsigned, "signed/unsigned must differ for {}", arr);
        }
    }
}
