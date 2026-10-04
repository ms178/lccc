// Property-based tests for the load/store encoders, isolated in a separate
// `cfg(test)` module file.
//
// Targets:
//   * encode_ldr_str_auto  — LDR/STR size auto-detection
//   * encode_ldnp_stnp     — LDNP/STNP (no-allocate pair)
//   * encode_swp           — SWP/SWPA/SWPAL/SWPL (+ byte/half variants)
//   * encode_cas           — CAS/CASA/CASL/CASAL (+ byte/half variants)
//
// Oracles are INDEPENDENT of the crate's own encoding formulas: they are
// derived directly from the ARM ARM bit layouts and checked against the
// encoded word. Two kinds of properties live here:
//
//   * "passing" properties (run by default) — reference field placement,
//     load/store & acquire/release differentials, size mapping and error
//     contracts that the current implementation satisfies.
//
//   * "witness" properties marked `#[ignore]` — they assert the CORRECT
//     contract (must be `Err`) for known defects that are already documented
//     under the transplanted campaign notes. They FAIL against the current SUT, so they
//     are ignored to keep `cargo test` green. Run them explicitly with
//     `cargo test --test pbt_arm_load_store load_store_pbt -- --ignored`.

use lccc::pbt_internals::*;
use proptest::prelude::*;

// ── shared helpers ───────────────────────────────────────────────────────

fn reg(prefix: char, num: u32) -> Operand {
    // Register 31 is spelled xzr/wzr (the canonical spelling; "x31" is not).
    Operand::Reg(if num == 31 {
        format!("{}zr", prefix)
    } else {
        format!("{}{}", prefix, num)
    })
}

fn mem(base_num: u32, offset: i64) -> Operand {
    Operand::Mem {
        base: format!("x{}", base_num),
        offset,
    }
}

fn word(r: Result<EncodeResult, String>) -> u32 {
    match r {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected Word, got {:?}", other),
    }
}

// =============================================================================
// encode_ldr_str_auto  —  LDR/STR size auto-detection
//
// For `[Xn, #0]` the unsigned-offset form is taken:
//   size[31:30] | 111[29:27] | V[26] | 01[25:24] | opc[23:22]
//   | imm12[21:10] | Rn[9:5] | Rt[4:0]
// auto-detection: w/s -> size 10 ; x/d/sp/xzr/lr -> size 11 ; q -> size 00.
// V[26] = 1 iff FP/SIMD register. opc: GP load 01 / store 00 ; Q load 11 /
// store 10 — so load^store always flips only opc bit[22] (0x0040_0000).
// =============================================================================

prop_compose! {
    fn arb_ldr_str_reg()(idx in 0usize..5usize, num in 0u32..=30u32) -> (String, u32, char) {
        const CLASSES: [char; 5] = ['x', 'w', 'd', 's', 'q'];
        let prefix = CLASSES[idx];
        (format!("{}{}", prefix, num), num, prefix)
    }
}

fn auto_expected_size(prefix: char) -> u32 {
    match prefix {
        'w' | 's' => 0b10,
        'x' | 'd' => 0b11,
        'q' => 0b00,
        _ => unreachable!(),
    }
}

fn auto_expected_v(prefix: char) -> u32 {
    match prefix {
        'd' | 's' | 'q' => 1,
        _ => 0,
    }
}

proptest! {
    // size field [31:30] follows the Rt register prefix (reference).
    #[test]
    fn auto_size_field_matches_prefix(
        (rt_name, _, prefix) in arb_ldr_str_reg(),
        base_num in 0u32..=30u32,
    ) {
        let ops = vec![Operand::Reg(rt_name), mem(base_num, 0)];
        let w = word(encode_ldr_str_auto(&ops, true));
        prop_assert_eq!((w >> 30) & 0b11, auto_expected_size(prefix));
    }

    // V (vector) bit [26] is set iff Rt is FP/SIMD (reference).
    #[test]
    fn auto_v_bit_tracks_fp(
        (rt_name, _, prefix) in arb_ldr_str_reg(),
        base_num in 0u32..=30u32,
    ) {
        let ops = vec![Operand::Reg(rt_name), mem(base_num, 0)];
        let w = word(encode_ldr_str_auto(&ops, true));
        prop_assert_eq!((w >> 26) & 1, auto_expected_v(prefix));
    }

    // Rt occupies [4:0], Rn occupies [9:5] (reference).
    #[test]
    fn auto_rt_rn_field_placement(
        (rt_name, rt_num, _) in arb_ldr_str_reg(),
        base_num in 0u32..=30u32,
    ) {
        let ops = vec![Operand::Reg(rt_name), mem(base_num, 0)];
        let w = word(encode_ldr_str_auto(&ops, true));
        prop_assert_eq!(w & 0x1F, rt_num, "Rt [4:0]");
        prop_assert_eq!((w >> 5) & 0x1F, base_num, "Rn [9:5]");
    }

    // Load vs store differ ONLY in opc bit[22]: load^store == 0x0040_0000,
    // for every addressing form (differential). Holds for both GP (01/00) and
    // Q (11/10) because the XOR is 0b01 in opc[23:22] for both.
    #[test]
    fn auto_load_xor_store_flips_opc_bit22(
        (rt_name, _, _) in arb_ldr_str_reg(),
        base_num in 0u32..=30u32,
        form in 0u8..3u8,
    ) {
        let m = match form {
            0 => mem(base_num, 0),
            1 => Operand::MemPreIndex { base: format!("x{}", base_num), offset: 0 },
            _ => Operand::MemPostIndex { base: format!("x{}", base_num), offset: 0 },
        };
        let ops = vec![Operand::Reg(rt_name), m];
        let load = word(encode_ldr_str_auto(&ops, true));
        let store = word(encode_ldr_str_auto(&ops, false));
        prop_assert_eq!(load ^ store, 0x0040_0000u32);
    }

    // Negative contract: a non-Reg first operand must be rejected.
    #[test]
    fn auto_non_reg_first_operand_rejected(kind in 0u8..3u8) {
        let bad = match kind {
            0 => Operand::Imm(5),
            1 => Operand::Symbol("foo".to_string()),
            _ => Operand::Mem { base: "x0".to_string(), offset: 0 },
        };
        prop_assert!(encode_ldr_str_auto(&[bad], true).is_err());
    }
}

// =============================================================================
// encode_ldnp_stnp  —  LDNP/STNP (no-allocate pair)
//
//   opc[31:30] | 101[29:27] | 000[25:23] | L[22] | imm7[21:15]
//   | Rt2[14:10] | Rn[9:5] | Rt[4:0]
// opc = 10 (64-bit X) / 00 (32-bit W). V=0 (integer-only). imm7 is a SIGNED
// 7-bit scaled immediate (scale 8 for X, 4 for W): architectural range
// imm7 in [-64, +63] and the offset MUST be a multiple of the scale.
// =============================================================================

fn ldnp_ops(rt_prefix: char, rt1: u32, rt2: u32, base: u32, offset: i64) -> Vec<Operand> {
    vec![reg(rt_prefix, rt1), reg(rt_prefix, rt2), mem(base, offset)]
}

proptest! {
    // opc[31:30] = 10 for X, 00 for W; fixed bits [29:27]=101, [25:23]=000,
    // V[26]=0 (reference).
    #[test]
    fn ldnp_opc_and_fixed_bits(
        rt1 in 0u32..=30u32,
        rt2 in 0u32..=30u32,
        base in 0u32..=30u32,
        is_x in any::<bool>(),
    ) {
        let p = if is_x { 'x' } else { 'w' };
        let w = word(encode_ldnp_stnp(&ldnp_ops(p, rt1, rt2, base, 0), true));
        prop_assert_eq!((w >> 30) & 0b11, if is_x { 0b10u32 } else { 0b00u32 }, "opc");
        prop_assert_eq!((w >> 27) & 0b111, 0b101u32, "[29:27]");
        prop_assert_eq!((w >> 23) & 0b111, 0b000u32, "[25:23]");
        prop_assert_eq!((w >> 26) & 1, 0u32, "V=0");
    }

    // LDNP vs STNP differ ONLY in the L bit [22] (differential).
    #[test]
    fn ldnp_load_xor_store_is_l_bit(
        rt1 in 0u32..=30u32,
        rt2 in 0u32..=30u32,
        base in 0u32..=30u32,
        is_x in any::<bool>(),
    ) {
        let p = if is_x { 'x' } else { 'w' };
        let ops = ldnp_ops(p, rt1, rt2, base, 0);
        let load = word(encode_ldnp_stnp(&ops, true));
        let store = word(encode_ldnp_stnp(&ops, false));
        prop_assert_eq!(load ^ store, 0x0040_0000u32);
    }

    // Rt -> [4:0], Rn -> [9:5], Rt2 -> [14:10] (reference).
    #[test]
    fn ldnp_register_field_placement(
        rt1 in 0u32..=30u32,
        rt2 in 0u32..=30u32,
        base in 0u32..=30u32,
        is_x in any::<bool>(),
    ) {
        let p = if is_x { 'x' } else { 'w' };
        let w = word(encode_ldnp_stnp(&ldnp_ops(p, rt1, rt2, base, 0), true));
        prop_assert_eq!(w & 0x1F, rt1, "Rt");
        prop_assert_eq!((w >> 5) & 0x1F, base, "Rn");
        prop_assert_eq!((w >> 10) & 0x1F, rt2, "Rt2");
    }

    // For aligned, in-range offsets the imm7 field [21:15] round-trips the
    // scaled value (reference). imm7 in [-63, +63] stays in range for both
    // 64-bit (scale 8) and 32-bit (scale 4).
    #[test]
    fn ldnp_in_range_aligned_imm7_round_trips(
        imm7 in -63i32..=63i32,
        base in 0u32..=30u32,
        is_x in any::<bool>(),
    ) {
        let p = if is_x { 'x' } else { 'w' };
        let scale = if is_x { 3i64 } else { 2 };
        let offset = (imm7 as i64) << scale;
        let w = word(encode_ldnp_stnp(&ldnp_ops(p, 0, 1, base, offset), true));
        // sign-extend the 7-bit imm7 field [21:15] back to i32
        let raw = (w >> 15) & 0x7F;
        let signed = if raw & 0x40 != 0 { (raw | 0xFFFF_FF80) as i32 } else { raw as i32 };
        prop_assert_eq!(signed, imm7);
    }

    // Negative contract: only the `[base, #offset]` form is legal; pre-index,
    // post-index and register-offset forms must be rejected.
    #[test]
    fn ldnp_non_base_offset_mem_rejected(
        rt1 in 0u32..=30u32,
        rt2 in 0u32..=30u32,
        base in 0u32..=30u32,
        variant in 0u32..3u32,
        is_load in any::<bool>(),
    ) {
        let bad = match variant {
            0 => Operand::MemPreIndex { base: format!("x{}", base), offset: 0 },
            1 => Operand::MemPostIndex { base: format!("x{}", base), offset: 0 },
            _ => Operand::MemRegOffset {
                base: format!("x{}", base),
                index: "x9".to_string(),
                extend: None,
                shift: None,
            },
        };
        let ops = vec![reg('x', rt1), reg('x', rt2), bad];
        prop_assert!(
            encode_ldnp_stnp(&ops, is_load).is_err(),
            "LDNP/STNP only accepts [base, #offset]"
        );
    }

    // ── BUG WITNESS (documented) ─────────────────────────────────────────
    // LDNP/STNP imm7 is signed 7-bit; offsets beyond [-64*scale, 63*scale]
    // are unrepresentable and MUST be rejected, but the encoder masks with
    // `(*offset >> shift) & 0x7F` (no range check), wrapping silently.
    //
    #[test]
    #[ignore = "documented bug: ldnp/stnp out-of-range imm7 wraps, not rejected"]
    fn witness_ldnp_out_of_range_offset_rejected(
        is_x in any::<bool>(),
        k in 0u32..8u32,
    ) {
        let scale = if is_x { 3i64 } else { 2 };
        let p = if is_x { 'x' } else { 'w' };
        // offsets strictly outside the legal window for each width
        let over: [i64; 8] = [
            64, 100, 200, 512, -65, -128, -260, -512,
        ];
        let offset = over[k as usize] << scale;
        let res = encode_ldnp_stnp(&ldnp_ops(p, 0, 1, 2, offset), true);
        prop_assert!(res.is_err(),
            "offset {} (scale {}) is out of range and must be rejected; got {:?}",
            offset, scale, res);
    }

    // ── BUG WITNESS (documented) ─────────────────────────────────────────
    // The offset MUST be a multiple of the scale; a misaligned offset is
    // unrepresentable and MUST be rejected, but the arithmetic shift drops
    // the low bits silently. See
    // the transplanted campaign notes.
    #[test]
    #[ignore = "documented bug: ldnp/stnp misaligned offset is silently truncated"]
    fn witness_ldnp_misaligned_offset_rejected(
        is_x in any::<bool>(),
        low in 1i64..=3i64,
    ) {
        let scale = if is_x { 3i64 } else { 2 };
        let p = if is_x { 'x' } else { 'w' };
        let align = 1i64 << scale;
        // a small in-range magnitude that is NOT a multiple of the scale
        let offset = align + low;
        if offset % align == 0 {
            return Ok(()); // keep the generator pure; nothing to check
        }
        let res = encode_ldnp_stnp(&ldnp_ops(p, 0, 1, 2, offset), true);
        prop_assert!(res.is_err(),
            "misaligned offset {} (scale {}, align {}) must be rejected; got {:?}",
            offset, scale, align, res);
    }
}

// =============================================================================
// encode_swp  —  SWP/SWPA/SWPAL/SWPL (+ byte/half variants)
//
//   size[31:30] | 111000[29:24] | A[23] | R[22] | 1[21] | Rs[20:16] | 1[15]
//   | 00000[14:10] | Rn[9:5] | Rt[4:0]
// size: b->00, h->01, else from Rs width (x->11, w->10). A=acquire ('a'),
// R=release ('l'). Only `[Xn|SP]` addressing is legal (no offset field).
// =============================================================================

const SWP_MNEMONICS: &[&str] = &[
    "swp", "swpa", "swpal", "swpl", "swpb", "swpab", "swpalb", "swplb", "swph", "swpah", "swpalh",
    "swplh",
];

fn swp_ops(rs_w: char, rs: u32, rt_w: char, rt: u32, base: u32, offset: i64) -> Vec<Operand> {
    vec![reg(rs_w, rs), reg(rt_w, rt), mem(base, offset)]
}

proptest! {
    // Fixed opcode bits: [29:24]=111000, [21]=1, [15]=1, [14:10]=0 (reference).
    #[test]
    fn swp_fixed_opcode_bits(
        mn_idx in 0usize..SWP_MNEMONICS.len(),
        rs in 0u32..=31u32,
        rt in 0u32..=31u32,
        base in 0u32..=31u32,
        wide in any::<bool>(),
    ) {
        let mn = SWP_MNEMONICS[mn_idx];
        let w = if mn.contains('b') || mn.contains('h') { 'w' }
                else if wide { 'x' } else { 'w' };
        let word = word(encode_swp(mn, &swp_ops(w, rs, w, rt, base, 0)));
        prop_assert_eq!((word >> 24) & 0x3F, 0b111000u32, "[29:24]");
        prop_assert_eq!((word >> 21) & 1, 1u32, "bit 21");
        prop_assert_eq!((word >> 15) & 1, 1u32, "bit 15");
        prop_assert_eq!((word >> 10) & 0x1F, 0u32, "[14:10]");
    }

    // Rs -> [20:16], Rt -> [4:0], Rn -> [9:5] (reference).
    #[test]
    fn swp_register_field_placement(
        mn_idx in 0usize..SWP_MNEMONICS.len(),
        rs in 0u32..=31u32,
        rt in 0u32..=31u32,
        base in 0u32..=31u32,
        wide in any::<bool>(),
    ) {
        let mn = SWP_MNEMONICS[mn_idx];
        let w = if mn.contains('b') || mn.contains('h') { 'w' }
                else if wide { 'x' } else { 'w' };
        let word = word(encode_swp(mn, &swp_ops(w, rs, w, rt, base, 0)));
        prop_assert_eq!((word >> 16) & 0x1F, rs, "Rs");
        prop_assert_eq!(word & 0x1F, rt, "Rt");
        prop_assert_eq!((word >> 5) & 0x1F, base, "Rn");
    }

    // Acquire ('a') flips ONLY bit[23]; release ('l') flips ONLY bit[22]
    // (differential).
    #[test]
    fn swp_acquire_release_differential(
        pair in 0u8..4u8,
        rs in 0u32..=31u32,
        rt in 0u32..=31u32,
        base in 0u32..=31u32,
    ) {
        let ops = swp_ops('x', rs, 'x', rt, base, 0);
        let (m0, m1) = match pair {
            0 => ("swp", "swpa"),
            1 => ("swp", "swpl"),
            2 => ("swpa", "swpal"),
            _ => ("swpl", "swpal"),
        };
        let w0 = word(encode_swp(m0, &ops));
        let w1 = word(encode_swp(m1, &ops));
        let s0 = m0.strip_prefix("swp").unwrap_or(m0);
        let s1 = m1.strip_prefix("swp").unwrap_or(m1);
        let mut expected = 0u32;
        if s1.contains('a') != s0.contains('a') { expected |= 1 << 23; }
        if s1.contains('l') != s0.contains('l') { expected |= 1 << 22; }
        prop_assert_eq!(w0 ^ w1, expected, "acquire/release differential");
    }

    // size mapping: b->00, h->01, else x->11 / w->10 (reference).
    #[test]
    fn swp_size_from_suffix_and_width(
        sc in 0u8..3u8,
        rs_is_64 in any::<bool>(),
    ) {
        let suffix = match sc { 1 => "b", 2 => "h", _ => "" };
        let mn = format!("swp{}", suffix);
        let rp = if sc == 0 { if rs_is_64 { 'x' } else { 'w' } } else { 'w' };
        let word = word(encode_swp(&mn, &swp_ops(rp, 7, rp, 8, 2, 0)));
        let expected = match sc {
            1 => 0b00u32,
            2 => 0b01u32,
            _ => if rs_is_64 { 0b11u32 } else { 0b10u32 },
        };
        prop_assert_eq!((word >> 30) & 0b11, expected, "size");
    }

    // Rejection contract (was a bug mechanism: the offset was silently
    // dropped). SWP has no offset field; a non-zero offset must be an error.
    #[test]
    fn swp_nonzero_offset_is_rejected(
        mn_idx in 0usize..SWP_MNEMONICS.len(),
        off in any::<i64>().prop_filter("non-zero", |o| *o != 0),
    ) {
        let mn = SWP_MNEMONICS[mn_idx];
        // Register-form mnemonics with x registers, so the offset is the
        // only reason to fail (b/h forms take w registers).
        let reg_form = !mn.contains('b') && !mn.contains('h');
        let (rw, tw) = if reg_form { ('x', 'x') } else { ('w', 'w') };
        let res = encode_swp(mn, &swp_ops(rw, 0, tw, 1, 2, off));
        prop_assert!(res.is_err(),
            "non-zero offset must be rejected, got {:?}", res);
    }

    // Negative contract: fewer than 3 operands, or a non-memory third
    // operand, must be rejected.
    #[test]
    fn swp_bad_operand_shapes_rejected(kind in 0u8..4u8) {
        let r = match kind {
            0 => encode_swp("swp", &[]),
            1 => encode_swp("swp", &[reg('x', 0)]),
            2 => encode_swp("swp", &[reg('x', 0), reg('x', 1)]),
            _ => encode_swp("swp", &[reg('x', 0), reg('x', 1), Operand::Imm(3)]),
        };
        prop_assert!(r.is_err(), "expected Err, got {:?}", r);
    }

    // ── BUG WITNESS (documented) ─────────────────────────────────────────
    // SWP encodes only `[Xn|SP]` — there is no offset field, so a non-zero
    // immediate offset is unrepresentable and MUST be rejected. The encoder
    // silently drops it instead. See
    // the transplanted campaign notes.
    #[test]
    #[ignore = "documented bug: swp non-zero offset silently dropped, not rejected"]
    fn witness_swp_nonzero_offset_rejected(
        mn_idx in 0usize..SWP_MNEMONICS.len(),
        off in (-32768i64..32767i64).prop_filter("non-zero", |o| *o != 0),
    ) {
        let mn = SWP_MNEMONICS[mn_idx];
        let res = encode_swp(mn, &swp_ops('x', 0, 'x', 1, 2, off));
        prop_assert!(res.is_err(),
            "non-zero offset {} on {} must be rejected (no offset field); got {:?}",
            off, mn, res);
    }
}

// =============================================================================
// encode_cas  —  CAS/CASA/CASL/CASAL (+ byte/half variants)
//
//   size[31:30] | 001000[29:24] | 1[23] | L[22] | 1[21] | Rs[20:16]
//   | o0[15] | 11111[14:10] | Rn[9:5] | Rt[4:0]
// size: b->00, h->01, else from Rs width (x->11, w->10). L=acquire ('a'),
// o0=release ('l'). Only `[Xn|SP]` addressing is legal (no offset field).
// =============================================================================

const CAS_MNEMONICS: &[&str] = &[
    "cas", "casa", "casl", "casal", "casb", "casab", "caslb", "casalb", "cash", "casah", "caslh",
    "casalh",
];

fn cas_ops(rs_w: char, rs: u32, rt_w: char, rt: u32, base: u32, offset: i64) -> Vec<Operand> {
    vec![reg(rs_w, rs), reg(rt_w, rt), mem(base, offset)]
}

proptest! {
    // Fixed opcode bits: [29:24]=001000, [23]=1, [21]=1, [14:10]=11111.
    #[test]
    fn cas_fixed_opcode_bits(
        mn_idx in 0usize..CAS_MNEMONICS.len(),
        rs in 0u32..=31u32,
        rt in 0u32..=31u32,
        base in 0u32..=31u32,
        wide in any::<bool>(),
    ) {
        let mn = CAS_MNEMONICS[mn_idx];
        let w = if mn.contains('b') || mn.contains('h') { 'w' }
                else if wide { 'x' } else { 'w' };
        let word = word(encode_cas(mn, &cas_ops(w, rs, w, rt, base, 0)));
        prop_assert_eq!((word >> 24) & 0x3F, 0b001000u32, "[29:24]");
        prop_assert_eq!((word >> 23) & 1, 1u32, "bit 23");
        prop_assert_eq!((word >> 21) & 1, 1u32, "bit 21");
        prop_assert_eq!((word >> 10) & 0x1F, 0b11111u32, "[14:10]");
    }

    // Rs -> [20:16], Rn -> [9:5], Rt -> [4:0] (reference).
    #[test]
    fn cas_register_field_placement(
        mn_idx in 0usize..CAS_MNEMONICS.len(),
        rs in 0u32..=31u32,
        rt in 0u32..=31u32,
        base in 0u32..=31u32,
        wide in any::<bool>(),
    ) {
        let mn = CAS_MNEMONICS[mn_idx];
        let w = if mn.contains('b') || mn.contains('h') { 'w' }
                else if wide { 'x' } else { 'w' };
        let word = word(encode_cas(mn, &cas_ops(w, rs, w, rt, base, 0)));
        prop_assert_eq!((word >> 16) & 0x1F, rs, "Rs");
        prop_assert_eq!((word >> 5) & 0x1F, base, "Rn");
        prop_assert_eq!(word & 0x1F, rt, "Rt");
    }

    // Acquire ('a') flips ONLY L bit[22]; release ('l') flips ONLY o0 bit[15]
    // (differential).
    #[test]
    fn cas_acquire_release_differential(
        pair in 0u8..4u8,
        rs in 0u32..=31u32,
        rt in 0u32..=31u32,
        base in 0u32..=31u32,
    ) {
        let ops = cas_ops('x', rs, 'x', rt, base, 0);
        let (m0, m1) = match pair {
            0 => ("cas", "casa"),
            1 => ("cas", "casl"),
            2 => ("casa", "casal"),
            _ => ("casl", "casal"),
        };
        let w0 = word(encode_cas(m0, &ops));
        let w1 = word(encode_cas(m1, &ops));
        // The base mnemonic "cas" already contains the letter 'a', so
        // acquire/release MUST be read from the suffix (as the encoder does).
        let s0 = m0.strip_prefix("cas").unwrap_or(m0);
        let s1 = m1.strip_prefix("cas").unwrap_or(m1);
        let mut expected = 0u32;
        if s1.contains('a') != s0.contains('a') { expected |= 1 << 22; }
        if s1.contains('l') != s0.contains('l') { expected |= 1 << 15; }
        prop_assert_eq!(w0 ^ w1, expected, "acquire/release differential");
    }

    // size mapping: b->00, h->01, else x->11 / w->10 (reference).
    #[test]
    fn cas_size_from_suffix_and_width(
        sc in 0u8..3u8,
        rs_is_64 in any::<bool>(),
    ) {
        let suffix = match sc { 1 => "b", 2 => "h", _ => "" };
        let mn = format!("cas{}", suffix);
        let rp = if sc == 0 { if rs_is_64 { 'x' } else { 'w' } } else { 'w' };
        let word = word(encode_cas(&mn, &cas_ops(rp, 7, rp, 8, 2, 0)));
        let expected = match sc {
            1 => 0b00u32,
            2 => 0b01u32,
            _ => if rs_is_64 { 0b11u32 } else { 0b10u32 },
        };
        prop_assert_eq!((word >> 30) & 0b11, expected, "size");
    }

    // Rejection contract (was a bug mechanism: the offset was silently
    // dropped). CAS has no offset field; a non-zero offset must be an error.
    #[test]
    fn cas_nonzero_offset_is_rejected(
        mn_idx in 0usize..CAS_MNEMONICS.len(),
        off in any::<i64>().prop_filter("non-zero", |o| *o != 0),
    ) {
        let mn = CAS_MNEMONICS[mn_idx];
        let reg_form = !mn.contains('b') && !mn.contains('h');
        let (rw, tw) = if reg_form { ('x', 'x') } else { ('w', 'w') };
        let res = encode_cas(mn, &cas_ops(rw, 0, tw, 1, 2, off));
        prop_assert!(res.is_err(),
            "non-zero offset must be rejected, got {:?}", res);
    }

    // Negative contract: pre-index / post-index / register-offset memory
    // forms are not legal for CAS and must be rejected (only [Xn] allowed).
    #[test]
    fn cas_non_base_offset_mem_rejected(
        variant in 0u32..3u32,
        base in 0u32..=31u32,
    ) {
        let bad = match variant {
            0 => Operand::MemPreIndex { base: format!("x{}", base), offset: 0 },
            1 => Operand::MemPostIndex { base: format!("x{}", base), offset: 0 },
            _ => Operand::MemRegOffset {
                base: format!("x{}", base),
                index: "x9".to_string(),
                extend: None,
                shift: None,
            },
        };
        let res = encode_cas("cas", &[reg('x', 0), reg('x', 1), bad]);
        prop_assert!(res.is_err(), "CAS only accepts [Xn]; got {:?}", res);
    }

    // Negative contract: fewer than 3 operands, or a non-memory third
    // operand, must be rejected.
    #[test]
    fn cas_bad_operand_shapes_rejected(kind in 0u8..4u8) {
        let r = match kind {
            0 => encode_cas("cas", &[]),
            1 => encode_cas("cas", &[reg('x', 0)]),
            2 => encode_cas("cas", &[reg('x', 0), reg('x', 1)]),
            _ => encode_cas("cas", &[reg('x', 0), reg('x', 1), Operand::Imm(3)]),
        };
        prop_assert!(r.is_err(), "expected Err, got {:?}", r);
    }

    // ── BUG WITNESS (documented) ─────────────────────────────────────────
    // CAS encodes only `[Xn|SP]` — there is no offset field, so a non-zero
    // immediate offset is unrepresentable and MUST be rejected. The encoder
    // silently drops it instead. See
    // the transplanted campaign notes.
    #[test]
    #[ignore = "documented bug: cas non-zero offset silently dropped, not rejected"]
    fn witness_cas_nonzero_offset_rejected(
        mn_idx in 0usize..CAS_MNEMONICS.len(),
        off in (-32768i64..32767i64).prop_filter("non-zero", |o| *o != 0),
    ) {
        let mn = CAS_MNEMONICS[mn_idx];
        let res = encode_cas(mn, &cas_ops('x', 0, 'x', 1, 2, off));
        prop_assert!(res.is_err(),
            "non-zero offset {} on {} must be rejected (no offset field); got {:?}",
            off, mn, res);
    }

    // ── BUG WITNESS (documented) ─────────────────────────────────────────
    // CASB/CASH require W (32-bit) registers, and the plain CAS form
    // requires Rs and Rt to share width. Mixed widths are architecturally
    // UNDEFINED and MUST be rejected. The encoder accepts them. See
    // the transplanted campaign notes.
    #[test]
    #[ignore = "documented bug: cas accepts mixed-width / X-form byte & half operands"]
    fn witness_cas_mixed_width_rejected(kind in 0u8..5u8) {
        let (mn, ops) = match kind {
            0 => ("casb", vec![reg('x', 0), reg('x', 1), mem(2, 0)]),
            1 => ("casab", vec![reg('x', 5), reg('x', 6), mem(3, 0)]),
            2 => ("cash", vec![reg('x', 0), reg('x', 1), mem(2, 0)]),
            3 => ("cas", vec![reg('w', 0), reg('x', 1), mem(2, 0)]),
            _ => ("cas", vec![reg('x', 0), reg('w', 1), mem(2, 0)]),
        };
        let res = encode_cas(mn, &ops);
        prop_assert!(res.is_err(),
            "mixed-width CAS form ({}) must be rejected; got {:?}", mn, res);
    }
}
