//! ArmCodegen: comparison operations.

use super::emit::{
    ArmCodegen, arm_fp_name, arm_int_cond_code, arm_invert_cond_code, callee_saved_name,
    callee_saved_name_32, is_arm_fp_phys,
};
use crate::backend::regalloc::PhysReg;
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

    /// Fused BitTest -> CondBranch: `tst base, #(1 << k)` for an immediate
    /// index, `lsr w0, base, index` + `tst w0, #1` for a variable one, then
    /// the conditional branch -- no 0/1 boolean is materialized.  Narrow
    /// widths reduce the index modulo their own width first (the base's
    /// register holds the canonical narrow value, whose low bits are the
    /// value's bits).  `tbz`/`tbnz` would save the `tst`, but they reach only
    /// +-32 KiB and this backend has no branch relaxation (the assembler
    /// rejects an out-of-range branch), so the +-1 MiB `b.cond` that `cbz`
    /// and every fused compare already rely on is used.
    pub(super) fn emit_fused_bit_test_branch_blocks_impl(
        &mut self,
        base: &Operand,
        index: &Operand,
        ty: IrType,
        true_block: crate::ir::reexports::BlockId,
        false_block: crate::ir::reexports::BlockId,
    ) {
        let width = (ty.size() * 8) as i64;
        debug_assert!(matches!(width, 8 | 16 | 32 | 64));
        let r = if width == 64 { 'x' } else { 'w' };
        let reg_text = |v: &Operand, this: &Self| match v {
            Operand::Value(v) => this
                .value_reg_name_pub(v)
                .map(|(x, w)| if width == 64 { x } else { w }),
            Operand::Const(_) => None,
        };
        let base_src = match reg_text(base, self) {
            Some(name) => name.to_string(),
            None => {
                self.operand_to_x0(base);
                self.state.emit("    mov x1, x0");
                format!("{r}1")
            }
        };
        match Self::const_as_imm12(index) {
            Some(imm) => {
                let bit = (imm as i64).rem_euclid(width);
                self.state
                    .emit_fmt(format_args!("    tst {base_src}, #{:#x}", 1u64 << bit));
            }
            None => {
                let amount = match reg_text(index, self) {
                    Some(name) if width >= 32 => name.to_string(),
                    Some(name) => {
                        self.state
                            .emit_fmt(format_args!("    and w0, {name}, #{}", width - 1));
                        "w0".to_string()
                    }
                    None => {
                        self.operand_to_x0(index);
                        if width < 32 {
                            self.state
                                .emit_fmt(format_args!("    and w0, w0, #{}", width - 1));
                        }
                        format!("{r}0")
                    }
                };
                self.state
                    .emit_fmt(format_args!("    lsr {r}0, {base_src}, {amount}"));
                self.state.emit_fmt(format_args!("    tst {r}0, #1"));
            }
        }
        let skip = self.state.fresh_label("skip");
        self.state.emit_fmt(format_args!("    b.eq {skip}"));
        self.state
            .emit_fmt(format_args!("    b {}", true_block.as_label()));
        self.state.emit_fmt(format_args!("{skip}:"));
        self.state
            .emit_fmt(format_args!("    b {}", false_block.as_label()));
        self.state.reg_cache.invalidate_all();
    }

    /// `dest = BitTest(base, index) == 0` (`bit_clear`) or `!= 0`, computed
    /// straight into the destination's GP home: the test itself
    /// ([`Self::emit_bit_test_into`]) and, for `== 0`, one `eor #1` -- no
    /// flags, no `cset`, no copy out of x0.  Declined for narrow widths and a
    /// homeless destination (the caller then emits both instructions).
    pub(super) fn emit_fused_bit_test_cmp_impl(
        &mut self,
        dest: &Value,
        base: &Operand,
        index: &Operand,
        ty: IrType,
        bit_clear: bool,
    ) -> bool {
        if !matches!(ty, IrType::I32 | IrType::U32 | IrType::I64 | IrType::U64) {
            return false;
        }
        let Some(d) = self.dest_reg(dest).filter(|r| !is_arm_fp_phys(*r)) else {
            return false;
        };
        let use_32bit = ty.size() == 4;
        if !self.emit_bit_test_into(d, base, index, use_32bit) {
            return false;
        }
        if bit_clear {
            // The comparison's own result is a 32-bit 0/1 whatever the test's
            // width, and the `ubfx`/`and #1` above already zeroed the rest.
            let w = callee_saved_name_32(d);
            self.state.emit_fmt(format_args!("    eor {w}, {w}, #1"));
        }
        self.state.reg_cache.invalidate_acc();
        true
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
        if self.fp_select_profitable(dest, true_val, false_val, ty) {
            let (t_name, f_name) = self.fp_select_arms(true_val, false_val, ty);
            self.emit_select_cond_cmp(cond);
            self.emit_fcsel_result(dest, &t_name, &f_name, ty, "ne");
            return;
        }
        let use_32bit = ty.size() <= 4;
        let f_name = self.select_arm_reg(false_val, "x1", use_32bit);
        let t_name = self.select_arm_reg(true_val, "x2", use_32bit);
        self.emit_select_cond_cmp(cond);
        if let Some(dp) = self.dest_reg(dest).filter(|r| !is_arm_fp_phys(*r)) {
            let d = if use_32bit {
                callee_saved_name_32(dp)
            } else {
                callee_saved_name(dp)
            };
            self.state
                .emit_fmt(format_args!("    csel {}, {}, {}, ne", d, t_name, f_name));
        } else {
            let accumulator = if use_32bit { "w0" } else { "x0" };
            self.state.emit_fmt(format_args!(
                "    csel {}, {}, {}, ne",
                accumulator, t_name, f_name
            ));
            // Keep the accumulator entry when `dest` has no durable
            // home (see store_x0_to_and_release) and skip the shared
            // invalidate below, which would orphan the value.
            self.store_x0_to_and_release(dest);
            return;
        }
        self.state.reg_cache.invalidate_acc();
    }

    /// Set Z from "`cond` == 0" for a Select on a materialized boolean. A
    /// condition in its GP home is tested in place; the test reads exactly
    /// the condition's width (`cond_is_w_view`, `emit_zero_test_reg`).
    fn emit_select_cond_cmp(&mut self, cond: &Operand) {
        self.emit_zero_test(cond);
    }

    #[cfg(test)]
    pub(super) fn emit_select_cond_cmp_for_test(&mut self, cond: &Operand) {
        self.emit_select_cond_cmp(cond);
    }

    /// Set Z from "`cond` == 0", reading exactly the condition's width.
    fn emit_zero_test(&mut self, cond: &Operand) {
        if let Some(phys) = self.operand_reg(cond).filter(|r| !is_arm_fp_phys(*r)) {
            self.emit_zero_test_reg(cond, phys);
        } else {
            let w_view = self.cond_is_w_view(cond);
            self.operand_to_x0(cond);
            self.state.emit(if w_view {
                "    cmp w0, #0"
            } else {
                "    cmp x0, #0"
            });
        }
    }

    /// The width-exact zero test of `cond` held in GP register `phys`. The
    /// fixed conditional-increment leaf keeps a parameter condition in its
    /// ABI register, where no ParamRef canonicalized it: AAPCS64 leaves every
    /// bit above the argument's width unspecified, so an 8- or 16-bit
    /// condition is masked by `tst` (Z is all the `cinc`/`csel` condition
    /// reads). Every other condition is canonical in its W view when it is
    /// 32 bits wide or narrower.
    fn emit_zero_test_reg(&mut self, cond: &Operand, phys: PhysReg) {
        let bytes = if self.conditional_increment_leaf {
            self.conditional_increment_leaf_condition_bytes
        } else if self.cond_is_w_view(cond) {
            4
        } else {
            8
        };
        let (x, w) = (callee_saved_name(phys), callee_saved_name_32(phys));
        match bytes {
            1 => self.state.emit_fmt(format_args!("    tst {}, #0xff", w)),
            2 => self.state.emit_fmt(format_args!("    tst {}, #0xffff", w)),
            4 => self.state.emit_fmt(format_args!("    cmp {}, #0", w)),
            _ => self.state.emit_fmt(format_args!("    cmp {}, #0", x)),
        }
    }

    /// Whether a Select of `ty` belongs in the FP domain: at least two of the
    /// result and the two arms live in FP registers. The `csel` form moves
    /// each of those through a GP register -- `double pick(double a, double
    /// b, int k) { return k ? a : b; }` was `fmov x1, d17; fmov x2, d16;
    /// cmp x4, #0; csel x0, x2, x1, ne; fmov d18, x0` -- where `fcsel` reads
    /// and writes them in place. With fewer FP homes the GP form is no longer.
    fn fp_select_profitable(
        &self,
        dest: &Value,
        true_val: &Operand,
        false_val: &Operand,
        ty: IrType,
    ) -> bool {
        if !matches!(ty, IrType::F32 | IrType::F64) {
            return false;
        }
        let fp = |r: Option<PhysReg>| r.is_some_and(is_arm_fp_phys);
        [
            fp(self.dest_reg(dest)),
            fp(self.operand_reg(true_val)),
            fp(self.operand_reg(false_val)),
        ]
        .into_iter()
        .filter(|&b| b)
        .count()
            >= 2
    }

    /// The arms of an FP-domain Select as FP register names: a home in
    /// place, anything else staged into the d0/d1 scratch (never allocation
    /// homes). Staging never touches the flags, so it may follow the compare.
    fn fp_select_arms(
        &mut self,
        true_val: &Operand,
        false_val: &Operand,
        ty: IrType,
    ) -> (String, String) {
        let (s0, s1) = if ty == IrType::F32 {
            ("s0", "s1")
        } else {
            ("d0", "d1")
        };
        let f_name = self.float_operand_reg(false_val, ty, s0);
        let t_name = self.float_operand_reg(true_val, ty, s1);
        (t_name, f_name)
    }

    /// `fcsel` with the flags set (`cc` selects `t_name`), straight into an
    /// FP home or through d0 to any other home.
    fn emit_fcsel_result(
        &mut self,
        dest: &Value,
        t_name: &str,
        f_name: &str,
        ty: IrType,
        cc: &str,
    ) {
        let target = match self.dest_reg(dest).filter(|r| is_arm_fp_phys(*r)) {
            Some(phys) => arm_fp_name(phys, ty),
            None if ty == IrType::F32 => "s0".to_string(),
            None => "d0".to_string(),
        };
        self.state.emit_fmt(format_args!(
            "    fcsel {}, {}, {}, {}",
            target, t_name, f_name, cc
        ));
        self.store_float_reg(dest, ty, &target);
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
        if self.fp_select_profitable(dest, true_val, false_val, sel_ty) {
            let (t_name, f_name) = self.fp_select_arms(true_val, false_val, sel_ty);
            self.emit_fcsel_result(dest, &t_name, &f_name, sel_ty, cc);
            return;
        }
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
            let accumulator = if use_32bit { "w0" } else { "x0" };
            self.state.emit_fmt(format_args!(
                "    csel {}, {}, {}, {}",
                accumulator, t_name, f_name, cc
            ));
            // Keep the accumulator entry when `dest` has no durable
            // home (see store_x0_to_and_release) and skip the shared
            // invalidate below, which would orphan the value.
            self.store_x0_to_and_release(dest);
            return;
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
        self.emit_zero_test(cond);
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
            // Keep the accumulator entry when `dest` has no durable
            // home (see store_x0_to_and_release) and skip the shared
            // invalidate below, which would orphan the value.
            self.store_x0_to_and_release(dest);
            return;
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
