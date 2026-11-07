//! ArmCodegen: prologue/epilogue and stack frame operations.

use super::emit::{
    ARM_ARG_REGS, ARM_CALLEE_SAVED, ARM_CALLER_SAVED, ArmCodegen, callee_saved_name,
    callee_saved_name_32, is_arm_fp_phys,
};
use crate::backend::call_abi::{ParamClass, classify_params};
use crate::backend::generation::{ParamAllocaIndex, calculate_stack_space_common};
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
    usize,
)> {
    use crate::backend::regalloc::PhysReg;
    use crate::common::fx_hash::FxHashMap;

    if func.blocks.len() != 1
        || func.is_variadic
        || func.uses_sret
        || func.ret_is_f128_sse
        || func.params.len() > 8
        || func.params.iter().any(|param| {
            param.struct_size.is_some()
                || param.is_f128_sse
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

    // A parameter condition is tested in its ABI register, where AAPCS64
    // leaves every bit above its width unspecified: the test reads exactly
    // `size` bytes. A Cmp result is a 0/1 `cset`.
    let condition_bytes = match condition {
        Operand::Value(value) if param_values.contains_key(&value.0) => {
            if compare.is_some() || add_index + 1 != select_index {
                return None;
            }
            let param_index = *param_values.get(&value.0)?;
            func.params[param_index].ty.size()
        }
        Operand::Value(value) => {
            let (compare_index, compare_dest) = compare?;
            if *value != compare_dest
                || compare_index + 2 != select_index
                || add_index != compare_index + 1
            {
                return None;
            }
            4
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
    Some((assignments, condition_bytes))
}

/// The instruction that writes the integer argument arriving in X register
/// `arg` into X register `dst` as a canonical `ty` value, or `None` when
/// `dst` already holds it.
///
/// AAPCS64 C.16 copies an integral argument narrower than 64 bits "to the
/// least significant bits" of x[n] and leaves the other bits unspecified:
/// GCC and Clang callers tail-call with whatever upper half the caller's own
/// value had. The callee therefore extends -- a 32-bit value is
/// zero-extended (`mov wD, wA`, also when D == A: what every 32-bit producer
/// leaves, and what `eliminate_self_moves` drops once bits 63:32 are proven
/// unobservable), an 8- or 16-bit value by its signedness exactly as the
/// IntNarrow cast extends it, so that every W-view consumer (`cbnz wN`,
/// `cmp wN, #0`, the 32-bit ALU forms) reads the value. A 64-bit value
/// moves unchanged; any other type keeps the plain width-of-type move.
pub(super) fn canonical_gp_param_move(dst: &str, arg: &str, ty: IrType) -> Option<String> {
    let w = |x: &str| format!("w{}", x.strip_prefix('x').unwrap_or(x));
    match ty {
        IrType::I8 => Some(format!("    sxtb {}, {}", dst, w(arg))),
        IrType::I16 => Some(format!("    sxth {}, {}", dst, w(arg))),
        IrType::U8 => Some(format!("    and {}, {}, #0xff", w(dst), w(arg))),
        IrType::U16 => Some(format!("    and {}, {}, #0xffff", w(dst), w(arg))),
        IrType::I32 | IrType::U32 => Some(format!("    mov {}, {}", w(dst), w(arg))),
        _ if dst == arg => None,
        _ if ArmCodegen::reg_for_type("x0", ty) == "w0" => {
            Some(format!("    mov {}, {}", w(dst), w(arg)))
        }
        _ => Some(format!("    mov {}, {}", dst, arg)),
    }
}

impl ArmCodegen {
    // ---- calculate_stack_space ----

    /// The `ARM_CALLER_SAVED` registers some emitter in `func` uses as
    /// unmodelled scratch, so no value may be homed there. An ALLOWLIST: an
    /// instruction proven to scratch nothing in the pool contributes nothing,
    /// the audited scratch users contribute their registers, and anything
    /// else (intrinsics, inline asm, dynamic stack, non-local control, PGO
    /// and libcall-backed helpers) closes the whole pool -- a future lowering
    /// that grabs a pool register then fails closed instead of clobbering a
    /// live home. Audited emitters:
    ///   * atomics (`atomics.rs`): `w4` is the `stxr` status register;
    ///   * variadics (`variadic.rs`): `x4`-`x6` address/copy temporaries;
    ///   * `Memcpy` (`memory.rs`): `ldp x12, x13` block-copy pair.
    /// Call staging (x4-x8 argument writes, x13/x14 temporaries) is modelled
    /// by `call_arg_regs` and the call points themselves; I128/F128 functions
    /// clear the pool before this is consulted.
    fn arm_pool_scratch(func: &IrFunction) -> &'static [u8] {
        use crate::ir::reexports::Instruction as I;
        const ALL: &[u8] = &[4, 5, 6, 7, 8, 13, 14];
        let (mut atomics, mut varargs, mut memcpy) = (false, false, false);
        for inst in func.blocks.iter().flat_map(|b| &b.instructions) {
            match inst {
                I::Alloca { .. }
                | I::Store { .. }
                | I::Load { .. }
                | I::BinOp { .. }
                | I::UnaryOp { .. }
                | I::Cmp { .. }
                | I::Call { .. }
                | I::CallIndirect { .. }
                | I::GetElementPtr { .. }
                | I::Cast { .. }
                | I::Copy { .. }
                | I::GlobalAddr { .. }
                | I::Phi { .. }
                | I::LabelAddr { .. }
                | I::Select { .. }
                | I::ParamRef { .. }
                | I::AtomicLoad { .. }
                | I::AtomicStore { .. }
                | I::Fence { .. }
                // `intrinsics.rs`: x0/x1/x9-x12 only, no branch-and-link.
                | I::Intrinsic { .. } => {}
                I::AtomicRmw { .. } | I::AtomicCmpxchg { .. } => atomics = true,
                I::VaArg { .. }
                | I::VaArgStruct { .. }
                | I::VaStart { .. }
                | I::VaCopy { .. } => varargs = true,
                I::Memcpy { .. } => memcpy = true,
                _ => return ALL,
            }
        }
        match (atomics, varargs, memcpy) {
            (false, false, false) => &[],
            (true, false, false) => &[4],
            (false, true, false) | (true, true, false) => &[4, 5, 6],
            (false, false, true) => &[13],
            (true, false, true) => &[4, 13],
            (_, true, true) => &[4, 5, 6, 13],
        }
    }

    /// Whether `func` computes on `long double` (F128) or `__int128` values:
    /// their helper calls and register pairs run through the emitter's
    /// caller-saved scratch, so such a function gets no caller-saved pool.
    fn uses_f128_or_i128(func: &IrFunction) -> bool {
        let wide = |ty: &IrType| matches!(ty, IrType::F128 | IrType::I128 | IrType::U128);
        func.blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .any(|inst| match inst {
                Instruction::BinOp { ty, .. }
                | Instruction::UnaryOp { ty, .. }
                | Instruction::Cmp { ty, .. }
                | Instruction::Load { ty, .. }
                | Instruction::Store { ty, .. } => wide(ty),
                Instruction::Cast { from_ty, to_ty, .. } => wide(from_ty) || wide(to_ty),
                _ => false,
            })
    }

    pub(super) fn calculate_stack_space_impl(&mut self, func: &IrFunction) -> i64 {
        use crate::backend::regalloc::PhysReg;
        use crate::ir::reexports::Instruction;

        self.value_types = crate::backend::common::compute_value_type_map(func);
        self.loop_promoted_f64_values = func.loop_promoted_f64_values.clone();
        self.conditional_increment_leaf = false;
        self.conditional_increment_leaf_condition_bytes = 8;
        self.frameless_leaf = false;

        // Non-negative ≤32-bit bitop/masked results have bit 31 provably zero,
        // and W-register instructions zero the upper half of the X register.
        // Their I32→I64 widening casts therefore skip `sxtw`.
        self.bitop_nonneg_values = crate::backend::generation::collect_bitop_nonneg_values(func);

        // Same-block div/rem pair fusion table: one sdiv/udiv serves a
        // div+rem couple with identical operands (the remainder comes from
        // msub, exactly GCC's shape). Constant-RHS pairs never fuse
        // (the strength reducer may claim them; the RA model must stay
        // exact).
        let pairs = crate::backend::regalloc::compute_i686_divrem_pairs_with_config(
            func,
            crate::backend::regalloc::DivRemTarget::AArch64,
            &self.state.ra_config,
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
        if Self::uses_f128_or_i128(func) {
            caller_saved_regs.clear();
        } else {
            // Emitter scratch the pool must never hold a value across.
            caller_saved_regs.retain(|r| !Self::arm_pool_scratch(func).contains(&r.0));
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
        // A nested function leaving through a nonlocal `goto` never runs its
        // epilogue's restores (see `leaves_through_nonlocal_goto`).
        if crate::backend::generation::leaves_through_nonlocal_goto(func) {
            available_regs.clear();
        }

        // AArch64 x4..x7 are argument registers 4..7 and x8 is the indirect-
        // result register; all are in the caller-saved pool. A value consumed
        // as a call argument must not be homed there (the staging writes them
        // in order before reading the value).
        //
        // x13/x14 belong here too: `emit_call_gp_to_temps` stages GP argument
        // k into `ARM_TMP_REGS[k]` (x9, x10, ...) in argument order, so the
        // fifth argument's staging overwrites x13 before a later argument
        // homed there is read (and likewise x14 for the sixth). The generic
        // Phase 2 routes argument-consumed values to the pool minus these
        // registers -- which, without x13/x14, is exactly {x13, x14}.
        let call_arg_regs = vec![
            crate::backend::regalloc::PhysReg(4),
            crate::backend::regalloc::PhysReg(5),
            crate::backend::regalloc::PhysReg(6),
            crate::backend::regalloc::PhysReg(7),
            crate::backend::regalloc::PhysReg(8),
            crate::backend::regalloc::PhysReg(13),
            crate::backend::regalloc::PhysReg(14),
        ];
        let (
            mut reg_assigned,
            cached_liveness,
            _caller_save_spans,
            accumulator_assignments,
            _ra_eligible,
        ) = crate::backend::generation::run_regalloc_and_merge_clobbers_ex(
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
            // AArch64 has no segment indexed emitters (ArchCodegen default: false).
            crate::backend::generation::collect_folded_index_links(func, false),
            // AArch64 folds constant offsets against allocas only.
            Vec::new(),
            &self.state.ra_config,
            // Not alias-aware yet: ignored scratch map keeps the pre-alias freshness behavior.
            &mut crate::common::fx_hash::FxHashMap::default(),
        );

        if std::env::var_os("CCC_NO_CSINC_FOLD").is_none() {
            if let Some((plan, condition_bytes)) = conditional_increment_leaf_plan(func) {
                self.conditional_increment_leaf = true;
                self.conditional_increment_leaf_condition_bytes = condition_bytes;
                self.reg_assignments = plan;
                reg_assigned = self.reg_assignments.clone();
                self.used_callee_saved.clear();
            }
        }
        self.rehome_adjacent_global_addr_ptrs(func, &mut reg_assigned);

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
                // Every class that occupies v-registers: scalars, long
                // double, and each member of a named HFA.  A named HFA that
                // missed the registers closed the SIMD file (C.3); the
                // `min(8)` below then gives `__vr_offs` = 0, as GCC's
                // va_start computes from the same NSRN.
                named_fp += match class {
                    crate::backend::call_abi::ParamClass::FloatReg { .. }
                    | crate::backend::call_abi::ParamClass::F128FpReg { .. } => 1,
                    crate::backend::call_abi::ParamClass::StructHfaReg { count, .. } => *count,
                    crate::backend::call_abi::ParamClass::StructStack { .. }
                    | crate::backend::call_abi::ParamClass::LargeStructStack { .. }
                        if func
                            .params
                            .get(i)
                            .and_then(|p| p.fp_agg_class)
                            .and_then(|c| c.hfa())
                            .is_some() =>
                    {
                        8
                    }
                    _ => 0,
                };
            }
            self.va_named_gp_count = named_gp.min(8);
            self.va_named_fp_count = named_fp.min(8);
            self.va_named_stack_bytes =
                crate::backend::call_abi::named_params_stack_bytes(&param_classes);
        }

        // An HFA/HVA/short-vector result: the body writes it through the
        // hidden return pointer, which is homed as the address of this
        // private buffer; the return loads v0..v3 from it.
        self.fp_ret_buffer = crate::backend::call_abi::hidden_fp_agg_return(
            func.uses_sret,
            func.params.first().and_then(|p| p.fp_agg_class),
        )
        .map(|class| {
            space = (space + 15) & !15;
            let offset = space;
            space += crate::backend::call_abi::fp_agg_return_buffer_bytes(class)
                .next_multiple_of(16) as i64;
            (offset, class)
        });

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
        // The GP width of the result, for the peephole (`ret_gp_reads`):
        // AAPCS64 leaves the bits of x0 above a narrow integer unspecified
        // (callers extend what they read), so an extension that only
        // rewrites those bits on the way to `ret` is unobservable; a result
        // in one GP register (64) leaves x1 unread at the `ret`, and a
        // `void` or `float`/`double` result (0, returned in v0) leaves x0
        // unread too.  Wider and aggregate results publish nothing (x0 and
        // x1 both count as read).  Never emitted: the peephole drops it.
        let ret_bits = match func.return_type {
            IrType::Void | IrType::F32 | IrType::F64 => Some(0),
            IrType::I8 | IrType::U8 => Some(8),
            IrType::I16 | IrType::U16 => Some(16),
            IrType::I32 | IrType::U32 => Some(32),
            IrType::I64 | IrType::U64 | IrType::Ptr => Some(64),
            _ => None,
        };
        if let Some(bits) = ret_bits.filter(|_| !func.uses_sret) {
            self.state.emit_fmt(format_args!(
                "    {}{bits}",
                super::peephole::RET_BITS_MARKER
            ));
        }
        self.frame_base_offset = None;
        self.frameless_leaf =
            !self.conditional_increment_leaf && self.frameless_leaf_ok(func, frame_size);
        if self.conditional_increment_leaf || self.frameless_leaf {
            return;
        }
        self.emit_prologue_arm(frame_size);

        // Every save gets its CFA-relative row. After `emit_prologue_arm` the
        // CFA is `x29 + frame_size` with x29 == sp in all three frame shapes,
        // so the slot at `[sp, #offset]` sits at `CFA + offset - frame_size`.
        // Without the rows an unwinder (C++ exceptions through C frames,
        // `_Unwind_ForcedUnwind` for thread cancellation, a debugger's
        // `finish`) resumes the caller with this function's values in
        // x19-x28 / d8-d15.
        let used_regs = self.used_callee_saved.clone();
        let base = self.callee_save_offset;
        let n = used_regs.len();
        let mut i = 0;
        while i + 1 < n {
            let r1 = callee_saved_name(used_regs[i]);
            let r2 = callee_saved_name(used_regs[i + 1]);
            let offset = base + (i as i64) * 8;
            self.emit_stp_to_sp(r1, r2, offset);
            self.emit_callee_save_cfi(r1, offset, frame_size);
            self.emit_callee_save_cfi(r2, offset + 8, frame_size);
            i += 2;
        }
        if i < n {
            let r = callee_saved_name(used_regs[i]);
            let offset = base + (i as i64) * 8;
            self.emit_store_to_sp(r, offset, "str");
            self.emit_callee_save_cfi(r, offset, frame_size);
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
            self.emit_callee_save_cfi(&r1, offset, frame_size);
            self.emit_callee_save_cfi(&r2, offset + 8, frame_size);
            j += 2;
        }
        if j < m {
            let r = format!("d{}", used_fp[j].0 - 24);
            let offset = fp_base + (j as i64) * 8;
            self.emit_store_to_sp(&r, offset, "str");
            self.emit_callee_save_cfi(&r, offset, frame_size);
        }
    }

    /// Distance from sp to the caller's outgoing-argument area -- where the
    /// stack-passed parameters start (AAPCS64: the entry sp): the frame
    /// size, or 0 in a frameless leaf, whose sp never moves.
    pub(super) fn incoming_stack_base(&self) -> i64 {
        if self.frameless_leaf {
            0
        } else {
            self.current_frame_size
        }
    }

    /// `.cfi_offset` row for a callee-saved register stored at
    /// `[sp, #offset]` of a `frame_size`-byte frame (see `emit_prologue_impl`).
    fn emit_callee_save_cfi(&mut self, reg: &str, offset: i64, frame_size: i64) {
        if self.state.emit_cfi {
            self.state.emit_fmt(format_args!(
                "    .cfi_offset {}, {}",
                reg,
                offset - frame_size
            ));
        }
    }

    /// The frameless-leaf form: no `stp x29, x30` / `mov x29, sp` / `ldp`,
    /// the return address stays in x30 and the CFA stays `sp + 0` (the CIE's
    /// initial rule), which is what GCC (`-momit-leaf-frame-pointer`, the
    /// AArch64 default) and Clang emit for a leaf with no frame content:
    /// `int f(int x) { return x + 1; }` is `add w0, w0, #1; ret`.
    ///
    /// Requirements, each load-bearing:
    /// * `frame_size == 16`: the frame holds the frame record only -- no
    ///   value slot, alloca, callee save, varargs area or return buffer.
    /// * no callee-saved GP/FP register in use (nothing to save).
    /// * frame-pointer omission requested (`-O1`+, not `-fno-omit-frame-
    ///   pointer`, no `__builtin_frame_address`, no dynamic alloca: the
    ///   shared `omit_frame_pointer` decision).
    /// * every parameter arrives in a register or as a stack scalar (read
    ///   from `[sp, #offset]` through [`Self::incoming_stack_base`]); an
    ///   aggregate, `__int128` or `_Float128` on the stack is copied into a
    ///   slot, which a 16-byte frame does not have.
    /// * a scalar (or void) result, so no `_Float128` widening call in the
    ///   return sequence.
    /// * [`Self::arm_frameless_leaf_ir`]: nothing in the body calls (x30
    ///   must survive to the `ret`) or addresses the frame through x29.
    fn frameless_leaf_ok(&self, func: &IrFunction, frame_size: i64) -> bool {
        let scalar_or_void = |ty: IrType| {
            matches!(
                ty,
                IrType::Void
                    | IrType::I8
                    | IrType::I16
                    | IrType::I32
                    | IrType::I64
                    | IrType::U8
                    | IrType::U16
                    | IrType::U32
                    | IrType::U64
                    | IrType::F32
                    | IrType::F64
                    | IrType::Ptr
            )
        };
        frame_size == 16
            && self.used_callee_saved.is_empty()
            && self.used_fp_callee_saved.is_empty()
            && self.state.omit_frame_pointer
            && !self.state.has_dyn_alloca
            && !func.is_variadic
            && !func.uses_sret
            && !self.state.uses_sret
            && self.fp_ret_buffer.is_none()
            && !func.ret_is_f128_sse
            && scalar_or_void(func.return_type)
            && {
                let config = self.call_abi_config_impl();
                classify_params(func, &config).iter().all(|c| {
                    matches!(
                        c,
                        ParamClass::IntReg { .. }
                            | ParamClass::FloatReg { .. }
                            | ParamClass::StackScalar { .. }
                    )
                })
            }
            && self.arm_frameless_leaf_ir(func, self.every_call_is_sibling(func))
    }

    /// Whether every call of `func` will be emitted as a sibling call --
    /// the block-tail shape the generation driver arms
    /// (`sibcall_function_use_counts` + `sibcall_block_hint`, the exact
    /// inputs it uses) and `sibcall_abi_admits` -- with scalar arguments and
    /// result, so neither the argument setup nor the result path calls a
    /// helper.  Such a function needs no frame record: x30 is never
    /// clobbered, and each tail is a bare `b`.  (`emit_call_instruction_impl`
    /// guards x30 around any call that is emitted after all.)
    fn every_call_is_sibling(&self, func: &IrFunction) -> bool {
        let Some(counts) = crate::backend::generation::sibcall_function_use_counts(
            func,
            &self.state.incoming_byval_slots,
        ) else {
            return false;
        };
        for block in &func.blocks {
            let hint = crate::backend::generation::sibcall_block_hint(block, &counts);
            for (idx, inst) in block.instructions.iter().enumerate() {
                let info = match inst {
                    Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => info,
                    _ => continue,
                };
                if hint.map(|h| h.instruction_index) != Some(idx)
                    || !self.sibcall_abi_admits(info)
                    || !info
                        .arg_types
                        .iter()
                        .all(|&t| super::calls::sibcall_scalar(t))
                    || !(info.return_type == IrType::Void
                        || super::calls::sibcall_scalar(info.return_type))
                {
                    return false;
                }
            }
        }
        true
    }

    /// Instructions whose AArch64 emitters never call (no `bl`/`blr`: the
    /// F128 and i128 helpers, the TLS descriptor of a dynamic-TLS address,
    /// libcalls, inline asm and intrinsics are all outside the list), never
    /// address the frame through x29 and need no slot of their own. An
    /// allowlist on purpose: an IR variant not proven call-free here keeps
    /// the frame, so a future lowering that emits a libcall closes the form
    /// instead of returning through a clobbered x30.
    fn arm_frameless_leaf_ir(&self, func: &IrFunction, sibling_calls: bool) -> bool {
        use crate::common::tls::TlsModel;
        use crate::ir::reexports::IrConst;
        let scalar = |ty: &IrType| {
            matches!(
                ty,
                IrType::I8
                    | IrType::I16
                    | IrType::I32
                    | IrType::I64
                    | IrType::U8
                    | IrType::U16
                    | IrType::U32
                    | IrType::U64
                    | IrType::F32
                    | IrType::F64
                    | IrType::Ptr
            )
        };
        for block in &func.blocks {
            for inst in &block.instructions {
                match inst {
                    Instruction::BinOp { ty, .. }
                    | Instruction::UnaryOp { ty, .. }
                    | Instruction::Cmp { ty, .. }
                    | Instruction::Load { ty, .. }
                    | Instruction::Store { ty, .. }
                    | Instruction::Select { ty, .. }
                    | Instruction::ParamRef { ty, .. }
                    | Instruction::Alloca { ty, .. } => {
                        if !scalar(ty) {
                            return false;
                        }
                    }
                    Instruction::Cast { from_ty, to_ty, .. } => {
                        if !scalar(from_ty) || !scalar(to_ty) {
                            return false;
                        }
                    }
                    Instruction::Copy { src, .. } => {
                        if matches!(
                            src,
                            Operand::Const(IrConst::LongDouble(..) | IrConst::I128(..))
                        ) {
                            return false;
                        }
                    }
                    Instruction::GetElementPtr { .. } | Instruction::LabelAddr { .. } => {}
                    // Every call a tail branch (`every_call_is_sibling`).
                    Instruction::Call { .. } | Instruction::CallIndirect { .. }
                        if sibling_calls => {}
                    // Scalar FP intrinsics lower to inline instructions on
                    // caller-saved scratch (`fabs`, `fsqrt`, `frint*`, the
                    // `fmadd` family, copysign's bit merge in x0/x1): no
                    // call and no x29.  `fabs(x)` kept a frame record.
                    Instruction::Intrinsic { op, .. } if arm_call_free_scalar_intrinsic(op) => {}
                    Instruction::GlobalAddr { name, .. } => {
                        if self.state.tls_symbols.contains(name)
                            && matches!(
                                self.state.tls_model(name),
                                TlsModel::GlobalDynamic | TlsModel::LocalDynamic
                            )
                        {
                            return false;
                        }
                    }
                    _ => return false,
                }
            }
        }
        true
    }

    // ---- emit_epilogue ----

    pub(super) fn emit_epilogue_impl(&mut self, frame_size: i64) {
        if self.conditional_increment_leaf || self.frameless_leaf {
            return;
        }
        self.emit_restore_callee_saved();
        self.emit_epilogue_arm(frame_size);
    }

    // ---- emit_store_params ----

    pub(super) fn emit_store_params_impl(&mut self, func: &IrFunction) {
        // One O(instructions + params) pass replaces every
        // parameter-home full scan in this prologue.
        let param_homes = ParamAllocaIndex::build(func);
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
                param_homes.get(i).and_then(|(dest, ty)| {
                    self.state.get_slot(dest.0).map(|slot| (slot, ty, dest.0))
                })
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
                            // Canonical, like every ParamRef: a 32-bit
                            // argument's upper half is unspecified.
                            let src_reg = ARM_ARG_REGS[actual_idx];
                            if let Some(line) =
                                canonical_gp_param_move(dest_reg, src_reg, func.params[i].ty)
                            {
                                self.state.emit(&line);
                            }
                            self.state.param_pre_stored.insert(i);
                        }
                    }
                }
            }
        }

        self.emit_store_gp_params(func, &param_homes, &param_classes);
        self.emit_store_hfa_params(func, &param_homes, &param_classes);
        self.emit_store_fp_params(func, &param_homes, &param_classes);
        self.emit_store_stack_params(func, &param_homes, &param_classes);

        // The parameter that arrives in x0 and is read only at its own
        // ParamRef (no pre-store, no alloca capture): until that ParamRef
        // runs, no other ParamRef may bounce through x0.
        self.x0_param_pending = param_classes.iter().enumerate().find_map(|(i, class)| {
            let ParamClass::IntReg { reg_idx } = *class else {
                return None;
            };
            let in_x0 = reg_idx == sret_shift && !(sret_shift > 0 && i == 0);
            let captured = self.state.param_pre_stored.contains(&i)
                || self
                    .state
                    .param_alloca_slots
                    .get(i)
                    .is_some_and(|slot| slot.is_some());
            (in_x0 && !captured).then_some(i)
        });
    }

    // ---- emit_param_ref ----

    pub(super) fn emit_param_ref_impl(&mut self, dest: &Value, param_idx: usize, ty: IrType) {
        if param_idx >= self.state.param_classes.len() {
            return;
        }

        // x0 is the ParamRef bounce register, and it is also where the first
        // integer parameter arrives. A ParamRef of any OTHER parameter that
        // runs while that one is still unread must not bounce through x0:
        // `double f(double d, int k)` with `k` homed in a slot (or in a
        // shared callee-saved register, which is not pre-stored) emitted
        // `fmov x0, d0` for `d` and then stored that as `k`. Those ParamRefs
        // bounce through x16 instead (IP0: never an argument register, never
        // an allocation home, and free at entry -- the large-offset paths use
        // x17). A destination with neither a register nor a slot lives in x0
        // alone, so it keeps x0.
        let x0_busy = self.x0_param_pending.is_some_and(|j| j != param_idx);
        if self.x0_param_pending == Some(param_idx) {
            self.x0_param_pending = None;
        }

        // If this param was pre-stored directly to its register-allocated
        // destination during emit_store_params, the value is already in place.
        // No code needs to be emitted — the register already holds the value.
        if self.state.param_pre_stored.contains(&param_idx) {
            return;
        }

        let homed =
            self.reg_assignments.contains_key(&dest.0) || self.state.get_slot(dest.0).is_some();
        let (sx, sw) = if x0_busy && homed {
            ("x16", "w16")
        } else {
            ("x0", "w0")
        };

        if param_idx < self.state.param_alloca_slots.len() {
            if let Some((slot, alloca_ty, alloca_id)) = self.state.param_alloca_slots[param_idx] {
                let ldr_instr = self.load_instr_for_type_impl(alloca_ty);
                let (actual_instr, reg) = Self::arm_parse_load_to_reg(ldr_instr, sx, sw);
                if self.state.alloca_over_align(alloca_id).is_some() {
                    // Over-aligned (>16) param alloca: the capture wrote the
                    // EFFECTIVE align_up'd address (x9 convention of
                    // emit_alloca_aligned_addr_impl); a raw slot load would
                    // read the alignment pad.
                    self.emit_alloca_aligned_addr_impl(slot, alloca_id);
                    self.state
                        .emit_fmt(format_args!("    {} {}, [x9]", actual_instr, reg));
                } else {
                    self.emit_load_from_sp(reg, slot.0, actual_instr);
                }
                self.store_param_bounce_to(dest, sx);
                return;
            }
        }

        let class = self.state.param_classes[param_idx];

        // AArch64 ABI: sret shifts GP register indices
        let sret_shift = if self.state.uses_sret { 1usize } else { 0 };

        match class {
            // The hidden return pointer of an FP-register result is the
            // address of the private return buffer.
            ParamClass::RegReturnBuffer { .. } => {
                if let Some((offset, _)) = self.fp_ret_buffer {
                    self.emit_add_sp_offset(sx, offset);
                    self.store_param_bounce_to(dest, sx);
                }
            }
            ParamClass::IntReg { reg_idx } => {
                let arg = if sret_shift > 0 && reg_idx == 0 && param_idx == 0 {
                    "x8"
                } else {
                    let actual_idx = if reg_idx >= sret_shift {
                        reg_idx - sret_shift
                    } else {
                        reg_idx
                    };
                    ARM_ARG_REGS[actual_idx]
                };
                // A caller-saved home (Phase 2p, `param_volatile_pool`) is
                // written straight from the argument register -- in place
                // for an arrival home -- with the bits the bounce produced
                // (`canonical_gp_param_move`), one move less. The pool never
                // holds another argument register, so the write cannot
                // overwrite a parameter that is still unread.
                if let Some(&home) = self.reg_assignments.get(&dest.0) {
                    if !is_arm_fp_phys(home) && !(19..=28).contains(&home.0) {
                        if let Some(line) =
                            canonical_gp_param_move(callee_saved_name(home), arg, ty)
                        {
                            self.state.emit(&line);
                        }
                        return;
                    }
                }
                if let Some(line) = canonical_gp_param_move(sx, arg, ty) {
                    self.state.emit(&line);
                }
                self.store_param_bounce_to(dest, sx);
            }
            ParamClass::FloatReg { reg_idx } => {
                // An FP home takes the argument register directly: `fmov s, s`
                // / `fmov d, d` leave the bits the GP bounce produced (the
                // value in the low lane, every other bit zero) without
                // touching x0 at all.
                if let Some(&home) = self.reg_assignments.get(&dest.0) {
                    if is_arm_fp_phys(home) && matches!(ty, IrType::F32 | IrType::F64) {
                        let lane = if ty == IrType::F32 { 's' } else { 'd' };
                        self.state.emit_fmt(format_args!(
                            "    fmov {}{}, {}{}",
                            lane,
                            home.0 - 24,
                            lane,
                            reg_idx
                        ));
                        return;
                    }
                }
                if ty == IrType::U16 {
                    self.state
                        .emit_fmt(format_args!("    fmov {}, s{}", sw, reg_idx));
                    self.state
                        .emit_fmt(format_args!("    and {}, {}, #0xffff", sw, sw));
                } else if ty == IrType::F32 {
                    self.state
                        .emit_fmt(format_args!("    fmov {}, s{}", sw, reg_idx));
                } else {
                    self.state
                        .emit_fmt(format_args!("    fmov {}, d{}", sx, reg_idx));
                }
                self.store_param_bounce_to(dest, sx);
            }
            ParamClass::StackScalar { offset } => {
                let src = self.incoming_stack_base() + offset;
                let ldr_instr = self.load_instr_for_type_impl(ty);
                let (actual_instr, reg) = Self::arm_parse_load_to_reg(ldr_instr, sx, sw);
                self.emit_load_from_sp(reg, src, actual_instr);
                self.store_param_bounce_to(dest, sx);
            }
            _ => {}
        }
    }

    /// Store a ParamRef's bounce register (`x0` or `x16`, see
    /// `emit_param_ref_impl`) to the destination's home. Only x0 carries the
    /// accumulator bookkeeping; an x16 bounce leaves x0 -- and the parameter
    /// still waiting in it -- untouched.
    fn store_param_bounce_to(&mut self, dest: &Value, bounce: &str) {
        if bounce == "x0" {
            self.store_x0_to(dest);
            return;
        }
        if let Some(&reg) = self.reg_assignments.get(&dest.0) {
            if is_arm_fp_phys(reg) {
                self.state
                    .emit_fmt(format_args!("    fmov d{}, {}", reg.0 - 24, bounce));
            } else {
                self.state.emit_fmt(format_args!(
                    "    mov {}, {}",
                    callee_saved_name(reg),
                    bounce
                ));
            }
        } else if let Some(slot) = self.state.get_slot(dest.0) {
            self.emit_store_to_sp(bounce, slot.0, "str");
        }
    }

    // ---- emit_epilogue_and_ret ----

    pub(super) fn emit_epilogue_and_ret_impl(&mut self, frame_size: i64) {
        self.state.flush_epilogue_loc();
        if !self.conditional_increment_leaf && !self.frameless_leaf {
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
            IrType::U32 | IrType::F32 | IrType::D32 => "ldr32",
            _ => "ldr64",
        }
    }

    fn rehome_adjacent_global_addr_ptrs(
        &mut self,
        func: &IrFunction,
        reg_assigned: &mut crate::common::fx_hash::FxHashMap<
            u32,
            crate::backend::regalloc::PhysReg,
        >,
    ) {
        use crate::backend::regalloc::PhysReg;
        // `state.uses_sret`, not the IR flag: an HFA result keeps the sret
        // shape but leaves x8 free.
        if self.state.uses_sret || func.is_variadic {
            return;
        }
        let x8 = PhysReg(8);
        if self.reg_assignments.values().any(|&r| r == x8) {
            return;
        }
        let mut use_counts = crate::common::fx_hash::FxHashMap::<u32, u32>::default();
        for block in &func.blocks {
            for inst in &block.instructions {
                inst.for_each_used_value(|id| *use_counts.entry(id).or_default() += 1);
            }
            block
                .terminator
                .for_each_used_value(|id| *use_counts.entry(id).or_default() += 1);
        }
        for block in &func.blocks {
            for pair in block.instructions.windows(2) {
                let Instruction::GlobalAddr { dest, .. } = &pair[0] else {
                    continue;
                };
                if use_counts.get(&dest.0).copied().unwrap_or(0) != 1 {
                    continue;
                }
                let adjacent_ptr = match &pair[1] {
                    Instruction::Load { ptr, .. } => ptr.0 == dest.0,
                    Instruction::Store { val, ptr, .. } => {
                        ptr.0 == dest.0 && !matches!(val, Operand::Value(v) if v.0 == dest.0)
                    }
                    _ => false,
                };
                if !adjacent_ptr {
                    continue;
                }
                if let Some(&old) = self.reg_assignments.get(&dest.0)
                    && (19..=28).contains(&old.0)
                {
                    self.reg_assignments.insert(dest.0, x8);
                    reg_assigned.insert(dest.0, x8);
                    if !self.reg_assignments.values().any(|&r| r == old) {
                        self.used_callee_saved.retain(|&r| r != old);
                    }
                    break;
                }
            }
        }
    }
}

/// Scalar FP intrinsics whose AArch64 lowering (`emit_intrinsic_arm`) is
/// inline instructions on caller-saved scratch registers only: no `bl`, no
/// frame-pointer access, no slot of its own.  The frameless-leaf allowlist
/// admits exactly these; every other intrinsic keeps the frame.
fn arm_call_free_scalar_intrinsic(op: &crate::ir::reexports::IntrinsicOp) -> bool {
    use crate::ir::reexports::IntrinsicOp;
    matches!(
        op,
        IntrinsicOp::FabsF32
            | IntrinsicOp::FabsF64
            | IntrinsicOp::SqrtF32
            | IntrinsicOp::SqrtF64
            | IntrinsicOp::RoundScalarF32(_)
            | IntrinsicOp::RoundScalarF64(_)
            | IntrinsicOp::FmaScalarF32
            | IntrinsicOp::FmaScalarF64
            | IntrinsicOp::FmaScalarF32Signed(..)
            | IntrinsicOp::FmaScalarF64Signed(..)
            | IntrinsicOp::CopysignF32
            | IntrinsicOp::CopysignF64
    )
}

#[cfg(test)]
mod canonical_param_move_tests {
    use super::*;

    /// AAPCS64 C.16 leaves the bits above a narrow argument unspecified:
    /// every register home receives the canonical value -- zero-extended
    /// for 32 bits (also in place), extended by signedness below that.
    #[test]
    fn narrow_arguments_are_extended_into_every_home() {
        let cases: [(&str, &str, IrType, Option<&str>); 11] = [
            ("x9", "x4", IrType::I32, Some("mov w9, w4")),
            ("x4", "x4", IrType::I32, Some("mov w4, w4")),
            ("x4", "x4", IrType::U32, Some("mov w4, w4")),
            ("x0", "x2", IrType::I8, Some("sxtb x0, w2")),
            ("x19", "x4", IrType::U8, Some("and w19, w4, #0xff")),
            ("x9", "x4", IrType::I16, Some("sxth x9, w4")),
            ("x4", "x4", IrType::U16, Some("and w4, w4, #0xffff")),
            ("x9", "x4", IrType::I64, Some("mov x9, x4")),
            ("x4", "x4", IrType::Ptr, None),
            ("x16", "x8", IrType::Ptr, Some("mov x16, x8")),
            ("x9", "x4", IrType::F32, Some("mov w9, w4")),
        ];
        for (dst, arg, ty, want) in cases {
            let got = canonical_gp_param_move(dst, arg, ty);
            assert_eq!(
                got.as_deref().map(str::trim),
                want,
                "{dst} <- {arg} as {ty:?}"
            );
        }
    }
}

#[cfg(test)]
mod pool_scratch_tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, Instruction, Terminator, Value};

    fn func(insts: Vec<Instruction>) -> IrFunction {
        let mut f = IrFunction::new("pool".to_string(), IrType::Void, vec![], false);
        f.blocks = vec![BasicBlock {
            label: BlockId(0),
            instructions: insts,
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        }];
        f
    }

    /// Each audited scratch user removes exactly its registers; anything not
    /// on the allowlist closes the whole pool.
    #[test]
    fn scratch_users_remove_their_registers_and_unknowns_fail_closed() {
        let memcpy = || Instruction::Memcpy {
            dest: Value(1),
            src: Value(2),
            size: 64,
        };
        let va = || Instruction::VaStart {
            va_list_ptr: Value(3),
        };
        let fence = || Instruction::Fence {
            ordering: crate::ir::reexports::AtomicOrdering::SeqCst,
        };
        assert_eq!(ArmCodegen::arm_pool_scratch(&func(vec![])), &[] as &[u8]);
        assert_eq!(
            ArmCodegen::arm_pool_scratch(&func(vec![fence()])),
            &[] as &[u8]
        );
        assert_eq!(ArmCodegen::arm_pool_scratch(&func(vec![memcpy()])), &[13]);
        assert_eq!(ArmCodegen::arm_pool_scratch(&func(vec![va()])), &[4, 5, 6]);
        assert_eq!(
            ArmCodegen::arm_pool_scratch(&func(vec![va(), memcpy()])),
            &[4, 5, 6, 13]
        );
        let unknown = Instruction::StackSave { dest: Value(4) };
        assert_eq!(
            ArmCodegen::arm_pool_scratch(&func(vec![memcpy(), unknown])),
            &[4, 5, 6, 7, 8, 13, 14]
        );
    }
}
