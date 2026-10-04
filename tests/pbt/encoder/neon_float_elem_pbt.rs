//! Property-based tests for `encode_neon_float_elem` — the AArch64
//! **"Advanced SIMD Floating-point by-element"** encoder used by `fmul`,
//! `fmla`, and `fmls` when their third operand is a vector lane
//! (e.g. `fmul v0.4s, v1.4s, v2.s[0]`).
//!
//! Reference encoding (ARMv8 ARM, "Advanced SIMD scalar/by-element"):
//!
//! ```text
//!   31 30 29 28-24 23 22 21 20-16 15-12 11 10 9-5 4-0
//!    0  Q  U  0 11111 sz  L  M:Rm   opcode  H  0  Rn  Rd
//! ```
//!
//! where `0 11111` spans bits [28:23] (bit 23 is a fixed `1`), `sz` selects
//! single (0) / double (1), and the lane index packs as:
//!   * `sz = 0` (.2s/.4s): `index = H:L`, valid range `[0, 3]`
//!   * `sz = 1` (.2d):     `index = H`,     valid range `[0, 1]`
//!
//! ## Oracle
//!
//! The reference word is re-assembled field-by-field from the layout above
//! (independent of the implementation) and pinned to an absolute golden table
//! produced by `llvm-mc-18 --triple=aarch64 --assemble --show-encoding`.
//! All three real call sites carry `U = 0`:
//!   `fmul v0.4s,v1.4s,v2.s[0]  = 0x4F829020`  (opcode=1001, U=0)
//!   `fmla v0.4s,v0.4s,v0.s[1]  = 0x4FA01000`  (opcode=0001, U=0)
//!   `fmls v5.2d,v7.2d,v9.d[1]  = 0x4FC958E5`  (opcode=0101, U=0, sz=1)
//!
//! ## Validation contracts
//!
//! Active properties compare valid words against the ARM encoding layout and
//! reject out-of-range lane indices. Deterministic negative checks cover lane
//! element-size and vector-arrangement mismatches as well as exact arity.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_float_elem;
use proptest::prelude::*;

// --- helpers --------------------------------------------------------------

/// Build a NEON vector-register operand `v{n}.<arr>`.
fn reg_arr(n: u32, arr: &str) -> Operand {
    Operand::RegArrangement {
        reg: format!("v{n}"),
        arrangement: arr.to_string(),
    }
}

/// Build a register-lane operand `v{rn}.<elem_size>[index]`.
fn lane(rn: u32, elem_size: &str, index: u32) -> Operand {
    Operand::RegLane {
        reg: format!("v{rn}"),
        elem_size: elem_size.to_string(),
        index,
    }
}

/// In-range vector register number 0..=31 (what `parse_reg_num` accepts).
fn reg_num_strategy() -> impl Strategy<Value = u32> {
    0u32..=31u32
}

/// The only arrangements this encoder accepts for the FP by-element group.
fn arr_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("2s"), Just("4s"), Just("2d")]
}

/// Map an arrangement to its `(Q, sz)` pair per the ARM ARM.
fn arr_qs(arr: &str) -> (u32, u32) {
    match arr {
        "2s" => (0, 0),
        "4s" => (1, 0),
        "2d" => (1, 1),
        _ => unreachable!("arr_qs: unsupported arrangement {arr}"),
    }
}

/// The `(Q, sz)` pair, derived as the encoder itself derives it.
fn qs_strategy() -> impl Strategy<Value = (u32, u32)> {
    prop_oneof![Just((0, 0)), Just((1, 0)), Just((1, 1))]
}

/// Architecturally valid max lane index for a given `sz`.
fn max_index(sz: u32) -> u32 {
    if sz == 0 { 3 } else { 1 }
}

/// The three real `(U, opcode)` parameter sets used by the dispatcher for the
/// float by-element group (all carry `U = 0`).
fn instr_strategy() -> impl Strategy<Value = (u32, u32)> {
    prop_oneof![
        Just((0u32, 0b1001u32)), // fmul
        Just((0u32, 0b0001u32)), // fmla
        Just((0u32, 0b0101u32)), // fmls
    ]
}

fn word_of(res: Result<EncodeResult, String>) -> u32 {
    match res {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {other:?}"),
    }
}

/// ARMv8 ARM reference word for the FP by-element layout. NOTE: bit 23 is set
/// (the fixed `1` of the `0 11111` group) — this is what a correct encoder
/// emits and what `llvm-mc-18` produces.
fn reference_word(
    (q, u, sz, opcode): (u32, u32, u32, u32),
    (rd, rn, rm, index): (u32, u32, u32, u32),
) -> u32 {
    let (h, l, m) = if sz == 0 {
        ((index >> 1) & 1, index & 1, (rm >> 4) & 1)
    } else {
        (index & 1, 0u32, (rm >> 4) & 1)
    };
    (q << 30)
        | (u << 29)
        | (0b011111u32 << 23) // bits [28:23] = 0 11111  -> bit 23 = 1
        | (sz << 22)
        | (l << 21)
        | (m << 20)
        | ((rm & 0xF) << 16)
        | (opcode << 12)
        | (h << 11)
        | (rn << 5)
        | rd
}

// --- properties (passing) -------------------------------------------------

proptest! {
    // === Oracle: field isolation (Rd / Rn / Rm / opcode) =================
    // Rd occupies [4:0], Rn [9:5], the index+Rm group [21:11] (L M Rm
    // opcode H), and opcode [15:12]. Varying one operand leaves every other
    // field bit-identical.
    #[test]
    fn prop_fields_isolated(
        sz in 0u32..=1u32,
        u in 0u32..=1u32,
        opcode in 0u32..=15u32,
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        idx_mod in 0u32..=15u32,
    ) {
        let q = 1u32; // any valid Q works for isolation (only 4s/2d reach here)
        let arr = match sz { 0 => "4s", _ => "2d" };
        let idx = idx_mod % (max_index(sz) + 1);
        let base = word_of(encode_neon_float_elem(&[reg_arr(0, arr), reg_arr(0, arr), lane(0, if sz == 0 { "s" } else { "d" }, idx)], u, opcode));

        // Rd round-trips in [4:0] and leaks nowhere above.
        let with_rd = word_of(encode_neon_float_elem(&[reg_arr(rd, arr), reg_arr(0, arr), lane(0, if sz == 0 { "s" } else { "d" }, idx)], u, opcode));
        prop_assert_eq!(with_rd & 0x1F, rd & 0x1F, "Rd not in [4:0]");
        prop_assert_eq!(with_rd & 0xFFFF_FFE0, base & 0xFFFF_FFE0, "Rd leaked above bit 4");

        // Rn round-trips in [9:5] and leaks nowhere else.
        let with_rn = word_of(encode_neon_float_elem(&[reg_arr(0, arr), reg_arr(rn, arr), lane(0, if sz == 0 { "s" } else { "d" }, idx)], u, opcode));
        prop_assert_eq!((with_rn >> 5) & 0x1F, rn, "Rn not in [9:5]");
        prop_assert_eq!(with_rn & !0x3E0, base & !0x3E0, "Rn leaked outside [9:5]");

        // Rm round-trips as the 5-bit group [20:16] (M:Rm) and leaks nowhere.
        let with_rm = word_of(encode_neon_float_elem(&[reg_arr(0, arr), reg_arr(0, arr), lane(rm, if sz == 0 { "s" } else { "d" }, idx)], u, opcode));
        prop_assert_eq!((with_rm >> 16) & 0x1F, rm & 0x1F, "Rm not in [20:16]");
        prop_assert_eq!(with_rm & !0x001F_0000, base & !0x001F_0000, "Rm leaked outside [20:16]");

        // opcode round-trips in [15:12] and leaks nowhere.
        let with_op = word_of(encode_neon_float_elem(&[reg_arr(0, arr), reg_arr(0, arr), lane(0, if sz == 0 { "s" } else { "d" }, idx)], u, opcode));
        prop_assert_eq!((with_op >> 12) & 0xF, opcode, "opcode not in [15:12]");
        let _ = with_op;
        let _ = q;
    }

    // === Oracle: structural (Q, sz, fixed bits) ==========================
    // Q and sz are determined by the destination arrangement; bit 10 is the
    // fixed `0` of the by-element group; bits [31]=0. (None of these involve
    // bit 23, so this passes.)
    #[test]
    fn prop_q_sz_fixed_bits(
        arr in arr_strategy(),
        u in 0u32..=1u32,
        opcode in 0u32..=15u32,
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        idx_mod in 0u32..=15u32,
    ) {
        let (q, sz) = arr_qs(arr);
        let elem = if sz == 0 { "s" } else { "d" };
        let idx = idx_mod % (max_index(sz) + 1);
        let w = word_of(encode_neon_float_elem(&[reg_arr(rd, arr), reg_arr(rn, arr), lane(rm, elem, idx)], u, opcode));
        prop_assert_eq!(w >> 31, 0u32, "bit 31 must be 0");
        prop_assert_eq!((w >> 30) & 1, q, "Q (bit 30) must track arrangement");
        prop_assert_eq!((w >> 22) & 1, sz, "sz (bit 22) must track arrangement");
        prop_assert_eq!((w >> 10) & 1, 0u32, "bit 10 must be the fixed 0");
        // U (bit 29) round-trips whatever the caller passes.
        prop_assert_eq!((w >> 29) & 1, u & 1, "U must round-trip");
    }

    // === Oracle: index round-trips when in range ========================
    // For an in-range lane index the encoded H/L bits decode back to that
    // index, so no information is lost. (Out-of-range indices are a separate
    // negative contract; see `prop_rejects_out_of_range_lane_index`.)
    #[test]
    fn prop_index_round_trips_in_range(
        (q, sz) in qs_strategy(),
        (u, opcode) in instr_strategy(),
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        idx_mod in 0u32..=15u32,
    ) {
        let idx = idx_mod % (max_index(sz) + 1);
        let arr = match (q, sz) { (0, 0) => "2s", (1, 0) => "4s", (1, 1) => "2d", _ => "4s" };
        let elem = if sz == 0 { "s" } else { "d" };
        let w = word_of(encode_neon_float_elem(&[reg_arr(rd, arr), reg_arr(rn, arr), lane(rm, elem, idx)], u, opcode));
        let h = (w >> 11) & 1;
        let l = (w >> 21) & 1;
        let decoded = if sz == 0 { (h << 1) | l } else { h };
        prop_assert_eq!(decoded, idx, "in-range index must round-trip via H/L");
    }
}

// --- active reference and negative-contract properties --------------------

proptest! {
    // === Oracle: differential / reference (ARM ARM template) =============
    // The encoded word must equal the ARM ARM
    // `0 Q U 0 11111 sz L M Rm opcode H 0 Rn Rd` template.
    #[test]
        fn prop_matches_arm_reference(
        (q, sz) in qs_strategy(),
        (u, opcode) in instr_strategy(),
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        rm in reg_num_strategy(),
        idx_mod in 0u32..=15u32,
    ) {
        let idx = idx_mod % (max_index(sz) + 1);
        let arr = match (q, sz) { (0, 0) => "2s", (1, 0) => "4s", (1, 1) => "2d", _ => "4s" };
        let elem = if sz == 0 { "s" } else { "d" };
        let ops = vec![reg_arr(rd, arr), reg_arr(rn, arr), lane(rm, elem, idx)];
        let w = word_of(encode_neon_float_elem(&ops, u, opcode));
        prop_assert_eq!(
            w,
            reference_word((q, u, sz, opcode), (rd, rn, rm, idx)),
            "encoded word must equal the ARM ARM template (bit 23 = 1)"
        );
    }

    // === Oracle: negative contract (out-of-range lane index) ============
    // The lane index is constrained by element size: .s -> [0,3], .d -> [0,1].
    // Out-of-range values must return `Err`, not wrap into a valid lane.
    #[test]
        fn prop_rejects_out_of_range_lane_index(
        bad in prop_oneof![
            Just((0u32, 4u32)),  // .s, index 4 (> max 3)
            Just((0u32, 5u32)),
            Just((1u32, 2u32)),  // .d, index 2 (> max 1)
            Just((1u32, 7u32)),
        ],
    ) {
        let (sz, bad_idx) = bad;
        let arr = if sz == 0 { "4s" } else { "2d" };
        let elem = if sz == 0 { "s" } else { "d" };
        let ops = vec![reg_arr(0, arr), reg_arr(0, arr), lane(0, elem, bad_idx)];
        prop_assert!(
            encode_neon_float_elem(&ops, 0, 0b1001).is_err(),
            "out-of-range lane [{bad_idx}] for .{elem} must be rejected, \
             not silently truncated; got {:?}", encode_neon_float_elem(&ops, 0, 0b1001));
    }
}

// --- golden differential regression (llvm-mc-18 anchors) ------------------
//
// Every word below was emitted by `llvm-mc-18 --triple=aarch64 --assemble
// --show-encoding`; the little-endian instruction bytes were reversed to form
// the 32-bit word. These anchor the reference oracle to the real assembler.

#[test]
fn golden_fmul_by_element() {
    // fmul v0.4s, v1.4s, v2.s[0]  -> [20 90 82 4f] -> 0x4F829020
    assert_eq!(
        word_of(encode_neon_float_elem(
            &[reg_arr(0, "4s"), reg_arr(1, "4s"), lane(2, "s", 0)],
            0,
            0b1001
        )),
        0x4F829020
    );
    // fmul v3.2s, v5.2s, v7.s[3]  -> [a3 98 a7 0f] -> 0x0FA798A3  (Q=0, max .s lane)
    assert_eq!(
        word_of(encode_neon_float_elem(
            &[reg_arr(3, "2s"), reg_arr(5, "2s"), lane(7, "s", 3)],
            0,
            0b1001
        )),
        0x0FA798A3
    );
    // fmul v9.2d, v2.2d, v6.d[0]  -> [49 90 c6 4f] -> 0x4FC69049  (sz=1)
    assert_eq!(
        word_of(encode_neon_float_elem(
            &[reg_arr(9, "2d"), reg_arr(2, "2d"), lane(6, "d", 0)],
            0,
            0b1001
        )),
        0x4FC69049
    );
    // fmul v4.2d, v4.2d, v4.d[1]  -> [84 98 c4 4f] -> 0x4FC49884  (sz=1, max .d lane)
    assert_eq!(
        word_of(encode_neon_float_elem(
            &[reg_arr(4, "2d"), reg_arr(4, "2d"), lane(4, "d", 1)],
            0,
            0b1001
        )),
        0x4FC49884
    );
}

#[test]
fn golden_fmla_by_element() {
    // fmla v0.4s, v0.4s, v0.s[1]   -> [00 10 a0 4f] -> 0x4FA01000
    assert_eq!(
        word_of(encode_neon_float_elem(
            &[reg_arr(0, "4s"), reg_arr(0, "4s"), lane(0, "s", 1)],
            0,
            0b0001
        )),
        0x4FA01000
    );
    // fmla v31.4s, v30.4s, v29.s[2] -> [df 1b 9d 4f] -> 0x4F9D1BDF  (Rm=29, M=1)
    assert_eq!(
        word_of(encode_neon_float_elem(
            &[reg_arr(31, "4s"), reg_arr(30, "4s"), lane(29, "s", 2)],
            0,
            0b0001
        )),
        0x4F9D1BDF
    );
}

#[test]
fn golden_fmls_by_element() {
    // fmls v5.2d, v7.2d, v9.d[1]    -> [e5 58 c9 4f] -> 0x4FC958E5  (sz=1)
    assert_eq!(
        word_of(encode_neon_float_elem(
            &[reg_arr(5, "2d"), reg_arr(7, "2d"), lane(9, "d", 1)],
            0,
            0b0101
        )),
        0x4FC958E5
    );
    // fmls v10.4s, v11.4s, v12.s[3] -> [6a 59 ac 4f] -> 0x4FAC596A
    assert_eq!(
        word_of(encode_neon_float_elem(
            &[reg_arr(10, "4s"), reg_arr(11, "4s"), lane(12, "s", 3)],
            0,
            0b0101
        )),
        0x4FAC596A
    );
}

// --- error-path regression (these PASS and stay green) --------------------

#[test]
fn rejects_too_few_operands() {
    assert!(
        encode_neon_float_elem(&[], 0, 0b1001).is_err(),
        "0 operands must error"
    );
    assert!(
        encode_neon_float_elem(&[reg_arr(0, "4s")], 0, 0b1001).is_err(),
        "1 operand must error"
    );
    assert!(
        encode_neon_float_elem(&[reg_arr(0, "4s"), reg_arr(1, "4s")], 0, 0b1001).is_err(),
        "2 operands must error"
    );
    // A well-formed in-range operand set must succeed.
    assert!(
        encode_neon_float_elem(
            &[reg_arr(0, "4s"), reg_arr(1, "4s"), lane(2, "s", 0)],
            0,
            0b1001
        )
        .is_ok()
    );
}

#[test]
fn rejects_non_lane_third_operand() {
    // third operand must be a vector RegLane, not a bare vector register
    let ops = vec![reg_arr(0, "4s"), reg_arr(1, "4s"), reg_arr(2, "4s")];
    assert!(encode_neon_float_elem(&ops, 0, 0b1001).is_err());
    let non_vector_lane = vec![
        reg_arr(0, "4s"),
        reg_arr(1, "4s"),
        Operand::RegLane { reg: "x2".into(), elem_size: "s".into(), index: 0 },
    ];
    assert!(encode_neon_float_elem(&non_vector_lane, 0, 0b1001).is_err());
}

#[test]
fn rejects_unsupported_arrangement() {
    // integer / byte / half arrangements are invalid for FP by-element
    for arr in ["8b", "16b", "4h", "8h", "1d", "2d_extra"] {
        let ops = vec![reg_arr(0, arr), reg_arr(1, arr), lane(2, "s", 0)];
        assert!(
            encode_neon_float_elem(&ops, 0, 0b1001).is_err(),
            "arrangement {arr} must be rejected"
        );
    }
}

#[test]
fn rejects_arrangement_and_lane_size_mismatches_and_extra_operands() {
    let mismatched_arrangement = vec![
        reg_arr(0, "4s"),
        reg_arr(1, "2s"),
        lane(2, "s", 0),
    ];
    assert!(encode_neon_float_elem(&mismatched_arrangement, 0, 0b1001).is_err());

    let mismatched_lane = vec![
        reg_arr(0, "4s"),
        reg_arr(1, "4s"),
        lane(2, "d", 0),
    ];
    assert!(encode_neon_float_elem(&mismatched_lane, 0, 0b1001).is_err());

    let extra = vec![
        reg_arr(0, "4s"),
        reg_arr(1, "4s"),
        lane(2, "s", 0),
        reg_arr(3, "4s"),
    ];
    assert!(encode_neon_float_elem(&extra, 0, 0b1001).is_err());
}
