//! ArmCodegen: comparison operations.

use super::emit::{
    arm_int_cond_code, arm_invert_cond_code, callee_saved_name, callee_saved_name_32,
    is_arm_fp_phys, ArmCodegen,
};
use crate::common::types::IrType;
use crate::ir::reexports::{IrCmpOp, Operand, Value};

impl ArmCodegen {
    pub(super) fn emit_float_cmp_impl(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) {
        if ty == IrType::F32 {
            let l = self.float_operand_reg(lhs, ty, "s0");
            let r = self.float_operand_reg(rhs, ty, "s1");
            self.state.emit_fmt(format_args!("    fcmp {}, {}", l, r));
        } else {
            let l = self.float_operand_reg(lhs, ty, "d0");
            let r = self.float_operand_reg(rhs, ty, "d1");
            self.state.emit_fmt(format_args!("    fcmp {}, {}", l, r));
        }
        let cond = match op {
            IrCmpOp::Eq => "eq",
            IrCmpOp::Ne => "ne",
            IrCmpOp::Slt | IrCmpOp::Ult => "mi",
            IrCmpOp::Sle | IrCmpOp::Ule => "ls",
            IrCmpOp::Sgt | IrCmpOp::Ugt => "gt",
            IrCmpOp::Sge | IrCmpOp::Uge => "ge",
        };
        self.state.emit_fmt(format_args!("    cset x0, {}", cond));
        self.store_x0_to(dest);
    }

    pub(super) fn emit_f128_cmp_impl(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
    ) {
        crate::backend::f128_softfloat::f128_cmp(self, dest, op, lhs, rhs);
    }

    pub(super) fn emit_int_cmp_impl(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) {
        self.emit_int_cmp_insn(lhs, rhs, ty);
        let cond = arm_int_cond_code(op);
        self.state.emit_fmt(format_args!("    cset x0, {}", cond));
        self.store_x0_to(dest);
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
        if ty.is_float() {
            // fcmp + conditional branch — no cset boolean materialization.
            // Condition codes match emit_float_cmp_impl exactly (mi/ls for
            // less-than shapes so unordered compares stay false), so branching
            // on flags is identical to branching on the materialized boolean.
            let (l, r) = if ty == IrType::F32 {
                (
                    self.float_operand_reg(lhs, ty, "s0"),
                    self.float_operand_reg(rhs, ty, "s1"),
                )
            } else {
                (
                    self.float_operand_reg(lhs, ty, "d0"),
                    self.float_operand_reg(rhs, ty, "d1"),
                )
            };
            self.state.emit_fmt(format_args!("    fcmp {}, {}", l, r));
            let cond = match op {
                IrCmpOp::Eq => "eq",
                IrCmpOp::Ne => "ne",
                IrCmpOp::Slt | IrCmpOp::Ult => "mi",
                IrCmpOp::Sle | IrCmpOp::Ule => "ls",
                IrCmpOp::Sgt | IrCmpOp::Ugt => "gt",
                IrCmpOp::Sge | IrCmpOp::Uge => "ge",
            };
            let inv_cc = arm_invert_cond_code(cond);
            let skip = self.state.fresh_label("skip");
            self.state
                .emit_fmt(format_args!("    b.{} {}", inv_cc, skip));
            self.state.emit_fmt(format_args!("    b {}", true_label));
            self.state.emit_fmt(format_args!("{}:", skip));
            self.state.emit_fmt(format_args!("    b {}", false_label));
            self.state.reg_cache.invalidate_all();
            return;
        }
        self.emit_int_cmp_insn(lhs, rhs, ty);
        let cc = arm_int_cond_code(op);
        let inv_cc = arm_invert_cond_code(cc);
        let skip = self.state.fresh_label("skip");
        self.state
            .emit_fmt(format_args!("    b.{} {}", inv_cc, skip));
        self.state.emit_fmt(format_args!("    b {}", true_label));
        self.state.emit_fmt(format_args!("{}:", skip));
        self.state.emit_fmt(format_args!("    b {}", false_label));
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
        // Register-direct Select (levkropp 8a052b9a, adapted onto the current
        // fused-select helper). Staging every arm through x0/x1/x2 added ~5
        // moves per if-converted diamond. Reuse `select_arm_reg` so the
        // fused and unfused paths cannot drift.
        let use_32bit = ty.size() <= 4;
        let f_name = self.select_arm_reg(false_val, "x1", use_32bit);
        let t_name = self.select_arm_reg(true_val, "x2", use_32bit);
        // Compare the condition in place. A 32-bit producer zero-extends into
        // the X register on A64, so a 64-bit `cmp Rn, #0` is width-correct.
        match cond {
            Operand::Value(v) => {
                if let Some(phys) = self
                    .reg_assignments
                    .get(&v.0)
                    .copied()
                    .filter(|r| !is_arm_fp_phys(*r))
                {
                    let c = callee_saved_name(phys);
                    self.state.emit_fmt(format_args!("    cmp {}, #0", c));
                } else {
                    self.operand_to_x0(cond);
                    self.state.emit("    cmp x0, #0");
                }
            }
            _ => {
                self.operand_to_x0(cond);
                self.state.emit("    cmp x0, #0");
            }
        }
        if let Some(dp) = self.dest_reg(dest).filter(|r| !is_arm_fp_phys(*r)) {
            let d = if use_32bit {
                callee_saved_name_32(dp)
            } else {
                callee_saved_name(dp)
            };
            self.state
                .emit_fmt(format_args!("    csel {}, {}, {}, ne", d, t_name, f_name));
        } else {
            self.state
                .emit_fmt(format_args!("    csel x0, {}, {}, ne", t_name, f_name));
            self.store_x0_to(dest);
        }
        self.state.reg_cache.invalidate_acc();
    }

    /// Fused integer compare-and-select: `cmp lhs, rhs` + `csel dest, tv, fv, cc`.
    /// The Cmp's boolean result was used only by this select, so the cset
    /// materialization is skipped entirely. Register-allocated select arms are
    /// used in place (no x0 round-trips), keeping the loop-carried dependency
    /// chain short (cmp; csel) for if-converted hot loops.
    pub(super) fn emit_fused_cmp_select_impl(
        &mut self,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        cmp_ty: IrType,
        true_val: &Operand,
        false_val: &Operand,
        dest: &Value,
        sel_ty: IrType,
    ) {
        self.emit_int_cmp_insn(lhs, rhs, cmp_ty);
        let cc = arm_int_cond_code(op);
        let use_32bit = sel_ty.size() <= 4;

        // Stage the select arms before the dest: x1/x2 scratch are never
        // allocator-assigned, so they cannot collide with phys regs.
        let f_name = self.select_arm_reg(false_val, "x1", use_32bit);
        let t_name = self.select_arm_reg(true_val, "x2", use_32bit);

        if let Some(dp) = self.dest_reg(dest).filter(|r| !is_arm_fp_phys(*r)) {
            let d = if use_32bit {
                callee_saved_name_32(dp)
            } else {
                callee_saved_name(dp)
            };
            self.state.emit_fmt(format_args!(
                "    csel {}, {}, {}, {}",
                d, t_name, f_name, cc
            ));
        } else {
            self.state
                .emit_fmt(format_args!("    csel x0, {}, {}, {}", t_name, f_name, cc));
            self.store_x0_to(dest);
        }
        self.state.reg_cache.invalidate_acc();
    }

    /// Conditional increment from a materialized boolean condition.
    pub(super) fn emit_conditional_increment_select_impl(
        &mut self,
        dest: &Value,
        cond: &Operand,
        base: &Operand,
        increment_on_true: bool,
        ty: IrType,
    ) {
        match cond {
            Operand::Value(value) => {
                if let Some(phys) = self
                    .reg_assignments
                    .get(&value.0)
                    .copied()
                    .filter(|reg| !is_arm_fp_phys(*reg))
                {
                    self.state
                        .emit_fmt(format_args!("    cmp {}, #0", callee_saved_name(phys)));
                } else {
                    self.operand_to_x0(cond);
                    self.state.emit("    cmp x0, #0");
                }
            }
            Operand::Const(_) => {
                self.operand_to_x0(cond);
                self.state.emit("    cmp x0, #0");
            }
        }
        // `ne` is the Select's true condition. CSINC increments when its own
        // condition is false, so increment-on-true uses `eq`.
        let csinc_cc = if increment_on_true { "eq" } else { "ne" };
        self.emit_csinc_result(dest, base, ty, csinc_cc);
    }

    /// Fused compare plus conditional increment. Generic SSA analysis proved
    /// that the omitted Add has one use and the opposite Select arm is the
    /// same base.
    pub(super) fn emit_fused_cmp_conditional_increment_select_impl(
        &mut self,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        cmp_ty: IrType,
        base: &Operand,
        increment_on_true: bool,
        dest: &Value,
        sel_ty: IrType,
    ) {
        self.emit_int_cmp_insn(lhs, rhs, cmp_ty);
        let cmp_cc = arm_int_cond_code(op);
        let csinc_cc = if increment_on_true {
            arm_invert_cond_code(cmp_cc)
        } else {
            cmp_cc
        };
        self.emit_csinc_result(dest, base, sel_ty, csinc_cc);
    }

    /// Emit CSINC after flags are ready. `condition` is the condition under
    /// which the unincremented base is selected.
    fn emit_csinc_result(&mut self, dest: &Value, base: &Operand, ty: IrType, condition: &str) {
        let use_32bit = ty.size() <= 4;
        let base_reg = self.select_arm_reg(base, "x1", use_32bit);
        if let Some(phys) = self.dest_reg(dest).filter(|reg| !is_arm_fp_phys(*reg)) {
            let dest_reg = if use_32bit {
                callee_saved_name_32(phys)
            } else {
                callee_saved_name(phys)
            };
            self.state.emit_fmt(format_args!(
                "    csinc {}, {}, {}, {}",
                dest_reg, base_reg, base_reg, condition
            ));
        } else {
            let accumulator = if use_32bit { "w0" } else { "x0" };
            self.state.emit_fmt(format_args!(
                "    csinc {}, {}, {}, {}",
                accumulator, base_reg, base_reg, condition
            ));
            self.store_x0_to(dest);
        }
        self.state.reg_cache.invalidate_acc();
    }

    /// Resolve a Select arm to a register name: its assigned phys reg when
    /// available, otherwise loaded into the given scratch register.
    fn select_arm_reg(&mut self, op: &Operand, scratch_x: &str, use_32bit: bool) -> String {
        if let Operand::Value(v) = op {
            if let Some(phys) = self.reg_assignments.get(&v.0).copied() {
                if !is_arm_fp_phys(phys) {
                    return (if use_32bit {
                        callee_saved_name_32(phys)
                    } else {
                        callee_saved_name(phys)
                    })
                    .to_string();
                }
            }
        }
        // Width must match on BOTH sides of the mov: `mov w2, x0` is not a
        // valid A64 form (operand mismatch at assembly time — huft_build
        // repro). 32-bit selects copy w0 into a w-scratch.
        let scratch = if use_32bit {
            scratch_x.replacen('x', "w", 1)
        } else {
            scratch_x.to_string()
        };
        let src = if use_32bit { "w0" } else { "x0" };
        self.operand_to_x0(op);
        self.state
            .emit_fmt(format_args!("    mov {}, {}", scratch, src));
        scratch
    }
}
