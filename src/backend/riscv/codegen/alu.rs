//! RiscvCodegen: integer/float arithmetic, unary ops, binop, copy.

use super::emit::RiscvCodegen;
use crate::common::types::IrType;
use crate::ir::reexports::{IrBinOp, IrConst, Operand, Value};

/// The I-type spelling of `lhs op rhs` when `rhs` is a constant the encoding
/// holds: `(mnemonic, immediate)`.
///
/// Each form computes, bit for bit, what the R-type form computes on the
/// value `operand_to_t0` would stage for the constant (`li` of its signed
/// value, so `K` here *is* that 64-bit value):
///
/// * `addi`/`addiw`, `andi`, `ori`, `xori` take a sign-extended 12-bit
///   immediate, exactly the register value when `-2048 <= K <= 2047`;
///   `addiw`, like `addw`, adds the low 32 bits and sign-extends, and the
///   logical forms are 64-bit in both spellings (there is no `andw`);
/// * `x - K` is `x + (-K)` in the ring, so it is `addi[w]` of `-K` when that
///   fits (it does not for `K == -2048`);
/// * `sll`/`srl`/`sra` read the low six bits of the count register and the
///   `w` forms the low five, so the immediate is `K & 63` (`K & 31`) --
///   the same count, including for an out-of-range constant.
///
/// Everything else (multiply, divide, remainder, rotates, the bit
/// operations, a constant that does not fit) keeps the register form.
fn immediate_form(op: IrBinOp, rhs: &Operand, use_32bit: bool) -> Option<(&'static str, i64)> {
    let Operand::Const(c) = rhs else {
        return None;
    };
    let k = match c {
        IrConst::I8(v) => i64::from(*v),
        IrConst::I16(v) => i64::from(*v),
        IrConst::I32(v) => i64::from(*v),
        IrConst::I64(v) => *v,
        IrConst::Zero => 0,
        _ => return None,
    };
    let fits = |v: i64| (-2048..=2047).contains(&v);
    let form = match (op, use_32bit) {
        (IrBinOp::Add, false) if fits(k) => ("addi", k),
        (IrBinOp::Add, true) if fits(k) => ("addiw", k),
        (IrBinOp::Sub, false) if k != i64::MIN && fits(-k) => ("addi", -k),
        (IrBinOp::Sub, true) if k != i64::MIN && fits(-k) => ("addiw", -k),
        (IrBinOp::And, _) if fits(k) => ("andi", k),
        (IrBinOp::Or, _) if fits(k) => ("ori", k),
        (IrBinOp::Xor, _) if fits(k) => ("xori", k),
        (IrBinOp::Shl, false) => ("slli", k & 63),
        (IrBinOp::Shl, true) => ("slliw", k & 31),
        (IrBinOp::LShr, false) => ("srli", k & 63),
        (IrBinOp::LShr, true) => ("srliw", k & 31),
        (IrBinOp::AShr, false) => ("srai", k & 63),
        (IrBinOp::AShr, true) => ("sraiw", k & 31),
        _ => return None,
    };
    Some(form)
}

impl RiscvCodegen {
    // ---- Unary ----

    pub(super) fn emit_float_neg_impl(&mut self, ty: IrType) {
        if ty == IrType::F64 {
            self.state.emit("    fmv.d.x ft0, t0");
            self.state.emit("    fneg.d ft0, ft0");
            self.state.emit("    fmv.x.d t0, ft0");
        } else {
            self.state.emit("    fmv.w.x ft0, t0");
            self.state.emit("    fneg.s ft0, ft0");
            self.state.emit("    fmv.x.w t0, ft0");
        }
    }

    pub(super) fn emit_int_neg_impl(&mut self, ty: IrType) {
        // Canonical-form discipline (rv64 codegen invariant): every <=32-bit
        // integer in a 64-bit register equals signext(low32); signed narrow
        // values are additionally sign-filled to their own width, unsigned
        // narrow/U32 keep an explicit zero-extension (sltu staging and U64
        // widening read the raw low bits).  Two's-complement negation
        // preserves sign-extension, so the refill below only re-establishes
        // the width canonical the consumers expect — and for I32 the *w
        // form IS the canonical signed-32 op (GCC-identical).
        match ty {
            IrType::I32 => self.state.emit("    negw t0, t0"),
            IrType::I8 => {
                self.state.emit("    neg t0, t0");
                self.state.emit("    slli t0, t0, 56");
                self.state.emit("    srai t0, t0, 56");
            }
            IrType::I16 => {
                self.state.emit("    neg t0, t0");
                self.state.emit("    slli t0, t0, 48");
                self.state.emit("    srai t0, t0, 48");
            }
            IrType::U8 | IrType::U16 | IrType::U32 => {
                // Zero-extend to 32 bits, matching the convention used by
                // emit_bswap/clz (the upper half of a 64-bit neg on a
                // zero-extended U32 is 0xFFFFFFFF — x86-64 audit, claim 1).
                self.state.emit("    neg t0, t0");
                self.state.emit("    slli t0, t0, 32");
                self.state.emit("    srli t0, t0, 32");
            }
            _ => self.state.emit("    neg t0, t0"),
        }
    }

    pub(super) fn emit_int_not_impl(&mut self, ty: IrType) {
        // Bitwise complement also preserves sign-extension of a canonical
        // input: ~sextN(x) = sextN(~x).  I32 therefore needs NO truncation
        // pair at all — plain 64-bit `not` (xori -1) is already sext32
        // canonical and GCC-identical; the old unconditional zext pair
        // violated the invariant.  Signed narrow widths refill their own
        // width; unsigned keep the explicit zero-extension.
        match ty {
            IrType::I32 => self.state.emit("    not t0, t0"),
            IrType::I8 => {
                self.state.emit("    not t0, t0");
                self.state.emit("    slli t0, t0, 56");
                self.state.emit("    srai t0, t0, 56");
            }
            IrType::I16 => {
                self.state.emit("    not t0, t0");
                self.state.emit("    slli t0, t0, 48");
                self.state.emit("    srai t0, t0, 48");
            }
            IrType::U8 | IrType::U16 | IrType::U32 => {
                self.state.emit("    not t0, t0");
                self.state.emit("    slli t0, t0, 32");
                self.state.emit("    srli t0, t0, 32");
            }
            _ => self.state.emit("    not t0, t0"),
        }
    }

    // ---- Integer binop ----

    pub(super) fn emit_int_binop_impl(
        &mut self,
        dest: &Value,
        op: IrBinOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) {
        // Note: i128 dispatch is handled by the shared emit_binop default in traits.rs.
        let use_32bit = ty == IrType::I32 || ty == IrType::U32;

        // `x op K` whose constant the I-type encoding holds is one
        // instruction instead of four (`li` of K, the copy of `x` into t1,
        // the copy of K into t2, the R-type form): see `immediate_form`.
        if let Some((mnemonic, imm)) = immediate_form(op, rhs, use_32bit) {
            self.operand_to_t0(lhs);
            self.state
                .emit_fmt(format_args!("    {mnemonic} t0, t0, {imm}"));
            self.state.reg_cache.invalidate_acc();
            self.store_t0_to(dest);
            return;
        }

        self.operand_to_t0(lhs);
        self.state.emit("    mv t1, t0");
        self.operand_to_t0(rhs);
        self.state.emit("    mv t2, t0");

        if op == IrBinOp::BitTest {
            // RISC-V has no BT (Zbs `bext` is not modelled): logical-shift the
            // base right by the index modulo the *operation* width and isolate
            // bit zero.  `srlw` reads only the low 5 bits of its count and
            // `srl` the low 6, which IS the modulo at 32/64 bits; subword
            // widths reduce explicitly and then use the XLEN shift.  The `andi`
            // result is 0 or 1, canonical in every integer type's register
            // form (signext(low32) and zero-extension agree on it), so no
            // re-extension follows.
            let bits = (ty.size() * 8) as u32;
            debug_assert!(matches!(bits, 8 | 16 | 32 | 64));
            if bits < 32 {
                self.state
                    .emit_fmt(format_args!("    andi t2, t2, {}", bits - 1));
            }
            let shift = if use_32bit { "srlw" } else { "srl" };
            self.state.emit_fmt(format_args!("    {shift} t0, t1, t2"));
            self.state.emit("    andi t0, t0, 1");
            self.store_t0_to(dest);
            return;
        }

        if matches!(op, IrBinOp::RotateLeft | IrBinOp::RotateRight) {
            // RV64I has no rotate: `rol`/`ror` are Zbb, and this backend does
            // not yet model Z-extension availability, so emit the portable
            // funnel shift.  It needs no scratch beyond the three registers
            // this path already stages into, because the second half's count
            // is `W - n` and `sll`/`srl` take their count from the low
            // lg2(XLEN) bits of the register -- so `-n` already IS `W - n`
            // modulo W and `neg` supplies the complement in one instruction.
            // t1 (the source) stays live until both halves are computed.
            let (first, second) = match (op, use_32bit) {
                (IrBinOp::RotateLeft, true) => ("sllw", "srlw"),
                (IrBinOp::RotateRight, true) => ("srlw", "sllw"),
                (IrBinOp::RotateLeft, false) => ("sll", "srl"),
                (IrBinOp::RotateRight, false) => ("srl", "sll"),
                _ => unreachable!("guarded by the matches! above"),
            };
            self.state.emit_fmt(format_args!("    {first} t0, t1, t2"));
            self.state.emit("    neg t2, t2");
            self.state.emit_fmt(format_args!("    {second} t2, t1, t2"));
            self.state.emit("    or t0, t0, t2");
            if use_32bit {
                // `*w` forms sign-extend their 32-bit result; the canonical
                // rotate value is the unsigned bit pattern, so normalize the
                // same way the BitTest path above does.
                self.state.emit("    slli t0, t0, 32");
                self.state.emit("    srli t0, t0, 32");
            }
            self.store_t0_to(dest);
            return;
        }

        let mnemonic = match (op, use_32bit) {
            (IrBinOp::Add, false) => "add",
            (IrBinOp::Add, true) => "addw",
            (IrBinOp::Sub, false) => "sub",
            (IrBinOp::Sub, true) => "subw",
            (IrBinOp::Mul, false) => "mul",
            (IrBinOp::Mul, true) => "mulw",
            (IrBinOp::SDiv, false) => "div",
            (IrBinOp::SDiv, true) => "divw",
            (IrBinOp::UDiv, false) => "divu",
            (IrBinOp::UDiv, true) => "divuw",
            (IrBinOp::SRem, false) => "rem",
            (IrBinOp::SRem, true) => "remw",
            (IrBinOp::URem, false) => "remu",
            (IrBinOp::URem, true) => "remuw",
            (IrBinOp::And, _) => "and",
            (IrBinOp::Or, _) => "or",
            (IrBinOp::Xor, _) => "xor",
            (IrBinOp::Shl, false) => "sll",
            (IrBinOp::Shl, true) => "sllw",
            (IrBinOp::AShr, false) => "sra",
            (IrBinOp::AShr, true) => "sraw",
            (IrBinOp::LShr, false) => "srl",
            (IrBinOp::LShr, true) => "srlw",
            (IrBinOp::BitTest, _) => unreachable!("BitTest handled above"),
            (IrBinOp::BitSet | IrBinOp::BitClear | IrBinOp::BitToggle, _) => {
                self.emit_bit_modify_impl(dest, op, ty);
                return;
            }
            (IrBinOp::Bzhi, _) => crate::ir::ops::bzhi_outside_x86_64(),
            (IrBinOp::UMulHigh, false) => "mulhu",
            (IrBinOp::SMulHigh, false) => "mulh",
            (IrBinOp::UMulHigh | IrBinOp::SMulHigh, true) => {
                crate::ir::ops::mul_high_unsupported_here()
            }
            (IrBinOp::RotateLeft, _) | (IrBinOp::RotateRight, _) => {
                unreachable!("rotate handled above")
            }
        };
        self.state
            .emit_fmt(format_args!("    {} t0, t1, t2", mnemonic));

        self.store_t0_to(dest);
    }

    /// `BitSet`/`BitClear`/`BitToggle` with the base already staged in `t1`
    /// and the index in `t2`.  RV64I has no Zbs `bset`/`bclr`/`bext`-modify,
    /// so build a one-hot mask and OR/AND-NOT/XOR it.  The shift count is reduced modulo the *operation* width,
    /// not XLEN: an 8/16/32-bit operation must wrap at 8/16/32 even though
    /// `sll` itself masks at 64.  The result is then put back into the rv64
    /// canonical form for its type (signed narrow values sign-filled,
    /// unsigned narrow values zero-extended) because downstream comparisons
    /// and widening reads trust that invariant rather than re-truncating.
    fn emit_bit_modify_impl(&mut self, dest: &Value, op: IrBinOp, ty: IrType) {
        let bits = (ty.size() * 8) as u32;
        debug_assert!(matches!(bits, 8 | 16 | 32 | 64));
        if bits < 64 {
            self.state
                .emit_fmt(format_args!("    andi t2, t2, {}", bits - 1));
        }
        self.state.emit("    li t0, 1");
        // I32 needs a sign-extended word mask so OR/AND with a canonical
        // sign-extended base stays sign-extended.  U32 and the narrow widths
        // use an XLEN mask; narrow results are canonicalized explicitly below.
        if ty == IrType::I32 {
            self.state.emit("    sllw t0, t0, t2");
        } else {
            self.state.emit("    sll t0, t0, t2");
        }
        if op == IrBinOp::BitSet {
            self.state.emit("    or t0, t1, t0");
        } else if op == IrBinOp::BitClear {
            self.state.emit("    xori t0, t0, -1");
            self.state.emit("    and t0, t1, t0");
        } else {
            // The only remaining caller-gated operation is BitToggle.
            self.state.emit("    xor t0, t1, t0");
        }
        match ty {
            IrType::I8 => {
                self.state.emit("    slli t0, t0, 56");
                self.state.emit("    srai t0, t0, 56");
            }
            IrType::I16 => {
                self.state.emit("    slli t0, t0, 48");
                self.state.emit("    srai t0, t0, 48");
            }
            IrType::U8 => self.state.emit("    andi t0, t0, 255"),
            IrType::U16 => {
                self.state.emit("    slli t0, t0, 48");
                self.state.emit("    srli t0, t0, 48");
            }
            // I32 preserved sign-extension through the sllw mask; U32 used a
            // zero-extended XLEN mask and a zero-extended base, so the result
            // is already canonical.  64-bit results need no normalization.
            _ => {}
        }
        self.store_t0_to(dest);
    }

    // ---- Copy i128 ----

    pub(super) fn emit_copy_i128_impl(&mut self, dest: &Value, src: &Operand) {
        self.operand_to_t0_t1(src);
        self.store_t0_t1_to(dest);
    }
}

#[cfg(test)]
mod bit_modify_width_tests {
    use super::*;
    use crate::backend::regalloc::PhysReg;

    fn emit(op: IrBinOp, ty: IrType) -> String {
        let mut cg = RiscvCodegen::new();
        // a0/a1 hold the base/index; a2 receives the result. The index is a
        // runtime Value, so the emitter must perform the modulo-W reduction
        // itself rather than relying on a constant folder.
        cg.reg_assignments.insert(1, PhysReg(12));
        cg.reg_assignments.insert(2, PhysReg(13));
        cg.reg_assignments.insert(3, PhysReg(14));
        cg.emit_int_binop_impl(
            &Value(3),
            op,
            &Operand::Value(Value(1)),
            &Operand::Value(Value(2)),
            ty,
        );
        cg.state.out.buf
    }

    #[test]
    fn index_is_reduced_modulo_the_operation_width_not_xlen() {
        for ty in [
            IrType::I8,
            IrType::U8,
            IrType::I16,
            IrType::U16,
            IrType::I32,
            IrType::U32,
            IrType::I64,
            IrType::U64,
        ] {
            let width = (ty.size() * 8) as u32;
            for op in [IrBinOp::BitSet, IrBinOp::BitClear, IrBinOp::BitToggle] {
                let asm = emit(op, ty);
                if width < 64 {
                    assert!(
                        asm.contains(&format!("andi t2, t2, {}", width - 1)),
                        "{op:?} {ty:?} must mask the index to {width} bits:\n{asm}"
                    );
                }
            }
        }
    }

    #[test]
    fn narrow_bit_test_reduces_the_index_modulo_the_operation_width() {
        for ty in [IrType::I8, IrType::U8, IrType::I16, IrType::U16] {
            let width = (ty.size() * 8) as u32;
            let asm = emit(IrBinOp::BitTest, ty);
            assert!(
                asm.contains(&format!("andi t2, t2, {}", width - 1)),
                "BitTest {ty:?} must mask the index to {width} bits:\n{asm}"
            );
            assert!(
                asm.contains("srl t0, t1, t2") && asm.contains("andi t0, t0, 1"),
                "BitTest {ty:?} must extract a canonical 0/1 result:\n{asm}"
            );
        }
    }

    #[test]
    fn result_keeps_the_rv64_canonical_integer_form() {
        for op in [IrBinOp::BitSet, IrBinOp::BitClear, IrBinOp::BitToggle] {
            let asm8 = emit(op, IrType::I8);
            assert!(
                asm8.contains("slli t0, t0, 56") && asm8.contains("srai t0, t0, 56"),
                "I8 {op:?} must sign-extend the result:\n{asm8}"
            );
            let asm16 = emit(op, IrType::I16);
            assert!(
                asm16.contains("slli t0, t0, 48") && asm16.contains("srai t0, t0, 48"),
                "I16 {op:?} must sign-extend the result:\n{asm16}"
            );
            let asm32 = emit(op, IrType::I32);
            assert!(
                asm32.contains("sllw t0, t0, t2"),
                "I32 {op:?} must build a sign-extended word mask:\n{asm32}"
            );
            let asm_u32 = emit(op, IrType::U32);
            assert!(
                asm_u32.contains("sll t0, t0, t2") && !asm_u32.contains("sllw"),
                "U32 {op:?} must build a zero-extended mask:\n{asm_u32}"
            );
        }
    }
}

#[cfg(test)]
mod immediate_form_tests {
    use super::*;
    use crate::backend::regalloc::PhysReg;

    /// `a2 = a0 op K`, through the real emitter.
    fn emit(op: IrBinOp, ty: IrType, k: IrConst) -> String {
        let mut cg = RiscvCodegen::new();
        cg.reg_assignments.insert(1, PhysReg(12));
        cg.reg_assignments.insert(3, PhysReg(14));
        cg.emit_int_binop_impl(
            &Value(3),
            op,
            &Operand::Value(Value(1)),
            &Operand::Const(k),
            ty,
        );
        cg.state.out.buf
    }

    #[test]
    fn a_fitting_constant_selects_the_i_type_form() {
        let cases: [(IrBinOp, IrType, IrConst, &str); 16] = [
            (
                IrBinOp::Add,
                IrType::I32,
                IrConst::I32(1),
                "addiw t0, t0, 1",
            ),
            (
                IrBinOp::Add,
                IrType::U32,
                IrConst::I32(-7),
                "addiw t0, t0, -7",
            ),
            (
                IrBinOp::Add,
                IrType::I64,
                IrConst::I64(2047),
                "addi t0, t0, 2047",
            ),
            (
                IrBinOp::Add,
                IrType::I64,
                IrConst::I64(-2048),
                "addi t0, t0, -2048",
            ),
            (
                IrBinOp::Sub,
                IrType::I32,
                IrConst::I32(5),
                "addiw t0, t0, -5",
            ),
            (
                IrBinOp::Sub,
                IrType::I64,
                IrConst::I64(2048),
                "addi t0, t0, -2048",
            ),
            (
                IrBinOp::And,
                IrType::I32,
                IrConst::I32(255),
                "andi t0, t0, 255",
            ),
            (
                IrBinOp::And,
                IrType::U64,
                IrConst::I64(-16),
                "andi t0, t0, -16",
            ),
            (IrBinOp::Or, IrType::I64, IrConst::I64(16), "ori t0, t0, 16"),
            (
                IrBinOp::Xor,
                IrType::I32,
                IrConst::I32(-1),
                "xori t0, t0, -1",
            ),
            (
                IrBinOp::Shl,
                IrType::I32,
                IrConst::I32(3),
                "slliw t0, t0, 3",
            ),
            (
                IrBinOp::Shl,
                IrType::I64,
                IrConst::I64(40),
                "slli t0, t0, 40",
            ),
            (
                IrBinOp::LShr,
                IrType::U32,
                IrConst::I32(31),
                "srliw t0, t0, 31",
            ),
            (
                IrBinOp::LShr,
                IrType::U64,
                IrConst::I64(63),
                "srli t0, t0, 63",
            ),
            (
                IrBinOp::AShr,
                IrType::I32,
                IrConst::I32(31),
                "sraiw t0, t0, 31",
            ),
            (
                IrBinOp::AShr,
                IrType::I64,
                IrConst::I64(3),
                "srai t0, t0, 3",
            ),
        ];
        for (op, ty, k, want) in cases {
            let asm = emit(op, ty, k);
            assert!(
                asm.contains(want),
                "{op:?} {ty:?} {k:?}: want `{want}`:\n{asm}"
            );
            assert!(
                !asm.contains("t2") && !asm.lines().any(|l| l.trim_start().starts_with("li ")),
                "{op:?} {ty:?} {k:?}: no staging left:\n{asm}"
            );
        }
    }

    #[test]
    fn a_shift_count_is_reduced_like_the_register_form_reduces_it() {
        // `sll`/`srl`/`sra` read six count bits and the `w` forms five, so
        // an out-of-range constant count means the same reduced count.
        for (op, ty, k, want) in [
            (
                IrBinOp::Shl,
                IrType::I32,
                IrConst::I32(33),
                "slliw t0, t0, 1",
            ),
            (
                IrBinOp::Shl,
                IrType::I64,
                IrConst::I64(65),
                "slli t0, t0, 1",
            ),
            (
                IrBinOp::AShr,
                IrType::I32,
                IrConst::I32(-1),
                "sraiw t0, t0, 31",
            ),
            (
                IrBinOp::LShr,
                IrType::I64,
                IrConst::I64(-1),
                "srli t0, t0, 63",
            ),
            (IrBinOp::Shl, IrType::I8, IrConst::I8(9), "slli t0, t0, 9"),
        ] {
            let asm = emit(op, ty, k);
            assert!(
                asm.contains(want),
                "{op:?} {ty:?} {k:?}: want `{want}`:\n{asm}"
            );
        }
    }

    #[test]
    fn a_constant_the_encoding_cannot_hold_keeps_the_register_form() {
        for (op, ty, k) in [
            (IrBinOp::Add, IrType::I32, IrConst::I32(2048)),
            (IrBinOp::Add, IrType::I64, IrConst::I64(-2049)),
            (IrBinOp::Sub, IrType::I64, IrConst::I64(-2048)), // -K == 2048
            (IrBinOp::Sub, IrType::I64, IrConst::I64(i64::MIN)),
            (IrBinOp::And, IrType::U32, IrConst::I64(4_294_967_295)),
            (IrBinOp::Or, IrType::I64, IrConst::I64(1 << 40)),
            (IrBinOp::Xor, IrType::I32, IrConst::I32(4096)),
        ] {
            let asm = emit(op, ty, k);
            assert!(
                asm.contains("t0, t1, t2"),
                "{op:?} {ty:?} {k:?}: register form expected:\n{asm}"
            );
        }
    }

    #[test]
    fn operations_without_an_i_type_twin_keep_the_register_form() {
        for op in [
            IrBinOp::Mul,
            IrBinOp::SDiv,
            IrBinOp::UDiv,
            IrBinOp::SRem,
            IrBinOp::URem,
        ] {
            let asm = emit(op, IrType::I64, IrConst::I64(3));
            assert!(asm.contains("t0, t1, t2"), "{op:?}:\n{asm}");
        }
        let rot = emit(IrBinOp::RotateLeft, IrType::I64, IrConst::I64(3));
        assert!(
            rot.contains("neg t2, t2"),
            "rotate keeps its funnel:\n{rot}"
        );
        let bt = emit(IrBinOp::BitTest, IrType::I64, IrConst::I64(3));
        assert!(
            bt.contains("srl t0, t1, t2"),
            "bit test keeps its shift:\n{bt}"
        );
    }

    #[test]
    fn every_i_type_form_equals_its_register_twin_on_rv64() {
        // A model of the two RV64 semantics over boundary operands: the
        // register form sees `li`'s value K; the I-type form sees the
        // sign-extended 12-bit immediate (or the 5/6-bit count).
        fn sext32(v: i64) -> i64 {
            i64::from(v as i32)
        }
        let xs = [
            0i64,
            1,
            -1,
            0x7fff_ffff,
            -0x8000_0000,
            0x1234_5678_9abc_def0,
            i64::MIN,
            i64::MAX,
        ];
        let ks = [0i64, 1, -1, 31, 32, 63, 64, 2047, -2048, 2048, -2049];
        for &x in &xs {
            for &k in &ks {
                for (op, w) in [
                    (IrBinOp::Add, false),
                    (IrBinOp::Add, true),
                    (IrBinOp::Sub, false),
                    (IrBinOp::Sub, true),
                    (IrBinOp::And, false),
                    (IrBinOp::Or, true),
                    (IrBinOp::Xor, false),
                    (IrBinOp::Shl, false),
                    (IrBinOp::Shl, true),
                    (IrBinOp::LShr, false),
                    (IrBinOp::LShr, true),
                    (IrBinOp::AShr, false),
                    (IrBinOp::AShr, true),
                ] {
                    let Some((mnem, imm)) = immediate_form(op, &Operand::Const(IrConst::I64(k)), w)
                    else {
                        continue;
                    };
                    let reg = match (op, w) {
                        (IrBinOp::Add, false) => x.wrapping_add(k),
                        (IrBinOp::Add, true) => sext32(x.wrapping_add(k)),
                        (IrBinOp::Sub, false) => x.wrapping_sub(k),
                        (IrBinOp::Sub, true) => sext32(x.wrapping_sub(k)),
                        (IrBinOp::And, _) => x & k,
                        (IrBinOp::Or, _) => x | k,
                        (IrBinOp::Xor, _) => x ^ k,
                        (IrBinOp::Shl, false) => x.wrapping_shl((k & 63) as u32),
                        (IrBinOp::Shl, true) => {
                            sext32((x as i32).wrapping_shl((k & 31) as u32) as i64)
                        }
                        (IrBinOp::LShr, false) => ((x as u64) >> (k & 63)) as i64,
                        (IrBinOp::LShr, true) => sext32(((x as u32) >> (k & 31)) as i64),
                        (IrBinOp::AShr, false) => x >> (k & 63),
                        (IrBinOp::AShr, true) => sext32(((x as i32) >> (k & 31)) as i64),
                        _ => unreachable!(),
                    };
                    assert!((-2048..=2047).contains(&imm) || mnem.starts_with('s'));
                    let ity = match mnem {
                        "addi" => x.wrapping_add(imm),
                        "addiw" => sext32(x.wrapping_add(imm)),
                        "andi" => x & imm,
                        "ori" => x | imm,
                        "xori" => x ^ imm,
                        "slli" => x.wrapping_shl(imm as u32),
                        "slliw" => sext32((x as i32).wrapping_shl(imm as u32) as i64),
                        "srli" => ((x as u64) >> imm) as i64,
                        "srliw" => sext32(((x as u32) >> imm) as i64),
                        "srai" => x >> imm,
                        "sraiw" => sext32(((x as i32) >> imm) as i64),
                        _ => unreachable!("{mnem}"),
                    };
                    assert_eq!(reg, ity, "{op:?} w={w} x={x:#x} k={k} -> {mnem} {imm}");
                }
            }
        }
    }
}
