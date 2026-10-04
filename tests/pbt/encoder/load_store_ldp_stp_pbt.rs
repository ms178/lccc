//! Property-based tests for `encode_ldp_stp` (LDP/STP — Load/Store Register
//! Pair, ARM ARM §C6.2.125 / §C6.2.274, all addressing forms).
//!
//! ```text
//!  opc 101 V 0  op2  L  imm7   Rt2  Rn  Rt
//!  31-30  29-27 26 25 24-23 22 21-15 14-10 9-5 4-0
//! ```
//!   op2: 010 = signed offset ([base,#imm]), 011 = pre-index ([base,#imm]!),
//!        001 = post-index ([base],#imm)
//!   opc/V/shift per register class:
//!     W (32-bit GP): opc=00 V=0 shift=2
//!     X (64-bit GP): opc=10 V=0 shift=3
//!     S (32-bit FP): opc=00 V=1 shift=2
//!     D (64-bit FP): opc=01 V=1 shift=3
//!     Q (128-bit FP):opc=10 V=1 shift=4
//!
//! imm7 is a SIGNED 7-bit scaled immediate, range [-64, 63]; the effective
//! byte offset is `imm7 << shift`. Therefore a representable offset MUST be
//! a multiple of `1 << shift` AND lie within:
//!     shift=2 → [-256,  252]     shift=3 → [-512,  504]     shift=4 → [-1024, 1008]
//!
//! ## Findings
//!
//! **Finding 1 — out-of-range offset silently masked.** The encoder computes
//! `let imm7 = ((*offset >> shift) as i32) & 0x7F;` with NO range check. An
//! offset whose scaled value falls outside [-64, 63] is silently wrapped by
//! the `& 0x7F` mask:
//!     X regs, offset=+520 → 520>>3=65 → 65&0x7F=65 → sign-extends to -63 →
//!     decoded byte offset -504.  So `stp x0,x1,[x2,#520]` is silently
//!     re-encoded as `stp x0,x1,[x2,#-504]`.
//! This is the same masking defect already documented for the sibling
//! `encode_ldtr_sized` / `encode_ldrs` encoders in this crate.
//! Witness: `prop_out_of_range_offset_rejected` (#[ignore]); mechanism
//! documented by `prop_out_of_range_offset_silently_corrupted` (passes).
//!
//! **Finding 2 — unaligned offset silently truncated.** The right-shift
//! `*offset >> shift` discards the low `shift` bits, so an offset that is
//! not a multiple of the access size is silently rounded down with no error:
//!     X regs, offset=6 → 6>>3=0 → encoded as imm7=0 (byte offset 0).
//!     `stp x0,x1,[x2,#6]` is silently re-encoded as `stp x0,x1,[x2]`.
//! The ARM ARM requires the offset to be a multiple of the access size; a
//! non-multiple is unrepresentable and MUST be rejected.
//! Witness: `prop_unaligned_offset_rejected` (#[ignore]).

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_ldp_stp;
use proptest::prelude::*;

// ── Test scaffolding ─────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
struct RegClass {
    prefix: char,
    opc: u32,
    v: u32,
    shift: u32,
}

const CLASSES: [RegClass; 5] = [
    RegClass {
        prefix: 'w',
        opc: 0b00,
        v: 0,
        shift: 2,
    },
    RegClass {
        prefix: 'x',
        opc: 0b10,
        v: 0,
        shift: 3,
    },
    RegClass {
        prefix: 's',
        opc: 0b00,
        v: 1,
        shift: 2,
    },
    RegClass {
        prefix: 'd',
        opc: 0b01,
        v: 1,
        shift: 3,
    },
    RegClass {
        prefix: 'q',
        opc: 0b10,
        v: 1,
        shift: 4,
    },
];

fn class_strategy() -> impl Strategy<Value = RegClass> {
    prop::sample::select(CLASSES.to_vec())
}

/// Build an operand list `Rt1, Rt2, [Xbase, #off]` in the given addressing
/// form (0 = signed offset, 1 = pre-index, 2 = post-index).
fn ops(cls: RegClass, rt1: u32, rt2: u32, base: u32, off: i64, form: u32) -> Vec<Operand> {
    let rt1o = Operand::Reg(format!("{}{}", cls.prefix, rt1));
    let rt2o = Operand::Reg(format!("{}{}", cls.prefix, rt2));
    let base_s = format!("x{}", base);
    let mem = match form {
        0 => Operand::Mem {
            base: base_s,
            offset: off,
        },
        1 => Operand::MemPreIndex {
            base: base_s,
            offset: off,
        },
        _ => Operand::MemPostIndex {
            base: base_s,
            offset: off,
        },
    };
    vec![rt1o, rt2o, mem]
}

fn word(r: Result<EncodeResult, String>) -> u32 {
    match r {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected Word, got {:?}", other),
    }
}

/// Expected op2 discriminator ([24:23]) per addressing form.
fn expected_op2(form: u32) -> u32 {
    match form {
        0 => 0b010,
        1 => 0b011,
        _ => 0b001,
    }
}

/// Sign-extend the imm7 field ([21:15]) back to i32.
fn decode_imm7(w: u32) -> i32 {
    let raw = (w >> 15) & 0x7F;
    if raw & 0x40 != 0 {
        (raw | 0xFFFFFF80) as i32
    } else {
        raw as i32
    }
}

proptest! {
    // ── Property 1: all bit-fields round-trip for valid encodings (reference).
    // PASSES today. Exercises opc, V, the op2 addressing-form discriminator,
    // the L (load/store) bit, imm7, Rt1, Rt2, Rn across every register class
    // and all three addressing forms, using in-range aligned offsets.
    #[test]
    fn prop_fields_round_trip(
        cls in class_strategy(),
        is_load in any::<bool>(),
        form in 0u32..3u32,
        rt1 in 0u32..=31u32,
        rt2 in 0u32..=31u32,
        base in 0u32..=31u32,
        imm7_raw in -64i32..=63i32,
    ) {
        let off = (imm7_raw as i64) << cls.shift;
        let w = word(encode_ldp_stp(&ops(cls, rt1, rt2, base, off, form), is_load));

        prop_assert_eq!( w        & 0x1F,        rt1,          "Rt1 field");
        prop_assert_eq!((w >> 5)  & 0x1F,        base,         "Rn field");
        prop_assert_eq!((w >> 10) & 0x1F,        rt2,          "Rt2 field");
        prop_assert_eq!(decode_imm7(w),          imm7_raw,     "imm7 field");
        prop_assert_eq!((w >> 22) & 0x1,         is_load as u32, "L (load) bit");
        prop_assert_eq!((w >> 23) & 0x7,         expected_op2(form), "op2 addressing-form field");
        prop_assert_eq!((w >> 26) & 0x1,         cls.v,        "V (simd) bit");
        prop_assert_eq!((w >> 30) & 0x3,         cls.opc,      "opc field");
    }

    // ── Property 2: in-range aligned offset round-trips through imm7.
    // PASSES today. For a representable offset (a multiple of the access
    // size within the scaled range), the decoded imm7 must equal the input
    // scaled value, so the effective byte offset is recovered exactly.
    #[test]
    fn prop_in_range_offset_round_trips(
        cls in class_strategy(),
        is_load in any::<bool>(),
        form in 0u32..3u32,
        imm7_raw in -64i32..=63i32,
    ) {
        let off = (imm7_raw as i64) << cls.shift;
        let w = word(encode_ldp_stp(&ops(cls, 0, 1, 2, off, form), is_load));
        prop_assert_eq!(
            (decode_imm7(w) as i64) << cls.shift, off,
            "in-range aligned offset must round-trip"
        );
    }

    // ── Property 3: NEGATIVE CONTRACT — out-of-range offset must be Err.
    // imm7 is a signed 7-bit field, so a scaled value outside [-64, 63] is
    // unrepresentable and MUST be rejected rather than silently masked.
    //
    // EXPECTED TO FAIL today: the encoder masks with `& 0x7F` and returns Ok.
    // This failure IS Finding 1. #[ignore]'d so default `cargo test` stays
    // green; run with `cargo test -- --ignored`.
    #[test]
    #[ignore = "documented bug: ldp/stp imm7 offsets outside [-64,63] are masked, not rejected"]
    fn prop_out_of_range_offset_rejected(
        cls in class_strategy(),
        is_load in any::<bool>(),
        form in 0u32..3u32,
        // scaled value deliberately outside the signed-7-bit range
        imm7_out in (64i32..1024i32).prop_union(-1024i32..-65i32),
    ) {
        let off = (imm7_out as i64) << cls.shift;
        let res = encode_ldp_stp(&ops(cls, 0, 1, 2, off, form), is_load);
        prop_assert!(
            res.is_err(),
            "scaled offset {} (class {}:{}) is outside the signed-7-bit range [-64,63] and \
             must be rejected; got {:?}",
            imm7_out, cls.prefix, off, res
        );
    }

    // ── Property 4: out-of-range offset is silently corrupted (mechanism).
    // PASSES today — the smoking gun for Finding 1. Because only the low 7
    // bits survive the mask, the decoder cannot recover the original
    // out-of-range scaled value.
    #[test]
    fn prop_out_of_range_offset_silently_corrupted(
        cls in class_strategy(),
        is_load in any::<bool>(),
        form in 0u32..3u32,
        // Any scaled value >= 64 is outside [-64,63]; since decode_imm7 always
        // sign-extends back into [-64,63] it can never recover such a value.
        imm7_out in 64i32..1024i32,
    ) {
        let off = (imm7_out as i64) << cls.shift;
        let w = word(encode_ldp_stp(&ops(cls, 0, 1, 2, off, form), is_load));
        prop_assert_ne!(
            decode_imm7(w), imm7_out,
            "scaled offset {} was silently truncated into the 7-bit imm7 field", imm7_out
        );
    }

    // ── Property 5: NEGATIVE CONTRACT — unaligned offset must be Err.
    // The ARM ARM requires the offset to be a multiple of the access size
    // (1 << shift). A non-multiple is unrepresentable and MUST be rejected
    // rather than silently rounded down by the `>> shift`.
    //
    // EXPECTED TO FAIL today: the encoder does `*offset >> shift` and returns
    // Ok, dropping the low bits. This failure IS Finding 2. #[ignore]'d so
    // default `cargo test` stays green; run with `cargo test -- --ignored`.
    #[test]
    #[ignore = "documented bug: ldp/stp unaligned offsets are silently truncated by >> shift"]
    fn prop_unaligned_offset_rejected(
        cls in class_strategy(),
        is_load in any::<bool>(),
        form in 0u32..3u32,
        imm7_raw in -64i32..=63i32,
        low in 1u32..16u32, // remainder bits below the access size
    ) {
        let align: i64 = 1 << cls.shift;
        let low = (low as i64) % align;
        prop_assume!(low != 0, "need a non-zero misalignment");
        let off = ((imm7_raw as i64) << cls.shift) + low; // in range but unaligned
        let res = encode_ldp_stp(&ops(cls, 0, 1, 2, off, form), is_load);
        prop_assert!(
            res.is_err(),
            "offset {} is not a multiple of the {}-bit access size and must be rejected; got {:?}",
            off, align, res
        );
    }
}
