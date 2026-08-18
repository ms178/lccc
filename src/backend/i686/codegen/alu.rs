//! I686Codegen: ALU operations (integer arithmetic, bitwise, shifts).

use crate::ir::reexports::{IrBinOp, Operand, Value};
use crate::common::types::IrType;
use crate::emit;
use super::emit::{I686Codegen, alu_mnemonic, shift_mnemonic};

/// Location of a 32-bit binop operand for the direct-to-dest path.
enum BinopLoc {
    Reg(&'static str),
    Slot(String),
    Imm(i64),
}

impl I686Codegen {
    pub(super) fn emit_float_neg_impl(&mut self, ty: IrType) {
        if ty == IrType::F32 {
            self.state.emit("    movd %eax, %xmm0");
            self.state.emit("    movl $0x80000000, %ecx");
            self.state.emit("    movd %ecx, %xmm1");
            self.state.emit("    xorps %xmm1, %xmm0");
            self.state.emit("    movd %xmm0, %eax");
        } else {
            self.state.emit("    xorl $0x80000000, %eax");
        }
    }

    pub(super) fn emit_int_neg_impl(&mut self, _ty: IrType) {
        self.state.emit("    negl %eax");
    }

    pub(super) fn emit_int_not_impl(&mut self, _ty: IrType) {
        self.state.emit("    notl %eax");
    }

    pub(super) fn emit_int_clz_impl(&mut self, ty: IrType) {
        if matches!(ty, IrType::I32 | IrType::U32 | IrType::Ptr) {
            self.state.emit("    lzcntl %eax, %eax");
        } else if matches!(ty, IrType::I16 | IrType::U16) {
            self.state.emit("    lzcntw %ax, %ax");
        } else {
            self.state.emit("    lzcntl %eax, %eax");
        }
    }

    pub(super) fn emit_int_ctz_impl(&mut self, _ty: IrType) {
        // tzcntl works for all integer widths on i686: the value is in %eax
        // and trailing zero count is the same regardless of nominal width.
        self.state.emit("    tzcntl %eax, %eax");
    }

    pub(super) fn emit_int_bswap_impl(&mut self, ty: IrType) {
        match ty {
            IrType::I16 | IrType::U16 => self.state.emit("    rolw $8, %ax"),
            IrType::I32 | IrType::U32 | IrType::Ptr => self.state.emit("    bswapl %eax"),
            _ => self.state.emit("    bswapl %eax"),
        }
    }

    pub(super) fn emit_int_popcount_impl(&mut self, _ty: IrType) {
        self.state.emit("    popcntl %eax, %eax");
    }

    /// Operand location for the direct-to-dest ALU path.
    fn binop_loc(&self, op: &Operand) -> Option<BinopLoc> {
        match op {
            Operand::Const(_) => Self::const_as_imm32(op).map(BinopLoc::Imm),
            Operand::Value(v) => {
                if self.state.wide_values.contains(&v.0)
                    || self.state.is_alloca(v.0)
                    || self.state.f128_direct_slots.contains(&v.0)
                {
                    // Alloca "values" are addresses (leal), wide values are
                    // pairs — neither fits a single 32-bit operand.
                    return None;
                }
                if let Some(&phys) = self.reg_assignments.get(&v.0) {
                    return Some(BinopLoc::Reg(super::emit::phys_reg_name(phys)));
                }
                self.state.get_slot(v.0).map(|slot| BinopLoc::Slot(self.slot_ref(slot)))
            }
        }
    }

    /// Compute a 32-bit int binop straight into the destination's register,
    /// bypassing the %eax round-trip (movl src,%eax; op ...; movl %eax,%dst
    /// -> op src,%dst). Returns false when the shape doesn't fit; the caller
    /// falls back to the accumulator path. %eax/%ecx/%edx are untouched, so
    /// the accumulator cache stays valid.
    fn try_emit_int_binop_direct(&mut self, dest: &Value, op: IrBinOp, lhs: &Operand, rhs: &Operand) -> bool {
        use BinopLoc::*;
        // Only simple two-address ALU ops, imm shifts, and imull.
        let is_alu = matches!(op, IrBinOp::Add | IrBinOp::Sub | IrBinOp::And | IrBinOp::Or | IrBinOp::Xor);
        let is_mul = op == IrBinOp::Mul;
        let is_shift = matches!(op, IrBinOp::Shl | IrBinOp::AShr | IrBinOp::LShr);
        if !is_alu && !is_mul && !is_shift {
            return false;
        }
        let Some(dphys) = self.dest_reg(dest) else { return false };
        let d = super::emit::phys_reg_name(dphys);
        let Some(l) = self.binop_loc(lhs) else { return false };
        let Some(r) = self.binop_loc(rhs) else { return false };
        // Shifts: only immediate counts (variable counts need %cl).
        if is_shift && !matches!(r, Imm(_)) {
            return false;
        }
        let commutative = matches!(op, IrBinOp::Add | IrBinOp::And | IrBinOp::Or | IrBinOp::Xor | IrBinOp::Mul);
        let lhs_is_d = matches!(&l, Reg(n) if *n == d);
        let rhs_is_d = matches!(&r, Reg(n) if *n == d);

        // imull has a true 3-operand immediate form: no copy needed at all.
        if is_mul {
            if let Imm(i) = r {
                match &l {
                    Reg(n) => emit!(self.state, "    imull ${}, %{}, %{}", i, n, d),
                    Slot(sr) => emit!(self.state, "    imull ${}, {}, %{}", i, sr, d),
                    Imm(li) => {
                        emit!(self.state, "    movl ${}, %{}", li.wrapping_mul(i), d);
                    }
                }
                return true;
            }
        }

        let mnem: String = if is_shift {
            shift_mnemonic(op).to_string()
        } else if is_mul {
            "imull".to_string()
        } else {
            format!("{}l", alu_mnemonic(op))
        };
        let rhs_str = |loc: &BinopLoc| -> String {
            match loc {
                Reg(n) => format!("%{}", n),
                Slot(sr) => sr.clone(),
                Imm(i) => format!("${}", i),
            }
        };

        if lhs_is_d {
            // dest op= rhs. rhs may be d itself (fine: d op= d).
            emit!(self.state, "    {} {}, %{}", mnem, rhs_str(&r), d);
            return true;
        }
        if rhs_is_d {
            if !commutative {
                return false; // sub/shift with dest as rhs: needs a temp.
            }
            // dest op= lhs (commutative).
            emit!(self.state, "    {} {}, %{}", mnem, rhs_str(&l), d);
            return true;
        }
        // dest gets a fresh copy of lhs, then op rhs. rhs proven != d above.
        match &l {
            Reg(n) => emit!(self.state, "    movl %{}, %{}", n, d),
            Slot(sr) => emit!(self.state, "    movl {}, %{}", sr, d),
            Imm(i) => emit!(self.state, "    movl ${}, %{}", i, d),
        }
        emit!(self.state, "    {} {}, %{}", mnem, rhs_str(&r), d);
        true
    }

    pub(super) fn emit_int_binop_impl(&mut self, dest: &Value, op: IrBinOp, lhs: &Operand, rhs: &Operand, _ty: IrType) {
        // Direct-to-dest path: skip the accumulator entirely when the dest
        // has a register and both operands are register/slot/imm-resident.
        if self.try_emit_int_binop_direct(dest, op, lhs, rhs) {
            return;
        }
        // Immediate optimization for ALU ops
        if matches!(op, IrBinOp::Add | IrBinOp::Sub | IrBinOp::And | IrBinOp::Or | IrBinOp::Xor) {
            if let Some(imm) = Self::const_as_imm32(rhs) {
                self.operand_to_eax(lhs);
                let mnem = alu_mnemonic(op);
                emit!(self.state, "    {}l ${}, %eax", mnem, imm);
                self.state.reg_cache.invalidate_acc();
                self.store_eax_to(dest);
                return;
            }
        }

        // Immediate multiply
        if op == IrBinOp::Mul {
            if let Some(imm) = Self::const_as_imm32(rhs) {
                self.operand_to_eax(lhs);
                match imm {
                    3 => emit!(self.state, "    leal (%eax, %eax, 2), %eax"),
                    5 => emit!(self.state, "    leal (%eax, %eax, 4), %eax"),
                    9 => emit!(self.state, "    leal (%eax, %eax, 8), %eax"),
                    _ => emit!(self.state, "    imull ${}, %eax, %eax", imm),
                }
                self.state.reg_cache.invalidate_acc();
                self.store_eax_to(dest);
                return;
            }
        }

        // Immediate shift
        if matches!(op, IrBinOp::Shl | IrBinOp::AShr | IrBinOp::LShr) {
            if let Some(imm) = Self::const_as_imm32(rhs) {
                self.operand_to_eax(lhs);
                let mnem = shift_mnemonic(op);
                let shift_amount = (imm as u32) & 31;
                emit!(self.state, "    {} ${}, %eax", mnem, shift_amount);
                self.state.reg_cache.invalidate_acc();
                self.store_eax_to(dest);
                return;
            }
        }

        // General case: load lhs to eax, rhs to ecx
        self.operand_to_eax(lhs);
        self.operand_to_ecx(rhs);

        match op {
            IrBinOp::Add => self.state.emit("    addl %ecx, %eax"),
            IrBinOp::Sub => self.state.emit("    subl %ecx, %eax"),
            IrBinOp::Mul => self.state.emit("    imull %ecx, %eax"),
            IrBinOp::And => self.state.emit("    andl %ecx, %eax"),
            IrBinOp::Or => self.state.emit("    orl %ecx, %eax"),
            IrBinOp::Xor => self.state.emit("    xorl %ecx, %eax"),
            IrBinOp::Shl => self.state.emit("    shll %cl, %eax"),
            IrBinOp::AShr => self.state.emit("    sarl %cl, %eax"),
            IrBinOp::LShr => self.state.emit("    shrl %cl, %eax"),
            IrBinOp::SDiv => {
                self.state.emit("    cltd");
                self.state.emit("    idivl %ecx");
            }
            IrBinOp::UDiv => {
                self.state.emit("    xorl %edx, %edx");
                self.state.emit("    divl %ecx");
            }
            IrBinOp::SRem => {
                self.state.emit("    cltd");
                self.state.emit("    idivl %ecx");
                self.state.emit("    movl %edx, %eax");
            }
            IrBinOp::URem => {
                self.state.emit("    xorl %edx, %edx");
                self.state.emit("    divl %ecx");
                self.state.emit("    movl %edx, %eax");
            }
        }
        self.state.reg_cache.invalidate_acc();
        self.store_eax_to(dest);
    }
}
