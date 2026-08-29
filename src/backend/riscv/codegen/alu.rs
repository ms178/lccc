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
            // RISC-V has no BT. The canonical operation is i32, but keep 64-bit
            // behavior for the shared integer default: logical-shift the base
            // right by the index, then isolate bit zero. This is exactly what
            // every backend would otherwise reconstruct from `(x >> i) & 1`.
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
        };
        self.state
            .emit_fmt(format_args!("    {} t0, t1, t2", mnemonic));

        self.store_t0_to(dest);
    }

    // ---- Copy i128 ----

    pub(super) fn emit_copy_i128_impl(&mut self, dest: &Value, src: &Operand) {
        self.operand_to_t0_t1(src);
        self.store_t0_t1_to(dest);
    }
}
