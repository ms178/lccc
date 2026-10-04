//! Property-based tests for `encode_neon_ins`.
//!
//! `encode_neon_ins` encodes the AArch64 NEON `INS` instruction (alias of
//! `MOV`, vector copy group) in two forms:
//!   * general : `INS Vd.Ts[dst], Wn/Xn` (W for .b/.h/.s, X for .d)
//!    -> `0 1 0 01110 000 imm5 0 0111 1 Rn Rd`
//!   * element : `INS Vd.Ts[dst], Vn.Ts[src]`
//!    -> `0 1 1 01110 000 imm5 0 imm4 1 Rn Rd`
//!
//! Reference encodings used as golden oracles below were captured from
//! `llvm-mc-18 -triple=aarch64 -show-encoding` and confirmed identical to this
//! encoder's output for all *valid* inputs.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_ins;
use proptest::prelude::*;

// --- helpers --------------------------------------------------------------

/// `Operand::RegLane { reg: "v{rd}", elem_size, index }`
fn lane(rd: u32, elem_size: &str, index: u32) -> Operand {
    Operand::RegLane {
        reg: format!("v{rd}"),
        elem_size: elem_size.to_string(),
        index,
    }
}

fn gp(num: u32) -> Operand {
    Operand::Reg(format!("x{num}"))
}

/// Correct-width GP source for `INS Vd.Ts[index], Rn`.
fn gp_for_elem(num: u32, elem: &str) -> Operand {
    let prefix = if elem == "d" { "x" } else { "w" };
    let name = if num == 31 {
        format!("{prefix}zr")
    } else {
        format!("{prefix}{num}")
    };
    Operand::Reg(name)
}

/// Per-element-size (size) metadata derived from the ARM ARM `imm5` element-size
/// encoding: the sentinel is the single low bit of `imm5`, and the lane index
/// occupies the bits above it.
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

fn elem_size_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("b"), Just("h"), Just("s"), Just("d")]
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

// --- properties -----------------------------------------------------------

proptest! {
    // === Oracle: reference / round-trip (general form) ====================
    // Rd (bits 4..0) and Rn (bits 9..5) must round-trip the destination
    // vector register and source general-purpose register. This field
    // placement is fixed by the ARMv8 "Advanced SIMD copy" encoding and was
    // cross-checked against `llvm-mc`.
    #[test]
    fn prop_gp_reg_fields_roundtrip(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        elem in elem_size_strategy(),
        idx_mod in 0u32..=15u32,
    ) {
        // The lane index must respect the element-size bound (b<=15, h<=7,
        // s<=3, d<=1); out-of-range lanes are now rejected by the encoder.
        let idx = idx_mod % (size_meta(elem).unwrap().2 + 1);
        let ops = vec![lane(rd, elem, idx), gp_for_elem(rn, elem)];
        let w = word_of(encode_neon_ins(&ops));
        prop_assert_eq!(w & 0x1F, rd, "Rd field must equal destination register");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn field must equal source GP register");
    }

    // === Oracle: algebraic / invariance (general form, imm5) ==============
    // The imm5 field (bits 20..16) packs element size (sentinel bit) and lane
    // index: imm5 == (index << shift) | sentinel. For an in-range index this
    // must hold exactly; the sentinel uniquely tags the element size.
    #[test]
    fn prop_gp_imm5_encodes_size_and_index(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        elem in elem_size_strategy(),
    ) {
        let (sentinel, shift, max_lane) = size_meta(elem).unwrap();
        let idx = max_lane; // boundary value: exercises top index bit
        let w = word_of(encode_neon_ins(&[lane(rd, elem, idx), gp_for_elem(rn, elem)]));
        let imm5 = (w >> 16) & 0x1F;
        prop_assert_eq!(imm5 & sentinel, sentinel, "size sentinel must be set");
        prop_assert_eq!(imm5, (idx << shift) | sentinel, "imm5 must pack index+size");
        // bits 23..21 are the fixed `000` of the copy group.
        prop_assert_eq!((w >> 21) & 0x7, 0b000);
    }

    // === Oracle: reference / round-trip (element-to-element form) =========
    // Rd (4..0) and Rn (9..5) round-trip; imm4 (bits 14..11) packs the source
    // lane index with the size-dependent shift (b:0, h:1, s:2, d:3). The
    // element-form top bits are `011 01110 000` (verified vs llvm-mc).
    #[test]
    fn prop_elem_fields_roundtrip_and_imm4(
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        elem in elem_size_strategy(),
        dst_idx in 0u32..=15u32,
        src_idx in 0u32..=15u32,
    ) {
        let (_, _, max_lane) = size_meta(elem).unwrap();
        let imm4_shift = match elem { "b" => 0, "h" => 1, "s" => 2, "d" => 3, _ => unreachable!() };
        let dst = dst_idx % (max_lane + 1);
        let src = src_idx % (max_lane + 1);
        let w = word_of(encode_neon_ins(&[lane(rd, elem, dst), lane(rn, elem, src)]));
        prop_assert_eq!(w & 0x1F, rd, "Rd");
        prop_assert_eq!((w >> 5) & 0x1F, rn, "Rn");
        let imm4 = (w >> 11) & 0xF;
        prop_assert_eq!(imm4, (src & max_lane) << imm4_shift, "imm4 packs src lane");
        prop_assert_eq!((w >> 29) & 0x7, 0b011, "element-form fixed top bits");
        prop_assert_eq!((w >> 10) & 0x1, 1, "element-form fixed bit 10");
    }

    // === Oracle: negative contract (malformed operands) ===================
    // Too few operands, a first operand that is not a RegLane, an unsupported
    // element size, or an unparseable register name must all be rejected with
    // Err rather than producing a (possibly garbage) encoding.
    #[test]
    fn prop_rejects_malformed_operands(
        // any short lowercase string that is NOT exactly b/h/s/d
        bad_size in "[a-z]{1,3}".prop_filter(
            "not a valid element size", |s| !matches!(s.as_str(), "b" | "h" | "s" | "d")),
        bad_reg in "(x|v)(3[2-9]|[4-9][0-9])", // numeric, always > 31
    ) {
        // empty operand list
        prop_assert!(encode_neon_ins(&[]).is_err());
        // single operand only
        prop_assert!(encode_neon_ins(&[lane(0, "s", 0)]).is_err());
        // first operand not a RegLane
        prop_assert!(encode_neon_ins(&[gp(1), gp(2)]).is_err());
        // unsupported element size
        let ops: Vec<Operand> = vec![
            Operand::RegLane { reg: "v0".into(), elem_size: bad_size.clone(), index: 0 },
            gp(1),
        ];
        prop_assert!(encode_neon_ins(&ops).is_err(),
            "elem_size {bad_size:?} must be rejected");
        // unparseable (out-of-range) destination register name
        let ops: Vec<Operand> = vec![
            Operand::RegLane { reg: bad_reg.clone(), elem_size: "s".into(), index: 0 },
            gp(1),
        ];
        prop_assert!(encode_neon_ins(&ops).is_err(),
            "register {bad_reg:?} must be rejected");
    }

    // === Oracle: negative contract (out-of-range lane index) ==============
    // The ARMv8 ARM constrains the lane index per element size:
    //   .b -> [0,15], .h -> [0,7], .s -> [0,3], .d -> [0,1].
    // `llvm-mc-18` rejects out-of-range lanes, e.g.
    //   `ins v0.b[16], v1.b[0]` -> error: vector lane must be an integer in range [0, 15].
    // No AArch64 spec defines wrapping/truncation as intentional here, so the
    // encoder MUST return Err for an out-of-range lane.
    //
    // EXPECTED BEHAVIOR: Err.  See COVERAGE/bug report if this fails.
    #[test]
    fn prop_rejects_out_of_range_lane_index(
        elem in elem_size_strategy(),
        over in 1u32..=16u32, // small overshoot above the per-size maximum
    ) {
        let (_, _, max_lane) = size_meta(elem).unwrap();
        let bad_index = max_lane + over;
        let ops: Vec<Operand> = vec![
            Operand::RegLane { reg: "v0".into(), elem_size: elem.into(), index: bad_index },
            gp_for_elem(1, elem),
        ];
        prop_assert!(encode_neon_ins(&ops).is_err(),
            "out-of-range lane [{bad_index}] for .{elem} (max {max_lane}) must be rejected, \
             not silently truncated");
    }
}

// --- golden differential regression (llvm-mc-18 anchor) -------------------
//
// Golden 32-bit words captured from:
//   `llvm-mc-18 -triple=aarch64 -show-encoding -assemble "<insn>"`
// (llvm-mc prints little-endian bytes; reconstructed to a u32 word below.)
#[test]
fn golden_matches_llvm_mc() {
    // general form:  INS Vd.Ts[idx], Wn/Xn   -> MOV (vector from general)
    //  ins v0.s[0], w1   = 0x4e041c20
    assert_eq!(
        word_of(encode_neon_ins(&[lane(0, "s", 0), gp_for_elem(1, "s")])),
        0x4e041c20
    );
    //  ins v5.s[2], w3   = 0x4e141c65
    assert_eq!(
        word_of(encode_neon_ins(&[lane(5, "s", 2), gp_for_elem(3, "s")])),
        0x4e141c65
    );
    //  ins v9.b[0], w10  = 0x4e011d49
    assert_eq!(
        word_of(encode_neon_ins(&[lane(9, "b", 0), gp_for_elem(10, "b")])),
        0x4e011d49
    );
    //  ins v2.h[7], w4   = 0x4e1e1c82
    assert_eq!(
        word_of(encode_neon_ins(&[lane(2, "h", 7), gp_for_elem(4, "h")])),
        0x4e1e1c82
    );
    //  ins v3.d[0], x9   = 0x4e081d23
    assert_eq!(
        word_of(encode_neon_ins(&[lane(3, "d", 0), gp_for_elem(9, "d")])),
        0x4e081d23
    );
    //  ins v31.d[1], x30 = 0x4e181fdf
    assert_eq!(
        word_of(encode_neon_ins(&[lane(31, "d", 1), gp_for_elem(30, "d")])),
        0x4e181fdf
    );

    // element form:  INS Vd.Ts[dst], Vn.Ts[src]  -> MOV (vector element)
    //  ins v5.s[2], v7.s[3] = 0x6e1464e5
    assert_eq!(
        word_of(encode_neon_ins(&[lane(5, "s", 2), lane(7, "s", 3)])),
        0x6e1464e5
    );
    //  ins v0.b[15], v1.b[0] = 0x6e1f0420
    assert_eq!(
        word_of(encode_neon_ins(&[lane(0, "b", 15), lane(1, "b", 0)])),
        0x6e1f0420
    );
    //  ins v2.h[7], v3.h[1] = 0x6e1e1462
    assert_eq!(
        word_of(encode_neon_ins(&[lane(2, "h", 7), lane(3, "h", 1)])),
        0x6e1e1462
    );
    //  ins v4.d[1], v6.d[0] = 0x6e1804c4
    assert_eq!(
        word_of(encode_neon_ins(&[lane(4, "d", 1), lane(6, "d", 0)])),
        0x6e1804c4
    );
}

#[test]
fn ins_gp_form_requires_element_width_and_gpr_class() {
    for elem in ["b", "h", "s", "d"] {
        let correct = gp_for_elem(1, elem);
        assert!(encode_neon_ins(&[lane(0, elem, 0), correct]).is_ok());

        let wrong = if elem == "d" {
            Operand::Reg("w1".into())
        } else {
            Operand::Reg("x1".into())
        };
        assert!(
            encode_neon_ins(&[lane(0, elem, 0), wrong]).is_err(),
            "wrong GP width must be rejected for .{elem}"
        );
    }
    assert!(encode_neon_ins(&[lane(0, "s", 0), Operand::Reg("v1".into())]).is_err());
    assert!(encode_neon_ins(&[lane(0, "s", 0), gp_for_elem(1, "s"), gp_for_elem(2, "s")]).is_err());
}
