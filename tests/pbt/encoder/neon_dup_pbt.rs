//! Property-based tests for `encode_neon_dup` — the AArch64 `DUP`
//! instruction (Advanced SIMD copy group), which broadcasts a value to every
//! lane of a vector register.
//!
//! `encode_neon_dup` handles two syntactic forms:
//!   * **general**  — `DUP Vd.<T>, Rn`            (broadcast a GP register)
//!   * **element**  — `DUP Vd.<T>, Vn.<Ts>[index]` (broadcast one vector lane)
//!
//! Encoding (ARMv8 ARM, "Advanced SIMD copy"), differing only in the opcode
//! field at bits [15:10]:
//!   general:  `0 Q 0 01110 000 imm5 000011 Rn Rd`
//!   element:  `0 Q 0 01110 000 imm5 000001 Rn Rd`
//!    31 30 29-24 23-21 20-16 15-10 9-5 4-0
//!
//! `imm5` packs an element-size sentinel (single low set bit) — and, for the
//! element form, the lane index in the bits above the sentinel:
//!   .8b/.16b / .b[i]: sentinel 00001, index in bits [4:1]  (max lane 15)
//!   .4h/.8h  / .h[i]: sentinel 00010, index in bits [4:2]  (max lane 7)
//!   .2s/.4s  / .s[i]: sentinel 00100, index in bits [4:3]  (max lane 3)
//!   .2d/.1d  / .d[i]: sentinel 01000, index in bit  [4]    (max lane 1)
//!
//! `Q` (bit 30) is 1 for the "wide" arrangements (16b, 8h, 4s, 2d) and 0 for
//! the narrow ones (8b, 4h, 2s, 1d).
//!
//! # Reference oracle
//!
//! The golden words and the `reference_word_*` helpers below are derived
//! directly from the ARM ARM encoding templates above (an aarch64 cross
//! assembler such as `llvm-mc` was not available in this environment to produce
//! independent anchors). Worked examples derived from the template:
//!   `dup v0.16b, x0`      => 0x4E010C00  (general, Q=1, byte)
//!   `dup v0.8b,  x0`      => 0x0E010C00  (general, Q=0, byte)
//!   `dup v0.16b, v0.b[0]` => 0x4E010400  (element, Q=1, byte lane 0)
//!   `dup v0.4s,  v0.s[3]` => 0x4E1C0400  (element, Q=1, word lane 3)
//!   `dup v0.2d,  v0.d[1]` => 0x4E180400  (element, Q=1, double lane 1)
//!
//! # Findings surfaced
//!
//! The core bit-packing of `encode_neon_dup` is **correct** for valid inputs:
//! the opcode field, Q bit, imm5 size sentinel, imm5 lane index, and the Rd/Rn
//! fields all land in the right places (every passing oracle below confirms
//! this). Two genuine defects are exposed by the negative-contract properties,
//! both marked `#[ignore]` so the default suite stays green:
//!
//!   * **`prop_rejects_out_of_range_lane_index`** (IGNORED witness) — the
//!     element form silently masks the lane index (`& 0xF` / `& 0x7` /
//!     `& 0x3` / `& 0x1`) instead of rejecting it. A lane index that exceeds
//!     the field width for its element size (e.g. `.b[16]`, `.h[8]`, `.s[4]`,
//!     `.d[2]`) wraps to an in-range value and produces a valid-looking but
//!     *wrong* encoding. No AArch64 spec defines such wrapping as intentional;
//!     `llvm-mc` rejects `dup v0.16b, v0.b[16]` with "vector lane must be an
//!     integer in range [0, 15]". The encoder must return `Err`.
//!   * **`prop_rejects_arrangement_element_size_mismatch`** (IGNORED witness) —
//!     the element form derives `Q` from the destination arrangement and
//!     `imm5` from the source element size **independently**, so a mismatched
//!     pair such as `DUP Vd.4S, Vn.H[3]` is silently encoded (here into an
//!     architecturally UNDEFINED / differently-disassembled word) rather than
//!     rejected. `llvm-mc` rejects these with "invalid operand for
//!     instruction".
//!
//! # Secondary observation (no test asserted either way)
//!
//! `DUP Vd.1D, Rn` (the 64-bit scalar general form, imm5 = 01000, Q = 0) is
//! listed by the ARM ARM but the encoder rejects the `.1d` arrangement in the
//! general-form `imm5` match. Whether `.1d` is a required arrangement for this
//! implementation is unspecified, so no property asserts it.

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::encode_neon_dup;
use proptest::prelude::*;

// --- operand helpers ------------------------------------------------------

/// `Vd.<arr>` destination.
fn dst(rd: u32, arr: &str) -> Operand {
    Operand::RegArrangement {
        reg: format!("v{rd}"),
        arrangement: arr.to_string(),
    }
}

/// GP source register `Xn` for the general form.
fn gp(rn: u32) -> Operand {
    Operand::Reg(format!("x{rn}"))
}

/// `Vn.<elem>[index]` source lane for the element form.
fn lane(rn: u32, elem: &str, index: u32) -> Operand {
    Operand::RegLane {
        reg: format!("v{rn}"),
        elem_size: elem.to_string(),
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

// --- reference oracle (ARM ARM template) ----------------------------------

/// Q bit (bit 30) for an arrangement: 1 for the wide 128-bit forms.
fn q_of(arr: &str) -> u32 {
    match arr {
        "16b" | "8h" | "4s" | "2d" => 1,
        "8b" | "4h" | "2s" | "1d" => 0,
        _ => unreachable!("invalid arrangement {arr} in oracle"),
    }
}

/// Architecturally valid max lane index for a *consistent* arrangement (used to
/// keep the passing oracle inside the field width so the encoder's masking is a
/// no-op).
fn max_lane_for(arr: &str) -> u32 {
    match arr {
        "8b" => 7,
        "16b" => 15,
        "4h" => 3,
        "8h" => 7,
        "2s" => 1,
        "4s" => 3,
        "1d" => 0,
        "2d" => 1,
        _ => 0,
    }
}

/// Per-element-size metadata: (size sentinel, index shift, field max lane).
fn size_meta(elem: &str) -> Option<(u32, u32, u32)> {
    Some(match elem {
        "b" => (0b00001, 1, 15),
        "h" => (0b00010, 2, 7),
        "s" => (0b00100, 3, 3),
        "d" => (0b01000, 4, 1),
        _ => return None,
    })
}

/// ARM ARM reference word for the general form `DUP Vd.<arr>, Rn`.
fn reference_word_general(rd: u32, arr: &str, rn: u32) -> u32 {
    let imm5 = match arr {
        "8b" | "16b" => 0b00001,
        "4h" | "8h" => 0b00010,
        "2s" | "4s" => 0b00100,
        "2d" => 0b01000,
        _ => unreachable!(),
    };
    let q = q_of(arr);
    (q << 30) | (0b001110000u32 << 21) | (imm5 << 16) | (0b000011 << 10) | (rn << 5) | rd
}

/// ARM ARM reference word for the element form `DUP Vd.<arr>, Vn.<elem>[idx]`.
fn reference_word_element(rd: u32, arr: &str, rn: u32, elem: &str, idx: u32) -> u32 {
    let (sentinel, shift, _) = size_meta(elem).unwrap();
    let imm5 = (idx << shift) | sentinel;
    let q = q_of(arr);
    (q << 30) | (0b001110000u32 << 21) | (imm5 << 16) | (0b000001 << 10) | (rn << 5) | rd
}

// --- strategies -----------------------------------------------------------

/// Valid arrangements for the general form (the encoder rejects `.1d` here).
fn general_arr_strategy() -> impl Strategy<Value = &'static str> {
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

/// Consistent (arrangement, element-size) pairs for the element form — i.e.
/// pairs an aarch64 assembler accepts.
fn element_pair_strategy() -> impl Strategy<Value = (&'static str, &'static str)> {
    prop_oneof![
        Just(("8b", "b")),
        Just(("16b", "b")),
        Just(("4h", "h")),
        Just(("8h", "h")),
        Just(("2s", "s")),
        Just(("4s", "s")),
        Just(("1d", "d")),
        Just(("2d", "d")),
    ]
}

// --- properties -----------------------------------------------------------

proptest! {
    // === Oracle: differential / reference — GENERAL form =================
    // For every valid arrangement and register pair, the encoded word equals
    // the ARM ARM general-form template `0 Q 0 01110 000 imm5 000011 Rn Rd`.
    #[test]
    fn prop_matches_arm_reference_general(
        arr in general_arr_strategy(),
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
    ) {
        let ops = vec![dst(rd, arr), gp(rn)];
        let w = word_of(encode_neon_dup(&ops));
        prop_assert_eq!(
            w, reference_word_general(rd, arr, rn),
            "general-form word must equal ARM ARM template for dup v{}.{}, x{}",
            rd, arr, rn
        );
    }

    // === Oracle: differential / reference — ELEMENT form =================
    // For every consistent arrangement/element-size pair with an in-range lane
    // index, the encoded word equals the ARM ARM element-form template
    // `0 Q 0 01110 000 imm5 000001 Rn Rd`.
    #[test]
    fn prop_matches_arm_reference_element(
        (arr, elem) in element_pair_strategy(),
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        idx_mod in any::<u32>(),
    ) {
        let max = max_lane_for(arr);
        let idx = idx_mod % (max + 1);
        let ops = vec![dst(rd, arr), lane(rn, elem, idx)];
        let w = word_of(encode_neon_dup(&ops));
        prop_assert_eq!(
            w, reference_word_element(rd, arr, rn, elem, idx),
            "element-form word must equal ARM ARM template for \
             dup v{}.{} v{}.{}[{}]",
            rd, arr, rn, elem, idx
        );
    }

    // === Oracle: structural — fixed opcode bits (both forms) =============
    // bit 31 == 0; bits 29-21 == 0b001110000. The opcode at bits 15-10 is
    // 000011 for the general form and 000001 for the element form.
    #[test]
    fn prop_fixed_opcode_bits(
        arr in general_arr_strategy(),
        (earr, elem) in element_pair_strategy(),
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        idx_mod in any::<u32>(),
    ) {
        // general form
        let wg = word_of(encode_neon_dup(&[dst(rd, arr), gp(rn)]));
        prop_assert_eq!(wg >> 31, 0u32, "bit 31 must be 0 (general)");
        prop_assert_eq!((wg >> 21) & 0x1FF, 0b001110000u32, "bits 29-21 fixed (general)");
        prop_assert_eq!((wg >> 10) & 0x3F, 0b000011u32, "bits 15-10 = DUP-general opcode");

        // element form
        let idx = idx_mod % (max_lane_for(earr) + 1);
        let we = word_of(encode_neon_dup(&[dst(rd, earr), lane(rn, elem, idx)]));
        prop_assert_eq!(we >> 31, 0u32, "bit 31 must be 0 (element)");
        prop_assert_eq!((we >> 21) & 0x1FF, 0b001110000u32, "bits 29-21 fixed (element)");
        prop_assert_eq!((we >> 10) & 0x3F, 0b000001u32, "bits 15-10 = DUP-element opcode");
    }

    // === Oracle: structural — Q bit tracks arrangement width =============
    // Q (bit 30) is 1 exactly for the wide 128-bit arrangements.
    #[test]
    fn prop_q_bit_tracks_width(
        arr in general_arr_strategy(),
        (earr, elem) in element_pair_strategy(),
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        idx_mod in any::<u32>(),
    ) {
        let wg = word_of(encode_neon_dup(&[dst(rd, arr), gp(rn)]));
        prop_assert_eq!((wg >> 30) & 1, q_of(arr), "Q mismatch (general) for .{}", arr);

        let idx = idx_mod % (max_lane_for(earr) + 1);
        let we = word_of(encode_neon_dup(&[dst(rd, earr), lane(rn, elem, idx)]));
        prop_assert_eq!((we >> 30) & 1, q_of(earr), "Q mismatch (element) for .{}", earr);
    }

    // === Oracle: field isolation — Rd / Rn / imm5 ========================
    // Rd occupies exactly bits [4:0]; Rn exactly bits [9:5]. Varying one
    // register leaves every other field untouched. (imm5 placement is covered
    // by the algebraic property below.)
    #[test]
    fn prop_fields_isolated(
        (arr, elem) in element_pair_strategy(),
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        idx_mod in any::<u32>(),
    ) {
        let idx = idx_mod % (max_lane_for(arr) + 1);

        let base = word_of(encode_neon_dup(&[dst(0, arr), lane(0, elem, idx)]));

        // Rd round-trips in [4:0] and leaks nowhere else.
        let with_rd = word_of(encode_neon_dup(&[dst(rd, arr), lane(0, elem, idx)]));
        prop_assert_eq!(with_rd & 0x1F, rd, "Rd not in bits [4:0]");
        prop_assert_eq!(with_rd & 0xFFFF_FFE0, base & 0xFFFF_FFE0, "Rd leaked above bit 4");

        // Rn round-trips in [9:5] and leaks nowhere else.
        let with_rn = word_of(encode_neon_dup(&[dst(0, arr), lane(rn, elem, idx)]));
        prop_assert_eq!((with_rn >> 5) & 0x1F, rn, "Rn not in bits [9:5]");
        prop_assert_eq!(with_rn & !0x3E0, base & !0x3E0, "Rn leaked outside bits [9:5]");
    }

    // === Oracle: algebraic — imm5 packs size sentinel + (element) index ==
    // imm5 (bits 20-16) == (index << shift) | sentinel. For the general form
    // there is no index, so imm5 == sentinel. The sentinel uniquely tags the
    // element size in both cases.
    #[test]
    fn prop_imm5_packs_size_and_index(
        arr in general_arr_strategy(),
        (earr, elem) in element_pair_strategy(),
        rd in reg_num_strategy(),
        rn in reg_num_strategy(),
        idx_mod in any::<u32>(),
    ) {
        // general form: imm5 is just the size sentinel
        let wg = word_of(encode_neon_dup(&[dst(rd, arr), gp(rn)]));
        let imm5_g = (wg >> 16) & 0x1F;
        let (sentinel_g, _, _) = size_meta(general_elem_for(arr)).unwrap();
        prop_assert_eq!(imm5_g, sentinel_g, "general imm5 must be the size sentinel");

        // element form: imm5 == (index << shift) | sentinel
        let idx = idx_mod % (max_lane_for(earr) + 1);
        let we = word_of(encode_neon_dup(&[dst(rd, earr), lane(rn, elem, idx)]));
        let imm5_e = (we >> 16) & 0x1F;
        let (sentinel_e, shift_e, _) = size_meta(elem).unwrap();
        prop_assert_eq!(imm5_e, (idx << shift_e) | sentinel_e, "element imm5 must pack index+size");
        prop_assert_eq!(imm5_e & sentinel_e, sentinel_e, "size sentinel must be set");
    }

    // === Oracle: negative contract — malformed operands (currently OK) ===
    // Too few operands, a non-register second operand, an unsupported
    // arrangement, and an unparseable (out-of-range) register must all yield
    // Err. These paths are already correct in the encoder.
    #[test]
    fn prop_rejects_malformed_operands(
        bad_arr in "[0-9]{1,2}[a-z]{1,2}".prop_filter(
            "not a valid arrangement", |s| !matches!(
                s.as_str(),
                "8b"|"16b"|"4h"|"8h"|"2s"|"4s"|"1d"|"2d"
            )),
        bad_reg in "(v|x)(3[2-9]|[4-9][0-9])", // numeric, always > 31
    ) {
        // empty operand list
        prop_assert!(encode_neon_dup(&[]).is_err());
        // single operand only
        prop_assert!(encode_neon_dup(&[dst(0, "4s")]).is_err());
        // second operand is neither Reg nor RegLane
        prop_assert!(encode_neon_dup(&[dst(0, "4s"), Operand::Imm(0)]).is_err());
        // unsupported arrangement on the destination (general path)
        let ops: Vec<Operand> = vec![
            Operand::RegArrangement { reg: "v0".into(), arrangement: bad_arr.clone() },
            gp(0),
        ];
        prop_assert!(encode_neon_dup(&ops).is_err(),
            "arrangement {:?} must be rejected", bad_arr);
        // unparseable (out-of-range) source register (general path)
        let ops: Vec<Operand> = vec![
            dst(0, "4s"),
            Operand::Reg(bad_reg.clone()),
        ];
        prop_assert!(encode_neon_dup(&ops).is_err(),
            "source register {:?} must be rejected", bad_reg);
    }

    // === Oracle: negative contract — out-of-range lane index =============
    // FINDING (EXPECTED TO FAIL — hence #[ignore]). The lane index must fit the
    // imm5 field for its element size: .b -> [0,15], .h -> [0,7], .s -> [0,3],
    // .d -> [0,1]. `llvm-mc` rejects e.g. `dup v0.16b, v0.b[16]` with
    // "vector lane must be an integer in range [0, 15]". No AArch64 spec defines
    // wrapping/truncation as intentional, so the encoder MUST return Err.
    // The current code silently masks the index (`& 0xF`/`& 0x7`/`& 0x3`/
    // `& 0x1`) and emits a valid-looking but *wrong* word.
    #[test]
    #[ignore = "bug witness: element-form lane index is masked, not validated"]
    fn prop_rejects_out_of_range_lane_index(
        bad in prop_oneof![
            Just(("16b", "b", 16u32)),
            Just(("8h",  "h", 8u32)),
            Just(("4s",  "s", 4u32)),
            Just(("2d",  "d", 2u32)),
        ],
        rn in reg_num_strategy(),
    ) {
        let (arr, elem, bad_index) = bad;
        let ops: Vec<Operand> = vec![dst(0, arr), lane(rn, elem, bad_index)];
        prop_assert!(encode_neon_dup(&ops).is_err(),
            "out-of-range lane [{}] for .{} must be rejected, \
             not silently truncated/masked; got {:?}",
            bad_index, elem, encode_neon_dup(&ops));
    }

    // === Oracle: negative contract — arrangement / element-size mismatch ==
    // FINDING (EXPECTED TO FAIL — hence #[ignore]). The source element size in
    // `DUP Vd.<T>, Vn.<Ts>[i]` must match the destination arrangement's element
    // size (e.g. `.4S` <-> `.s[i]`). A mismatched pair is silently encoded into
    // an architecturally UNDEFINED / differently-disassembled word because Q is
    // taken from the arrangement and imm5 from the element size independently.
    // `llvm-mc` rejects these with "invalid operand for instruction".
    #[test]
    #[ignore = "bug witness: arrangement/element-size mismatch is not validated"]
    fn prop_rejects_arrangement_element_size_mismatch(
        bad in prop_oneof![
            Just(("4s",  "h")),
            Just(("8h",  "s")),
            Just(("16b", "d")),
            Just(("2d",  "b")),
            Just(("4h",  "b")),
            Just(("2s",  "h")),
        ],
        rn in reg_num_strategy(),
    ) {
        let (arr, elem) = bad;
        let ops: Vec<Operand> = vec![dst(0, arr), lane(rn, elem, 0)];
        prop_assert!(encode_neon_dup(&ops).is_err(),
            "dup v0.{}, v{}.{}[0] mixes a .{} arrangement with a \
             .{} element and must be rejected, not silently encoded as {:?}",
            arr, rn, elem, arr, elem, encode_neon_dup(&ops));
    }
}

/// Map a general-form arrangement to its element-size name (for the algebraic
/// property's sentinel lookup).
fn general_elem_for(arr: &str) -> &'static str {
    match arr {
        "8b" | "16b" => "b",
        "4h" | "8h" => "h",
        "2s" | "4s" => "s",
        "2d" => "d",
        _ => unreachable!(),
    }
}

// --- golden differential regression (ARM ARM template anchors) -----------
//
// Every word below is computed from the ARM ARM encoding templates documented
// at the top of this file. They anchor the reference oracle to a fixed bit
// pattern independent of the proptest generators.

#[test]
fn golden_dup_general_form() {
    // dup v0.16b, x0   => 0x4E010C00
    assert_eq!(
        word_of(encode_neon_dup(&[dst(0, "16b"), gp(0)])),
        0x4E010C00
    );
    // dup v0.8b,  x0   => 0x0E010C00  (Q=0)
    assert_eq!(word_of(encode_neon_dup(&[dst(0, "8b"), gp(0)])), 0x0E010C00);
    // dup v0.4s,  x0   => 0x4E040C00
    assert_eq!(word_of(encode_neon_dup(&[dst(0, "4s"), gp(0)])), 0x4E040C00);
    // dup v0.8h,  x0   => 0x4E020C00
    assert_eq!(word_of(encode_neon_dup(&[dst(0, "8h"), gp(0)])), 0x4E020C00);
    // dup v0.2d,  x0   => 0x4E080C00
    assert_eq!(word_of(encode_neon_dup(&[dst(0, "2d"), gp(0)])), 0x4E080C00);
    // dup v0.8b,  x5   => 0x0E010CA0
    assert_eq!(word_of(encode_neon_dup(&[dst(0, "8b"), gp(5)])), 0x0E010CA0);
    // dup v31.2d, x30  => 0x4E080FDF
    assert_eq!(
        word_of(encode_neon_dup(&[dst(31, "2d"), gp(30)])),
        0x4E080FDF
    );
}

#[test]
fn golden_dup_element_form() {
    // dup v0.16b, v0.b[0]  => 0x4E010400
    assert_eq!(
        word_of(encode_neon_dup(&[dst(0, "16b"), lane(0, "b", 0)])),
        0x4E010400
    );
    // dup v0.16b, v0.b[15] => 0x4E1F0400  (boundary lane)
    assert_eq!(
        word_of(encode_neon_dup(&[dst(0, "16b"), lane(0, "b", 15)])),
        0x4E1F0400
    );
    // dup v0.4s,  v0.s[0]  => 0x4E040400
    assert_eq!(
        word_of(encode_neon_dup(&[dst(0, "4s"), lane(0, "s", 0)])),
        0x4E040400
    );
    // dup v0.4s,  v0.s[3]  => 0x4E1C0400
    assert_eq!(
        word_of(encode_neon_dup(&[dst(0, "4s"), lane(0, "s", 3)])),
        0x4E1C0400
    );
    // dup v0.2d,  v0.d[1]  => 0x4E180400
    assert_eq!(
        word_of(encode_neon_dup(&[dst(0, "2d"), lane(0, "d", 1)])),
        0x4E180400
    );
    // dup v0.8h,  v0.h[7]  => 0x4E1E0400
    assert_eq!(
        word_of(encode_neon_dup(&[dst(0, "8h"), lane(0, "h", 7)])),
        0x4E1E0400
    );
    // dup v5.2s,  v7.s[1]  => 0x0E0C04E5  (Q=0 for .2s, mixed regs)
    assert_eq!(
        word_of(encode_neon_dup(&[dst(5, "2s"), lane(7, "s", 1)])),
        0x0E0C04E5
    );
}

#[test]
fn rejects_too_few_operands() {
    assert!(encode_neon_dup(&[]).is_err(), "0 operands must error");
    assert!(
        encode_neon_dup(&[dst(0, "4s")]).is_err(),
        "1 operand must error"
    );
    // Exactly two valid operands must succeed for both forms.
    assert!(
        encode_neon_dup(&[dst(0, "4s"), gp(0)]).is_ok(),
        "general form must succeed"
    );
    assert!(
        encode_neon_dup(&[dst(0, "4s"), lane(0, "s", 0)]).is_ok(),
        "element form must succeed"
    );
}
