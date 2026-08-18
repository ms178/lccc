//! I686Codegen: comparison operations (float, int, fused branches, select).

use crate::ir::reexports::{IrCmpOp, Operand, Value};
use crate::common::types::IrType;
use crate::emit;
use crate::backend::traits::ArchCodegen;
use super::emit::{I686Codegen, phys_reg_name};

impl I686Codegen {
    /// Register name for a scalar value safe to compare in place (not an
    /// alloca address, not a wide pair). None for slot-resident values and
    /// immediates.
    fn cmp_operand_reg(&self, op: &Operand) -> Option<&'static str> {
        if let Operand::Value(v) = op {
            if self.state.is_alloca(v.0)
                || self.state.wide_values.contains(&v.0)
                || self.state.f128_direct_slots.contains(&v.0)
            {
                return None;
            }
            if let Some(&phys) = self.reg_assignments.get(&v.0) {
                return Some(phys_reg_name(phys));
            }
        }
        None
    }

    /// Emit the flag-setting compare for an integer compare, staging the LHS
    /// through the accumulator unless it is register-resident (in which case
    /// `testl %R,%R` / `cmpl $imm,%R` / `cmpl %R,%L` is emitted in place and
    /// %eax stays untouched). `%ecx` is the slot-operand scratch, so a LHS in
    /// %ecx never takes the direct path against a slot-resident RHS.
    fn emit_int_cmp_flags_direct(&mut self, lhs: &Operand, rhs: &Operand, ty: IrType) -> bool {
        let Some(lreg) = self.cmp_operand_reg(lhs) else { return false };
        if let Some(imm) = Self::const_as_imm32(rhs) {
            let imm = Self::normalize_cmp_imm(imm, ty);
            if imm == 0 {
                emit!(self.state, "    testl %{}, %{}", lreg, lreg);
            } else {
                emit!(self.state, "    cmpl ${}, %{}", imm, lreg);
            }
        } else if let Some(rreg) = self.cmp_operand_reg(rhs) {
            emit!(self.state, "    cmpl %{}, %{}", rreg, lreg);
        } else {
            if lreg == "ecx" {
                return false; // staging rhs into %ecx would clobber lhs
            }
            self.operand_to_ecx(rhs);
            emit!(self.state, "    cmpl %ecx, %{}", lreg);
        }
        true
    }

    pub(super) fn emit_float_cmp_impl(&mut self, dest: &Value, op: IrCmpOp, lhs: &Operand, rhs: &Operand, ty: IrType) {
        if ty == IrType::F64 {
            let swap = matches!(op, IrCmpOp::Slt | IrCmpOp::Ult | IrCmpOp::Sle | IrCmpOp::Ule);
            let (first, second) = if swap { (lhs, rhs) } else { (rhs, lhs) };
            self.emit_f64_load_to_x87(first);
            self.emit_f64_load_to_x87(second);
            self.state.emit("    fucomip %st(1), %st");
            self.state.emit("    fstp %st(0)");

            match op {
                IrCmpOp::Eq => {
                    self.state.emit("    setnp %al");
                    self.state.emit("    sete %cl");
                    self.state.emit("    andb %cl, %al");
                }
                IrCmpOp::Ne => {
                    self.state.emit("    setp %al");
                    self.state.emit("    setne %cl");
                    self.state.emit("    orb %cl, %al");
                }
                IrCmpOp::Slt | IrCmpOp::Ult | IrCmpOp::Sgt | IrCmpOp::Ugt => {
                    self.state.emit("    seta %al");
                }
                IrCmpOp::Sle | IrCmpOp::Ule | IrCmpOp::Sge | IrCmpOp::Uge => {
                    self.state.emit("    setae %al");
                }
            }
            self.state.emit("    movzbl %al, %eax");
            self.state.reg_cache.invalidate_acc();
            self.store_eax_to(dest);
            return;
        }
        // F32: Use SSE for float comparisons
        let swap_operands = matches!(op, IrCmpOp::Slt | IrCmpOp::Ult | IrCmpOp::Sle | IrCmpOp::Ule);
        let (first, second) = if swap_operands { (rhs, lhs) } else { (lhs, rhs) };

        self.operand_to_eax(first);
        self.state.emit("    movd %eax, %xmm0");
        self.operand_to_ecx(second);
        self.state.emit("    movd %ecx, %xmm1");
        self.state.emit("    ucomiss %xmm1, %xmm0");

        match op {
            IrCmpOp::Eq => {
                self.state.emit("    setnp %al");
                self.state.emit("    sete %cl");
                self.state.emit("    andb %cl, %al");
            }
            IrCmpOp::Ne => {
                self.state.emit("    setp %al");
                self.state.emit("    setne %cl");
                self.state.emit("    orb %cl, %al");
            }
            IrCmpOp::Slt | IrCmpOp::Ult | IrCmpOp::Sgt | IrCmpOp::Ugt => {
                self.state.emit("    seta %al");
            }
            IrCmpOp::Sle | IrCmpOp::Ule | IrCmpOp::Sge | IrCmpOp::Uge => {
                self.state.emit("    setae %al");
            }
        }
        self.state.emit("    movzbl %al, %eax");
        self.state.reg_cache.invalidate_acc();
        self.store_eax_to(dest);
    }

    pub(super) fn emit_f128_cmp_impl(&mut self, dest: &Value, op: IrCmpOp, lhs: &Operand, rhs: &Operand) {
        let swap = matches!(op, IrCmpOp::Slt | IrCmpOp::Ult | IrCmpOp::Sle | IrCmpOp::Ule);
        let (first, second) = if swap { (lhs, rhs) } else { (rhs, lhs) };
        self.emit_f128_load_to_x87(first);
        self.emit_f128_load_to_x87(second);
        self.state.emit("    fucomip %st(1), %st");
        self.state.emit("    fstp %st(0)");

        match op {
            IrCmpOp::Eq => {
                self.state.emit("    setnp %al");
                self.state.emit("    sete %cl");
                self.state.emit("    andb %cl, %al");
            }
            IrCmpOp::Ne => {
                self.state.emit("    setp %al");
                self.state.emit("    setne %cl");
                self.state.emit("    orb %cl, %al");
            }
            IrCmpOp::Slt | IrCmpOp::Ult | IrCmpOp::Sgt | IrCmpOp::Ugt => {
                self.state.emit("    seta %al");
            }
            IrCmpOp::Sle | IrCmpOp::Ule | IrCmpOp::Sge | IrCmpOp::Uge => {
                self.state.emit("    setae %al");
            }
        }
        self.state.emit("    movzbl %al, %eax");
        self.state.reg_cache.invalidate_acc();
        self.store_eax_to(dest);
    }

    /// Normalize a compare immediate to the OPERAND's width and signedness.
    ///
    /// lhs reaches %eax via movzbl/movswl/movsbl (operand_to_eax extends
    /// sub-int loads to 32 bits), so the 32-bit compare is exact ONLY when
    /// the immediate is represented the same way the extension represents
    /// the operand. `(unsigned char)255 == (char)-1` miscompiled to
    /// `cmpl $-1, %eax` against a zero-extended 255 (i686-only; x86-64 uses
    /// width-matched cmpb). Truncate to the type's width, then zero- or
    /// sign-extend by the type's signedness — matching operand_to_eax.
    fn normalize_cmp_imm(imm: i64, ty: IrType) -> i64 {
        match ty {
            IrType::I8 => imm as i8 as i64,
            IrType::U8 => imm as u8 as i64,
            IrType::I16 => imm as i16 as i64,
            IrType::U16 => imm as u16 as i64,
            // I32/U32/Ptr compare as full 32-bit; U32 immediates keep their
            // bit pattern under the i32 print (cmpl is width-exact).
            _ => imm as i32 as i64,
        }
    }

    pub(super) fn emit_int_cmp_impl(&mut self, dest: &Value, op: IrCmpOp, lhs: &Operand, rhs: &Operand, ty: IrType) {
        // Accumulator-bypass: compare a register-resident LHS in place
        // (`testl %R,%R` / `cmpl $imm,%R` / `cmpl %R,%L`) instead of the
        // `movl %R,%eax; cmpl …, %eax` round-trip.
        if !self.emit_int_cmp_flags_direct(lhs, rhs, ty) {
            self.operand_to_eax(lhs);
            // Constant rhs: compare against the immediate directly instead of
            // staging it in %ecx. `movl $C,%ecx; cmpl %ecx,%eax` is 8 bytes where
            // `cmpl $C,%eax` is 3..6 and `testl %eax,%eax` is 2 (flags are
            // identical for ALL Jcc: eax-0 and eax&eax both clear CF/OF and set
            // SF/ZF/PF from eax). GCC emits exactly these forms. This also keeps
            // %ecx untouched, which the register allocator relies on when it
            // places a value in %ecx across a constant compare.
            if let Some(imm) = Self::const_as_imm32(rhs) {
                let imm = Self::normalize_cmp_imm(imm, ty);
                if imm == 0 {
                    self.state.emit("    testl %eax, %eax");
                } else {
                    emit!(self.state, "    cmpl ${}, %eax", imm);
                }
            } else {
                self.operand_to_ecx(rhs);
                self.state.emit("    cmpl %ecx, %eax");
            }
        }

        let set_instr = match op {
            IrCmpOp::Eq => "sete",
            IrCmpOp::Ne => "setne",
            IrCmpOp::Slt => "setl",
            IrCmpOp::Sle => "setle",
            IrCmpOp::Sgt => "setg",
            IrCmpOp::Sge => "setge",
            IrCmpOp::Ult => "setb",
            IrCmpOp::Ule => "setbe",
            IrCmpOp::Ugt => "seta",
            IrCmpOp::Uge => "setae",
        };
        emit!(self.state, "    {} %al", set_instr);
        self.state.emit("    movzbl %al, %eax");
        self.state.reg_cache.invalidate_acc();
        self.store_eax_to(dest);
    }

    pub(super) fn emit_fused_cmp_branch_impl(
        &mut self,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
        true_label: &str,
        false_label: &str,
    ) {
        // Accumulator-bypass: compare a register-resident LHS in place; the
        // direct flag emitter leaves %eax untouched (see emit_int_cmp_impl).
        if !self.emit_int_cmp_flags_direct(lhs, rhs, ty) {
            self.operand_to_eax(lhs);
            // Same immediate forms as emit_int_cmp_impl (see there for the
            // testl flags argument); keeps %ecx clean across fused compares.
            if let Some(imm) = Self::const_as_imm32(rhs) {
                let imm = Self::normalize_cmp_imm(imm, ty);
                if imm == 0 {
                    self.state.emit("    testl %eax, %eax");
                } else {
                    emit!(self.state, "    cmpl ${}, %eax", imm);
                }
            } else {
                self.operand_to_ecx(rhs);
                self.state.emit("    cmpl %ecx, %eax");
            }
        }

        let jcc = match op {
            IrCmpOp::Eq  => "je",
            IrCmpOp::Ne  => "jne",
            IrCmpOp::Slt => "jl",
            IrCmpOp::Sle => "jle",
            IrCmpOp::Sgt => "jg",
            IrCmpOp::Sge => "jge",
            IrCmpOp::Ult => "jb",
            IrCmpOp::Ule => "jbe",
            IrCmpOp::Ugt => "ja",
            IrCmpOp::Uge => "jae",
        };
        emit!(self.state, "    {} {}", jcc, true_label);
        emit!(self.state, "    jmp {}", false_label);
        self.state.reg_cache.invalidate_all();
    }

    pub(super) fn emit_select_impl(&mut self, dest: &Value, cond: &Operand, true_val: &Operand, false_val: &Operand, ty: IrType) {
        use crate::ir::reexports::IrConst;
        // Constant-fold wide conditions at compile time
        match cond {
            Operand::Const(IrConst::I64(v)) => {
                self.emit_copy_value(dest, if *v != 0 { true_val } else { false_val });
                return;
            }
            Operand::Const(IrConst::F64(fval)) => {
                self.emit_copy_value(dest, if *fval != 0.0 { true_val } else { false_val });
                return;
            }
            _ => {}
        }

        let cond_is_wide = matches!(cond, Operand::Value(v) if self.state.is_wide_value(v.0));
        let result_is_wide = matches!(ty, IrType::F64 | IrType::I64 | IrType::U64);

        if !cond_is_wide && !result_is_wide {
            let label_id = self.state.next_label_id();
            let true_label = format!(".Lsel_true_{}", label_id);
            let end_label = format!(".Lsel_end_{}", label_id);
            self.emit_load_operand(cond);
            self.emit_branch_nonzero(&true_label);
            // When dest is register-allocated, materialize each arm DIRECTLY
            // into that register (movl slot,%esi) instead of the
            // load-to-%eax + movl %eax,%esi detour. The peephole cannot fuse
            // these afterwards because the pair sits right before a label
            // (deadness unprovable across the barrier), so the win must
            // happen here. Falls back to the accumulator path per-arm.
            let dest_phys = self.get_phys_reg_for_value(dest.0);
            if let Some(d) = dest_phys {
                if !self.emit_load_direct_to_phys_reg(false_val, d) {
                    self.emit_load_operand(false_val);
                    self.emit_store_result(dest);
                }
            } else {
                self.emit_load_operand(false_val);
                self.emit_store_result(dest);
            }
            self.emit_branch(&end_label);
            self.state.emit_fmt(format_args!("{}:", true_label));
            if let Some(d) = dest_phys {
                if !self.emit_load_direct_to_phys_reg(true_val, d) {
                    self.emit_load_operand(true_val);
                    self.emit_store_result(dest);
                }
            } else {
                self.emit_load_operand(true_val);
                self.emit_store_result(dest);
            }
            self.state.emit_fmt(format_args!("{}:", end_label));
            self.state.reg_cache.invalidate_acc();
            return;
        }

        let label_id = self.state.next_label_id();
        let true_label = format!(".Lsel_true_{}", label_id);
        let end_label = format!(".Lsel_end_{}", label_id);

        if cond_is_wide {
            if let Operand::Value(v) = cond {
                self.emit_wide_value_to_eax_ored(v.0);
                self.state.reg_cache.invalidate_acc();
            }
        } else {
            self.operand_to_eax(cond);
        }

        self.emit_branch_nonzero(&true_label);

        self.emit_copy_value(dest, false_val);
        self.emit_branch(&end_label);

        self.state.emit_fmt(format_args!("{}:", true_label));
        self.emit_copy_value(dest, true_val);

        self.state.emit_fmt(format_args!("{}:", end_label));
    }
}
