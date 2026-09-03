//! ArmCodegen: prologue/epilogue and stack frame operations.

use super::emit::{
    callee_saved_name, ArmCodegen, ARM_ARG_REGS, ARM_CALLEE_SAVED, ARM_CALLER_SAVED,
};
use crate::backend::call_abi::{classify_params, ParamClass};
use crate::backend::generation::{calculate_stack_space_common, find_param_alloca};
use crate::common::types::IrType;
use crate::ir::reexports::{Instruction, IrBinOp, IrFunction, Operand, Terminator, Value};

/// Exact fixed-register plan for the tiny conditional-increment leaf family.
///
/// Keeping incoming integer parameters in x0..x7 and the result in x0 gives
/// AAPCS64's optimal `cmp; cinc; ret` shape. This is intentionally a strict
/// machine-combine companion, not a general caller-register allocator: every
/// non-ParamRef producer must be an Add/Cmp proven to be consumed by the one
/// Select and therefore skipped by generic fused emission. Any extra IR makes
/// the function use the ordinary scratch-aware allocator.
fn conditional_increment_leaf_plan(
    func: &IrFunction,
) -> Option<(
    crate::common::fx_hash::FxHashMap<u32, crate::backend::regalloc::PhysReg>,
    bool,
)> {
    use crate::backend::regalloc::PhysReg;
    use crate::common::fx_hash::FxHashMap;

    if func.blocks.len() != 1
        || func.is_variadic
        || func.uses_sret
        || func.params.len() > 8
        || func.params.iter().any(|param| {
            param.struct_size.is_some()
                || param.ty.is_float()
                || param.ty.is_long_double()
                || param.ty.is_128bit()
        })
    {
        return None;
    }
    let block = &func.blocks[0];
    let Terminator::Return(Some(Operand::Value(returned))) = &block.terminator else {
        return None;
    };

    let mut assignments = FxHashMap::default();
    let mut param_values = FxHashMap::default();
    let mut seen_param_indices = crate::common::fx_hash::FxHashSet::default();
    let mut add = None;
    let mut compare = None;
    let mut select = None;
    for (index, instruction) in block.instructions.iter().enumerate() {
        match instruction {
            Instruction::ParamRef {
                dest,
                param_idx,
                ty,
            } if *param_idx < 8 && ty.is_integer() && !ty.is_128bit() => {
                if !seen_param_indices.insert(*param_idx) || param_values.contains_key(&dest.0) {
                    return None;
                }
                assignments.insert(dest.0, PhysReg(*param_idx as u8));
                param_values.insert(dest.0, *param_idx);
            }
            Instruction::BinOp {
                dest,
                op: IrBinOp::Add,
                lhs,
                rhs,
                ty,
            } if matches!(ty.size(), 4 | 8) => {
                if add.replace((index, *dest, lhs, rhs, *ty)).is_some() {
                    return None;
                }
            }
            Instruction::Cmp { dest, ty, .. } if ty.is_integer() && !ty.is_128bit() => {
                if compare.replace((index, *dest)).is_some() {
                    return None;
                }
            }
            Instruction::Select {
                dest,
                cond,
                true_val,
                false_val,
                ty,
            } if ty.is_integer() && !ty.is_128bit() => {
                if select
                    .replace((index, *dest, cond, true_val, false_val, *ty))
                    .is_some()
                {
                    return None;
                }
            }
            _ => return None,
        }
    }
    let (add_index, add_dest, add_lhs, add_rhs, add_ty) = add?;
    let (select_index, select_dest, condition, true_value, false_value, select_ty) = select?;
    if select_dest != *returned || add_index >= select_index {
        return None;
    }
    let is_one = |operand: &Operand| matches!(operand, Operand::Const(constant) if constant.to_i64() == Some(1));
    let same = |a: &Operand, b: &Operand| match (a, b) {
        (Operand::Value(a), Operand::Value(b)) => a == b,
        (Operand::Const(a), Operand::Const(b)) => a.to_hash_key() == b.to_hash_key(),
        _ => false,
    };
    let (increment, base) = if matches!(true_value, Operand::Value(value) if *value == add_dest) {
        (true_value, false_value)
    } else if matches!(false_value, Operand::Value(value) if *value == add_dest) {
        (false_value, true_value)
    } else {
        return None;
    };
    let _ = increment;
    if !((same(add_lhs, base) && is_one(add_rhs)) || (same(add_rhs, base) && is_one(add_lhs))) {
        return None;
    }
    let width_ok = add_ty == select_ty
        || (add_ty.is_integer() && select_ty.is_integer() && add_ty.size() == select_ty.size())
        || (add_ty == IrType::U32 && select_ty.size() >= 4);
    if !width_ok {
        return None;
    }
    let Operand::Value(base_value) = base else {
        return None;
    };
    if !param_values.contains_key(&base_value.0) {
        return None;
    }

    let condition_is_32 = match condition {
        Operand::Value(value) if param_values.contains_key(&value.0) => {
            if compare.is_some() || add_index + 1 != select_index {
                return None;
            }
            let param_index = *param_values.get(&value.0)?;
            func.params[param_index].ty.size() <= 4
        }
        Operand::Value(value) => {
            let (compare_index, compare_dest) = compare?;
            if *value != compare_dest
                || compare_index + 2 != select_index
                || add_index != compare_index + 1
            {
                return None;
            }
            true
        }
        Operand::Const(_) => return None,
    };

    // The Add and optional Cmp are never materialized by the fused selector,
    // but homes prevent stack-slot allocation before emission sees the fold.
    assignments.insert(add_dest.0, PhysReg(8));
    if let Some((_, compare_dest)) = compare {
        assignments.insert(compare_dest.0, PhysReg(8));
    }
    assignments.insert(select_dest.0, PhysReg(0));
    Some((assignments, condition_is_32))
}

impl ArmCodegen {
    // ---- calculate_stack_space ----

    pub(super) fn calculate_stack_space_impl(&mut self, func: &IrFunction) -> i64 {
        use crate::backend::regalloc::PhysReg;
        use crate::ir::reexports::Instruction;

        self.loop_promoted_f64_values = func.loop_promoted_f64_values.clone();
        self.conditional_increment_leaf = false;
        self.conditional_increment_leaf_condition_32 = false;

        // Clz/Ctz/Popcount on ≤32-bit integer types produce results in
        // [0, bitwidth] — bit 31 is provably zero, and the emitters write
        // the result with W-register instructions (zeroing the upper half
        // of the X register). Their I32→I64 widening casts therefore skip
        // the `sxtw` (see cast_ops and the x86-64 bitop_nonneg_values
        // design this transfers).
        {
            let mut nonneg = crate::common::fx_hash::FxHashSet::default();
            for block in &func.blocks {
                for inst in &block.instructions {
                    if let Instruction::UnaryOp { dest, op, ty, .. } = inst {
                        if matches!(
                            op,
                            crate::ir::reexports::IrUnaryOp::Clz
                                | crate::ir::reexports::IrUnaryOp::Ctz
                                | crate::ir::reexports::IrUnaryOp::Popcount
                        ) && !ty.is_float()
                            && ty.size() <= 4
                        {
                            nonneg.insert(dest.0);
                        }
                    }
                }
            }
            self.bitop_nonneg_values = nonneg;
        }

        // Same-block div/rem pair fusion table: one sdiv/udiv serves a
        // div+rem couple with identical operands (the remainder comes from
        // msub, exactly GCC's shape). Constant-RHS pairs never fuse
        // (the strength reducer may claim them; the RA model must stay
        // exact).
        let pairs = crate::backend::regalloc::compute_i686_divrem_pairs(
            func,
            crate::backend::regalloc::DivRemTarget::AArch64,
        );
        self.divrem_tail_dests = pairs.tail_dests;
        self.divrem_head_partners = pairs
            .head_partners
            .into_iter()
            .map(|(k, (partner, _))| (k, partner))
            .collect();
        let mut asm_clobbered_regs: Vec<PhysReg> = Vec::new();
        Self::prescan_inline_asm_callee_saved(func, &mut asm_clobbered_regs);
        let base_regs: &[PhysReg] = if func.is_variadic {
            &[]
        } else {
            &ARM_CALLEE_SAVED
        };
        let mut available_regs =
            crate::backend::generation::filter_available_regs(base_regs, &asm_clobbered_regs);

        let mut caller_saved_regs: Vec<PhysReg> = if func.is_variadic {
            Vec::new()
        } else {
            ARM_CALLER_SAVED.to_vec()
        };
        let mut has_f128_ops = false;
        let mut has_i128_ops = false;
        for block in &func.blocks {
            for inst in &block.instructions {
                match inst {
                    Instruction::BinOp { ty, .. }
                    | Instruction::UnaryOp { ty, .. }
                    | Instruction::Cmp { ty, .. }
                    | Instruction::Load { ty, .. }
                    | Instruction::Store { ty, .. }
                        if *ty == IrType::F128 =>
                    {
                        has_f128_ops = true;
                    }
                    Instruction::Cast { to_ty, .. } if *to_ty == IrType::F128 => {
                        has_f128_ops = true;
                    }
                    Instruction::Cast { from_ty, .. } if *from_ty == IrType::F128 => {
                        has_f128_ops = true;
                    }
                    Instruction::BinOp { ty, .. }
                    | Instruction::UnaryOp { ty, .. }
                    | Instruction::Cmp { ty, .. }
                    | Instruction::Load { ty, .. }
                    | Instruction::Store { ty, .. }
                        if matches!(ty, IrType::I128 | IrType::U128) =>
                    {
                        has_i128_ops = true
                    }
                    Instruction::Cast { from_ty, to_ty, .. }
                        if matches!(from_ty, IrType::I128 | IrType::U128)
                            || matches!(to_ty, IrType::I128 | IrType::U128) =>
                    {
                        has_i128_ops = true
                    }
                    _ => {}
                }
            }
        }
        if has_f128_ops || has_i128_ops {
            caller_saved_regs.clear();
        } else {
            // Pure scalar-ALU functions never invoke the address, memcpy,
            // call-staging, intrinsic, or inline-assembly scratch paths. Make
            // the otherwise-reserved caller-saved registers available to
            // high-pressure kernels (cryptographic rounds, arithmetic loops).
            let pure_scalar_alu = func.blocks.iter().all(|block| {
                block.instructions.iter().all(|inst| {
                    matches!(
                        inst,
                        Instruction::Alloca { .. }
                            | Instruction::ParamRef { .. }
                            | Instruction::Copy { .. }
                            | Instruction::Phi { .. }
                            | Instruction::BinOp { .. }
                            | Instruction::UnaryOp { .. }
                            | Instruction::Cmp { .. }
                            | Instruction::Cast { .. }
                            | Instruction::Select { .. }
                    )
                })
            });
            if pure_scalar_alu {
                caller_saved_regs.extend([
                    PhysReg(9),
                    PhysReg(10),
                    PhysReg(11),
                    PhysReg(12),
                    PhysReg(15),
                ]);
            }
        }

        if self.state.disable_regalloc {
            available_regs.clear();
            caller_saved_regs.clear();
        }

        // AArch64 x4..x7 are argument registers 4..7 and x8 is the indirect-
        // result register; all are in the caller-saved pool. A value consumed
        // as a call argument must not be homed there (the staging writes them
        // in order before reading the value).
        let call_arg_regs = vec![
            crate::backend::regalloc::PhysReg(4),
            crate::backend::regalloc::PhysReg(5),
            crate::backend::regalloc::PhysReg(6),
            crate::backend::regalloc::PhysReg(7),
            crate::backend::regalloc::PhysReg(8),
        ];
        let (mut reg_assigned, cached_liveness, _caller_save_spans, accumulator_assignments) =
            crate::backend::generation::run_regalloc_and_merge_clobbers_ex(
                func,
                available_regs,
                caller_saved_regs,
                &asm_clobbered_regs,
                &mut self.reg_assignments,
                &mut self.used_callee_saved,
                false,
                None,
                call_arg_regs,
                Vec::new(),
                // ARM emits indexed addressing [base, index, lsl #N] directly at
                // the Load/Store with no IR-visible use of the index there; the
                // allocator must keep the index live to the consumer's end.
                crate::backend::generation::collect_folded_index_links(func),
            );

        if std::env::var_os("CCC_NO_CSINC_FOLD").is_none() {
            if let Some((plan, condition_is_32)) = conditional_increment_leaf_plan(func) {
                self.conditional_increment_leaf = true;
                self.conditional_increment_leaf_condition_32 = condition_is_32;
                self.reg_assignments = plan;
                reg_assigned = self.reg_assignments.clone();
                self.used_callee_saved.clear();
            }
        }

        // Callee-saved FP registers (d8-d14, allocator IDs 32-38) assigned by
        // the FP scan must be preserved by the prologue/epilogue.
        self.used_fp_callee_saved = {
            let mut v: Vec<PhysReg> = self
                .reg_assignments
                .values()
                .copied()
                .filter(|r| (32..=38).contains(&r.0))
                .collect();
            v.sort_by_key(|r| r.0);
            v.dedup();
            v
        };

        self.state.ra_accumulator_values =
            accumulator_assignments.iter().map(|a| a.value_id).collect();
        let mut space = calculate_stack_space_common(
            &mut self.state,
            func,
            16,
            |space, alloc_size, align| {
                let effective_align = if align > 0 { align.max(8) } else { 8 };
                let slot = (space + effective_align - 1) & !(effective_align - 1);
                let new_space = slot + ((alloc_size + 7) & !7).max(8);
                (slot, new_space)
            },
            &reg_assigned,
            &ARM_CALLEE_SAVED,
            cached_liveness,
        );

        if func.is_variadic {
            space = (space + 7) & !7;
            self.va_gp_save_offset = space;
            space += 64;

            if !self.general_regs_only {
                space = (space + 15) & !15;
                self.va_fp_save_offset = space;
                space += 128;
            }

            let config = self.call_abi_config_impl();
            let param_classes = crate::backend::call_abi::classify_params(func, &config);
            let mut named_gp = 0usize;
            let mut named_fp = 0usize;
            for (i, class) in param_classes.iter().enumerate() {
                // On ARM64, the sret pointer goes in x8 (a dedicated register),
                // NOT in x0-x7. Don't count it as consuming a GP argument register,
                // otherwise va_start computes the wrong __gr_offs and skips the
                // first variadic argument.
                if self.state.uses_sret && i == 0 {
                    continue;
                }
                named_gp += class.gp_reg_count();
                if matches!(
                    class,
                    crate::backend::call_abi::ParamClass::FloatReg { .. }
                        | crate::backend::call_abi::ParamClass::F128FpReg { .. }
                ) {
                    named_fp += 1;
                }
            }
            self.va_named_gp_count = named_gp.min(8);
            self.va_named_fp_count = named_fp.min(8);
            self.va_named_stack_bytes =
                crate::backend::call_abi::named_params_stack_bytes(&param_classes);
        }

        let save_count = self.used_callee_saved.len() as i64;
        if save_count > 0 {
            space = (space + 7) & !7;
            self.callee_save_offset = space;
            space += save_count * 8;
        }

        let fp_save_count = self.used_fp_callee_saved.len() as i64;
        if fp_save_count > 0 {
            space = (space + 7) & !7;
            self.fp_callee_save_offset = space;
            space += fp_save_count * 8;
        }

        space
    }

    // ---- aligned_frame_size ----

    pub(super) fn aligned_frame_size_impl(&self, raw_space: i64) -> i64 {
        (raw_space + 15) & !15
    }

    // ---- emit_prologue ----

    pub(super) fn emit_prologue_impl(&mut self, func: &IrFunction, frame_size: i64) {
        self.current_return_type = func.return_type;
        self.func_ret_is_f128_sse = func.ret_is_f128_sse;
        self.current_frame_size = frame_size;
        self.frame_base_offset = None;
        if self.conditional_increment_leaf {
            return;
        }
        self.emit_prologue_arm(frame_size);

        let used_regs = self.used_callee_saved.clone();
        let base = self.callee_save_offset;
        let n = used_regs.len();
        let mut i = 0;
        while i + 1 < n {
            let r1 = callee_saved_name(used_regs[i]);
            let r2 = callee_saved_name(used_regs[i + 1]);
            let offset = base + (i as i64) * 8;
            self.emit_stp_to_sp(r1, r2, offset);
            i += 2;
        }
        if i < n {
            let r = callee_saved_name(used_regs[i]);
            let offset = base + (i as i64) * 8;
            self.emit_store_to_sp(r, offset, "str");
        }

        // Save callee-saved FP registers (d8-d14) used by the FP allocator.
        let used_fp = self.used_fp_callee_saved.clone();
        let fp_base = self.fp_callee_save_offset;
        let m = used_fp.len();
        let mut j = 0;
        while j + 1 < m {
            let r1 = format!("d{}", used_fp[j].0 - 24);
            let r2 = format!("d{}", used_fp[j + 1].0 - 24);
            let offset = fp_base + (j as i64) * 8;
            self.emit_stp_to_sp(&r1, &r2, offset);
            j += 2;
        }
        if j < m {
            let r = format!("d{}", used_fp[j].0 - 24);
            let offset = fp_base + (j as i64) * 8;
            self.emit_store_to_sp(&r, offset, "str");
        }
    }

    // ---- emit_epilogue ----

    pub(super) fn emit_epilogue_impl(&mut self, frame_size: i64) {
        if self.conditional_increment_leaf {
            return;
        }
        self.emit_restore_callee_saved();
        self.emit_epilogue_arm(frame_size);
    }

    // ---- emit_store_params ----

    pub(super) fn emit_store_params_impl(&mut self, func: &IrFunction) {
        if func.is_variadic {
            self.emit_save_variadic_regs();
        }

        let config = self.call_abi_config_impl();
        let param_classes = classify_params(func, &config);
        self.state.param_classes = param_classes.clone();
        self.state.num_params = func.params.len();
        self.state.func_is_variadic = func.is_variadic;

        self.state.param_alloca_slots = (0..func.params.len())
            .map(|i| {
                find_param_alloca(func, i)
                    .and_then(|(dest, ty)| self.state.get_slot(dest.0).map(|slot| (slot, ty)))
            })
            .collect();

        // Pre-store optimization: when a GP param's alloca is dead (promoted by
        // mem2reg) but the ParamRef dest is register-allocated to a callee-saved
        // register, store the ABI arg register directly to that callee-saved
        // register in the prologue.  This is critical because:
        // 1. Dead alloca means no stack slot exists for this param
        // 2. The ABI register (x0-x7) will be clobbered by subsequent codegen
        //    (ARM uses x0 as the universal scratch/result register)
        // 3. We must save the value NOW, before any other code runs
        // 4. emit_param_ref will see param_pre_stored and skip code generation
        let sret_shift = if self.state.uses_sret { 1usize } else { 0 };
        let mut paramref_dests: Vec<Option<Value>> = vec![None; func.params.len()];
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Instruction::ParamRef {
                    dest, param_idx, ..
                } = inst
                {
                    if *param_idx < paramref_dests.len() {
                        paramref_dests[*param_idx] = Some(*dest);
                    }
                }
            }
        }
        // Build a map from physical register -> list of param indices that use it,
        // so we can detect when two params share the same callee-saved register.
        let mut reg_to_params: crate::common::fx_hash::FxHashMap<u8, Vec<usize>> =
            crate::common::fx_hash::FxHashMap::default();
        for (i, _) in func.params.iter().enumerate() {
            if let Some(paramref_dest) = paramref_dests[i] {
                if let Some(&phys_reg) = self.reg_assignments.get(&paramref_dest.0) {
                    reg_to_params.entry(phys_reg.0).or_default().push(i);
                }
            }
        }

        for (i, _) in func.params.iter().enumerate() {
            let class = param_classes[i];
            if !class.uses_gp_reg() {
                continue;
            }
            // Skip params that have an alloca slot (they'll be handled by emit_store_gp_params)
            let has_slot = self
                .state
                .param_alloca_slots
                .get(i)
                .and_then(|opt| opt.as_ref())
                .is_some();
            if has_slot {
                continue;
            }

            if let Some(paramref_dest) = paramref_dests[i] {
                if let Some(&phys_reg) = self.reg_assignments.get(&paramref_dest.0) {
                    if self.conditional_increment_leaf {
                        if let ParamClass::IntReg { reg_idx } = class {
                            if phys_reg.0 == reg_idx as u8 {
                                // The fixed leaf plan deliberately keeps this
                                // parameter in its incoming ABI register.
                                self.state.param_pre_stored.insert(i);
                                continue;
                            }
                        }
                    }
                    // Only pre-store to callee-saved registers (x19-x28).
                    // x19 is in ARM_CALLEE_SAVED and is a normal allocatable
                    // home. Excluding it let an F128 parameter spill call
                    // clobber x0 before a later ParamRef of a GP parameter
                    // assigned to x19 (gcc torture 20020413-1: eval pointer
                    // became the truncated long-double bits). Caller-saved
                    // registers (x13, x14) remain excluded.
                    let is_callee_saved = phys_reg.0 >= 19 && phys_reg.0 <= 28;
                    if is_callee_saved {
                        // Safety check: if another param's dest is also assigned
                        // to this register, skip pre-store to avoid conflicts.
                        // The register allocator may assign the same register to
                        // two params whose live ranges don't overlap, but pre-store
                        // extends the effective lifetime to function entry.
                        if let Some(users) = reg_to_params.get(&phys_reg.0) {
                            if users.len() > 1 {
                                continue;
                            }
                        }
                        let dest_reg = callee_saved_name(phys_reg);
                        if let ParamClass::IntReg { reg_idx } = class {
                            let actual_idx = if sret_shift > 0 && reg_idx == 0 && i == 0 {
                                // sret: the pointer comes in x8
                                self.state
                                    .emit_fmt(format_args!("    mov {}, x8", dest_reg));
                                self.state.param_pre_stored.insert(i);
                                continue;
                            } else if reg_idx >= sret_shift {
                                reg_idx - sret_shift
                            } else {
                                reg_idx
                            };
                            let src_reg = ARM_ARG_REGS[actual_idx];
                            self.state
                                .emit_fmt(format_args!("    mov {}, {}", dest_reg, src_reg));
                            self.state.param_pre_stored.insert(i);
                        }
                    }
                }
            }
        }

        self.emit_store_gp_params(func, &param_classes);
        self.emit_store_fp_params(func, &param_classes);
        self.emit_store_stack_params(func, &param_classes);
    }

    // ---- emit_param_ref ----

    pub(super) fn emit_param_ref_impl(&mut self, dest: &Value, param_idx: usize, ty: IrType) {
        if param_idx >= self.state.param_classes.len() {
            return;
        }

        // If this param was pre-stored directly to its register-allocated
        // destination during emit_store_params, the value is already in place.
        // No code needs to be emitted — the register already holds the value.
        if self.state.param_pre_stored.contains(&param_idx) {
            return;
        }

        if param_idx < self.state.param_alloca_slots.len() {
            if let Some((slot, alloca_ty)) = self.state.param_alloca_slots[param_idx] {
                let ldr_instr = self.load_instr_for_type_impl(alloca_ty);
                let (actual_instr, reg) = Self::arm_parse_load(ldr_instr);
                self.emit_load_from_sp(reg, slot.0, actual_instr);
                self.store_x0_to(dest);
                return;
            }
        }

        let class = self.state.param_classes[param_idx];
        let frame_size = self.current_frame_size;

        // AArch64 ABI: sret shifts GP register indices
        let sret_shift = if self.state.uses_sret { 1usize } else { 0 };

        match class {
            ParamClass::IntReg { reg_idx } => {
                let actual_reg = if sret_shift > 0 && reg_idx == 0 && param_idx == 0 {
                    Self::reg_for_type("x8", ty)
                } else {
                    let actual_idx = if reg_idx >= sret_shift {
                        reg_idx - sret_shift
                    } else {
                        reg_idx
                    };
                    Self::reg_for_type(ARM_ARG_REGS[actual_idx], ty)
                };
                let dst = Self::reg_for_type("x0", ty);
                if actual_reg != dst {
                    self.state
                        .emit_fmt(format_args!("    mov {}, {}", dst, actual_reg));
                }
                self.store_x0_to(dest);
            }
            ParamClass::FloatReg { reg_idx } => {
                if ty == IrType::F32 {
                    self.state
                        .emit_fmt(format_args!("    fmov w0, s{}", reg_idx));
                } else {
                    self.state
                        .emit_fmt(format_args!("    fmov x0, d{}", reg_idx));
                }
                self.store_x0_to(dest);
            }
            ParamClass::StackScalar { offset } => {
                let src = frame_size + offset;
                let ldr_instr = self.load_instr_for_type_impl(ty);
                let (actual_instr, reg) = Self::arm_parse_load(ldr_instr);
                self.emit_load_from_sp(reg, src, actual_instr);
                self.store_x0_to(dest);
            }
            _ => {}
        }
    }

    // ---- emit_epilogue_and_ret ----

    pub(super) fn emit_epilogue_and_ret_impl(&mut self, frame_size: i64) {
        if !self.conditional_increment_leaf {
            self.emit_restore_callee_saved();
            self.emit_epilogue_arm(frame_size);
        }
        self.state.emit("    ret");
    }

    // ---- store_instr_for_type / load_instr_for_type ----

    pub(super) fn store_instr_for_type_impl(&self, ty: IrType) -> &'static str {
        Self::str_for_type(ty)
    }

    pub(super) fn load_instr_for_type_impl(&self, ty: IrType) -> &'static str {
        match ty {
            IrType::I8 => "ldrsb",
            IrType::U8 => "ldrb",
            IrType::I16 => "ldrsh",
            IrType::U16 => "ldrh",
            IrType::I32 => "ldrsw",
            IrType::U32 | IrType::F32 => "ldr32",
            _ => "ldr64",
        }
    }
}
