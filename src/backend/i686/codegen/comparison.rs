//! I686Codegen: comparison operations (float, int, fused branches, select).

use super::emit::{phys_reg_name, I686Codegen};
use crate::backend::state::SlotAddr;
use crate::backend::traits::ArchCodegen;
use crate::common::types::IrType;
use crate::emit;
use crate::ir::reexports::{IrCmpOp, Operand, Value};

fn reg_for_type(reg: &str, ty: IrType) -> Option<&'static str> {
    match ty {
        IrType::I8 | IrType::U8 => match reg {
            "eax" => Some("al"),
            "ebx" => Some("bl"),
            "ecx" => Some("cl"),
            "edx" => Some("dl"),
            _ => None,
        },
        IrType::I16 | IrType::U16 => match reg {
            "eax" => Some("ax"),
            "ebx" => Some("bx"),
            "ecx" => Some("cx"),
            "edx" => Some("dx"),
            "esi" => Some("si"),
            "edi" => Some("di"),
            "ebp" => Some("bp"),
            _ => None,
        },
        _ => match reg {
            "eax" => Some("eax"),
            "ebx" => Some("ebx"),
            "ecx" => Some("ecx"),
            "edx" => Some("edx"),
            "esi" => Some("esi"),
            "edi" => Some("edi"),
            "ebp" => Some("ebp"),
            _ => None,
        },
    }
}

fn cmp_mnemonic(ty: IrType) -> &'static str {
    match ty {
        IrType::I8 | IrType::U8 => "cmpb",
        IrType::I16 | IrType::U16 => "cmpw",
        _ => "cmpl",
    }
}

fn test_mnemonic(ty: IrType) -> &'static str {
    match ty {
        IrType::I8 | IrType::U8 => "testb",
        IrType::I16 | IrType::U16 => "testw",
        _ => "testl",
    }
}

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
        let Some(lreg_32) = self.cmp_operand_reg(lhs) else {
            return false;
        };
        let Some(lreg) = reg_for_type(lreg_32, ty) else {
            return false;
        };
        let cmp_op = cmp_mnemonic(ty);
        let test_op = test_mnemonic(ty);

        if let Some(imm) = Self::const_as_imm32(rhs) {
            let imm = Self::normalize_cmp_imm(imm, ty);
            if imm == 0 {
                emit!(self.state, "    {} %{}, %{}", test_op, lreg, lreg);
            } else {
                emit!(self.state, "    {} ${}, %{}", cmp_op, imm, lreg);
            }
        } else if let Some(rreg_32) = self.cmp_operand_reg(rhs) {
            let Some(rreg) = reg_for_type(rreg_32, ty) else {
                return false;
            };
            emit!(self.state, "    {} %{}, %{}", cmp_op, rreg, lreg);
        } else {
            if lreg_32 == "ecx" {
                return false; // staging rhs into %ecx would clobber lhs
            }
            let Some(ecx_sub) = reg_for_type("ecx", ty) else {
                return false;
            };
            self.operand_to_ecx(rhs);
            emit!(self.state, "    {} %{}, %{}", cmp_op, ecx_sub, lreg);
        }
        true
    }

    pub(super) fn emit_float_cmp_impl(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) {
        if ty == IrType::F64 {
            let swap = matches!(
                op,
                IrCmpOp::Slt | IrCmpOp::Ult | IrCmpOp::Sle | IrCmpOp::Ule
            );
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
        let swap_operands = matches!(
            op,
            IrCmpOp::Slt | IrCmpOp::Ult | IrCmpOp::Sle | IrCmpOp::Ule
        );
        let (first, second) = if swap_operands {
            (rhs, lhs)
        } else {
            (lhs, rhs)
        };

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

    pub(super) fn emit_f128_cmp_impl(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
    ) {
        let swap = matches!(
            op,
            IrCmpOp::Slt | IrCmpOp::Ult | IrCmpOp::Sle | IrCmpOp::Ule
        );
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

    /// Emit `cmp{b,w,l} $imm, (ptr)` staging the pointer exactly like an
    /// indirect load. Returns false when the pointer has no resolvable
    /// slot/register home (the caller re-materializes the skipped load).
    pub(super) fn emit_mem_cmp_imm(&mut self, ptr: u32, ty: IrType, imm: i64) -> bool {
        let mnemonic = match ty {
            IrType::I8 | IrType::U8 => "cmpb",
            IrType::I16 | IrType::U16 => "cmpw",
            IrType::I32 | IrType::U32 | IrType::Ptr => "cmpl",
            _ => return false,
        };
        let Some(addr) = self.state.resolve_slot_addr(ptr) else {
            return false;
        };
        match addr {
            SlotAddr::Direct(slot) => {
                let sr = self.slot_ref(slot);
                emit!(self.state, "    {} ${}, {}", mnemonic, imm, sr);
            }
            SlotAddr::Indirect(slot) => {
                self.emit_load_ptr_from_slot(slot, ptr);
                emit!(self.state, "    {} ${}, (%ecx)", mnemonic, imm);
            }
            SlotAddr::OverAligned(slot, id) => {
                self.emit_alloca_aligned_addr(slot, id);
                emit!(self.state, "    {} ${}, (%ecx)", mnemonic, imm);
            }
            SlotAddr::Reg(reg) => emit!(
                self.state,
                "    {} ${}, (%{})",
                mnemonic,
                imm,
                phys_reg_name(reg)
            ),
        }
        true
    }

    /// Consume a pending load→memory-compare fold (see
    /// `generation::detect_load_cmp_mem_fold`): when `lhs` is a Load whose
    /// single use is this `Eq`/`Ne`-vs-imm compare, emit
    /// `cmp{b,w,l} $imm,(mem)` and report success so the caller skips the
    /// register-resident compare.
    fn take_load_cmp_fold(&mut self, lhs: &Operand, rhs: &Operand, op: IrCmpOp) -> bool {
        let Operand::Value(lv) = lhs else {
            return false;
        };
        let Some((ptr, lty, imm)) = self.state.pending_load_cmp.remove(&lv.0) else {
            return false;
        };
        if !matches!(op, IrCmpOp::Eq | IrCmpOp::Ne)
            || crate::backend::generation::cmp_fold_imm(rhs, lty) != Some(imm)
        {
            // Detection guarantees this cannot happen; re-materialize the load
            // so the compare sees a real value instead of garbage.
            crate::backend::traits::emit_load_default(self, &Value(lv.0), &Value(ptr), lty);
            return false;
        }
        if !self.emit_mem_cmp_imm(ptr, lty, imm) {
            // Pointer is not resolvable (e.g. a global address): fall back to
            // the materialized load + normal compare.
            crate::backend::traits::emit_load_default(self, &Value(lv.0), &Value(ptr), lty);
            return false;
        }
        true
    }

    pub(super) fn emit_int_cmp_impl(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) {
        // Load→memory-compare fold: `movsbl (mem),%r; testl %r,%r` collapses
        // to `cmpb $0,(mem)`, freeing the register and the push/pop pair.
        let folded = self.take_load_cmp_fold(lhs, rhs, op);
        // Accumulator-bypass: compare a register-resident LHS in place
        // (`testl %R,%R` / `cmpl $imm,%R` / `cmpl %R,%L`) instead of the
        // `movl %R,%eax; cmpl …, %eax` round-trip.
        if !folded && !self.emit_int_cmp_flags_direct(lhs, rhs, ty) {
            self.operand_to_eax(lhs);
            let eax_sub = reg_for_type("eax", ty).unwrap_or("eax");
            let cmp_op = cmp_mnemonic(ty);
            let test_op = test_mnemonic(ty);

            // Constant rhs: compare against the immediate directly instead of
            // staging it in %ecx.
            if let Some(imm) = Self::const_as_imm32(rhs) {
                let imm = Self::normalize_cmp_imm(imm, ty);
                if imm == 0 {
                    emit!(self.state, "    {} %{}, %{}", test_op, eax_sub, eax_sub);
                } else {
                    emit!(self.state, "    {} ${}, %{}", cmp_op, imm, eax_sub);
                }
            } else if let Operand::Value(rv) = rhs {
                // A REGISTER-resident rhs compares in place;
                // the read is identical to the `movl %reg,%ecx` staging it
                // replaces. Slot values keep staging (deferred-slot safety;
                // the peephole folds them post-materialization).
                let rreg = self
                    .direct_reg_src_ref(rv)
                    .and_then(|r| reg_for_type(r.trim_start_matches('%'), ty));
                match rreg {
                    Some(r) => emit!(self.state, "    {} %{}, %{}", cmp_op, r, eax_sub),
                    None => {
                        let ecx_sub = reg_for_type("ecx", ty).unwrap_or("ecx");
                        self.operand_to_ecx(rhs);
                        emit!(self.state, "    {} %{}, %{}", cmp_op, ecx_sub, eax_sub);
                    }
                }
            } else {
                let ecx_sub = reg_for_type("ecx", ty).unwrap_or("ecx");
                self.operand_to_ecx(rhs);
                emit!(self.state, "    {} %{}, %{}", cmp_op, ecx_sub, eax_sub);
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

    /// Fused BitTest→CondBranch: `btl index, base; jc/jnc`. CF holds the
    /// selected bit; nothing is materialized. Mirrors the x86-64 hook —
    /// only 32-bit masks reach this target (legality gates in simplify /
    /// set_membership).
    pub(super) fn emit_fused_bit_test_branch_blocks_impl(
        &mut self,
        base: &Operand,
        index: &Operand,
        _ty: IrType,
        true_block: crate::ir::reexports::BlockId,
        false_block: crate::ir::reexports::BlockId,
    ) {
        // Stage the BASE (mask) — register home preferred, else %eax.
        let base_ref: String = match base {
            Operand::Value(v) => match self.direct_reg_src_ref(v) {
                Some(r) => r,
                None => {
                    self.operand_to_eax(base);
                    "%eax".to_string()
                }
            },
            Operand::Const(_) => {
                self.operand_to_eax(base);
                "%eax".to_string()
            }
        };
        let const_index = match index {
            Operand::Const(c) => c.to_i64().filter(|v| *v >= 0),
            _ => None,
        };
        if let Some(imm) = const_index {
            let bit = (imm as u32) % 32;
            emit!(self.state, "    btl ${}, {}", bit, base_ref);
        } else {
            // Variable index: register home or %ecx staging. %ecx is safe —
            // the base staging above uses %eax only.
            let idx_ref: String = match index {
                Operand::Value(v) => match self.direct_reg_src_ref(v) {
                    Some(r) => r,
                    None => {
                        self.operand_to_ecx(index);
                        "%ecx".to_string()
                    }
                },
                _ => {
                    self.operand_to_ecx(index);
                    "%ecx".to_string()
                }
            };
            emit!(self.state, "    btl {}, {}", idx_ref, base_ref);
        }
        let true_label = true_block.as_label();
        let false_label = false_block.as_label();
        emit!(self.state, "    jc {}", true_label);
        emit!(self.state, "    jmp {}", false_label);
        self.state.reg_cache.invalidate_all();
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
        // Load→memory-compare fold (see emit_int_cmp_impl): the branch path
        // fuses the Cmp into the CondBranch terminator, so the fold must be
        // taken here too or the skipped load never materializes.
        let folded = self.take_load_cmp_fold(lhs, rhs, op);
        // Accumulator-bypass: compare a register-resident LHS in place; the
        // direct flag emitter leaves %eax untouched (see emit_int_cmp_impl).
        if !folded && !self.emit_int_cmp_flags_direct(lhs, rhs, ty) {
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
            } else if let Operand::Value(rv) = rhs {
                // A REGISTER-resident rhs compares in place (`cmpl %reg,%eax`);
                // the read is identical to the `movl %reg,%ecx` staging it
                // replaces. Slot values keep staging (deferred-slot safety;
                // the peephole folds them post-materialization).
                match self.direct_reg_src_ref(rv) {
                    Some(r) => emit!(self.state, "    cmpl {}, %eax", r),
                    None => {
                        self.operand_to_ecx(rhs);
                        self.state.emit("    cmpl %ecx, %eax");
                    }
                }
            } else {
                self.operand_to_ecx(rhs);
                self.state.emit("    cmpl %ecx, %eax");
            }
        }

        let jcc = match op {
            IrCmpOp::Eq => "je",
            IrCmpOp::Ne => "jne",
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

    pub(super) fn emit_select_impl(
        &mut self,
        dest: &Value,
        cond: &Operand,
        true_val: &Operand,
        false_val: &Operand,
        ty: IrType,
    ) {
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
