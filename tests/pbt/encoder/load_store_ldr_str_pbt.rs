//! Property-based tests for `encode_ldr_str` (AArch64 LDR/STR encoder).
//!
//! These tests live in a separate file and are compiled only under
//! `#[cfg(test)]`. Bug-witness properties (properties that fail against the
//! current implementation) are marked `#[ignore]` so the default
//! `cargo test` run stays green; run them explicitly with
//! `cargo test -- --ignored`.

use lccc::pbt_internals::*;
use proptest::prelude::*;

// ── Independent oracle ───────────────────────────────────────────────────
// ORACLE: spec-conformance / negative contract — FIELD PLACEMENT +
// OFFSET VALIDATION.
//
// Target: `encode_ldr_str` (LDR/STR — Load/Store Register, ARM ARM
// §C6.2.111 / §C6.2.275, addressing forms: unsigned immediate, unscaled
// (LDUR), pre-index, post-index, register offset, and LDR-literal).
//
// The function is called with an explicit `size` (10=32-bit, 11=64-bit for
// GP registers), `is_load`, `is_signed`, and `is_128bit` flags. We exercise
// only GP (non-FP) registers here so V=0; FP/SIMD paths are exercised by
// sibling suites.
//
// Encodings emitted (relevant to these tests):
//   unsigned offset : size 111 V 01 opc imm12        Rn Rt   ([25:24]=01)
//   unscaled (LDUR) : size 111 V 00 opc 0 imm9 00    Rn Rt   (Mem fallback)
//   pre-index       : size 111 V 00 opc 0 imm9 11    Rn Rt
//   post-index      : size 111 V 00 opc 0 imm9 01    Rn Rt
//
// The imm9 field ([20:12]) is a SIGNED 9-bit immediate in the range
// [-256, 255]. Offsets outside that range are NOT representable in the
// unscaled / pre-index / post-index forms and MUST be rejected with Err.
//
// BUG: in the unscaled (LDUR) fallback, pre-index, and post-index arms the
// encoder computes `imm9 = (*offset as i32) & 0x1FF` with NO range check,
// so out-of-range offsets are silently corrupted:
//   +500  -> low9=0x1F4 -> decoded -12
//   +512  -> low9=0x000 -> decoded   0
//   -300  -> low9=0x0D4 -> decoded +212
//   -512  -> low9=0x000 -> decoded   0
// This is the same masking defect already documented for the sibling
// `encode_ldtr_sized` / `encode_ldrs` encoders. Properties 4 (negative
// contract) and 6 (common-offset anchors) fail today; they are marked
// `#[ignore]` and that failure IS the bug being reported.

fn gp_reg(width: char, num: u32) -> Operand {
    Operand::Reg(format!("{}{}", width, num))
}
fn word(r: Result<EncodeResult, String>) -> u32 {
    match r {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected Word, got {:?}", other),
    }
}
/// Sign-extend the encoded 9-bit imm9 field ([20:12]) back to i32.
fn decode_imm9(w: u32) -> i32 {
    let enc = (w >> 12) & 0x1FF;
    if enc & 0x100 != 0 {
        (enc | 0xFFFFFE00) as i32
    } else {
        enc as i32
    }
}

/// opc bits ([23:22]) the encoder produces for GP registers.
fn expected_opc_gp(is_load: bool, is_signed: bool) -> u32 {
    if is_load {
        if is_signed { 0b10 } else { 0b01 }
    } else {
        0b00
    }
}

proptest! {
    // ── Property 1: unsigned-offset field placement (reference). ──
    // For an aligned, in-range positive offset the encoder must select the
    // unsigned-offset form and place every field correctly. PASSES today.
    #[test]
    fn prop_unsigned_offset_fields_round_trip(
        size in 2u32..=3u32,           // 10 (W) or 11 (X)
        is_load in any::<bool>(),
        is_signed in any::<bool>(),
        rt_num in 0u32..=31u32,
        base_num in 0u32..=31u32,
        imm12 in 0u32..=4095u32,       // full 12-bit range
    ) {
        let shift = size;                 // align = 1 << size for GP regs
        let offset = (imm12 as i64) * (1i64 << shift);
        let ops = vec![
            gp_reg(if size == 3 { 'x' } else { 'w' }, rt_num),
            Operand::Mem { base: format!("x{}", base_num), offset },
        ];
        let w = word(encode_ldr_str(&ops, is_load, size, is_signed, false));

        prop_assert_eq!( w        & 0x1F, rt_num,                  "Rt field");
        prop_assert_eq!((w >> 5)  & 0x1F, base_num,                "Rn field");
        prop_assert_eq!((w >> 10) & 0xFFF, imm12,                  "imm12 field");
        prop_assert_eq!((w >> 22) & 0x3,  expected_opc_gp(is_load, is_signed), "opc field");
        prop_assert_eq!((w >> 26) & 0x1,  0u32,                    "V=0 for GP");
        prop_assert_eq!((w >> 30) & 0x3,  size,                    "size field");
        prop_assert_eq!((w >> 24) & 0x3,  0b01u32,                 "[25:24]=01 unsigned-offset form");
    }

    // ── Property 2: pre/post-index field placement (reference). ──
    // For in-range offsets the imm9 field round-trips and the Rt/Rn fields
    // and discriminator bits [11:10] are placed correctly. PASSES today.
    #[test]
    fn prop_pre_post_index_fields_round_trip(
        size in 2u32..=3u32,
        is_load in any::<bool>(),
        is_signed in any::<bool>(),
        rt_num in 0u32..=31u32,
        base_num in 0u32..=31u32,
        off in -256i64..=255i64,
        is_pre in any::<bool>(),
    ) {
        let mem = if is_pre {
            Operand::MemPreIndex { base: format!("x{}", base_num), offset: off }
        } else {
            Operand::MemPostIndex { base: format!("x{}", base_num), offset: off }
        };
        let ops = vec![gp_reg(if size == 3 { 'x' } else { 'w' }, rt_num), mem];
        let w = word(encode_ldr_str(&ops, is_load, size, is_signed, false));

        prop_assert_eq!( w       & 0x1F, rt_num,                   "Rt field");
        prop_assert_eq!((w >> 5) & 0x1F, base_num,                 "Rn field");
        prop_assert_eq!(decode_imm9(w) as i64, off,                "imm9 round-trips");
        prop_assert_eq!((w >> 10) & 0x3, if is_pre { 0b11u32 } else { 0b01u32 },
            "[11:10] discriminator (11=pre, 01=post)");
        prop_assert_eq!((w >> 24) & 0x3, 0b00u32,                  "[25:24]=00 for pre/post");
    }

    // ── Property 3: unscaled (LDUR) fallback round-trips in-range offsets.──
    // A negative offset cannot use the unsigned form, so it must take the
    // LDUR fallback. For offsets in [-256, -1] the imm9 field must
    // round-trip. PASSES today.
    #[test]
    fn prop_ldur_fallback_imm9_round_trips(
        size in 2u32..=3u32,
        is_load in any::<bool>(),
        is_signed in any::<bool>(),
        off in -256i64..=-1i64,
    ) {
        let ops = vec![
            gp_reg(if size == 3 { 'x' } else { 'w' }, 0),
            Operand::Mem { base: "x1".to_string(), offset: off },
        ];
        let w = word(encode_ldr_str(&ops, is_load, size, is_signed, false));
        prop_assert_eq!(decode_imm9(w) as i64, off,
            "in-range negative offset must round-trip via the LDUR fallback");
        // LDUR discriminator: [25:24]=00, [11:10]=00
        prop_assert_eq!((w >> 24) & 0x3, 0b00u32);
        prop_assert_eq!((w >> 10) & 0x3, 0b00u32);
    }

    // ── Property 4: NEGATIVE CONTRACT — out-of-range imm9 must be Err. ──
    // The imm9 field is a signed 9-bit field (range [-256, 255]). For the
    // pre-index, post-index, and LDUR-fallback (Mem) forms, any offset
    // outside that range is unrepresentable and MUST be rejected rather than
    // silently masked with `& 0x1FF`.
    //
    // EXPECTED TO FAIL today: the encoder masks and returns Ok. This
    // failure IS the bug being reported.
    #[test]
    #[ignore = "documented bug: ldr/str imm9 offsets outside [-256,255] are masked, not rejected"]
    fn prop_out_of_range_imm9_rejected(
        size in 2u32..=3u32,
        is_load in any::<bool>(),
        is_signed in any::<bool>(),
        form in 0u32..3u32,            // 0=pre, 1=post, 2=Mem(LDUR fallback)
        off in (-4096i64..4096i64).prop_filter(
            "out of signed-9-bit range", |o| *o < -256 || *o > 255),
    ) {
        // For the Mem form, force the LDUR fallback: a negative offset
        // always bypasses the unsigned form (which requires offset >= 0).
        let mem = match form {
            0 => Operand::MemPreIndex  { base: "x1".to_string(), offset: off },
            1 => Operand::MemPostIndex { base: "x1".to_string(), offset: off },
            _ => Operand::Mem { base: "x1".to_string(),
                                offset: if off < 0 { off } else { -(off.abs()) } },
        };
        let ops = vec![gp_reg(if size == 3 { 'x' } else { 'w' }, 0), mem];
        let res = encode_ldr_str(&ops, is_load, size, is_signed, false);
        prop_assert!(res.is_err(),
            "offset {} is outside the signed 9-bit range [-256,255] and must be \
             rejected; got {:?}", off, res);
    }

    // ── Property 5: out-of-range offset is silently corrupted (mechanism).──
    // Documents HOW the bug manifests: for an out-of-range negative offset
    // reaching the LDUR fallback, only the low 9 bits survive, so the
    // decoder cannot recover the original value. PASSES today — it is the
    // smoking gun for the masking.
    #[test]
    fn prop_out_of_range_offset_silently_corrupted(
        size in 2u32..=3u32,
        is_load in any::<bool>(),
        is_signed in any::<bool>(),
        off in (-4096i64..-257i64),
    ) {
        let ops = vec![
            gp_reg(if size == 3 { 'x' } else { 'w' }, 0),
            Operand::Mem { base: "x1".to_string(), offset: off },
        ];
        let w = word(encode_ldr_str(&ops, is_load, size, is_signed, false));
        prop_assert_ne!(decode_imm9(w) as i64, off,
            "offset {} was silently truncated into the 9-bit imm9 field", off);
    }

    // ── Property 6: NEGATIVE CONTRACT — common programmer offsets. ──
    // Concrete regression anchors for the offsets a human is likely to write
    // on a pre/post-indexed or unscaled load/store (positive, negative,
    // page-aligned). All are outside the signed 9-bit range and MUST be
    // rejected. EXPECTED TO FAIL today.
    #[test]
    #[ignore = "documented bug: common out-of-range ldr/str imm9 offsets are masked"]
    fn prop_common_offsets_rejected(
        size in 2u32..=3u32,
        is_load in any::<bool>(),
        form in 0u32..3u32,
        off_idx in 0usize..8usize,
    ) {
        let offsets = [256i64, 257, 512, 1000, 4096, -257, -258, -512];
        let off = offsets[off_idx];
        let mem = match form {
            0 => Operand::MemPreIndex  { base: "x1".to_string(), offset: off },
            1 => Operand::MemPostIndex { base: "x1".to_string(), offset: off },
            _ => Operand::Mem { base: "x1".to_string(),
                                offset: if off < 0 { off } else { -off } },
        };
        let ops = vec![gp_reg(if size == 3 { 'x' } else { 'w' }, 0), mem];
        let res = encode_ldr_str(&ops, is_load, size, false, false);
        prop_assert!(res.is_err(),
            "ldr/str offset {} is outside [-256,255] and must be Err; got {:?}",
            off, res);
    }

    // ── Property 7: too-large aligned offset also escapes the field. ──
    // An aligned positive offset whose imm12 >= 4096 (e.g. 4096*8 for X
    // registers) does NOT fit the unsigned form and falls through to LDUR,
    // where it is masked. The decoder recovers neither the unsigned nor a
    // sane signed value — so it must be Err. EXPECTED TO FAIL today.
    #[test]
    #[ignore = "documented bug: oversized aligned ldr/str offset masked in LDUR fallback"]
    fn prop_oversized_aligned_offset_rejected(
        size in 2u32..=3u32,
        is_load in any::<bool>(),
        imm12_overflow in 1u32..=16u32,
    ) {
        let shift = size;
        let offset = ((4096u64 + imm12_overflow as u64) << shift) as i64;
        let ops = vec![
            gp_reg(if size == 3 { 'x' } else { 'w' }, 0),
            Operand::Mem { base: "x1".to_string(), offset },
        ];
        let res = encode_ldr_str(&ops, is_load, size, false, false);
        prop_assert!(res.is_err(),
            "offset {} exceeds the 12-bit unsigned-offset field and is outside \
             the LDUR signed-9-bit range; must be Err; got {:?}",
            offset, res);
    }

    // ── Property 8: LDR-literal only valid for loads; STR rejects it. ──
    // `ldr Rt, label` is a PC-relative load; there is no store-literal form,
    // so `str Rt, label` must be rejected. PASSES today.
    #[test]
    fn prop_str_literal_rejected(sym_idx in 0usize..4usize) {
        let syms = ["foo", "bar", ".L0", "data"];
        let sym = syms[sym_idx].to_string();
        let ops = vec![Operand::Reg("x0".to_string()), Operand::Symbol(sym.clone())];
        let res = encode_ldr_str(&ops, false /*store*/, 0b11, false, false);
        prop_assert!(res.is_err(),
            "str Rt, <symbol> has no encoding (no store-literal form); got {:?}", res);
    }

    // ── Property 9: LDR-literal emits the Ldr19 relocation. ──
    // `ldr Rt, label` must produce a WordWithReloc carrying Ldr19 so the
    // linker can patch the 19-bit PC-relative immediate. PASSES today.
    #[test]
    fn prop_ldr_literal_emits_ldr19_reloc(
        is_x in any::<bool>(),
        rt_num in 0u32..=31u32,
    ) {
        let ops = vec![
            Operand::Reg(format!("{}{}", if is_x { 'x' } else { 'w' }, rt_num)),
            Operand::Symbol("target".to_string()),
        ];
        match encode_ldr_str(&ops, true, if is_x { 0b11 } else { 0b10 }, false, false) {
            Ok(EncodeResult::WordWithReloc { word, reloc }) => {
                prop_assert!(matches!(reloc.reloc_type, RelocType::Ldr19),
                    "expected Ldr19 relocation, got {:?}", reloc.reloc_type);
                prop_assert_eq!(reloc.symbol, "target");
                prop_assert_eq!(reloc.addend, 0);
                // opc[31:30] 011[29:27] V=0[26] Rt[4:0]; imm19 left zero for linker.
                let expected_opc: u32 = if is_x { 0b01 } else { 0b00 };
                prop_assert_eq!((word >> 30) & 0x3, expected_opc, "opc field");
                prop_assert_eq!((word >> 27) & 0x7, 0b011u32,    "011 literal discriminator");
                prop_assert_eq!((word >> 26) & 0x1, 0u32,        "V=0 for GP");
                prop_assert_eq!( word       & 0x1F, rt_num,      "Rt field");
                prop_assert_eq!((word >> 5) & 0x7FFFF, 0u32,     "imm19 zero before relocation");
            }
            other => prop_assert!(false, "expected WordWithReloc, got {:?}", other),
        }
    }
}
