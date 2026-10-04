//! Property-based tests focused on **register-class validation** and
//! **immediate / field-range validation** for the five branch encoders in
//! `compare_branch.rs`:
//!
//!   * `encode_br`, `encode_blr`, `encode_ret` — register operand `<Xn>`
//!   * `encode_bl`, `encode_branch`           — symbolic branch/call target
//!
//! ## Oracle
//!
//! Register-class and field-range contracts are taken from the ARMv8-A
//! Architecture Reference Manual and cross-checked against the system
//! conforming assembler `llvm-mc-18` (AArch64), which rejects every
//! "should be Err" input witnessed below (e.g. `br w0`, `br sp`, `br v0`,
//! `blr w0`, `ret sp`, `b x0`, `bl x0`). `br/blr/ret xN` for `N` in 0..=30,
//! `br xzr`, `ret`, `ret x30`/`ret lr`, and `b/bl <label>` are all accepted
//! by `llvm-mc` and serve as the positive domain.
//!
//! ## Bug witnesses
//!
//! Every property that *witnesses a defect* is marked `#[ignore]` so the
//! default `cargo test` stays green. Each ignore string points at the
//! per-function bug. Run the witnesses
//! explicitly with:
//!
//! ```sh
//! cargo test --test pbt_arm_core compare_branch_regclass_pbt -- --ignored
//! ```
//!
//! These defects are already reported per-function; this file adds the
//! stand-alone, default-green, register-class + field-range suite requested
//! for `encode_br` / `encode_blr` / `encode_ret` / `encode_bl` /
//! `encode_branch`.

use lccc::pbt_internals::Operand;
use lccc::pbt_internals::*; // encode_br/blr/ret/bl/branch + EncodeResult/Relocation/RelocType + helpers
use proptest::prelude::*;

// ── helpers ──────────────────────────────────────────────────────────────

fn word(r: Result<EncodeResult, String>) -> u32 {
    match r {
        Ok(EncodeResult::Word(w)) => w,
        Ok(EncodeResult::WordWithReloc { word, .. }) => word,
        other => panic!("expected Word/WordWithReloc, got {:?}", other),
    }
}

fn reloc(r: Result<EncodeResult, String>) -> Relocation {
    match r {
        Ok(EncodeResult::WordWithReloc { reloc, .. }) => reloc,
        other => panic!("expected WordWithReloc, got {:?}", other),
    }
}

const BR_OPCODE: u32 = 0xD61F_0000;
const BLR_OPCODE: u32 = 0xD63F_0000;
const RET_OPCODE: u32 = 0xD65F_0000;
const B_OPCODE: u32 = 0b000101u32 << 26; // 0x1400_0000
const BL_OPCODE: u32 = 0b100101u32 << 26; // 0x9400_0000
const RN_MASK: u32 = 0x1Fu32 << 5; // bits [9:5]

// A valid general-purpose register number that is *neither* the
// `xzr`/`sp`-encoded 31 nor out of range: 0..=30. (`x31` is the `xzr`
// alias and is handled separately.)
prop_compose! {
    fn arb_gp_reg_num()(n in 0u32..=30u32) -> u32 { n }
}

// An FP/SIMD register spelling (wrong register class for BR/BLR/RET).
prop_compose! {
    fn arb_fpsimd_reg()(prefix_idx in 0usize..6usize, n in 0u32..=31u32) -> (String, u32) {
        let p = ["d", "s", "q", "v", "h", "b"][prefix_idx];
        (format!("{}{}", p, n), n)
    }
}

// A branch/call target operand that `get_symbol` accepts, paired with the
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

// ── encode_br : `BR <Xn>` (ARM ARM C5.6.17) ──────────────────────────────

proptest! {
    // Positive + field range: every in-range GP register x0..x30 encodes to
    // the fixed BR opcode with Rn in [9:5] and reserved bits zero.
    #[test]
    fn prop_br_in_range_gp_reg_encodes_rn(n in arb_gp_reg_num()) {
        let ops = vec![Operand::Reg(format!("x{}", n))];
        let w = word(encode_br(&ops));
        prop_assert_eq!(w & !RN_MASK, BR_OPCODE);
        prop_assert_eq!((w >> 5) & 0x1F, n);
        prop_assert_eq!(w, BR_OPCODE | (n << 5));
    }

    // Field-range validation: the Rn field is a 5-bit unsigned value (0..=31).
    // A register number above 31 MUST be rejected, not silently masked by
    // `rn << 5`. (Correctly enforced today via parse_reg_num.)
    #[test]
    fn prop_br_rejects_regnum_above_31(n in 32u32..=999u32, is_64 in any::<bool>()) {
        let name = if is_64 { format!("x{}", n) } else { format!("w{}", n) };
        prop_assert!(
            encode_br(&[Operand::Reg(name.clone())]).is_err(),
            "br {} (Rn field is 0..=31) must be rejected", name
        );
    }

    // WITNESS — register class. BR's sole operand is `<Xn>`; the 32-bit W
    // form is unallocated. llvm-mc rejects `br w0`. The encoder discards
    // `is_64`, so it accepts `br wN` and encodes it identically to `br xN`.
    #[test]
    fn prop_br_rejects_w_form(n in arb_gp_reg_num()) {
        let res = encode_br(&[Operand::Reg(format!("w{}", n))]);
        prop_assert!(res.is_err(), "br w{} must be rejected, got {:?}", n, res);
    }

    // WITNESS — register class. BR's Rn field value 31 denotes XZR; there is
    // no SP-using form. llvm-mc rejects `br sp`/`br wsp`. The encoder maps
    // sp/wsp to 31, silently aliasing them to xzr/wzr.
    #[test]
    fn prop_br_rejects_sp_wsp(which in 0usize..2usize) {
        let name = if which == 0 { "sp" } else { "wsp" };
        let res = encode_br(&[Operand::Reg(name.into())]);
        prop_assert!(res.is_err(), "br {} must be rejected, got {:?}", name, res);
    }

    // WITNESS — register class. BR requires a GP register; FP/SIMD operands
    // are the wrong class. llvm-mc rejects `br v0`/`br d0`. The encoder
    // accepts them and encodes them as `br xN`.
    #[ignore = "documented bug: encode_br accepts FP/SIMD registers (BR requires a GP register)"]
    #[test]
    fn prop_br_rejects_fpsimd((name, n) in arb_fpsimd_reg()) {
        let res = encode_br(&[Operand::Reg(name.clone())]);
        prop_assert!(res.is_err(), "br {} must be rejected, got {:?}", name, res);
        // Current behavior: silently encodes identically to br x{n}.
        prop_assert_eq!(word(encode_br(&[Operand::Reg(name)])), BR_OPCODE | (n << 5));
    }
}

// ── encode_blr : `BLR <Xn>` (ARM ARM C5.6.18) ────────────────────────────

proptest! {
    // Positive + field range.
    #[test]
    fn prop_blr_in_range_gp_reg_encodes_rn(n in arb_gp_reg_num()) {
        let ops = vec![Operand::Reg(format!("x{}", n))];
        let w = word(encode_blr(&ops));
        prop_assert_eq!(w & !RN_MASK, BLR_OPCODE);
        prop_assert_eq!((w >> 5) & 0x1F, n);
        prop_assert_eq!(w, BLR_OPCODE | (n << 5));
    }

    // Differential: BLR and BR share the operand contract and differ ONLY in
    // bit 21 (the link sub-field). blr^br == 1<<21 for any valid register.
    #[test]
    fn prop_blr_xor_br_is_bit21(n in arb_gp_reg_num()) {
        let ops = vec![Operand::Reg(format!("x{}", n))];
        prop_assert_eq!(word(encode_blr(&ops)) ^ word(encode_br(&ops)), 1u32 << 21);
    }

    // Field-range validation: Rn is 0..=31; num > 31 must be rejected.
    #[test]
    fn prop_blr_rejects_regnum_above_31(n in 32u32..=999u32, is_64 in any::<bool>()) {
        let name = if is_64 { format!("x{}", n) } else { format!("w{}", n) };
        prop_assert!(
            encode_blr(&[Operand::Reg(name.clone())]).is_err(),
            "blr {} (Rn field is 0..=31) must be rejected", name
        );
    }

    // WITNESS — register class (32-bit W form unallocated; llvm-mc rejects
    // `blr w0`).
    #[test]
    fn prop_blr_rejects_w_form(n in arb_gp_reg_num()) {
        let res = encode_blr(&[Operand::Reg(format!("w{}", n))]);
        prop_assert!(res.is_err(), "blr w{} must be rejected, got {:?}", n, res);
    }

    // WITNESS — register class (no SP form; field 31 == XZR).
    #[test]
    fn prop_blr_rejects_sp_wsp(which in 0usize..2usize) {
        let name = if which == 0 { "sp" } else { "wsp" };
        let res = encode_blr(&[Operand::Reg(name.into())]);
        prop_assert!(res.is_err(), "blr {} must be rejected, got {:?}", name, res);
    }

    // WITNESS — register class (BLR requires a GP register).
    #[ignore = "documented bug: encode_blr accepts FP/SIMD registers (BLR requires a GP register)"]
    #[test]
    fn prop_blr_rejects_fpsimd((name, n) in arb_fpsimd_reg()) {
        let res = encode_blr(&[Operand::Reg(name.clone())]);
        prop_assert!(res.is_err(), "blr {} must be rejected, got {:?}", name, res);
        prop_assert_eq!(word(encode_blr(&[Operand::Reg(name)])), BLR_OPCODE | (n << 5));
    }
}

// ── encode_ret : `RET [<Xn>]` (ARM ARM C5.6.20) ──────────────────────────

proptest! {
    // Positive: RET with no operand defaults to X30 (LR).
    #[test]
    fn prop_ret_empty_defaults_to_lr(_d in 0u32..=0u32) {
        let empty: Vec<Operand> = vec![];
        let from_empty = word(encode_ret(&empty));
        let from_x30 = word(encode_ret(&[Operand::Reg("x30".into())]));
        let from_lr = word(encode_ret(&[Operand::Reg("lr".into())]));
        prop_assert_eq!(from_empty, RET_OPCODE | (30u32 << 5));
        prop_assert_eq!(from_empty, from_x30);
        prop_assert_eq!(from_empty, from_lr);
    }

    // Positive + field range.
    #[test]
    fn prop_ret_in_range_gp_reg_encodes_rn(n in arb_gp_reg_num()) {
        let ops = vec![Operand::Reg(format!("x{}", n))];
        let w = word(encode_ret(&ops));
        prop_assert_eq!(w & !RN_MASK, RET_OPCODE);
        prop_assert_eq!((w >> 5) & 0x1F, n);
        prop_assert_eq!(w, RET_OPCODE | (n << 5));
    }

    // Field-range validation: Rn is 0..=31; num > 31 must be rejected.
    #[test]
    fn prop_ret_rejects_regnum_above_31(n in 32u32..=999u32, is_64 in any::<bool>()) {
        let name = if is_64 { format!("x{}", n) } else { format!("w{}", n) };
        prop_assert!(
            encode_ret(&[Operand::Reg(name.clone())]).is_err(),
            "ret {} (Rn field is 0..=31) must be rejected", name
        );
    }

    // WITNESS — register class (32-bit W form unallocated; llvm-mc rejects
    // `ret w0`).
    #[test]
    fn prop_ret_rejects_w_form(n in arb_gp_reg_num()) {
        let res = encode_ret(&[Operand::Reg(format!("w{}", n))]);
        prop_assert!(res.is_err(), "ret w{} must be rejected, got {:?}", n, res);
    }

    // WITNESS — register class (no SP form; field 31 == XZR).
    #[test]
    fn prop_ret_rejects_sp_wsp(which in 0usize..2usize) {
        let name = if which == 0 { "sp" } else { "wsp" };
        let res = encode_ret(&[Operand::Reg(name.into())]);
        prop_assert!(res.is_err(), "ret {} must be rejected, got {:?}", name, res);
    }

    // WITNESS — register class (RET requires a GP register).
    #[ignore = "documented bug: encode_ret accepts FP/SIMD registers (RET requires a GP register)"]
    #[test]
    fn prop_ret_rejects_fpsimd((name, n) in arb_fpsimd_reg()) {
        let res = encode_ret(&[Operand::Reg(name.clone())]);
        prop_assert!(res.is_err(), "ret {} must be rejected, got {:?}", name, res);
        prop_assert_eq!(word(encode_ret(&[Operand::Reg(name)])), RET_OPCODE | (n << 5));
    }
}

// ── encode_bl : `BL <target>` (ARM ARM C5.6.21) ──────────────────────────

proptest! {
    // Positive: a symbol/label target encodes to the BL opcode with the imm26
    // field left zero and a Call26 relocation forwarding (symbol, addend).
    #[test]
    fn prop_bl_symbol_encodes_call26((op, sym, off) in arb_target()) {
        let w = word(encode_bl(std::slice::from_ref(&op)));
        prop_assert_eq!(w, BL_OPCODE); // imm26 left zero for the linker
        let r = reloc(encode_bl(&[op]));
        prop_assert!(matches!(r.reloc_type, RelocType::Call26));
        prop_assert_eq!(r.symbol, sym);
        prop_assert_eq!(r.addend, off);
    }

    // Operand-class validation: operand kinds that are not branch/call targets
    // must be rejected. (The encoder is relocation-only; a raw numeric offset
    // `Operand::Imm` is also rejected — `llvm-mc` accepts `bl #4`, so this is a
    // deliberate scope limit, not a silent mis-encode. The assertion pins that
    // non-target kinds fail safe with Err.)
    #[test]
    fn prop_bl_rejects_non_target_operands(idx in 0usize..9usize) {
        let rejected: Vec<Operand> = vec![
            Operand::Imm(42),
            Operand::Mem { base: "x0".into(), offset: 0 },
            Operand::MemPreIndex { base: "x0".into(), offset: 8 },
            Operand::MemPostIndex { base: "x0".into(), offset: 8 },
            Operand::MemRegOffset { base: "x0".into(), index: "x1".into(), extend: None, shift: None },
            Operand::Shift { kind: "lsl".into(), amount: 2 },
            Operand::Extend { kind: "sxtw".into(), amount: 0 },
            Operand::Expr("a + b".into()),
            Operand::RegList(vec![Operand::Reg("x0".into())]),
        ];
        let op = &rejected[idx];
        prop_assert!(encode_bl(std::slice::from_ref(op)).is_err(), "encode_bl should reject {:?}", op);
        let empty: Vec<Operand> = vec![];
        prop_assert!(encode_bl(&empty).is_err(), "encode_bl needs a target");
    }

    // WITNESS — operand/register class. BL takes a label/symbol target, not a
    // register, condition code, or barrier mnemonic. llvm-mc rejects
    // `bl x0` ("expected label or encodable integer pc offset"). `get_symbol`
    // forwards Reg/Cond/Barrier text as a relocation symbol, so they are
    // accepted here.
    #[ignore = "documented bug: encode_bl forwards Reg/Cond/Barrier operands as call targets"]
    #[test]
    fn prop_bl_rejects_reg_cond_barrier_targets(idx in 0usize..6usize) {
        let invalid = [
            Operand::Reg("x0".into()),
            Operand::Reg("wzr".into()),
            Operand::Cond("eq".into()),
            Operand::Cond("nv".into()),
            Operand::Barrier("sy".into()),
            Operand::Barrier("ish".into()),
        ];
        let op = &invalid[idx];
        let res = encode_bl(std::slice::from_ref(op));
        prop_assert!(res.is_err(), "bl {:?} must be rejected (expected label/symbol), got {:?}", op, res);
    }
}

// ── encode_branch : `B <target>` (ARM ARM C5.6.5) ────────────────────────

proptest! {
    // Positive: a symbol/label target encodes to the B opcode with imm26 left
    // zero and a Jump26 relocation forwarding (symbol, addend).
    #[test]
    fn prop_branch_symbol_encodes_jump26((op, sym, off) in arb_target()) {
        let w = word(encode_branch(std::slice::from_ref(&op)));
        prop_assert_eq!(w, B_OPCODE);
        let r = reloc(encode_branch(&[op]));
        prop_assert!(matches!(r.reloc_type, RelocType::Jump26));
        prop_assert_eq!(r.symbol, sym);
        prop_assert_eq!(r.addend, off);
    }

    // Differential: B and BL share the imm26 layout and differ ONLY in bit 31.
    #[test]
    fn prop_branch_xor_bl_is_bit31((op, _s, _o) in arb_target()) {
        let ops = vec![op];
        prop_assert_eq!(word(encode_branch(&ops)) ^ word(encode_bl(&ops)), 1u32 << 31);
    }

    // Operand-class validation: non-target operand kinds must be rejected
    // (same relocation-only scope limit and fail-safe Err as encode_bl).
    #[test]
    fn prop_branch_rejects_non_target_operands(idx in 0usize..9usize) {
        let rejected: Vec<Operand> = vec![
            Operand::Imm(42),
            Operand::Mem { base: "x0".into(), offset: 0 },
            Operand::MemPreIndex { base: "x0".into(), offset: 8 },
            Operand::MemPostIndex { base: "x0".into(), offset: 8 },
            Operand::MemRegOffset { base: "x0".into(), index: "x1".into(), extend: None, shift: None },
            Operand::Shift { kind: "lsl".into(), amount: 2 },
            Operand::Extend { kind: "sxtw".into(), amount: 0 },
            Operand::Expr("a + b".into()),
            Operand::RegList(vec![Operand::Reg("x0".into())]),
        ];
        let op = &rejected[idx];
        prop_assert!(encode_branch(std::slice::from_ref(op)).is_err(), "encode_branch should reject {:?}", op);
        let empty: Vec<Operand> = vec![];
        prop_assert!(encode_branch(&empty).is_err(), "encode_branch needs a target");
    }

    // WITNESS — operand/register class. B takes a label/symbol target; llvm-mc
    // rejects `b x0` ("expected label or encodable integer pc offset").
    // `get_symbol` forwards Reg/Cond/Barrier text as a relocation symbol.
    #[ignore = "documented bug: encode_branch forwards Reg/Cond/Barrier operands as branch targets"]
    #[test]
    fn prop_branch_rejects_reg_cond_barrier_targets(idx in 0usize..6usize) {
        let invalid = [
            Operand::Reg("x0".into()),
            Operand::Reg("wzr".into()),
            Operand::Cond("eq".into()),
            Operand::Cond("nv".into()),
            Operand::Barrier("sy".into()),
            Operand::Barrier("ish".into()),
        ];
        let op = &invalid[idx];
        let res = encode_branch(std::slice::from_ref(op));
        prop_assert!(res.is_err(), "b {:?} must be rejected (expected label/symbol), got {:?}", op, res);
    }
}
