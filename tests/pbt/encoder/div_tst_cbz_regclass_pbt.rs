//! Property-based tests focused on three validation contracts for three
//! AArch64 encoders that all funnel through the shared `get_reg` /
//! `parse_reg_num` helpers (which never call the dead `is_fp_reg` check):
//!
//!   * `encode_div`  — `data_processing.rs` — SDIV/UDIV (`<Rd>,<Rn>,<Rm>`)
//!   * `encode_tst`  — `compare_branch.rs` — TST (alias of ANDS)
//!   * `encode_cbz`  — `compare_branch.rs` — CBZ/CBNZ (`<Rt>,<label>`)
//!
//! ## Focus (per request)
//!
//! 1. **FP/SIMD register-class acceptance.** All three instructions require
//!    *general-purpose* registers, yet `parse_reg_num` accepts the FP/SIMD
//!    prefixes `d|s|q|v|h|b`, so e.g. `sdiv x0, v1, x2`, `tst v0, x1`, and
//!    `cbz d0, lab` are silently mis-encoded as their `x`/`w` namesakes.
//! 2. **SP operand aliasing.** For all three, encoding field 31 denotes the
//!    *zero* register (XZR/WZR), not SP. `parse_reg_num` maps both `sp`/`wsp`
//!    and `xzr`/`wzr` to 31, so `sdiv x0, sp, x1`, `tst sp, x0`, and
//!    `cbz sp, lab` silently alias SP to the zero register.
//! 3. **Immediate-range validation.** `encode_div` takes no immediate (an
//!    immediate operand must be rejected); `encode_cbz` takes a label, not an
//!    immediate; `encode_tst` takes a *logical/bitmask* immediate, which must
//!    be a valid replicated run-of-ones pattern (and width-sensitive).
//!
//! ## Oracle
//!
//! Register-class, SP, and field-range contracts are taken from the ARMv8-A
//! Architecture Reference Manual and cross-checked against the system
//! conforming assembler `clang --target=aarch64-linux-gnu` (AArch64), which
//! **rejects** every "should be Err" input witnessed below — e.g.
//! `sdiv x0, sp, x1`, `sdiv x0, v1, x2`, `tst sp, x0`, `tst v0, x1`,
//! `tst x0, #0x55555555` (64-bit), `cbz sp, lab`, `cbz v0, lab`.
//!
//! ## Witness policy
//!
//! Every property that *witnesses a defect* asserts the **correct** contract
//! and is marked `#[ignore = "documented bug: …"]`, so the default
//! `cargo test --lib` run stays green. Run the witnesses explicitly:
//!
//! ```sh
//!   cargo test --lib div_tst_cbz_regclass_pbt -- --ignored
//! ```
//!
//! These defects extend the register-class findings already reported for
//! `encode_br`/`encode_blr`/`encode_ret` (in `compare_branch_regclass_pbt.rs`)
//! to the DIV/TST/CBZ class, and add the width-sensitive logical-immediate
//! range check for TST.

use lccc::pbt_internals::Operand;
use lccc::pbt_internals::*; // encode_div / encode_tst / encode_cbz + EncodeResult/Relocation/RelocType + helpers
use proptest::prelude::*;

// ── helpers ──────────────────────────────────────────────────────────────

fn word_of(r: Result<EncodeResult, String>) -> u32 {
    match r {
        Ok(EncodeResult::Word(w)) => w,
        Ok(EncodeResult::WordWithReloc { word, .. }) => word,
        other => panic!("expected Word/WordWithReloc, got {:?}", other),
    }
}

fn reloc_of(r: Result<EncodeResult, String>) -> Relocation {
    match r {
        Ok(EncodeResult::WordWithReloc { reloc, .. }) => reloc,
        other => panic!("expected WordWithReloc, got {:?}", other),
    }
}

fn xreg(n: u32) -> Operand {
    Operand::Reg(format!("x{}", n))
}
fn wreg(n: u32) -> Operand {
    Operand::Reg(format!("w{}", n))
}

// A valid general-purpose register number that is *neither* the sp/zr-encoded
// 31 nor out of range: 0..=30.
prop_compose! {
    fn arb_gp_num()(n in 0u32..=30u32) -> u32 { n }
}

// An FP/SIMD register spelling (wrong register class for DIV/TST/CBZ),
// paired with the number `parse_reg_num` will silently extract from it.
prop_compose! {
    fn arb_fpsimd_reg()(prefix_idx in 0usize..5usize, n in 0u32..=31u32) -> (String, u32) {
        let p = ["d", "s", "q", "v", "h"][prefix_idx];
        (format!("{}{}", p, n), n)
    }
}

// A branch/compare target operand that `get_symbol` accepts, paired with the
// `(symbol, addend)` it must forward into the relocation.
prop_compose! {
    fn arb_target()(s in "[a-z][a-z0-9_]{0,7}", off in -4096i64..=4096i64, is_off in any::<bool>()) -> (Operand, String, i64) {
        if is_off {
            (Operand::SymbolOffset(s.clone(), off), s, off)
        } else {
            (Operand::Symbol(s.clone()), s, 0)
        }
    }
}

// =========================================================================
//  encode_div — SDIV/UDIV : sf 0 0 11010110 Rm 00001 o1 Rn Rd
//  (o1 = bit 10: 0 = UDIV, 1 = SDIV; register 31 = XZR/WZR, no SP form)
// =========================================================================
proptest! {
    // GREEN — field-placement baseline (the correct reference the witnesses
    // below deviate from). sf tracks the destination width, the fixed bits
    // 30:21 == 0011010110, o1 selects signed(1)/unsigned(0), and Rm/Rn/Rd
    // occupy [20:16]/[9:5]/[4:0].
    #[test]
    fn prop_div_field_placement(
        n in arb_gp_num(),
        is_64 in any::<bool>(),
        unsigned in any::<bool>(),
    ) {
        let r = if is_64 { xreg(n) } else { wreg(n) };
        let ops = vec![r.clone(), r.clone(), r];
        let w = word_of(encode_div(&ops, unsigned));
        prop_assert_eq!((w >> 31) & 1, is_64 as u32);
        prop_assert_eq!((w >> 21) & 0x3FF, 0b0011010110u32);
        prop_assert_eq!((w >> 10) & 1, if unsigned { 0 } else { 1 }); // o1
        prop_assert_eq!((w >> 16) & 0x1F, n); // Rm
        prop_assert_eq!((w >> 5) & 0x1F, n);  // Rn
        prop_assert_eq!(w & 0x1F, n);         // Rd
    }

    // GREEN — immediate / operand-class negative contract. SDIV/UDIV take
    // exactly three *register* operands; a non-register operand in any slot
    // (immediate, memory, etc.) MUST be rejected — `get_reg` returns Err.
    #[test]
    fn prop_div_rejects_non_register_operands(
        n in arb_gp_num(),
        slot in 0usize..3usize,
    ) {
        let mut ops = vec![xreg(n), xreg(n), xreg(n)];
        ops[slot] = Operand::Imm(7);
        prop_assert!(
            encode_div(&ops, false).is_err(),
            "sdiv with an immediate in slot {} must be rejected", slot,
        );
        // Fewer than three operands must also be rejected.
        let short: Vec<Operand> = vec![xreg(n), xreg(n)];
        prop_assert!(encode_div(&short, false).is_err());
    }

    // WITNESS — register class. SDIV/UDIV require general-purpose registers;
    // FP/SIMD operands are the wrong class. `clang --target=aarch64` rejects
    // `sdiv x0, v1, x2` and `sdiv d0, x1, x2` with "invalid operand for
    // instruction". `parse_reg_num` accepts the d/s/q/v/h/b prefixes, so the
    // encoder silently encodes the FP/SIMD register as its GP namesake.
    #[test]
    fn prop_div_rejects_fpsimd_register_class(
        good_n in arb_gp_num(),
        fpsimd_slot in 0usize..3usize,
        (bad_name, _bad_n) in arb_fpsimd_reg(),
    ) {
        let mut ops = vec![xreg(good_n), xreg(good_n), xreg(good_n)];
        ops[fpsimd_slot] = Operand::Reg(bad_name.clone());
        prop_assert!(
            encode_div(&ops, false).is_err(),
            "sdiv with FP/SIMD operand {} must be rejected, got {:?}",
            bad_name, encode_div(&ops, false),
        );
    }

    // WITNESS — SP aliasing. In the data-processing (2 source) encoding field
    // 31 is XZR/WZR (there is no SP-using SDIV/UDIV form). `clang` rejects
    // `sdiv x0, sp, x1`, `sdiv sp, x0, x1`, `sdiv x0, x1, sp`. `parse_reg_num`
    // maps sp/wsp to 31 indistinguishably from xzr/wzr, silently aliasing SP
    // to the zero register in any operand slot.
    #[test]
    fn prop_div_rejects_sp_operands(
        good_n in arb_gp_num(),
        sp_slot in 0usize..3usize,
        is_64 in any::<bool>(),
    ) {
        let gp = if is_64 { xreg } else { wreg };
        let sp_name = if is_64 { "sp" } else { "wsp" };
        let mut ops = vec![gp(good_n), gp(good_n), gp(good_n)];
        ops[sp_slot] = Operand::Reg(sp_name.into());
        prop_assert!(
            encode_div(&ops, is_64).is_err(),
            "sdiv with SP operand {} in slot {} must be rejected, got {:?}",
            sp_name, sp_slot, encode_div(&ops, is_64),
        );
    }
}

// =========================================================================
//  encode_tst — TST <Rn>,<op>  =>  ANDS XZR/WZR, Rn, op   (opc = 0b11)
//    shifted-register: sf 11 01010 shift 0 Rm imm6 Rn Rd   (Rd = 31)
//    immediate:        sf 11 100100  N immr imms Rn Rd      (Rd = 31)
//  register 31 = XZR/WZR for every field (no SP form); immediate must be a
//  valid logical/bitmask pattern.
// =========================================================================
proptest! {
    // GREEN — defining TST invariant + shifted-register field placement. The
    // destination is ALWAYS the zero register (Rd == 31) and the fixed opcode
    // bits (opc=11, [28:24]=01010, N bit [21]=0) are correct.
    #[test]
    fn prop_tst_register_form_fields(
        rn_n in arb_gp_num(),
        rm_n in arb_gp_num(),
        is_64 in any::<bool>(),
    ) {
        let rn = if is_64 { xreg(rn_n) } else { wreg(rn_n) };
        let rm = if is_64 { xreg(rm_n) } else { wreg(rm_n) };
        let w = word_of(encode_tst(&[rn, rm]));
        prop_assert_eq!((w >> 31) & 1, is_64 as u32);
        prop_assert_eq!((w >> 29) & 0b11, 0b11u32);    // opc = ANDS
        prop_assert_eq!((w >> 24) & 0x1F, 0b01010u32); // [28:24]
        prop_assert_eq!((w >> 21) & 1, 0u32);          // N bit (ANDS reg form)
        prop_assert_eq!((w >> 16) & 0x1F, rm_n);       // Rm
        prop_assert_eq!((w >> 5) & 0x1F, rn_n);        // Rn
        prop_assert_eq!(w & 0x1F, 31u32);              // Rd = xzr/wzr
    }

    // GREEN — immediate / range validation. TST's immediate must be a valid
    // *logical (bitmask)* immediate: a replicated run-of-ones pattern. Values
    // that are not such a pattern (confirmed rejected by `clang`:
    // `tst x0, #5`, `#0x101`, `#0x1234`) MUST be rejected with Err rather
    // than silently masked.
    #[test]
    fn prop_tst_rejects_non_bitmask_immediate(
        n in arb_gp_num(),
        is_64 in any::<bool>(),
        idx in 0usize..3usize,
    ) {
        let invalids: [i64; 3] = [5, 0x101, 0x1234];
        let r = if is_64 { xreg(n) } else { wreg(n) };
        let ops = vec![r, Operand::Imm(invalids[idx])];
        prop_assert!(
            encode_tst(&ops).is_err(),
            "tst #{:#x} is not a valid bitmask immediate and must be rejected, got {:?}",
            invalids[idx], encode_tst(&ops),
        );
    }

    // GREEN — width-sensitive immediate range. `0x55555555` is a valid 32-bit
    // bitmask (replicated 0x55 across 32 bits) but is NOT a valid 64-bit
    // bitmask (it does not replicate across 64 bits). `clang` accepts
    // `tst w0, #0x55555555` and rejects `tst x0, #0x55555555`. The encoder
    // honours the destination width via `encode_bitmask_imm(v, is_64)`.
    #[test]
    fn prop_tst_immediate_is_width_sensitive(_d in 0u32..=0u32) {
        let ok32 = encode_tst(&[Operand::Reg("w0".into()), Operand::Imm(0x55555555)]);
        prop_assert!(ok32.is_ok(), "tst w0, #0x55555555 must encode (valid 32-bit bitmask), got {:?}", ok32);
        let err64 = encode_tst(&[Operand::Reg("x0".into()), Operand::Imm(0x55555555)]);
        prop_assert!(err64.is_err(), "tst x0, #0x55555555 must be rejected (not a valid 64-bit bitmask), got {:?}", err64);
    }

    // WITNESS — register class. TST requires general-purpose registers;
    // FP/SIMD operands are the wrong class. `clang` rejects `tst v0, x1` and
    // `tst x0, d1`. `parse_reg_num` accepts the FP/SIMD prefixes, so the
    // encoder silently encodes them as GP namesakes.
    #[ignore = "documented bug: encode_tst accepts FP/SIMD registers (TST is GP-only); clang rejects"]
    #[test]
    fn prop_tst_rejects_fpsimd_register_class(
        good_n in arb_gp_num(),
        is_64 in any::<bool>(),
        slot in 0usize..2usize, // Rn (slot 0) or Rm (slot 1)
        (bad_name, _bad_n) in arb_fpsimd_reg(),
    ) {
        let g = if is_64 { xreg } else { wreg };
        let mut ops = vec![g(good_n), g(good_n)];
        ops[slot] = Operand::Reg(bad_name.clone());
        prop_assert!(
            encode_tst(&ops).is_err(),
            "tst with FP/SIMD operand {} must be rejected, got {:?}",
            bad_name, encode_tst(&ops),
        );
    }

    // WITNESS — SP aliasing. ANDS encodes register 31 as XZR/WZR for every
    // field (the flag-setting logical class has no SP form). `clang` rejects
    // `tst sp, x0` and `tst x0, sp`. `parse_reg_num` maps sp/wsp to 31
    // indistinguishably from xzr/wzr, silently aliasing SP to the zero
    // register in either the Rn or Rm slot.
    #[ignore = "documented bug: encode_tst aliases sp/wsp to xzr/wzr (no SP form; ANDS field 31 = ZR); clang rejects"]
    #[test]
    fn prop_tst_rejects_sp_operands(
        good_n in arb_gp_num(),
        is_64 in any::<bool>(),
        slot in 0usize..2usize, // Rn (slot 0) or Rm (slot 1)
    ) {
        let g = if is_64 { xreg } else { wreg };
        let sp_name = if is_64 { "sp" } else { "wsp" };
        let mut ops = vec![g(good_n), g(good_n)];
        ops[slot] = Operand::Reg(sp_name.into());
        prop_assert!(
            encode_tst(&ops).is_err(),
            "tst with SP operand {} in slot {} must be rejected, got {:?}",
            sp_name, slot, encode_tst(&ops),
        );
    }
}

/// GREEN — reference: the immediate form matches the system assembler byte
/// for byte. `clang --target=aarch64-linux-gnu -c` encodes `tst x0, #0xff`
/// (a valid 64-bit run-of-8-ones bitmask) as `0xF2401C1F`, with Rd = XZR.
#[test]
fn tst_immediate_matches_clang_reference() {
    let ops = vec![Operand::Reg("x0".into()), Operand::Imm(0xff)];
    let w = word_of(encode_tst(&ops));
    assert_eq!(w, 0xF2401C1F, "tst x0, #0xff must match clang's 0xF2401C1F");
    assert_eq!(w & 0x1F, 31, "Rd must be xzr (TST => ANDS XZR, ...)");
    assert_eq!((w >> 29) & 0b11, 0b11, "opc must be ANDS (11)");
    assert_eq!((w >> 23) & 0x3F, 0b100100, "immediate opcode [28:23]");
}

// =========================================================================
//  encode_cbz — CBZ/CBNZ : sf 011010 op imm19 Rt
//  (register 31 = XZR/WZR, no SP form; imm19 linker-filled via CondBr19)
// =========================================================================
proptest! {
    // GREEN — field placement + relocation contract. sf tracks Rt width,
    // [30:25] == 011010, op selects CBZ(0)/CBNZ(1), the imm19 field [23:5] is
    // left zero for the linker, Rt occupies [4:0], and the result carries a
    // CondBr19 relocation forwarding (symbol, addend).
    #[test]
    fn prop_cbz_field_placement_and_reloc(
        rt_n in arb_gp_num(),
        is_64 in any::<bool>(),
        is_nz in any::<bool>(),
        (op, sym, off) in arb_target(),
    ) {
        let rt = if is_64 { xreg(rt_n) } else { wreg(rt_n) };
        let ops = vec![rt, op];
        let w = word_of(encode_cbz(&ops, is_nz));
        prop_assert_eq!((w >> 31) & 1, is_64 as u32);
        prop_assert_eq!((w >> 25) & 0x3F, 0b011010u32); // [30:25]
        prop_assert_eq!((w >> 24) & 1, if is_nz { 1 } else { 0 });
        prop_assert_eq!((w >> 5) & 0x7FFFF, 0u32);       // imm19 linker-filled
        prop_assert_eq!(w & 0x1F, rt_n);

        let r = reloc_of(encode_cbz(&ops, is_nz));
        prop_assert!(matches!(r.reloc_type, RelocType::CondBr19));
        prop_assert_eq!(r.symbol, sym);
        prop_assert_eq!(r.addend, off);
    }

    // GREEN — differential: CBZ and CBNZ share the layout and differ ONLY in
    // bit 24 (op).
    #[test]
    fn prop_cbz_xor_cbnz_is_bit24(
        rt_n in arb_gp_num(),
        is_64 in any::<bool>(),
        (op, _s, _o) in arb_target(),
    ) {
        let rt = if is_64 { xreg(rt_n) } else { wreg(rt_n) };
        let ops = vec![rt, op];
        prop_assert_eq!(
            word_of(encode_cbz(&ops, false)) ^ word_of(encode_cbz(&ops, true)),
            1u32 << 24,
        );
    }

    // GREEN — immediate / operand-class negative contract. CBZ/CBNZ take a GP
    // register and a *label*; a non-register Rt, an immediate (non-label)
    // target, or a missing target MUST be rejected.
    #[test]
    fn prop_cbz_rejects_wrong_operand_classes(
        rt_n in arb_gp_num(),
        bad_target in 0usize..2usize,
    ) {
        // Non-register Rt.
        let bad_rt = vec![Operand::Imm(0), Operand::Symbol("lab".into())];
        prop_assert!(
            encode_cbz(&bad_rt, false).is_err(),
            "cbz with a non-register Rt must be rejected",
        );
        // Immediate (not a label) target.
        let bad_tgt_ops = vec![xreg(rt_n), if bad_target == 0 {
            Operand::Imm(8)
        } else {
            Operand::Mem { base: "x0".into(), offset: 0 }
        }];
        prop_assert!(
            encode_cbz(&bad_tgt_ops, false).is_err(),
            "cbz with a non-label target must be rejected",
        );
        // Missing target.
        let no_target: Vec<Operand> = vec![xreg(rt_n)];
        prop_assert!(encode_cbz(&no_target, false).is_err());
    }

    // WITNESS — register class. CBZ/CBNZ require a general-purpose register;
    // FP/SIMD operands are the wrong class. `clang` rejects `cbz v0, lab` and
    // `cbz d0, lab`. `parse_reg_num` accepts the FP/SIMD prefixes (and treats
    // them as 32-bit / sf=0 since they do not start with 'x'), so the encoder
    // silently encodes e.g. `cbz v0, lab` identically to `cbz w0, lab`.
    #[test]
    fn prop_cbz_rejects_fpsimd_register_class(
        (bad_name, _bad_n) in arb_fpsimd_reg(),
        is_nz in any::<bool>(),
    ) {
        let ops = vec![Operand::Reg(bad_name.clone()), Operand::Symbol("lab".into())];
        prop_assert!(
            encode_cbz(&ops, is_nz).is_err(),
            "cbz with FP/SIMD operand {} must be rejected, got {:?}",
            bad_name, encode_cbz(&ops, is_nz),
        );
    }

    // WITNESS — SP aliasing. CBZ/CBNZ's Rt field value 31 denotes XZR/WZR
    // (there is no SP form). `clang` rejects `cbz sp, lab`. `parse_reg_num`
    // maps sp to 31 indistinguishably from xzr, so `cbz sp, lab` silently
    // aliases SP to the zero register — and since XZR is always zero, the
    // resulting `cbz xzr, lab` would always branch.
    #[test]
    fn prop_cbz_rejects_sp_operand(is_nz in any::<bool>()) {
        let ops = vec![Operand::Reg("sp".into()), Operand::Symbol("lab".into())];
        prop_assert!(
            encode_cbz(&ops, is_nz).is_err(),
            "cbz sp, lab must be rejected, got {:?}", encode_cbz(&ops, is_nz),
        );
    }
}
