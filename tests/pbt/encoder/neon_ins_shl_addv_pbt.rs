//! Property-based tests for three AArch64 NEON encoders in `neon.rs`:
//!   * `encode_neon_ins`  — `INS Vd.Ts[dst], (Wn/Xn | Vn.Ts[src])`
//!     (W for .b/.h/.s, X for .d; Advanced SIMD copy)
//!   * `encode_neon_shl`  — `SHL Vd.T, Vn.T, #shift`           (shift left by immediate)
//!   * `encode_neon_addv` — `ADDV Vd.T, Vn.T`                   (integer add across lanes)
//!
//! Conventions follow the sibling `neon_*_pbt.rs` files: independent `proptest`
//! suites, and golden words captured from
//! `llvm-mc-18 -triple=aarch64 -assemble -show-encoding` (llvm-mc prints
//! little-endian bytes; words below are reconstructed to a `u32`).
//!
//! ## Findings
//! The INS lane-index and GP-width contracts are checked in the active tests.
//! SHL and ADDV retain separate witnesses for findings outside this remediation.
//!
//! Confirmed findings referenced here:
//!   * `ADDV` opcode field one bit too low  — the transplanted campaign notes     (issue #191)
//!   * `ADDV` accepts unallocated arrs      — the transplanted campaign notes (issue #190)
//!   * `SHL`  out-of-range shift masking    — the transplanted campaign notes
//!     (new finding from this suite)

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::{encode_neon_addv, encode_neon_ins, encode_neon_shl, neon_arr_to_q_size};
use proptest::prelude::*;

// ──────────────────────────────────────────────────────────────────────────
// shared helpers
// ──────────────────────────────────────────────────────────────────────────

/// `Operand::RegArrangement { reg: "v{n}", arrangement }`
fn va(n: u32, arr: &str) -> Operand {
    Operand::RegArrangement {
        reg: format!("v{n}"),
        arrangement: arr.to_string(),
    }
}

/// `Operand::RegLane { reg: "v{rd}", elem_size, index }`
fn lane(rd: u32, elem_size: &str, index: u32) -> Operand {
    Operand::RegLane {
        reg: format!("v{rd}"),
        elem_size: elem_size.to_string(),
        index,
    }
}

/// `Operand::Reg("x{n}")` — a general-purpose (X) register.
fn gp(n: u32) -> Operand {
    Operand::Reg(format!("x{n}"))
}

/// Width-correct GP source for the general INS form.
fn gp_for_elem(n: u32, elem: &str) -> Operand {
    let prefix = if elem == "d" { "x" } else { "w" };
    let name = if n == 31 {
        format!("{prefix}zr")
    } else {
        format!("{prefix}{n}")
    };
    Operand::Reg(name)
}

/// `Operand::Imm(v)`.
fn imm(v: i64) -> Operand {
    Operand::Imm(v)
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

// ══════════════════════════════════════════════════════════════════════════
// encode_neon_ins
// ══════════════════════════════════════════════════════════════════════════
//
// INS (alias of MOV, Advanced SIMD copy group), two forms:
//   general : INS Vd.Ts[dst], Xn
//              0 Q 0 01110 000 imm5 0 0111 1 Rn Rd
//   element : INS Vd.Ts[dst], Vn.Ts[src]
//              0 Q 1 01110 000 imm5 0 imm4 1 Rn Rd
// imm5 packs element size (sentinel low bit) + lane index:
//   b -> 0b00001, idx<<1   (max lane 15)
//   h -> 0b00010, idx<<2   (max lane  7)
//   s -> 0b00100, idx<<3   (max lane  3)
//   d -> 0b01000, idx<<4   (max lane  1)

/// (element-size sentinel, index shift, max lane) for each `.b/.h/.s/.d`.
fn ins_size_meta(elem: &str) -> Option<(u32, u32, u32)> {
    Some(match elem {
        "b" => (0b00001, 1, 15),
        "h" => (0b00010, 2, 7),
        "s" => (0b00100, 3, 3),
        "d" => (0b01000, 4, 1),
        _ => return None,
    })
}

fn ins_elem_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("b"), Just("h"), Just("s"), Just("d")]
}

/// Golden words from `llvm-mc-18 -show-encoding` for the GENERAL form.
const INS_GP_GOLDEN: &[(u32, u32, &str, u32, u32)] = &[
    // (rd, rn, elem_size, index, expected_word)
    (0, 1, "s", 0, 0x4E041C20),   // ins v0.s[0], w1
    (5, 3, "s", 2, 0x4E141C65),   // ins v5.s[2], w3
    (9, 10, "b", 0, 0x4E011D49),  // ins v9.b[0], w10
    (2, 4, "h", 7, 0x4E1E1C82),   // ins v2.h[7], w4
    (3, 9, "d", 0, 0x4E081D23),   // ins v3.d[0], x9
    (31, 30, "d", 1, 0x4E181FDF), // ins v31.d[1], x30
];

/// Golden words from `llvm-mc-18 -show-encoding` for the ELEMENT form.
const INS_ELEM_GOLDEN: &[(u32, u32, &str, u32, u32, u32)] = &[
    // (rd, rn, elem_size, dst_idx, src_idx, expected_word)
    (5, 7, "s", 2, 3, 0x6E1464E5),  // ins v5.s[2], v7.s[3]
    (0, 1, "b", 15, 0, 0x6E1F0420), // ins v0.b[15], v1.b[0]
    (2, 3, "h", 7, 1, 0x6E1E1462),  // ins v2.h[7], v3.h[1]
    (4, 6, "d", 1, 0, 0x6E1804C4),  // ins v4.d[1], v6.d[0]
];

// --- INS: golden known-answer (valid inputs produce correct words) ---------

#[test]
fn ins_gp_form_matches_llvm() {
    for &(rd, rn, elem, idx, want) in INS_GP_GOLDEN {
        let got = word_of(encode_neon_ins(&[
            lane(rd, elem, idx),
            gp_for_elem(rn, elem),
        ]));
        assert_eq!(
            got, want,
            "ins v{rd}.{elem}[{idx}], x{rn}: got 0x{got:08X}, want 0x{want:08X}"
        );
    }
}

#[test]
fn ins_elem_form_matches_llvm() {
    for &(rd, rn, elem, dst, src, want) in INS_ELEM_GOLDEN {
        let got = word_of(encode_neon_ins(&[lane(rd, elem, dst), lane(rn, elem, src)]));
        assert_eq!(
            got, want,
            "ins v{rd}.{elem}[{dst}], v{rn}.{elem}[{src}]: got 0x{got:08X}, want 0x{want:08X}"
        );
    }
}

proptest! {
    // === Oracle: reference / round-trip (general form) ====================
    // Rd (4:0) and Rn (9:5) round-trip for any in-range lane index; imm5
    // (20:16) packs size sentinel + index exactly; bits 23:21 are the fixed
    // `000` of the copy group.  Verified against llvm-mc golden above.
    #[test]
    fn ins_gp_fields_and_imm5(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        elem in ins_elem_strategy(),
        idx in 0u32..=15u32,
    ) {
        let (sentinel, shift, max_lane) = ins_size_meta(elem).unwrap();
        let idx = idx % (max_lane + 1); // keep within per-size range
        let w = word_of(encode_neon_ins(&[lane(rd, elem, idx), gp_for_elem(rn, elem)]));
        prop_assert_eq!(w & 0x1F, rd, "Rd");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn");
        prop_assert_eq!((w >> 16) & 0x1F, (idx << shift) | sentinel, "imm5 == index|sentinel");
        prop_assert_eq!((w >> 21) & 0x7, 0b000, "fixed bits 23:21");
    }

    // === Oracle: reference / round-trip (element form) ====================
    // Rd / Rn round-trip; imm4 (14:11) packs the source lane with the
    // size-dependent shift (b:0, h:1, s:2, d:3); element-form top bits are
    // `011 01110 000` with bit 10 set.
    #[test]
    fn ins_elem_fields_and_imm4(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        elem in ins_elem_strategy(),
        dst_idx in 0u32..=15u32,
        src_idx in 0u32..=15u32,
    ) {
        let (_, _, max_lane) = ins_size_meta(elem).unwrap();
        let imm4_shift = match elem { "b" => 0, "h" => 1, "s" => 2, "d" => 3, _ => unreachable!() };
        let dst = dst_idx % (max_lane + 1);
        let src = src_idx % (max_lane + 1);
        let w = word_of(encode_neon_ins(&[lane(rd, elem, dst), lane(rn, elem, src)]));
        prop_assert_eq!(w & 0x1F, rd, "Rd");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn");
        prop_assert_eq!((w >> 11) & 0xF, src << imm4_shift, "imm4 == src lane");
        prop_assert_eq!((w >> 29) & 0x7, 0b011, "element-form fixed top bits");
        prop_assert_eq!(w & (1 << 10), 1 << 10, "bit 10 set");
    }

    // === Oracle: negative contract — too few / wrong-typed operands ========
    #[test]
    fn ins_rejects_malformed_operands(
        bad_size in "[a-z]{1,3}".prop_filter(
            "not a valid element size", |s| !matches!(s.as_str(), "b"|"h"|"s"|"d")),
    ) {
        prop_assert!(encode_neon_ins(&[]).is_err(), "empty operands");
        prop_assert!(encode_neon_ins(&[lane(0, "s", 0)]).is_err(), "single operand");
        prop_assert!(encode_neon_ins(&[gp(1), gp(2)]).is_err(), "first operand not a RegLane");
        let ops: Vec<Operand> = vec![
            Operand::RegLane { reg: "v0".into(), elem_size: bad_size.clone(), index: 0 },
            gp(1),
        ];
        prop_assert!(encode_neon_ins(&ops).is_err(), "elem_size {bad_size:?} must be rejected");
    }

    // === Oracle: negative contract — out-of-range lane index ==============
    // The ARMv8 ARM constrains the lane index per element size:
    //   .b -> [0,15], .h -> [0,7], .s -> [0,3], .d -> [0,1].
    // llvm-mc-18 rejects `ins v0.b[16], w1` with
    //   "vector lane must be an integer in range [0, 15]".
    // The encoder must return `Err` rather than truncate the index.
    //
    // EXPECTED: Err; kept active as a regression contract.
    #[test]
    fn ins_rejects_out_of_range_lane(
        elem in ins_elem_strategy(),
        over in 1u32..=16u32,
    ) {
        let (_, _, max_lane) = ins_size_meta(elem).unwrap();
        let bad = max_lane + over;
        let ops: Vec<Operand> = vec![
            Operand::RegLane { reg: "v0".into(), elem_size: elem.into(), index: bad },
            gp_for_elem(1, elem),
        ];
        prop_assert!(encode_neon_ins(&ops).is_err(),
            "out-of-range lane [{bad}] for .{elem} (max {max_lane}) must be Err, not truncated");
    }
}

/// Deterministic regression check for out-of-range INS lane indices.
#[test]
fn ins_lane_truncation_regression() {
    // .b max lane is 15; index 16 must be rejected, not `16 & 0xF = 0`.
    let ops = vec![lane(0, "b", 16), gp_for_elem(1, "b")];
    assert!(
        encode_neon_ins(&ops).is_err(),
        "INS .b[16] must be rejected (valid range 0..=15)"
    );
    // .d max lane is 1; index 2 must be rejected, not `2 & 0x1 = 0`.
    let ops = vec![lane(0, "d", 2), gp_for_elem(1, "d")];
    assert!(
        encode_neon_ins(&ops).is_err(),
        "INS .d[2] must be rejected (valid range 0..=1)"
    );
}

// ══════════════════════════════════════════════════════════════════════════
// encode_neon_shl
// ══════════════════════════════════════════════════════════════════════════
//
// SHL Vd.T, Vn.T, #shift   (Advanced SIMD shift by immediate)
//   0 Q U 011110 immh:immb 010101 Rn Rd
//   bit31=0, bit30=Q, bit29=U=0, bits28:23=011110,
//   bits22:16 = immh:immb = esize + shift, bits15:10 = 010101.
//
// Valid per-arrangement element sizes & shift ranges (from ARMv8-A ARM /
// llvm-mc-18 "immediate must be an integer in range [0, N]"):
//   8B/16B -> esize 8,  shift 0..=7
//   4H/8H  -> esize 16, shift 0..=15
//   2S/4S  -> esize 32, shift 0..=31
//   2D     -> esize 64, shift 0..=63
// (`.1d` is NOT a valid SHL vector arrangement — llvm-mc rejects it.)

fn shl_esize(arr: &str) -> Option<u32> {
    Some(match arr {
        "8b" | "16b" => 8,
        "4h" | "8h" => 16,
        "2s" | "4s" => 32,
        "2d" => 64,
        _ => return None,
    })
}

fn shl_arr_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![
        Just("8b"),
        Just("16b"),
        Just("4h"),
        Just("8h"),
        Just("2s"),
        Just("4s"),
        Just("2d"),
    ]
}

/// Golden words from `llvm-mc-18 -show-encoding` (valid shifts).
const SHL_GOLDEN: &[(u32, u32, &str, u32, u32)] = &[
    // (rd, rn, arr, shift, expected_word)
    (0, 1, "8b", 0, 0x0F085420),
    (0, 1, "8b", 1, 0x0F095420),
    (0, 1, "8b", 7, 0x0F0F5420),
    (0, 1, "16b", 3, 0x4F0B5420),
    (0, 1, "4h", 15, 0x0F1F5420),
    (0, 1, "8h", 7, 0x4F175420),
    (0, 1, "2s", 3, 0x0F235420),
    (0, 1, "4s", 8, 0x4F285420),
    (0, 1, "4s", 31, 0x4F3F5420),
    (2, 9, "4s", 8, 0x4F285522),
    (0, 1, "2d", 5, 0x4F455420),
    (0, 1, "2d", 63, 0x4F7F5420),
    (31, 30, "2d", 1, 0x4F4157DF),
];

#[test]
fn shl_matches_llvm() {
    for &(rd, rn, arr, shift, want) in SHL_GOLDEN {
        let got = word_of(encode_neon_shl(&[
            va(rd, arr),
            va(rn, arr),
            imm(shift as i64),
        ]));
        assert_eq!(
            got, want,
            "shl v{rd}.{arr}, v{rn}.{arr}, #{shift}: got 0x{got:08X}, want 0x{want:08X}"
        );
    }
}

proptest! {
    // === Oracle: algebraic / field placement (valid inputs) ===============
    // For a valid arrangement + in-range shift, the architecturally-fixed
    // fields are constant and the variable fields are placed/derived from the
    // ARMv8-A ARM layout (independent of the SUT implementation).
    #[test]
    fn shl_fixed_bits_and_field_placement(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in shl_arr_strategy(),
        shift in 0u32..=63u32,
    ) {
        let esize = shl_esize(arr).unwrap();
        let shift = shift % esize;            // keep within [0, esize-1]
        let (q, _) = neon_arr_to_q_size(arr).unwrap();
        let w = word_of(encode_neon_shl(&[va(rd, arr), va(rn, arr), imm(shift as i64)]));

        // fixed bits (spec-derived, not SUT-derived)
        prop_assert_eq!(w >> 31, 0, "bit 31 = 0");
        prop_assert_eq!((w >> 29) & 1, 0, "U bit = 0");
        prop_assert_eq!((w >> 23) & 0x3F, 0b011110, "bits 28:23 = 011110");
        prop_assert_eq!((w >> 10) & 0x3F, 0b010101, "bits 15:10 = 010101");
        // variable fields
        prop_assert_eq!(w & 0x1F, rd, "Rd");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn");
        prop_assert_eq!((w >> 30) & 1, q, "Q bit");
        // element size is encoded ONLY in immh:immb (no separate size field
        // for this encoding group); immh:immb == esize + shift is the SHL rule.
        prop_assert_eq!((w >> 16) & 0x7F, esize + shift, "immh:immb == esize + shift");
    }

    // === Oracle: negative contract — unsupported arrangement rejected =====
    #[test]
    fn shl_rejects_unsupported_arrangement(
        arr in "[a-z0-9]{1,3}".prop_filter("unknown arrangement", |s| {
            !matches!(s.as_str(), "8b"|"16b"|"4h"|"8h"|"2s"|"4s"|"2d")
        }),
    ) {
        let ops = vec![va(0, arr.as_str()), va(1, arr.as_str()), imm(1)];
        prop_assert!(encode_neon_shl(&ops).is_err(),
            "unsupported arrangement .{arr} must be rejected");
    }

    // === Oracle: negative contract — out-of-range shift ==================
    // llvm-mc-18 rejects e.g. `shl v0.8b, v1.8b, #8` with
    //   "immediate must be an integer in range [0, 7]".
    // The encoder currently computes `immh:immb = (esize + shift) & mask`,
    // so an out-of-range shift is silently wrapped to a wrong / UNALLOCATED
    // encoding (e.g. shift 8 on .8b -> immh:immb 0 -> UNALLOCATED) instead of
    // returning `Err`.  See encode_neon_shl_out_of_range_shift_masking.md.
    //
    // EXPECTED: Err.  Run: `cargo test --test pbt_arm_neon_b shl_rejects_out_of_range_shift -- --ignored`
    #[test]
    #[ignore]
    fn shl_rejects_out_of_range_shift(
        arr in shl_arr_strategy(),
        over in 0u32..=7u32,
    ) {
        let esize = shl_esize(arr).unwrap();
        let bad_shift = esize + over;            // just past the valid upper bound
        let ops = vec![va(0, arr), va(1, arr), imm(bad_shift as i64)];
        prop_assert!(encode_neon_shl(&ops).is_err(),
            "shl .{arr}, #{bad_shift} must be Err (valid shift range is 0..={})",
            esize - 1);
    }
}

/// Deterministic regression witness for the SHL out-of-range masking bug,
/// one boundary per element size.  `#[ignore]`d; reproduce with
/// `cargo test --test pbt_arm_neon_b shl_out_of_range_shift_regression -- --ignored`.
#[test]
#[ignore]
fn shl_out_of_range_shift_regression() {
    // (arrangement, shift one-past-max)
    for &(arr, shift) in &[
        ("8b", 8u32),
        ("16b", 8),
        ("4h", 16),
        ("8h", 16),
        ("2s", 32),
        ("4s", 32),
        ("2d", 64),
    ] {
        let ops = vec![va(0, arr), va(1, arr), imm(shift as i64)];
        let res = encode_neon_shl(&ops);
        let got = res
            .as_ref()
            .ok()
            .map(|e| match e {
                EncodeResult::Word(w) => *w,
                _ => 0,
            })
            .unwrap_or(0);
        assert!(
            res.is_err(),
            "shl v0.{arr}, v1.{arr}, #{shift} must be rejected; got Ok(0x{got:08X}) \
             (silently masked immh:immb = 0x{:02X})",
            (got >> 16) & 0x7F
        );
    }
}

// ══════════════════════════════════════════════════════════════════════════
// encode_neon_addv
// ══════════════════════════════════════════════════════════════════════════
//
// ADDV Vd.T, Vn.T   (Advanced SIMD across lanes, integer add reduction)
//   0 Q U 01110 size 11000 opcode 10 Rn Rd
//   U=0, opcode=11011, bits21:17=11000, bits11:10=10.
//
// Architecturally VALID arrangements: 8B, 16B, 4H, 8H, 4S.
// `.2s` / `.1d` / `.2d` (size=0b11, or the half-width 2s) are UNALLOCATED.

fn addv_valid_arr_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("8b"), Just("16b"), Just("4h"), Just("8h"), Just("4s")]
}

/// Correct field-by-field assembler (mirrors the *correct* sibling
/// `encode_neon_across(0, 0b11011)`, NOT the buggy `encode_neon_addv`).
fn ref_encode_addv(rd: u32, rn: u32, arr: &str) -> u32 {
    let (q, size) = neon_arr_to_q_size(arr).unwrap();
    (q << 30)
        | (0b01110u32 << 24)
        | (size << 22)
        | (0b11000u32 << 17) // bits 21:17
        | (0b11011u32 << 12) // opcode, bits 16:12   <-- the field the SUT gets wrong
        | (0b10u32 << 10)    // bits 11:10
        | (rn << 5)
        | rd
}

/// Golden words from `llvm-mc-18 -show-encoding` (llvm-mc spells the dest as a
/// scalar, e.g. `addv s0, v1.4s`, but the 32-bit word is identical: Rd is the
/// 5-bit register number).
const ADDV_GOLDEN: &[(u32, u32, &str, u32)] = &[
    // (rd, rn, arr, expected_word)
    (0, 1, "4s", 0x4EB1B820),   // addv s0, v1.4s
    (0, 1, "4h", 0x0E71B820),   // addv h0, v1.4h  (Q=0)
    (5, 6, "16b", 0x4E31B8C5),  // addv b5, v6.16b
    (31, 30, "8h", 0x4E71BCDF), // addv h31, v30.8h
    (0, 0, "8b", 0x0E31B800),   // addv b0, v0.8b   (Q=0,size=00)
    (10, 20, "4h", 0x0E71BA8A), // addv h10, v20.4h (Q=0)
];

proptest! {
    // === Oracle: algebraic / field placement (passing today) ==============
    // The five-bit register fields and Q/size lie outside the buggy opcode
    // region, so they round-trip / map correctly.
    #[test]
    fn addv_fields_round_trip_and_map_arrangement(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in addv_valid_arr_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let w = word_of(encode_neon_addv(&ops));
        let (q, size) = neon_arr_to_q_size(arr).unwrap();
        prop_assert_eq!(w & 0x1F, rd, "Rd");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn");
        prop_assert_eq!((w >> 30) & 1, q, "Q bit");
        prop_assert_eq!((w >> 22) & 0x3, size, "size field");
    }

    // === Oracle: negative contract — garbage arrangement rejected =========
    #[test]
    fn addv_rejects_unsupported_arrangement(
        arr in "[a-z0-9]{1,3}".prop_filter("unknown arrangement", |s| {
            !matches!(s.as_str(), "8b"|"16b"|"4h"|"8h"|"2s"|"4s"|"1d"|"2d")
        }),
    ) {
        let ops = vec![va(0, arr.as_str()), va(1, arr.as_str())];
        prop_assert!(encode_neon_addv(&ops).is_err(),
            "unsupported arrangement .{arr} must be rejected");
    }

    // === Oracle: differential vs correct reference encoder (BUGGY) ========
    // For every valid ADDV arrangement, the SUT must equal the correct
    // field-by-field word.  FAILS today: the opcode field is assembled one bit
    // too low (`0b110111 << 10` instead of `(0b11011<<12)|(0b10<<10)`).
    // See encode_neon_addv_wrong_opcode_field.md (issue #191).
    //
    // EXPECTED: equal.  Run: `cargo test --test pbt_arm_neon_b addv_matches_reference_encoder -- --ignored`
    #[test]
    fn addv_matches_reference_encoder(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in addv_valid_arr_strategy(),
    ) {
        let ops = vec![va(rd, arr), va(rn, arr)];
        let got = word_of(encode_neon_addv(&ops));
        let want = ref_encode_addv(rd, rn, arr);
        prop_assert_eq!(got, want, "addv: got 0x{:08X}, want 0x{:08X}", got, want);
    }

    // === Oracle: fixed-bits invariant (BUGGY) =============================
    // bits21:17=11000, opcode(16:12)=11011, bits11:10=10 must be constant.
    // FAILS today on the opcode + bits11:10 region.
    // Run: `cargo test --test pbt_arm_neon_b addv_fixed_bits_are_constant -- --ignored`
    #[test]
    fn addv_fixed_bits_are_constant(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        arr in addv_valid_arr_strategy(),
    ) {
        let w = word_of(encode_neon_addv(&[va(rd, arr), va(rn, arr)]));
        prop_assert_eq!(w >> 31, 0, "bit 31 = 0");
        prop_assert_eq!((w >> 29) & 1, 0, "U = 0");
        prop_assert_eq!((w >> 24) & 0x1F, 0b01110, "bits 28:24");
        prop_assert_eq!((w >> 17) & 0x1F, 0b11000, "bits 21:17");
        prop_assert_eq!((w >> 12) & 0x1F, 0b11011, "opcode bits 16:12");
        prop_assert_eq!((w >> 10) & 0x3, 0b10, "bits 11:10");
    }

    // === Oracle: negative contract — unallocated arrangements (BUGGY) ====
    // `.2s` / `.1d` / `.2d` are not valid ADDV reductions; llvm-mc rejects
    // them.  The SUT accepts them and emits a word.  See
    // encode_neon_addv_accepts_unallocated_arrangements.md (issue #190).
    //
    // EXPECTED: Err.  Run: `cargo test --test pbt_arm_neon_b addv_rejects_unallocated -- --ignored`
    #[test]
    #[ignore]
    fn addv_rejects_unallocated(
        arr in prop_oneof![Just("2s"), Just("1d"), Just("2d")],
    ) {
        let ops = vec![va(0, arr), va(1, arr)];
        let res = encode_neon_addv(&ops);
        let got = res.as_ref().ok().map(|e| match e {
            EncodeResult::Word(w) => *w,
            _ => 0,
        }).unwrap_or(0);
        prop_assert!(res.is_err(),
            "ADDV does not support .{arr}; expected Err but got Ok(0x{got:08X})");
    }
}

/// Golden known-answer.  FAILS today (opcode-field bug).  `#[ignore]`d;
/// reproduce with `cargo test --test pbt_arm_neon_b addv_matches_llvm -- --ignored`.
#[test]
#[ignore]
fn addv_matches_llvm() {
    for &(rd, rn, arr, want) in ADDV_GOLDEN {
        let got = word_of(encode_neon_addv(&[va(rd, arr), va(rn, arr)]));
        assert_eq!(
            got, want,
            "addv v{rd}.{arr}, v{rn}.{arr}: got 0x{got:08X}, want 0x{want:08X}"
        );
    }
}
