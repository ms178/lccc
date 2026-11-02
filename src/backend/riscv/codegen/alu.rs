//! RiscvCodegen: integer/float arithmetic, unary ops, binop, copy.

use super::emit::RiscvCodegen;
use crate::common::types::IrType;
use crate::ir::reexports::{IrBinOp, Operand, Value};

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
        self.operand_to_t0(lhs);
        self.state.emit("    mv t1, t0");
        self.operand_to_t0(rhs);
        self.state.emit("    mv t2, t0");

        let use_32bit = ty == IrType::I32 || ty == IrType::U32;

        if op == IrBinOp::BitTest {
            // RISC-V has no BT. Reduce the index modulo the *operation* width,
            // then logical-shift the base right and isolate bit zero. `srlw`
            // is the 32-bit form; subword operations use XLEN shifts after the
            // explicit modulo, and the 0/1 result is already canonical.
            let bits = (ty.size() * 8) as u32;
            debug_assert!(matches!(bits, 8 | 16 | 32 | 64));
            if bits < 64 {
                self.state
                    .emit_fmt(format_args!("    andi t2, t2, {}", bits - 1));
            }
            let shift = if use_32bit { "srlw" } else { "srl" };
            self.state.emit_fmt(format_args!("    {shift} t0, t1, t2"));
            self.state.emit("    andi t0, t0, 1");
            if use_32bit {
                self.state.emit("    slli t0, t0, 32");
                self.state.emit("    srli t0, t0, 32");
            }
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
