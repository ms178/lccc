//! Property-based tests for `encode_neon_umov` — the **general-purpose-register
//! (GP) form** of AArch64 `UMOV` (Advanced SIMD copy group).
//!
//! `encode_neon_umov` always targets a GP register destination:
//!   * `UMOV Wd, Vn.<Ts>[index]`  (Ts in {B,H,S}; 32-bit GP destination, Q=0)
//!   * `UMOV Xd, Vn.D[index]`      (64-bit GP destination, Q=1)
//!
//! Encoding (ARMv8 ARM, "Advanced SIMD copy"):
//!   `0 Q 0 01110 000 imm5 0 0111 1 Rn Rd`
//!    31 30 29-24 23-21 20-16 15 14-10 9-5 4-0
//!
//! `imm5` packs the element-size sentinel (the single low set bit) with the lane
//! index in the bits above it.
//!
//! # Reference oracle
//!
//! All golden words below were produced by `llvm-mc-18 --triple=aarch64
//! --assemble --show-encoding` (the authoritative AArch64 assembler) and are the
//! ground truth the properties are checked against. Key examples:
//!   `umov w0, v0.s[0]`   => `0x0E043C00`  (Q=0)
//!   `umov x0, v0.d[0]`   => `0x4E083C00`  (Q=1  — the 64-bit GP form sets Q=1)
//!   `umov w0, v0.b[15]`  => `0x0E1F3C00`
//!   `umov x9, v3.d[1]`   => `0x4E183C69`
//!
//! # Validation contracts
//!
//! The active properties verify the required width pairing (X for `.d`, W for
//! `.b/.h/.s`), lane bounds, vector-register source class, and exact operand
//! count, alongside the reference encoding for valid inputs.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_umov;
use proptest::prelude::*;

// --- helpers --------------------------------------------------------------

/// GP destination register. `dest_x == true` => `xN` (64-bit), else `wN`.
fn gp(dest_x: bool, num: u32) -> Operand {
    let prefix = if dest_x { "x" } else { "w" };
    let name = if num == 31 {
        format!("{prefix}zr")
    } else {
        format!("{prefix}{num}")
    };
    Operand::Reg(name)
}

/// `Operand::RegLane { reg: "v{rn}", elem_size, index }` — the source vector element.
fn lane(rn: u32, elem_size: &str, index: u32) -> Operand {
    Operand::RegLane {
        reg: format!("v{rn}"),
        elem_size: elem_size.to_string(),
        index,
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

/// Per-element-size metadata derived from the ARM ARM `imm5` element-size
/// encoding: the sentinel is the single low bit of `imm5`, the lane index
/// occupies the bits above it, and `max_lane` is the architecturally valid
/// upper bound.
///   b -> sentinel 0b00001, idx shift 1, max lane 15
///   h -> sentinel 0b00010, idx shift 2, max lane 7
///   s -> sentinel 0b00100, idx shift 3, max lane 3
///   d -> sentinel 0b01000, idx shift 4, max lane 1
fn size_meta(elem_size: &str) -> Option<(u32, u32, u32)> {
    // (sentinel, index_shift, max_lane)
    Some(match elem_size {
        "b" => (0b00001, 1, 15),
        "h" => (0b00010, 2, 7),
        "s" => (0b00100, 3, 3),
        "d" => (0b01000, 4, 1),
        _ => return None,
    })
}

/// A *consistent* (GP-destination-width, element-size) pair, i.e. one that
/// `llvm-mc-18` accepts. X goes only with D; W goes with B/H/S.
fn valid_width_elem_strategy() -> impl Strategy<Value = (bool, &'static str)> {
    prop_oneof![
        Just((true, "d")),
        Just((false, "b")),
        Just((false, "h")),
        Just((false, "s")),
    ]
}

/// The ARM ARM reference word for a consistent UMOV operand set, derived
/// directly from the `0 Q 0 01110 000 imm5 0 0111 1 Rn Rd` template.
fn reference_word(dest_x: bool, rd: u32, rn: u32, elem: &str, index: u32) -> u32 {
    let (sentinel, shift, _) = size_meta(elem).unwrap();
    let imm5 = (index << shift) | sentinel;
    let q: u32 = if dest_x { 1 } else { 0 };
    (q << 30) | (0b001110000u32 << 21) | (imm5 << 16) | (0b001111 << 10) | (rn << 5) | rd
}

// --- properties -----------------------------------------------------------

proptest! {
    // === Oracle: differential / reference (ARM ARM template) =============
    // For every *consistent* operand set, the encoded word equals the ARM ARM
    // `0 Q 0 01110 000 imm5 0 0111 1 Rn Rd` template. Q tracks the GP
    // destination width (0 for W, 1 for X), which for consistent inputs is
    // exactly the element-size rule (Q=1 iff element is D).
    #[test]
    fn prop_matches_arm_reference(
        (dest_x, elem) in valid_width_elem_strategy(),
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        idx_mod in 0u32..=63u32,
    ) {
        let (_, _, max_lane) = size_meta(elem).unwrap();
        let idx = idx_mod % (max_lane + 1);
        let ops = vec![gp(dest_x, rd), lane(rn, elem, idx)];
        let w = word_of(encode_neon_umov(&ops));
        prop_assert_eq!(
            w,
            reference_word(dest_x, rd, rn, elem, idx),
            "encoded word must equal ARM ARM template for umov {:?}, v{}.[{}]",
            if dest_x { "Xd" } else { "Wd" },
            rn,
            idx
        );
    }

    // === Oracle: structural (fixed opcode bits) ==========================
    // The fixed fields of the UMOV encoding are independent of every operand:
    //   bit 31      = 0
    //   bits 29-21  = 0b001110000
    //   bits 15-10  = 0b001111   (the UMOV opcode within the copy group)
    #[test]
    fn prop_fixed_opcode_bits(
        (dest_x, elem) in valid_width_elem_strategy(),
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        idx_mod in 0u32..=63u32,
    ) {
        let (_, _, max_lane) = size_meta(elem).unwrap();
        let idx = idx_mod % (max_lane + 1);
        let w = word_of(encode_neon_umov(&[gp(dest_x, rd), lane(rn, elem, idx)]));
        prop_assert_eq!(w >> 31, 0u32, "bit 31 must be 0");
        prop_assert_eq!((w >> 21) & 0x1FF, 0b001110000u32, "bits 29-21 fixed");
        prop_assert_eq!((w >> 10) & 0x3F, 0b001111u32, "bits 15-10 = UMOV opcode 001111");
    }

    // === Oracle: structural (Q bit tracks GP destination width) ==========
    // CORRECTED: for a CONSISTENT operand set, Q (bit 30) is 1 exactly for the
    // 64-bit Xd form (element D), and 0 for the 32-bit Wd form (B/H/S). This
    // matches `llvm-mc-18` (e.g. `umov x0, v0.d[0]` => `0x4E083C00`, Q=1).
    #[test]
    fn prop_q_bit_tracks_gp_width(
        (dest_x, elem) in valid_width_elem_strategy(),
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        idx_mod in 0u32..=63u32,
    ) {
        let (_, _, max_lane) = size_meta(elem).unwrap();
        let idx = idx_mod % (max_lane + 1);
        let w = word_of(encode_neon_umov(&[gp(dest_x, rd), lane(rn, elem, idx)]));
        prop_assert_eq!((w >> 30) & 1, u32::from(dest_x),
            "Q (bit 30) must equal 1 for Xd/D and 0 for Wd/(B,H,S)");
    }

    // === Oracle: field isolation (Rd, Rn, imm5) ==========================
    // Rd occupies exactly bits [4:0]; Rn exactly bits [9:5]; imm5 exactly bits
    // [20:16]. Varying one register leaves all other fields untouched.
    #[test]
    fn prop_fields_isolated(
        (dest_x, elem) in valid_width_elem_strategy(),
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        idx_mod in 0u32..=63u32,
    ) {
        let (_, _, max_lane) = size_meta(elem).unwrap();
        let idx = idx_mod % (max_lane + 1);

        let base = word_of(encode_neon_umov(&[gp(dest_x, 0), lane(0, elem, idx)]));

        // Rd round-trips in [4:0] and leaks nowhere else.
        let with_rd = word_of(encode_neon_umov(&[gp(dest_x, rd), lane(0, elem, idx)]));
        prop_assert_eq!(with_rd & 0x1F, rd, "Rd not in bits [4:0]");
        prop_assert_eq!(with_rd & 0xFFFF_FFE0, base & 0xFFFF_FFE0, "Rd leaked above bit 4");

        // Rn round-trips in [9:5] and leaks nowhere else.
        let with_rn = word_of(encode_neon_umov(&[gp(dest_x, 0), lane(rn, elem, idx)]));
        prop_assert_eq!((with_rn >> 5) & 0x1F, rn, "Rn not in bits [9:5]");
        prop_assert_eq!(with_rn & !0x3E0, base & !0x3E0, "Rn leaked outside bits [9:5]");
    }

    // === Oracle: algebraic (imm5 packs size sentinel + index) ===========
    // imm5 (bits 20-16) == (index << shift) | sentinel; the sentinel uniquely
    // tags the element size. Holds for every in-range lane index.
    #[test]
    fn prop_imm5_packs_size_and_index(
        (dest_x, elem) in valid_width_elem_strategy(),
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        idx_mod in 0u32..=63u32,
    ) {
        let (sentinel, shift, max_lane) = size_meta(elem).unwrap();
        let idx = idx_mod % (max_lane + 1);
        let w = word_of(encode_neon_umov(&[gp(dest_x, rd), lane(rn, elem, idx)]));
        let imm5 = (w >> 16) & 0x1F;
        prop_assert_eq!(imm5, (idx << shift) | sentinel, "imm5 must pack index+size");
        prop_assert_eq!(imm5 & sentinel, sentinel, "size sentinel must be set");
    }

    // === Oracle: negative contract (GP-width / element-size mismatch) ===
    // The GP destination width must be consistent with the element size:
    // X goes with D, W goes with B/H/S.
    #[test]
    fn prop_rejects_width_element_size_mismatch(
        bad in prop_oneof![
            // 64-bit GP destination paired with a non-D element
            Just((true, "b")),
            Just((true, "h")),
            Just((true, "s")),
            // 32-bit GP destination paired with a D element
            Just((false, "d")),
        ],
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
    ) {
        let (dest_x, elem) = bad;
        let (_, _, max_lane) = size_meta(elem).unwrap();
        let ops: Vec<Operand> = vec![gp(dest_x, rd), lane(rn, elem, 0)];
        prop_assert!(
            encode_neon_umov(&ops).is_err(),
            "umov {:?}, v{}.{}[0] (lane 0 <= max {max_lane}) is an invalid width/size \
             combination and must be rejected, not silently encoded as {:?}",
            if dest_x { "Xd" } else { "Wd" },
            rn,
            elem,
            encode_neon_umov(&ops)
        );
        let _ = max_lane;
    }

    // === Oracle: negative contract (out-of-range lane index) ============
    // The ARMv8 ARM constrains lane indices to b<=15, h<=7, s<=3 and d<=1.
    #[test]
    fn prop_rejects_out_of_range_lane_index(
        bad in prop_oneof![
            Just(("b", 16u32)),
            Just(("h", 8u32)),
            Just(("s", 4u32)),
            Just(("d", 2u32)),
        ],
    ) {
        let (elem, bad_index) = bad;
        // keep the GP width consistent with the element size so the only
        // invalid thing is the lane index.
        let dest = if elem == "d" { gp(true, 0) } else { gp(false, 0) };
        let ops: Vec<Operand> = vec![dest, lane(0, elem, bad_index)];
        prop_assert!(encode_neon_umov(&ops).is_err(),
            "out-of-range lane [{bad_index}] for .{elem} must be rejected, \
             not silently truncated/masked; got {:?}", encode_neon_umov(&ops));
    }

    // === Oracle: negative contract (malformed operands) ==================
    // Too few operands, a second operand that is not a RegLane, an unsupported
    // element size, or an unparseable (out-of-range) source vector register
    // must all be rejected with Err.
    #[test]
    fn prop_rejects_malformed_operands(
        bad_size in "[a-z]{1,3}".prop_filter(
            "not a valid element size", |s| !matches!(s.as_str(), "b" | "h" | "s" | "d")),
        bad_reg in "(x|v)(3[2-9]|[4-9][0-9])", // numeric, always > 31
    ) {
        // empty operand list
        prop_assert!(encode_neon_umov(&[]).is_err());
        // single operand only
        prop_assert!(encode_neon_umov(&[gp(true, 0)]).is_err());
        // second operand is not a RegLane
        prop_assert!(encode_neon_umov(&[gp(false, 0), gp(true, 1)]).is_err());
        // unsupported element size
        let ops: Vec<Operand> = vec![
            gp(false, 0),
            Operand::RegLane { reg: "v0".into(), elem_size: bad_size.clone(), index: 0 },
        ];
        prop_assert!(encode_neon_umov(&ops).is_err(),
            "elem_size {bad_size:?} must be rejected");
        // unparseable (out-of-range) source vector register
        let ops: Vec<Operand> = vec![
            gp(false, 0),
            Operand::RegLane { reg: bad_reg.clone(), elem_size: "s".into(), index: 0 },
        ];
        prop_assert!(encode_neon_umov(&ops).is_err(),
            "source register {bad_reg:?} must be rejected");
    }
}

// --- golden differential regression (llvm-mc-18 anchors) ------------------
//
// Every word below was emitted by `llvm-mc-18 --triple=aarch64 --assemble
// --show-encoding`. The little-endian instruction bytes were reversed to form
// the 32-bit word. These anchor the reference oracle to the real assembler.

#[test]
fn golden_umov_w_form_q0() {
    // 32-bit GP destination (Wd) — Q=0.
    //  umov w0,  v0.s[0]  -> 00 3c 04 0e -> 0x0E043C00
    assert_eq!(
        word_of(encode_neon_umov(&[gp(false, 0), lane(0, "s", 0)])),
        0x0E043C00
    );
    //  umov w5,  v2.b[7]  -> 45 3c 0f 0e -> 0x0E0F3C45
    assert_eq!(
        word_of(encode_neon_umov(&[gp(false, 5), lane(2, "b", 7)])),
        0x0E0F3C45
    );
    //  umov w15, v31.s[3] -> ef 3f 1c 0e -> 0x0E1C3FEF
    assert_eq!(
        word_of(encode_neon_umov(&[gp(false, 15), lane(31, "s", 3)])),
        0x0E1C3FEF
    );
    //  umov w7,  v4.h[5]  -> 87 3c 16 0e -> 0x0E163C87
    assert_eq!(
        word_of(encode_neon_umov(&[gp(false, 7), lane(4, "h", 5)])),
        0x0E163C87
    );
    //  boundary lanes:
    //  umov w0,  v0.b[15] -> 00 3c 1f 0e -> 0x0E1F3C00
    assert_eq!(
        word_of(encode_neon_umov(&[gp(false, 0), lane(0, "b", 15)])),
        0x0E1F3C00
    );
    //  umov w0,  v0.h[7]  -> 00 3c 1e 0e -> 0x0E1E3C00
    assert_eq!(
        word_of(encode_neon_umov(&[gp(false, 0), lane(0, "h", 7)])),
        0x0E1E3C00
    );
    //  umov w0,  v0.s[3]  -> 00 3c 1c 0e -> 0x0E1C3C00
    assert_eq!(
        word_of(encode_neon_umov(&[gp(false, 0), lane(0, "s", 3)])),
        0x0E1C3C00
    );
    //  umov w30, v31.b[0] -> fe 3f 01 0e -> 0x0E013FFE
    assert_eq!(
        word_of(encode_neon_umov(&[gp(false, 30), lane(31, "b", 0)])),
        0x0E013FFE
    );
}

#[test]
fn golden_umov_x_form_q1() {
    // 64-bit GP destination (Xd) — Q=1 (the correct, 0x4E... form).
    //  umov x0,  v0.d[0]  -> 00 3c 08 4e -> 0x4E083C00
    assert_eq!(
        word_of(encode_neon_umov(&[gp(true, 0), lane(0, "d", 0)])),
        0x4E083C00
    );
    //  umov x9,  v3.d[1]  -> 69 3c 18 4e -> 0x4E183C69
    assert_eq!(
        word_of(encode_neon_umov(&[gp(true, 9), lane(3, "d", 1)])),
        0x4E183C69
    );
    //  umov x0,  v0.d[1]  -> 00 3c 18 4e -> 0x4E183C00  (boundary lane)
    assert_eq!(
        word_of(encode_neon_umov(&[gp(true, 0), lane(0, "d", 1)])),
        0x4E183C00
    );
    //  umov x31, v31.d[0] -> ff 3f 08 4e -> 0x4E083FFF  (xzr / v31)
    assert_eq!(
        word_of(encode_neon_umov(&[gp(true, 31), lane(31, "d", 0)])),
        0x4E083FFF
    );
}

#[test]
fn rejects_too_few_operands() {
    assert!(encode_neon_umov(&[]).is_err(), "0 operands must error");
    assert!(
        encode_neon_umov(&[gp(true, 0)]).is_err(),
        "1 operand must error"
    );
    // Exactly two valid operands must succeed.
    assert!(encode_neon_umov(&[gp(false, 0), lane(0, "s", 0)]).is_ok());
    assert!(encode_neon_umov(&[gp(true, 0), lane(0, "d", 0)]).is_ok());
}

#[test]
fn rejects_non_gpr_destinations_and_extra_operands() {
    assert!(encode_neon_umov(&[Operand::Reg("v0".into()), lane(1, "s", 0)]).is_err());
    assert!(encode_neon_umov(&[gp(false, 0), lane(1, "s", 0), gp(false, 1)]).is_err());
}

#[test]
fn rejects_non_vector_lane_sources() {
    let fp_lane = Operand::RegLane {
        reg: "d1".into(),
        elem_size: "s".into(),
        index: 0,
    };
    let gpr_lane = Operand::RegLane {
        reg: "x1".into(),
        elem_size: "s".into(),
        index: 0,
    };
    assert!(encode_neon_umov(&[gp(false, 0), fp_lane]).is_err());
    assert!(encode_neon_umov(&[gp(false, 0), gpr_lane]).is_err());
}
