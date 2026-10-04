//! Property-based tests for `encode_neon_fcvtl` (the NEON FCVTL/FCVTL2 encoder).
//!
//! `FCVTL Vd.<Td>, Vn.<Ts>` / `FCVTL2 Vd.<Td>, Vn.<Ts>` widens each float
//! lane of the source: half→single (`<Td>` = `.4s`, `<Ts>` = `.4h`/`.8h`) or
//! single→double (`<Td>` = `.2d`, `<Ts>` = `.2s`/`.4s`).
//!
//! It belongs to the AArch64 "Advanced SIMD two-register miscellaneous" group
//! and is the widening counterpart of `FCVTN` (it differs only in the 5-bit
//! opcode field: `10111` here vs `10110` for FCVTN).
//!
//! Encoding (ARMv8 ARM, section C4.1.3 "Advanced SIMD two-register miscellaneous"):
//!   `0 Q 0 01110 size 10000 10111 10 Rn Rd`
//!    31 30 29-24 23-22 21-17  16-12 11-10 9-5 4-0
//!
//! `size` = 0 for half→single (destination `.4s`), `size` = 1 for single→double
//! (destination `.2d`). `Q` = 0 for FCVTL, `Q` = 1 for FCVTL2.
//!
//! Golden words (computed from the ARMv8 ARM; no cross-assembler/llvm-mc was
//! available in this environment for differential validation):
//!   `fcvtl  v0.4s, v0.4h` => `0x0e217800`   (Q=0, size=00)
//!   `fcvtl  v0.2d, v0.2s` => `0x0e617800`   (Q=0, size=01)
//!   `fcvtl2 v0.4s, v0.8h` => `0x4e217800`   (Q=1, size=00)
//!   `fcvtl2 v0.2d, v0.4s` => `0x4e617800`   (Q=1, size=01)
//!
//! Findings (both are the symmetric counterpart of the FCVTN destination bug;
//! here the SOURCE register's arrangement is the one that is dropped):
//!  * `prop_fcvtl_rejects_invalid_source_arrangement` (#[ignore]) documents
//!    that the SOURCE register's arrangement is read then discarded (`(rn, _)`).
//!    Per the ARMv8 ARM the source must be `.4h` (FCVTL, half) / `.8h` (FCVTL2,
//!    half) for a `.4s` destination, or `.2s` (FCVTL, single) / `.4s` (FCVTL2,
//!    single) for a `.2d` destination — and it must match the destination type
//!    and the `is_high` qualifier. The encoder accepts any source arrangement
//!    (even a bare `Operand::Reg`), so it FAILS today. See the bug report.
//!  * `prop_fcvtl_rejects_2s_destination` (#[ignore]) documents that there is
//!    NO `.2s` FCVTL destination form (FCVTL widens a full source register, so
//!    the only valid destinations are `.4s` and `.2d`). The encoder maps `.2s`
//!    to `size=0` and silently accepts it, emitting the same word as a `.4s`
//!    destination. It FAILS today. See the bug report.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_fcvtl;
use proptest::prelude::*;

// --- helpers --------------------------------------------------------------

/// `Operand::RegArrangement { reg: "v{n}", arrangement: arr }`.
fn vreg_arr(reg: u32, arr: &str) -> Operand {
    Operand::RegArrangement {
        reg: format!("v{reg}"),
        arrangement: arr.to_string(),
    }
}

fn reg_num_strategy() -> impl Strategy<Value = u32> {
    0u32..=31u32
}

/// Architecturally valid FCVTL destinations: `.4s` (half→single) and `.2d`
/// (single→double). There is NO `.2s` FCVTL form (see `prop_fcvtl_rejects_2s_destination`).
fn valid_dest_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("4s"), Just("2d")]
}

/// Destination arrangements that are UNDEFINED for FCVTL.
fn invalid_dest_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![
        Just("8b"),
        Just("16b"),
        Just("4h"),
        Just("8h"),
        Just("1d"),
        Just("1q"),
    ]
}

fn word_of(res: Result<EncodeResult, String>) -> u32 {
    match res {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {other:?}"),
    }
}

/// Independent re-derivation of the ARMv8 ARM FCVTL encoding for the oracle.
fn fcvtl_reference(rd: u32, rn: u32, arr_d: &str, is_high: bool) -> u32 {
    let sz: u32 = match arr_d {
        "4s" | "2s" => 0,
        "2d" => 1,
        _ => unreachable!("oracle only called with valid destinations"),
    };
    let q: u32 = if is_high { 1 } else { 0 };
    (q << 30)
        | (0b01110 << 24)
        | (sz << 22)
        | (0b10000 << 17)
        | (0b10111 << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd
}

/// Constant opcode portion of the FCVTL encoding with Q=size=Rn=Rd=0.
const FCVTL_BASE: u32 = 0x0E21_7800;

// --- properties -----------------------------------------------------------

proptest! {
    /// Reference oracle: for every valid (rd, rn, destination, is_high) the
    /// encoded word equals the ARMv8 ARM reference encoding. (The source
    /// arrangement is ignored by the encoder, so a sensible matching source is
    /// supplied purely for realism.)
    #[test]
    fn prop_fcvtl_matches_arm_reference(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        dest in valid_dest_strategy(),
        is_high in any::<bool>(),
    ) {
        let src = if dest == "2d" { "2s" } else { "4h" };
        let ops = vec![vreg_arr(rd, dest), vreg_arr(rn, src)];
        let word = word_of(encode_neon_fcvtl(&ops, is_high));
        let expected = fcvtl_reference(rd, rn, dest, is_high);
        prop_assert_eq!(word, expected);
    }

    /// Field isolation: Rd occupies exactly bits [4:0], Rn exactly bits [9:5],
    /// and neither register number leaks into any other field (incl. when both
    /// are at their maximum value 31).
    #[test]
    fn prop_fcvtl_fields_isolated(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
    ) {
        let base = word_of(encode_neon_fcvtl(&[vreg_arr(0, "4s"), vreg_arr(0, "4h")], false));

        let with_rd = word_of(encode_neon_fcvtl(&[vreg_arr(rd, "4s"), vreg_arr(0, "4h")], false));
        prop_assert_eq!(with_rd & 0xFFFF_FFE0, base & 0xFFFF_FFE0, "Rd leaked above bit 4");
        prop_assert_eq!(with_rd & 0x1F, rd, "Rd not in bits [4:0]");

        let with_rn = word_of(encode_neon_fcvtl(&[vreg_arr(0, "4s"), vreg_arr(rn, "4h")], false));
        prop_assert_eq!(with_rn & !0x3E0, base & !0x3E0, "Rn leaked outside bits [9:5]");
        prop_assert_eq!((with_rn >> 5) & 0x1F, rn, "Rn not in bits [9:5]");
    }

    /// Q bit (bit 30) is driven solely by `is_high`; the `size` field (bit 22)
    /// is the sole difference between a `.4s` destination (size=0, half→single)
    /// and a `.2d` destination (size=1, single→double). Masking off
    /// Q|size|Rn|Rd leaves the constant opcode, equal to `FCVTL_BASE`.
    #[test]
    fn prop_fcvtl_q_size_and_fixed_bits(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        is_high in any::<bool>(),
    ) {
        let single = word_of(encode_neon_fcvtl(&[vreg_arr(rd, "4s"), vreg_arr(rn, "4h")], is_high));
        let double = word_of(encode_neon_fcvtl(&[vreg_arr(rd, "2d"), vreg_arr(rn, "2s")], is_high));

        // Q == 1 iff is_high.
        prop_assert_eq!(single & 0x4000_0000, if is_high { 0x4000_0000 } else { 0 });
        // half->single: size=0; single->double: size=1. Only bit 22 differs.
        prop_assert_eq!(single & (1 << 22), 0);
        prop_assert_eq!(double & (1 << 22), 1 << 22);
        prop_assert_eq!(single & !(1u32 << 22), double & !(1u32 << 22));

        // Everything outside Q|size|Rn|Rd is the constant opcode.
        let mask = !(0x4000_0000 | (0b11 << 22) | 0x3E0 | 0x1F);
        prop_assert_eq!(single & mask, FCVTL_BASE & mask);
        prop_assert_eq!(double & mask, FCVTL_BASE & mask);
    }

    /// NEGATIVE CONTRACT (PASSES): per the ARMv8 ARM the FCVTL destination must
    /// be `.4s` or `.2d`. Every other destination arrangement, as well as a
    /// bare destination register, must be rejected.
    #[test]
    fn prop_fcvtl_rejects_invalid_dest_arrangements(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        dest in invalid_dest_strategy(),
        is_high in any::<bool>(),
    ) {
        let ops = vec![vreg_arr(rd, dest), vreg_arr(rn, "4h")];
        prop_assert!(
            encode_neon_fcvtl(&ops, is_high).is_err(),
            "fcvtl with destination .{} should be rejected, got {:?}",
            dest,
            encode_neon_fcvtl(&ops, is_high),
        );

        // A bare destination register (no arrangement) is equally invalid.
        let bare = vec![Operand::Reg(format!("v{rd}")), vreg_arr(rn, "4h")];
        prop_assert!(
            encode_neon_fcvtl(&bare, is_high).is_err(),
            "fcvtl with bare-Reg destination should be rejected, got {:?}",
            encode_neon_fcvtl(&bare, is_high),
        );
    }

    /// NEGATIVE CONTRACT (EXPECTED TO FAIL, #[ignore]): the SOURCE register's
    /// arrangement is read then discarded (`(rn, _)`). Per the ARMv8 ARM the
    /// source must be `.4h` (FCVTL) / `.8h` (FCVTL2) for a half→single
    /// (`.4s`) destination, or `.2s` (FCVTL) / `.4s` (FCVTL2) for a
    /// single→double (`.2d`) destination, and it must match the destination
    /// type and the `is_high` qualifier. Any other source arrangement — and a
    /// bare source register — is an operand-size mismatch and must be rejected.
    /// The current implementation ignores the source arrangement entirely, so
    /// this property FAILS. Marked `#[ignore]` so `cargo test` stays green.
    #[test]
    #[ignore]
    fn prop_fcvtl_rejects_invalid_source_arrangement(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
    ) {
        // For each valid (dest, is_high), the architecture permits exactly one
        // source arrangement; every other arrangement (plus a bare register)
        // must be rejected.
        for (dest, is_high, valid_src) in [
            ("4s", false, "4h"),
            ("4s", true, "8h"),
            ("2d", false, "2s"),
            ("2d", true, "4s"),
        ] {
            for bad in ["8b", "16b", "4h", "8h", "2s", "4s", "2d", "1d"] {
                if bad == valid_src {
                    continue;
                }
                let ops = vec![vreg_arr(rd, dest), vreg_arr(rn, bad)];
                prop_assert!(
                    encode_neon_fcvtl(&ops, is_high).is_err(),
                    "fcvtl with source .{} (dest .{}, is_high={}) should be rejected, got {:?}",
                    bad, dest, is_high, encode_neon_fcvtl(&ops, is_high),
                );
            }
        }
        // A bare source register (no arrangement) is always a mismatch.
        let bare = vec![vreg_arr(rd, "4s"), Operand::Reg(format!("v{rn}"))];
        prop_assert!(
            encode_neon_fcvtl(&bare, false).is_err(),
            "fcvtl with bare-Reg source should be rejected, got {:?}",
            encode_neon_fcvtl(&bare, false),
        );
    }

    /// NEGATIVE CONTRACT (EXPECTED TO FAIL, #[ignore]): per the ARMv8 ARM there
    /// is NO `.2s` FCVTL destination form — FCVTL widens a full source register,
    /// so the only valid destinations are `.4s` (half→single) and `.2d`
    /// (single→double). The current implementation maps `.2s` to `size=0` and
    /// silently accepts it, emitting the same word as a `.4s` destination.
    /// This property asserts `.2s` is rejected; it FAILS today. Marked
    /// `#[ignore]` so `cargo test` stays green. (`.2s` IS a valid *source* for
    /// the single→double (`.2d`) case — the defect is specifically that it is
    /// accepted as a *destination*.)
    #[test]
    #[ignore]
    fn prop_fcvtl_rejects_2s_destination(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        is_high in any::<bool>(),
    ) {
        let ops = vec![vreg_arr(rd, "2s"), vreg_arr(rn, "2h")];
        prop_assert!(
            encode_neon_fcvtl(&ops, is_high).is_err(),
            "fcvtl with .2s destination is undefined and should be rejected, got {:?}",
            encode_neon_fcvtl(&ops, is_high),
        );
    }
}

// --- deterministic boundary / golden checks -------------------------------

#[test]
fn golden_fcvtl_matches_arm_reference() {
    // fcvtl  v0.4s, v0.4h => 0x0e217800  (Q=0, size=00)
    assert_eq!(
        word_of(encode_neon_fcvtl(
            &[vreg_arr(0, "4s"), vreg_arr(0, "4h")],
            false
        )),
        0x0E21_7800
    );
    // fcvtl  v0.2d, v0.2s => 0x0e617800  (Q=0, size=01)
    assert_eq!(
        word_of(encode_neon_fcvtl(
            &[vreg_arr(0, "2d"), vreg_arr(0, "2s")],
            false
        )),
        0x0E61_7800
    );
    // fcvtl2 v0.4s, v0.8h => 0x4e217800  (Q=1, size=00)
    assert_eq!(
        word_of(encode_neon_fcvtl(
            &[vreg_arr(0, "4s"), vreg_arr(0, "8h")],
            true
        )),
        0x4E21_7800
    );
    // fcvtl2 v0.2d, v0.4s => 0x4e617800  (Q=1, size=01)
    assert_eq!(
        word_of(encode_neon_fcvtl(
            &[vreg_arr(0, "2d"), vreg_arr(0, "4s")],
            true
        )),
        0x4E61_7800
    );

    // Non-zero registers: fcvtl v5.4s, v7.4h => base | (7<<5) | 5 = 0x0e2178e5
    assert_eq!(
        word_of(encode_neon_fcvtl(
            &[vreg_arr(5, "4s"), vreg_arr(7, "4h")],
            false
        )),
        0x0E21_7800 | (7 << 5) | 5,
    );
    assert_eq!(0x0E21_7800 | (7 << 5) | 5, 0x0E21_78E5);
}

#[test]
fn rejects_too_few_operands() {
    // 0 or 1 operands must error (get_neon_reg returns None and the function
    // surfaces that as Err).
    assert!(
        encode_neon_fcvtl(&[], false).is_err(),
        "0 operands must error"
    );
    assert!(
        encode_neon_fcvtl(&[vreg_arr(0, "4s")], false).is_err(),
        "1 operand must error"
    );
    // Exactly two valid operands must succeed.
    assert!(encode_neon_fcvtl(&[vreg_arr(0, "4s"), vreg_arr(1, "4h")], false).is_ok());
}
