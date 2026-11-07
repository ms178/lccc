//! ArmCodegen: function call operations.

use super::emit::{ArmCodegen, arm_fp_name, callee_saved_name, is_arm_fp_phys};
use crate::backend::call_abi::{CallAbiConfig, CallArgClass, compute_stack_arg_space};
use crate::backend::generation::is_i128_type;
use crate::backend::traits::ArchCodegen;
use crate::common::types::IrType;
use crate::ir::reexports::{IrConst, Operand, Value};

/// The AAPCS64 GP argument registers (bit N = xN) that a call with these
/// argument classes passes values in, or `None` when a class could stage a
/// GP register this does not model -- the call is then published without
/// the annotation and the peephole keeps assuming all of x0-x8.
///
/// FP/SIMD-register, stack and zero-size classes name no GP register.  A
/// span reaching past x7 cannot come from the AAPCS64 classifier, so it is
/// refused rather than truncated.
pub(super) fn call_gp_arg_reads(arg_classes: &[CallArgClass]) -> Option<u16> {
    let mut mask = 0u16;
    for class in arg_classes {
        let (base, count) = match *class {
            CallArgClass::IntReg { reg_idx } => (reg_idx, 1),
            CallArgClass::I128RegPair { base_reg_idx } => (base_reg_idx, 2),
            CallArgClass::StructByValReg { base_reg_idx, size } => {
                (base_reg_idx, size.div_ceil(8).max(1))
            }
            CallArgClass::FloatReg { .. }
            | CallArgClass::F128Reg { .. }
            | CallArgClass::StructHfaReg { .. }
            | CallArgClass::Stack
            | CallArgClass::F128Stack
            | CallArgClass::I128Stack
            | CallArgClass::StructByValStack { .. }
            | CallArgClass::LargeStructStack { .. }
            | CallArgClass::ZeroSizeSkip => continue,
            _ => return None,
        };
        let end = base.checked_add(count).filter(|&end| end <= 8)?;
        for reg in base..end {
            mask |= 1 << reg;
        }
    }
    Some(mask)
}

/// The significant width of the integer argument in each GP argument
/// register (`call_gp_narrow`): 8, 16 or 32 for an argument of that C width,
/// 0 for a 64-bit (pointer, `long`) one or a register no integer class
/// names.  Only `IntReg` classes count -- a struct or `__int128` in GP
/// registers carries all of its bits.
pub(super) fn call_gp_narrow_widths(arg_classes: &[CallArgClass], arg_types: &[IrType]) -> [u8; 8] {
    let mut widths = [0u8; 8];
    for (class, ty) in arg_classes.iter().zip(arg_types) {
        if let CallArgClass::IntReg { reg_idx } = *class
            && reg_idx < 8
        {
            widths[reg_idx] = match ty {
                IrType::I8 | IrType::U8 => 8,
                IrType::I16 | IrType::U16 => 16,
                IrType::I32 | IrType::U32 => 32,
                _ => 0,
            };
        }
    }
    widths
}

/// A type whose value travels in one GP or one FP/SIMD register and whose
/// call setup and result handling never call a helper: the argument and
/// result types a sibling call of a frameless function may have.
pub(super) fn sibcall_scalar(ty: IrType) -> bool {
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
}

impl ArmCodegen {
    /// The AAPCS64 sibling-call refusals of a call site (the x86-64 design
    /// of `X86Codegen::sibcall_abi_admits`, ported): whether the site may
    /// become `<epilogue>; b target`.  The arming below and the frameless
    /// prediction (`every_call_is_sibling`) share it, so they can never
    /// disagree.
    ///
    /// The soundness core is the zero-stack-argument guard: with every
    /// argument in x0-x7/v0-v7 (the epilogue restores only x19-x30, d8-d15
    /// and sp), the callee reads nothing of this frame, which the epilogue
    /// releases.  The function-level guards of `sibcall_function_use_counts`
    /// (no used local alloca -- its address could reach the callee -- no
    /// dynamic stack, no inline asm) and the driver's block-tail shape
    /// (`sibcall_block_hint`: the call is the block's last instruction, its
    /// result is the block's `Return` operand or unused under a void return,
    /// and the callee does not return twice) are the rest of the proof.
    /// Refused besides: non-default calling conventions, variadic, sret,
    /// fastcall/regparm callees; a function returning through a hidden
    /// buffer or a `_Float128` carrier (its return sequence does work after
    /// the call); and results other than the same scalar in the same
    /// register as this function's own (x0/w0 or s0/d0).
    pub(super) fn sibcall_abi_admits(&self, info: &crate::ir::reexports::CallInfo) -> bool {
        use crate::common::types::CallingConvention;
        if info.calling_convention != CallingConvention::Default
            || self.state.func_calling_convention != CallingConvention::Default
            || info.is_variadic
            || info.is_sret
            || info.is_fastcall
            || info.regparm.is_some()
            || info.ret_is_f128_sse
            || self.state.uses_sret
            || self.fp_ret_buffer.is_some()
            || self.func_ret_is_f128_sse
        {
            return false;
        }
        let ours = self.current_return_type;
        let result_ok = ours == IrType::Void
            || (info.return_type == ours && (sibcall_scalar(ours) || is_i128_type(ours)));
        if !result_ok {
            return false;
        }
        let config = self.call_abi_config_impl();
        let classes = crate::backend::call_abi::classify_call_args(
            &info.args,
            &info.arg_types,
            &info.struct_arg_sizes,
            &info.struct_arg_aligns,
            &info.struct_arg_classes,
            &info.struct_arg_fp_agg_classes,
            &info.struct_arg_is_f128_sse,
            None,
            &config,
        );
        !classes.iter().any(CallArgClass::is_stack)
    }

    pub(super) fn arm_sibcall_candidate_impl(&mut self, info: &crate::ir::reexports::CallInfo) {
        // One conversion in flight at a time; `emit_return_impl` consumes it
        // before the next block begins.
        if self.state.pending_sibcall.is_some() {
            return;
        }
        if self.sibcall_abi_admits(info) {
            self.state.pending_sibcall = Some(crate::backend::state::SibcallPending::Armed {
                preserve_upper_vector_lanes: false,
            });
        }
    }

    pub(super) fn call_abi_config_impl(&self) -> CallAbiConfig {
        CallAbiConfig::aapcs64()
    }

    pub(super) fn emit_call_compute_stack_space_impl(
        &self,
        arg_classes: &[CallArgClass],
        _arg_types: &[IrType],
        struct_arg_aligns: &[Option<usize>],
        _struct_arg_classes: &[Vec<crate::common::types::EightbyteClass>],
        _struct_arg_is_f128_sse: &[bool],
    ) -> usize {
        compute_stack_arg_space(arg_classes, struct_arg_aligns)
    }

    pub(super) fn emit_call_f128_pre_convert_impl(
        &mut self,
        args: &[Operand],
        arg_classes: &[CallArgClass],
        _arg_types: &[IrType],
        _stack_arg_space: usize,
    ) -> usize {
        let _ = (args, arg_classes);
        0
    }

    /// `x0` = the value homed in allocator register `reg`: the raw bits of
    /// an FP home (d8-d14, v16-v31) by `fmov`, a GP home by `mov`.  The
    /// stack-argument writer sees values of every class, and
    /// `callee_saved_name` knows only GP registers: a ninth `double` kept
    /// in d8 aborted the compiler with "invalid ARM register index".
    fn emit_home_to_x0(&mut self, reg: crate::backend::regalloc::PhysReg) {
        if is_arm_fp_phys(reg) {
            self.state.emit_fmt(format_args!(
                "    fmov x0, {}",
                arm_fp_name(reg, IrType::F64)
            ));
        } else {
            self.state
                .emit_fmt(format_args!("    mov x0, {}", callee_saved_name(reg)));
        }
    }

    pub(super) fn emit_call_stack_args_impl(
        &mut self,
        args: &[Operand],
        arg_classes: &[CallArgClass],
        _arg_types: &[IrType],
        stack_arg_space: usize,
        fptr_spill: usize,
        _f128_temp_space: usize,
        struct_arg_aligns: &[Option<usize>],
        _struct_arg_classes: &[Vec<crate::common::types::EightbyteClass>],
        _struct_arg_is_f128_sse: &[bool],
    ) -> i64 {
        if stack_arg_space > 0 {
            self.emit_sub_sp(stack_arg_space as i64);
            let src_adjust = if self.state.has_dyn_alloca {
                0
            } else {
                stack_arg_space as i64 + fptr_spill as i64
            };
            // Per-arg alignment padding (AAPCS64: a composite whose natural
            // alignment is 16 is placed 16-byte aligned in the overflow
            // area; F128/I128 are always 16-byte aligned). Must match
            // compute_stack_arg_space.
            let arg_padding = crate::backend::call_abi::compute_stack_arg_padding(
                arg_classes,
                struct_arg_aligns,
                16,
            );
            let mut stack_offset = 0i64;
            for (arg_idx, arg) in args.iter().enumerate() {
                if !arg_classes[arg_idx].is_stack() {
                    continue;
                }
                stack_offset += arg_padding[arg_idx] as i64;
                let cls = arg_classes[arg_idx];
                match cls {
                    CallArgClass::StructByValStack { size }
                    | CallArgClass::LargeStructStack { size } => {
                        let n_dwords = size.div_ceil(8);
                        match arg {
                            Operand::Value(v) => {
                                if let Some(&reg) = self.reg_assignments.get(&v.0) {
                                    self.emit_home_to_x0(reg);
                                } else if let Some(slot) = self.state.get_slot(v.0) {
                                    let adjusted = slot.0 + src_adjust;
                                    if self.state.is_alloca(v.0) {
                                        self.emit_alloca_addr("x0", v.0, adjusted);
                                    } else {
                                        self.emit_load_from_sp("x0", adjusted, "ldr");
                                    }
                                } else {
                                    self.state.emit("    mov x0, #0");
                                }
                            }
                            Operand::Const(_) => {
                                self.operand_to_x0(arg);
                            }
                        }
                        for qi in 0..n_dwords {
                            let src_off = (qi * 8) as i64;
                            self.emit_load_from_reg("x1", "x0", src_off, "ldr");
                            self.emit_store_to_raw_sp("x1", stack_offset + src_off, "str");
                        }
                        stack_offset += (n_dwords as i64) * 8;
                    }
                    CallArgClass::I128Stack => {
                        match arg {
                            Operand::Const(c) => {
                                if let IrConst::I128(v) = c {
                                    self.emit_load_imm64("x0", *v as u64 as i64);
                                    self.emit_store_to_raw_sp("x0", stack_offset, "str");
                                    self.emit_load_imm64("x0", (*v >> 64) as u64 as i64);
                                    self.emit_store_to_raw_sp("x0", stack_offset + 8, "str");
                                } else {
                                    self.operand_to_x0(arg);
                                    self.emit_store_to_raw_sp("x0", stack_offset, "str");
                                    self.state.emit("    mov x0, #0");
                                    self.emit_store_to_raw_sp("x0", stack_offset + 8, "str");
                                }
                            }
                            Operand::Value(v) => {
                                if let Some(slot) = self.state.get_slot(v.0) {
                                    let adjusted = slot.0 + src_adjust;
                                    if self.state.is_alloca(v.0) {
                                        self.emit_alloca_addr("x0", v.0, adjusted);
                                        self.emit_store_to_raw_sp("x0", stack_offset, "str");
                                        self.state.emit("    mov x0, #0");
                                        self.emit_store_to_raw_sp("x0", stack_offset + 8, "str");
                                    } else {
                                        self.emit_load_from_sp("x0", adjusted, "ldr");
                                        self.emit_store_to_raw_sp("x0", stack_offset, "str");
                                        self.emit_load_from_sp("x0", adjusted + 8, "ldr");
                                        self.emit_store_to_raw_sp("x0", stack_offset + 8, "str");
                                    }
                                } else {
                                    self.state.emit("    mov x0, #0");
                                    self.emit_store_to_raw_sp("x0", stack_offset, "str");
                                    self.emit_store_to_raw_sp("x0", stack_offset + 8, "str");
                                }
                            }
                        }
                        stack_offset += 16;
                    }
                    CallArgClass::F128Stack => {
                        match arg {
                            Operand::Const(c) => {
                                let bytes = match c {
                                    IrConst::LongDouble(_, f128_bytes) => *f128_bytes,
                                    _ => {
                                        let f64_val = c.to_f64().unwrap_or(0.0);
                                        crate::ir::reexports::f64_to_f128_bytes(f64_val)
                                    }
                                };
                                let lo = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
                                let hi = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
                                self.emit_load_imm64("x0", lo as i64);
                                self.emit_store_to_raw_sp("x0", stack_offset, "str");
                                self.emit_load_imm64("x0", hi as i64);
                                self.emit_store_to_raw_sp("x0", stack_offset + 8, "str");
                            }
                            Operand::Value(v) => {
                                let mut loaded_full = false;
                                if let Some((src_id, offset, is_indirect)) =
                                    self.state.get_f128_source(v.0)
                                {
                                    if !is_indirect {
                                        if let Some(src_slot) = self.state.get_slot(src_id) {
                                            let adj = src_slot.0 + offset + src_adjust;
                                            self.emit_load_from_sp("q0", adj, "ldr");
                                            self.emit_store_to_raw_sp("q0", stack_offset, "str");
                                            loaded_full = true;
                                        }
                                    } else if let Some(src_slot) = self.state.get_slot(src_id) {
                                        let adj = src_slot.0 + src_adjust;
                                        self.emit_load_from_sp("x17", adj, "ldr");
                                        if offset != 0 {
                                            if offset > 0 && offset <= 4095 {
                                                self.state.emit_fmt(format_args!(
                                                    "    add x17, x17, #{}",
                                                    offset
                                                ));
                                            } else {
                                                self.load_large_imm("x16", offset);
                                                self.state.emit("    add x17, x17, x16");
                                            }
                                        }
                                        self.state.emit("    ldr q0, [x17]");
                                        self.emit_store_to_raw_sp("q0", stack_offset, "str");
                                        loaded_full = true;
                                    }
                                }
                                if !loaded_full {
                                    if let Some(&reg) = self.reg_assignments.get(&v.0) {
                                        self.emit_home_to_x0(reg);
                                    } else if let Some(slot) = self.state.get_slot(v.0) {
                                        let adjusted = slot.0 + src_adjust;
                                        if self.state.is_alloca(v.0) {
                                            self.emit_alloca_addr("x0", v.0, adjusted);
                                        } else {
                                            self.emit_load_from_sp("x0", adjusted, "ldr");
                                        }
                                    } else {
                                        self.state.emit("    mov x0, #0");
                                    }
                                    self.state.emit("    fmov d0, x0");
                                    self.state.emit("    stp x9, x10, [sp, #-16]!");
                                    self.state.emit("    bl __extenddftf2");
                                    self.state.emit("    ldp x9, x10, [sp], #16");
                                    self.emit_store_to_raw_sp("q0", stack_offset, "str");
                                }
                            }
                        }
                        stack_offset += 16;
                    }
                    CallArgClass::Stack => {
                        match arg {
                            Operand::Value(v) => {
                                if let Some(&reg) = self.reg_assignments.get(&v.0) {
                                    self.emit_home_to_x0(reg);
                                } else if let Some(slot) = self.state.get_slot(v.0) {
                                    let adjusted = slot.0 + src_adjust;
                                    if self.state.is_alloca(v.0) {
                                        self.emit_alloca_addr("x0", v.0, adjusted);
                                    } else {
                                        self.emit_load_from_sp("x0", adjusted, "ldr");
                                    }
                                } else {
                                    self.state.emit("    mov x0, #0");
                                }
                            }
                            Operand::Const(_) => {
                                self.operand_to_x0(arg);
                            }
                        }
                        self.emit_store_to_raw_sp("x0", stack_offset, "str");
                        stack_offset += 8;
                    }
                    _ => {}
                }
            }
        }
        stack_arg_space as i64 + fptr_spill as i64
    }

    pub(super) fn emit_call_reg_args_impl(
        &mut self,
        args: &[Operand],
        arg_classes: &[CallArgClass],
        arg_types: &[IrType],
        total_sp_adjust: i64,
        _f128_temp_space: usize,
        _stack_arg_space: usize,
        _struct_arg_fp_agg_classes: &[Option<crate::common::types::FpAggClass>],
    ) {
        let slot_adjust = if self.state.has_dyn_alloca {
            0
        } else {
            total_sp_adjust
        };
        let needs_adjusted_load = total_sp_adjust > 0;
        self.call_gp_reads = call_gp_arg_reads(arg_classes);
        self.call_gp_narrow = call_gp_narrow_widths(arg_classes, arg_types);

        self.emit_call_gp_to_temps(args, arg_classes, slot_adjust, needs_adjusted_load);
        self.emit_call_fp_reg_args(
            args,
            arg_classes,
            arg_types,
            slot_adjust,
            needs_adjusted_load,
        );
        self.emit_call_move_temps_to_arg_regs(args, arg_classes);
        self.emit_call_i128_reg_args(args, arg_classes, slot_adjust, needs_adjusted_load);
        self.emit_call_struct_byval_reg_args(args, arg_classes, slot_adjust, needs_adjusted_load);
        self.emit_call_hfa_reg_args(args, arg_classes, slot_adjust, needs_adjusted_load);
    }

    pub(super) fn emit_call_instruction_impl(
        &mut self,
        direct_name: Option<&str>,
        _func_ptr: Option<&Operand>,
        indirect: bool,
        stack_arg_space: usize,
    ) {
        // Taken on every path: a stale set must never reach a later call.
        let reads = self.call_gp_reads.take();
        let narrow = std::mem::take(&mut self.call_gp_narrow);
        // Sibling call: the argument setup ran unchanged; the `bl`/`blr` is
        // replaced by the block's `Return` emitting `<epilogue>; b target`
        // (or `br x17`: IP1 survives the epilogue, and a BR through x16/x17
        // is the one indirect branch a BTI `bti c` landing pad admits).  The
        // call's read annotation travels to the tail branch as a marker row.
        if matches!(
            self.state.pending_sibcall,
            Some(crate::backend::state::SibcallPending::Armed { .. })
        ) {
            use crate::backend::state::{SibcallPending, SibcallTarget};
            let target = if let Some(name) = direct_name {
                Some(SibcallTarget::Direct(name.to_string()))
            } else if indirect {
                self.load_spilled_call_target(stack_arg_space as i64);
                Some(SibcallTarget::Reg("x17"))
            } else {
                None
            };
            if let Some(target) = target {
                self.state.sibcall_marker_lines = vec![format!(
                    "    {}{}{}",
                    super::peephole::TAIL_READS_MARKER,
                    reads.map_or_else(|| "511".to_string(), |m| m.to_string()),
                    super::peephole::late::format_call_narrow(&narrow)
                )];
                self.state.pending_sibcall = Some(SibcallPending::Ready {
                    target,
                    preserve_upper_vector_lanes: false,
                });
                return;
            }
            self.state.pending_sibcall = None;
        }
        // A frameless function was planned on the prediction that every
        // call becomes a sibling call (`every_call_is_sibling`); should a
        // call ever be emitted anyway, x30 is preserved around it rather
        // than returning through a clobbered link register.
        let guard_link = self.frameless_leaf;
        if guard_link {
            self.state.emit("    str x30, [sp, #-16]!");
        }
        self.emit_plain_call(direct_name, indirect, stack_arg_space, reads, &narrow);
        if guard_link {
            self.state.emit("    ldr x30, [sp], #16");
        }
    }

    /// `ldr x17, <the Phase 0 spill of an indirect call's target>`.
    fn load_spilled_call_target(&mut self, spill_offset: i64) {
        if Self::is_valid_imm_offset(spill_offset, "ldr", "x17") {
            self.state
                .emit_fmt(format_args!("    ldr x17, [sp, #{}]", spill_offset));
        } else {
            self.load_large_imm("x17", spill_offset);
            self.state.emit("    add x17, sp, x17");
            self.state.emit("    ldr x17, [x17]");
        }
    }

    /// The `bl`/`blr` of an ordinary call, annotated for the peephole.
    fn emit_plain_call(
        &mut self,
        direct_name: Option<&str>,
        indirect: bool,
        stack_arg_space: usize,
        reads: Option<u16>,
        narrow: &[u8; 8],
    ) {
        if let Some(name) = direct_name {
            match reads {
                Some(mask) => self.state.emit_fmt(format_args!(
                    "    bl {}{}{}{}",
                    name,
                    super::peephole::CALL_READS_TAG,
                    mask,
                    super::peephole::late::format_call_narrow(narrow)
                )),
                None => self.state.emit_fmt(format_args!("    bl {}", name)),
            }
        } else if indirect {
            self.load_spilled_call_target(stack_arg_space as i64);
            self.state.emit("    blr x17");
        }
    }

    pub(super) fn emit_call_cleanup_impl(
        &mut self,
        stack_arg_space: usize,
        _f128_temp_space: usize,
        indirect: bool,
    ) {
        let fptr_spill = if indirect { 16usize } else { 0 };
        let total = stack_arg_space + fptr_spill;
        if total > 0 {
            self.emit_add_sp(total as i64);
        }
    }

    pub(super) fn emit_call_store_i128_result_impl(&mut self, dest: &Value) {
        self.store_x0_x1_to(dest);
    }

    pub(super) fn set_call_ret_is_f128_sse_impl(&mut self, is_f128: bool) {
        self.call_ret_is_f128_sse = is_f128;
    }

    /// Result-store dispatch: a _Float128 carrier (ret_is_f128_sse) arrives
    /// in Q0 per AAPCS64 — store all 16 bytes and mark the dest slot as a
    /// full-precision f128 source. Everything else keeps the shared
    /// dispatch (I128 pair / long double / FP / integer).
    pub(super) fn emit_call_store_result_impl(&mut self, dest: &Value, return_type: IrType) {
        // Sibling call: the result is never materialised -- the tail
        // callee's exit is this function's, its result registers already
        // hold what our `Return` hands over.
        if self.state.pending_sibcall.is_some() {
            return;
        }
        if self.call_ret_is_f128_sse {
            if return_type == IrType::U16 {
                self.state.emit("    fmov w0, s0");
                self.state.emit("    and w0, w0, #0xffff");
                self.emit_store_result(dest);
                return;
            }
            self.emit_call_store_f128_result_impl(dest);
            return;
        }
        if is_i128_type(return_type) {
            self.emit_call_store_i128_result_impl(dest);
        } else if return_type.is_long_double() {
            self.emit_call_store_f128_result_impl(dest);
        } else if matches!(return_type, IrType::F32 | IrType::F64)
            && self.store_fp_call_result_direct(dest, return_type == IrType::F32)
        {
        } else if return_type == IrType::F32 {
            self.state.emit("    fmov w0, s0");
            self.emit_store_result(dest);
        } else if return_type.is_float() {
            self.state.emit("    fmov x0, d0");
            self.emit_store_result(dest);
        } else {
            self.emit_store_result(dest);
        }
    }

    /// Store an `f32`/`f64` call result from s0/d0 straight into its home:
    /// an FP register (`fmov d16, d0`), a general one (`fmov x5, d0`) or a
    /// frame slot (`str d0, [sp, #off]`) -- the generic path bounced it
    /// through x0 (`fmov x0, d0; str x0, [...]`).  x0 then holds whatever
    /// the callee left, so the accumulator cache forgets it.  `false` when
    /// `dest` has no home (the generic path keeps it in x0).
    fn store_fp_call_result_direct(&mut self, dest: &Value, single: bool) -> bool {
        let fp = if single { 's' } else { 'd' };
        if let Some(&home) = self.reg_assignments.get(&dest.0) {
            if is_arm_fp_phys(home) {
                self.state
                    .emit_fmt(format_args!("    fmov {fp}{}, {fp}0", home.0 - 24));
            } else {
                let gpr = if single {
                    super::emit::callee_saved_name_32(home)
                } else {
                    super::emit::callee_saved_name(home)
                };
                self.state.emit_fmt(format_args!("    fmov {gpr}, {fp}0"));
            }
        } else if let Some(slot) = self.state.get_slot(dest.0) {
            self.emit_store_to_sp(&format!("{fp}0"), slot.0, "str");
        } else {
            return false;
        }
        self.state.reg_cache.invalidate_acc();
        true
    }

    pub(super) fn emit_call_store_f128_result_impl(&mut self, dest: &Value) {
        if let Some(slot) = self.state.get_slot(dest.0) {
            self.emit_store_to_sp("q0", slot.0, "str");
            self.state.track_f128_self(dest.0);
        }
        // The q1 spill + __trunctfdf2 dance that used to live here was dead
        // code: q1 is caller-clobbered by the call (it held the second
        // argument, not the result), so it truncated garbage, and x0 was
        // overwritten before any consumer could read the "approximation".
        // One libcall + two memory round-trips saved per f128 call.  x0
        // holds whatever the callee left there, so the accumulator cache
        // must not claim it holds `dest`: every consumer reads the 16-byte
        // home.
        self.state.reg_cache.invalidate_all();
    }
}

#[cfg(test)]
mod call_gp_arg_reads_tests {
    use super::call_gp_arg_reads;
    use crate::backend::call_abi::CallArgClass;

    #[test]
    fn masks_follow_the_argument_classes() {
        use CallArgClass::*;
        let classes = [
            IntReg { reg_idx: 0 },
            FloatReg { reg_idx: 0 },
            I128RegPair { base_reg_idx: 2 },
            StructByValReg {
                base_reg_idx: 4,
                size: 12,
            },
            StructHfaReg {
                fp_idx: 1,
                count: 2,
                member_size: 8,
            },
            Stack,
            ZeroSizeSkip,
        ];
        assert_eq!(call_gp_arg_reads(&classes), Some(0b11_1101));
        assert_eq!(call_gp_arg_reads(&[]), Some(0));
        assert_eq!(
            call_gp_arg_reads(&[StructByValReg {
                base_reg_idx: 7,
                size: 1
            }]),
            Some(0x80)
        );
        // Classes outside the AAPCS64 model, or a span past x7, leave the
        // call unannotated.
        assert_eq!(
            call_gp_arg_reads(&[StructSplitRegStack {
                reg_idx: 7,
                size: 12
            }]),
            None
        );
        assert_eq!(call_gp_arg_reads(&[IntReg { reg_idx: 8 }]), None);
        assert_eq!(call_gp_arg_reads(&[I128RegPair { base_reg_idx: 7 }]), None);
    }
}
