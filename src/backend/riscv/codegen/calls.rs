//! RiscvCodegen: call ABI operations.

use super::emit::{RISCV_ARG_REGS, RiscvCodegen, callee_saved_name};
use crate::backend::call_abi::{CallAbiConfig, CallArgClass, compute_stack_arg_space};
use crate::backend::generation::is_i128_type;
use crate::common::types::IrType;
use crate::ir::reexports::{IrConst, Operand, Value};

/// The psABI GP argument registers (bit k = a_k) a call with these argument
/// classes passes values in, or `None` when a class could use a GP register
/// this does not model -- the call is then published without the annotation
/// and the peephole keeps assuming it reads every caller-saved register.
/// Pairs (`__int128`, `long double`, a 9..16-byte struct) take two
/// consecutive registers; a struct split across the last register and the
/// stack takes that register.  FP-register, stack and zero-size classes name
/// no GP register.
pub(super) fn rv_call_gp_arg_reads(arg_classes: &[CallArgClass]) -> Option<u8> {
    let mut mask = 0u16;
    for class in arg_classes {
        let (base, count) = match *class {
            CallArgClass::IntReg { reg_idx } => (reg_idx, 1),
            CallArgClass::I128RegPair { base_reg_idx } => (base_reg_idx, 2),
            CallArgClass::F128Reg { reg_idx } => (reg_idx, 2),
            CallArgClass::StructByValReg { base_reg_idx, size } => {
                (base_reg_idx, size.div_ceil(8).clamp(1, 2))
            }
            CallArgClass::StructSplitRegStack { reg_idx, .. } => (reg_idx, 1),
            CallArgClass::FloatReg { .. }
            | CallArgClass::StructSseReg { .. }
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
    u8::try_from(mask).ok()
}

/// A type whose value travels in one GP or one FP register and whose call
/// setup and result handling never call a helper: the argument and result
/// types a sibling call of a frameless function may have (the AArch64
/// backend's `sibcall_scalar`).
pub(super) fn rv_sibcall_scalar(ty: IrType) -> bool {
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

impl RiscvCodegen {
    pub(super) fn call_abi_config_impl(&self) -> CallAbiConfig {
        CallAbiConfig::riscv_lp64d()
    }

    /// The psABI sibling-call refusals of a call site (the AArch64 and
    /// x86-64 `sibcall_abi_admits`, ported): whether the site may become
    /// `<epilogue>; tail target`.  The arming below and the frameless
    /// prediction (`every_call_is_sibling`) share it, so they never
    /// disagree.
    ///
    /// The soundness core is the zero-stack-argument guard: with every
    /// argument in a0-a7/fa0-fa7 (the epilogue restores only s0-s11, ra and
    /// sp), the callee reads nothing of this frame, which the epilogue
    /// releases.  The function-level guards of `sibcall_function_use_counts`
    /// (no used local alloca, no dynamic stack, no inline asm) and the
    /// driver's block-tail shape (`sibcall_block_hint`) are the rest of the
    /// proof.  Refused besides: non-default calling conventions; variadic
    /// callees (the unnamed FP arguments travel in GP registers) and a
    /// variadic caller (its register save area sits above s0, against the
    /// caller's own stack arguments); sret; a function returning through a
    /// hidden buffer or a 16-bit FP carrier (its return sequence does work
    /// after the call); and results other than the same scalar or 2xXLEN
    /// value in the same registers as this function's own.
    pub(super) fn sibcall_abi_admits(&self, info: &crate::ir::reexports::CallInfo) -> bool {
        use crate::common::types::CallingConvention;
        if info.calling_convention != CallingConvention::Default
            || self.state.func_calling_convention != CallingConvention::Default
            || info.is_variadic
            || self.is_variadic
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
            || (info.return_type == ours && (rv_sibcall_scalar(ours) || is_i128_type(ours)));
        if !result_ok {
            return false;
        }
        let classes = crate::backend::call_abi::classify_call_args(
            &info.args,
            &info.arg_types,
            &info.struct_arg_sizes,
            &info.struct_arg_aligns,
            &info.struct_arg_classes,
            &info.struct_arg_fp_agg_classes,
            &info.struct_arg_is_f128_sse,
            None,
            &self.call_abi_config_impl(),
        );
        classes.len() == info.args.len() && !classes.iter().any(CallArgClass::is_stack)
    }

    pub(super) fn riscv_sibcall_candidate_impl(&mut self, info: &crate::ir::reexports::CallInfo) {
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
        arg_types: &[IrType],
        _stack_arg_space: usize,
    ) -> usize {
        // A `long double` split across a7 and the stack is staged like the
        // register pairs: its 16 bytes may need a conversion call, which
        // must run before any argument register is written.  The split
        // takes the last GP register, so its temp follows every pair's.
        let staged = |i: usize| {
            matches!(arg_classes[i], CallArgClass::F128Reg { .. })
                || Self::is_split_f128_arg(arg_classes, arg_types, i)
        };
        let mut f128_temp_space: i64 = 0;
        for (i, arg) in args.iter().enumerate() {
            if staged(i) {
                if let Operand::Value(_) = arg {
                    f128_temp_space += 16;
                }
            }
        }
        if f128_temp_space > 0 {
            self.emit_addi_sp(-f128_temp_space);
            let mut temp_offset: i64 = 0;
            for (i, arg) in args.iter().enumerate() {
                if !staged(i) {
                    continue;
                }
                if let Operand::Value(v) = arg {
                    // _Float128 lowered to the IrType::U128 carrier carries
                    // 16 REAL bytes in its own slot even when no f128 source
                    // tracking vouches for it. Load directly — the
                    // __extenddftf2-style fallback inside
                    // emit_f128_operand_to_a0_a1 would fabricate the value
                    // from the low qword reinterpreted as a double.
                    let mut staged = false;
                    if i < arg_types.len() && arg_types[i] == IrType::U128 {
                        if let Some(slot) = self.state.get_slot(v.0) {
                            self.emit_load_from_s0("a0", slot.0, "ld");
                            self.emit_load_from_s0("a1", slot.0 + 8, "ld");
                            staged = true;
                        }
                    }
                    if !staged {
                        self.emit_f128_operand_to_a0_a1(arg);
                    }
                    self.emit_store_to_sp("a0", temp_offset, "sd");
                    self.emit_store_to_sp("a1", temp_offset + 8, "sd");
                    temp_offset += 16;
                }
            }
        }
        f128_temp_space as usize
    }

    /// Whether argument `i` is a 2×XLEN scalar -- `__int128`, `long
    /// double`, or the `_Float128` U128 carrier -- rather than an aggregate
    /// address: the operand a `StructSplitRegStack` class moves by value.
    fn is_wide_scalar_arg(arg_types: &[IrType], i: usize) -> bool {
        matches!(
            arg_types.get(i),
            Some(IrType::I128 | IrType::U128 | IrType::F128)
        )
    }

    /// Whether argument `i` is a `long double` split across a7 and the
    /// stack (its bytes are staged by emit_call_f128_pre_convert_impl).
    fn is_split_f128_arg(arg_classes: &[CallArgClass], arg_types: &[IrType], i: usize) -> bool {
        matches!(arg_classes[i], CallArgClass::StructSplitRegStack { .. })
            && arg_types.get(i) == Some(&IrType::F128)
    }

    /// Load the XLEN bits at byte `half` (0: low, 8: high) of the 2×XLEN
    /// scalar argument `arg` (index `i`) into `dest`.  A `long double`
    /// value reads its staged copy: the temps sit above the
    /// `stack_arg_space` outgoing bytes, one 16-byte temp per staged
    /// argument in argument order, and a split argument is the last one.
    #[allow(clippy::too_many_arguments)]
    fn emit_wide_scalar_half(
        &mut self,
        dest: &str,
        arg: &Operand,
        args: &[Operand],
        arg_classes: &[CallArgClass],
        arg_types: &[IrType],
        i: usize,
        half: i64,
        stack_arg_space: i64,
    ) {
        let bytes: Option<[u8; 16]> = match arg {
            Operand::Const(IrConst::LongDouble(_, f128_bytes)) => Some(*f128_bytes),
            Operand::Const(c) if arg_types[i] == IrType::F128 => Some(
                crate::ir::reexports::f64_to_f128_bytes(c.to_f64().unwrap_or(0.0)),
            ),
            Operand::Const(c) => Some(c.to_i128().unwrap_or(0).to_le_bytes()),
            Operand::Value(_) => None,
        };
        if let Some(bytes) = bytes {
            let at = half as usize;
            let mut word = [0u8; 8];
            word.copy_from_slice(&bytes[at..at + 8]);
            self.state.emit_fmt(format_args!(
                "    li {}, {}",
                dest,
                i64::from_le_bytes(word)
            ));
            return;
        }
        let Operand::Value(v) = arg else {
            return;
        };
        if arg_types[i] == IrType::F128 {
            let staged_before = (0..i)
                .filter(|&j| {
                    matches!(arg_classes[j], CallArgClass::F128Reg { .. })
                        && matches!(args[j], Operand::Value(_))
                })
                .count() as i64;
            self.emit_load_from_sp(dest, stack_arg_space + staged_before * 16 + half, "ld");
        } else if let Some(slot) = self.state.get_slot(v.0) {
            // A 128-bit integer value lives in its 16-byte slot.
            self.emit_load_from_s0(dest, slot.0 + half, "ld");
        } else {
            self.state.emit_fmt(format_args!("    li {}, 0", dest));
        }
    }

    pub(super) fn emit_call_stack_args_impl(
        &mut self,
        args: &[Operand],
        arg_classes: &[CallArgClass],
        arg_types: &[IrType],
        stack_arg_space: usize,
        _fptr_spill: usize,
        _f128_temp_space: usize,
        struct_arg_aligns: &[Option<usize>],
        _struct_arg_classes: &[Vec<crate::common::types::EightbyteClass>],
        _struct_arg_is_f128_sse: &[bool],
    ) -> i64 {
        if stack_arg_space > 0 {
            // Per-arg alignment padding (RISC-V psABI: stack arguments whose
            // natural alignment exceeds XLEN — e.g. struct { long double } —
            // are aligned to 2*XLEN in the argument area; F128/I128 are
            // always 16-byte aligned). Must match compute_stack_arg_space.
            let arg_padding = crate::backend::call_abi::compute_stack_arg_padding(
                arg_classes,
                struct_arg_aligns,
                16,
            );
            self.emit_addi_sp(-(stack_arg_space as i64));
            let mut offset: usize = 0;
            for (arg_i, arg) in args.iter().enumerate() {
                if !arg_classes[arg_i].is_stack() {
                    continue;
                }
                offset += arg_padding[arg_i];
                match arg_classes[arg_i] {
                    CallArgClass::StructByValStack { size }
                    | CallArgClass::LargeStructStack { size } => {
                        let n_dwords = size.div_ceil(8);
                        match arg {
                            Operand::Value(v) => {
                                if let Some(&reg) = self.reg_assignments.get(&v.0) {
                                    let reg_name = callee_saved_name(reg);
                                    self.state.emit_fmt(format_args!("    mv t0, {}", reg_name));
                                } else if let Some(slot) = self.state.get_slot(v.0) {
                                    if self.state.is_alloca(v.0) {
                                        self.emit_alloca_addr("t0", v.0, slot.0);
                                    } else {
                                        self.emit_load_from_s0("t0", slot.0, "ld");
                                    }
                                } else {
                                    self.state.emit("    li t0, 0");
                                }
                            }
                            Operand::Const(_) => {
                                self.operand_to_t0(arg);
                            }
                        }
                        for qi in 0..n_dwords {
                            let src_off = (qi * 8) as i64;
                            Self::emit_load_from_reg(&mut self.state, "t1", "t0", src_off, "ld");
                            self.emit_store_to_sp("t1", offset as i64 + src_off, "sd");
                        }
                        offset += n_dwords * 8;
                    }
                    CallArgClass::I128Stack => {
                        match arg {
                            Operand::Const(c) => {
                                if let IrConst::I128(v) = c {
                                    self.state
                                        .emit_fmt(format_args!("    li t0, {}", *v as u64 as i64));
                                    self.emit_store_to_sp("t0", offset as i64, "sd");
                                    self.state.emit_fmt(format_args!(
                                        "    li t0, {}",
                                        (*v >> 64) as u64 as i64
                                    ));
                                    self.emit_store_to_sp("t0", (offset + 8) as i64, "sd");
                                } else {
                                    self.operand_to_t0(arg);
                                    self.emit_store_to_sp("t0", offset as i64, "sd");
                                    self.emit_store_to_sp("zero", (offset + 8) as i64, "sd");
                                }
                            }
                            Operand::Value(v) => {
                                if let Some(slot) = self.state.get_slot(v.0) {
                                    if self.state.is_alloca(v.0) {
                                        self.emit_alloca_addr("t0", v.0, slot.0);
                                        self.emit_store_to_sp("t0", offset as i64, "sd");
                                        self.emit_store_to_sp("zero", (offset + 8) as i64, "sd");
                                    } else {
                                        self.emit_load_from_s0("t0", slot.0, "ld");
                                        self.emit_store_to_sp("t0", offset as i64, "sd");
                                        self.emit_load_from_s0("t0", slot.0 + 8, "ld");
                                        self.emit_store_to_sp("t0", (offset + 8) as i64, "sd");
                                    }
                                }
                            }
                        }
                        offset += 16;
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
                                let lo = i64::from_le_bytes(bytes[0..8].try_into().unwrap());
                                let hi = i64::from_le_bytes(bytes[8..16].try_into().unwrap());
                                self.state.emit_fmt(format_args!("    li t0, {}", lo));
                                self.emit_store_to_sp("t0", offset as i64, "sd");
                                self.state.emit_fmt(format_args!("    li t0, {}", hi));
                                self.emit_store_to_sp("t0", (offset + 8) as i64, "sd");
                            }
                            Operand::Value(_) => {
                                self.emit_f128_operand_to_a0_a1(arg);
                                self.emit_store_to_sp("a0", offset as i64, "sd");
                                self.emit_store_to_sp("a1", (offset + 8) as i64, "sd");
                            }
                        }
                        offset += 16;
                    }
                    CallArgClass::StructSplitRegStack { .. }
                        if Self::is_wide_scalar_arg(arg_types, arg_i) =>
                    {
                        // The high XLEN bits of a 2×XLEN scalar; the low
                        // ones go in a7 (emit_call_reg_args_impl).
                        self.emit_wide_scalar_half(
                            "t0",
                            arg,
                            args,
                            arg_classes,
                            arg_types,
                            arg_i,
                            8,
                            stack_arg_space as i64,
                        );
                        self.emit_store_to_sp("t0", offset as i64, "sd");
                        offset += 8;
                    }
                    CallArgClass::StructSplitRegStack { size, .. } => {
                        match arg {
                            Operand::Value(v) => {
                                if let Some(&reg) = self.reg_assignments.get(&v.0) {
                                    let reg_name = callee_saved_name(reg);
                                    self.state.emit_fmt(format_args!("    mv t0, {}", reg_name));
                                } else if let Some(slot) = self.state.get_slot(v.0) {
                                    if self.state.is_alloca(v.0) {
                                        self.emit_alloca_addr("t0", v.0, slot.0);
                                    } else {
                                        self.emit_load_from_s0("t0", slot.0, "ld");
                                    }
                                } else {
                                    self.state.emit("    li t0, 0");
                                }
                            }
                            Operand::Const(_) => {
                                self.operand_to_t0(arg);
                            }
                        }
                        let stack_dwords = (size - 8).div_ceil(8);
                        for qi in 0..stack_dwords {
                            let src_off = 8 + (qi * 8) as i64;
                            Self::emit_load_from_reg(&mut self.state, "t1", "t0", src_off, "ld");
                            self.emit_store_to_sp("t1", offset as i64 + (qi * 8) as i64, "sd");
                        }
                        offset += stack_dwords * 8;
                    }
                    CallArgClass::Stack => {
                        self.operand_to_t0(arg);
                        if arg_types.get(arg_i) == Some(&IrType::U32) {
                            // The psABI extends a stack-passed narrow integer
                            // exactly like a register one: the slot of an
                            // `unsigned int` holds its sign extension, and a
                            // conforming callee may load it with `ld`.  t1
                            // carries it so t0 keeps the internal form.
                            self.state.emit("    sext.w t1, t0");
                            self.emit_store_to_sp("t1", offset as i64, "sd");
                        } else {
                            self.emit_store_to_sp("t0", offset as i64, "sd");
                        }
                        offset += 8;
                    }
                    _ => {}
                }
            }
        }
        0 // RISC-V loads operands from s0-relative slots (not SP-relative), so no SP adjust needed
    }

    pub(super) fn emit_call_reg_args_impl(
        &mut self,
        args: &[Operand],
        arg_classes: &[CallArgClass],
        arg_types: &[IrType],
        _total_sp_adjust: i64,
        _f128_temp_space: usize,
        stack_arg_space: usize,
        struct_arg_fp_agg_classes: &[Option<crate::common::types::FpAggClass>],
    ) {
        let float_arg_regs = ["fa0", "fa1", "fa2", "fa3", "fa4", "fa5", "fa6", "fa7"];
        self.call_gp_reads = rv_call_gp_arg_reads(arg_classes);

        // GP arg staging strategy: use only caller-saved temp registers (t3, t4, t5)
        // for staging, avoiding callee-saved s2-s6. This allows s2-s6 to be used
        // by the register allocator, significantly reducing stack frame sizes.
        //
        // Phase 1: Stage the first 3 GP args into t3/t4/t5, handle float args.
        // Phase 2: Move staged values to target a-registers.
        // Phase 3: Load remaining GP args (4th+) directly into target a-registers.
        //
        // Phase 3 reads its sources after Phase 2 (and the earlier Phase 3
        // loads) wrote a0..a{k-1}: the allocator never homes such a source in
        // one of those registers (`riscv_call_staging_depths` gives GP
        // argument k >= 3 depth k; the first three, read in Phase 1 before
        // any a-register is written, depth 0).
        let temp_regs = ["t3", "t4", "t5"];
        let mut staged_gp: Vec<(usize, &str)> = Vec::new(); // (target_reg_idx, temp_reg)
        let mut deferred_gp: Vec<(usize, usize)> = Vec::new(); // (target_reg_idx, arg_index)
        let mut temp_i = 0usize;
        // The RISC-V psABI widens an integer argument narrower than XLEN by
        // its type's signedness to 32 bits and then SIGN-extends it to 64:
        // an `unsigned int` travels sign-extended.  This backend keeps a U32
        // value zero-extended internally (its entry `slli; srli` restores
        // that from any caller), so the copy into the argument register
        // re-extends it; a conforming callee may compare the register with
        // a W-instruction result as is (`addw a1, a1, a0; sltu a0, a0, a1`
        // returned the wrong order for 0x80000001 + 0xffffffff).  t0 itself
        // keeps the internal form, so the accumulator cache stays valid.
        let copy_op = |i: usize| {
            if arg_types.get(i) == Some(&IrType::U32) {
                "sext.w"
            } else {
                "mv"
            }
        };
        for (i, arg) in args.iter().enumerate() {
            match arg_classes[i] {
                CallArgClass::IntReg { reg_idx } => {
                    if temp_i < temp_regs.len() {
                        // Stage into t3/t4/t5
                        self.operand_to_t0(arg);
                        self.state.emit_fmt(format_args!(
                            "    {} {}, t0",
                            copy_op(i),
                            temp_regs[temp_i]
                        ));
                        staged_gp.push((reg_idx, temp_regs[temp_i]));
                        temp_i += 1;
                    } else {
                        // Defer: will load directly into target a-register after Phase 2
                        deferred_gp.push((reg_idx, i));
                    }
                }
                CallArgClass::FloatReg { reg_idx } => {
                    self.operand_to_t0(arg);
                    let arg_ty = if i < arg_types.len() {
                        Some(arg_types[i])
                    } else {
                        None
                    };
                    if arg_ty == Some(IrType::U16) {
                        // 16-bit NaN-boxing in fa{reg_idx}: bits [63:16] = 1 (`fmv.w.x` boxes [63:32],
                        // `lui t1, 0xffff0; or` boxes [31:16]).
                        self.state.emit("    slli t0, t0, 48");
                        self.state.emit("    srli t0, t0, 48");
                        self.state.emit("    lui t1, 0xffff0");
                        self.state.emit("    or t0, t0, t1");
                        self.state
                            .emit_fmt(format_args!("    fmv.w.x {}, t0", float_arg_regs[reg_idx]));
                    } else if arg_ty == Some(IrType::F32) {
                        self.state
                            .emit_fmt(format_args!("    fmv.w.x {}, t0", float_arg_regs[reg_idx]));
                    } else {
                        self.state
                            .emit_fmt(format_args!("    fmv.d.x {}, t0", float_arg_regs[reg_idx]));
                    }
                }
                _ => {}
            }
        }

        // Phase 2: Move staged values from t3/t4/t5 to target a-registers
        for (target_idx, temp_reg) in &staged_gp {
            self.state.emit_fmt(format_args!(
                "    mv {}, {}",
                RISCV_ARG_REGS[*target_idx], temp_reg
            ));
        }

        // Phase 3: Load deferred GP args directly into target a-registers
        // (sound by the staging-hazard depths, see above). Invalidate the
        // accumulator cache first since Phase 2 emissions may have left stale
        // cache entries.
        if !deferred_gp.is_empty() {
            self.state.reg_cache.invalidate_acc();
        }
        for &(target_idx, arg_idx) in &deferred_gp {
            self.operand_to_t0(&args[arg_idx]);
            self.state.emit_fmt(format_args!(
                "    {} {}, t0",
                copy_op(arg_idx),
                RISCV_ARG_REGS[target_idx]
            ));
        }

        // Load F128 register pair args
        let mut f128_var_temp_offset: i64 = 0;
        for (i, arg) in args.iter().enumerate() {
            if let CallArgClass::F128Reg { reg_idx: base_reg } = arg_classes[i] {
                match arg {
                    Operand::Const(c) => {
                        let bytes = match c {
                            IrConst::LongDouble(_, f128_bytes) => *f128_bytes,
                            // _Float128 lowered to the IrType::U128 carrier:
                            // the I128 constant IS the IEEE binary128 bit
                            // pattern. Stage it bit-exactly — a to_f64()
                            // here would integer-convert the pattern and
                            // fabricate a huge value out of 1.5L's bits.
                            IrConst::I128(bits) => bits.to_le_bytes(),
                            _ => {
                                let f64_val = c.to_f64().unwrap_or(0.0);
                                crate::ir::reexports::f64_to_f128_bytes(f64_val)
                            }
                        };
                        let lo = i64::from_le_bytes(bytes[0..8].try_into().unwrap());
                        let hi = i64::from_le_bytes(bytes[8..16].try_into().unwrap());
                        self.state
                            .emit_fmt(format_args!("    li {}, {}", RISCV_ARG_REGS[base_reg], lo));
                        self.state.emit_fmt(format_args!(
                            "    li {}, {}",
                            RISCV_ARG_REGS[base_reg + 1],
                            hi
                        ));
                    }
                    Operand::Value(_) => {
                        let offset = f128_var_temp_offset + stack_arg_space as i64;
                        self.emit_load_from_sp(RISCV_ARG_REGS[base_reg], offset, "ld");
                        self.emit_load_from_sp(RISCV_ARG_REGS[base_reg + 1], offset + 8, "ld");
                        f128_var_temp_offset += 16;
                    }
                }
            }
        }

        // Load i128 register pair args
        for (i, arg) in args.iter().enumerate() {
            if let CallArgClass::I128RegPair { base_reg_idx } = arg_classes[i] {
                match arg {
                    Operand::Const(c) => {
                        if let IrConst::I128(v) = c {
                            self.state.emit_fmt(format_args!(
                                "    li {}, {}",
                                RISCV_ARG_REGS[base_reg_idx], *v as u64 as i64
                            ));
                            self.state.emit_fmt(format_args!(
                                "    li {}, {}",
                                RISCV_ARG_REGS[base_reg_idx + 1],
                                (*v >> 64) as u64 as i64
                            ));
                        } else {
                            self.operand_to_t0(arg);
                            self.state.emit_fmt(format_args!(
                                "    mv {}, t0",
                                RISCV_ARG_REGS[base_reg_idx]
                            ));
                            self.state.emit_fmt(format_args!(
                                "    mv {}, zero",
                                RISCV_ARG_REGS[base_reg_idx + 1]
                            ));
                        }
                    }
                    Operand::Value(v) => {
                        if let Some(slot) = self.state.get_slot(v.0) {
                            if self.state.is_alloca(v.0) {
                                self.emit_alloca_addr(RISCV_ARG_REGS[base_reg_idx], v.0, slot.0);
                                self.state.emit_fmt(format_args!(
                                    "    mv {}, zero",
                                    RISCV_ARG_REGS[base_reg_idx + 1]
                                ));
                            } else {
                                self.emit_load_from_s0(RISCV_ARG_REGS[base_reg_idx], slot.0, "ld");
                                self.emit_load_from_s0(
                                    RISCV_ARG_REGS[base_reg_idx + 1],
                                    slot.0 + 8,
                                    "ld",
                                );
                            }
                        }
                    }
                }
            }
        }

        // Load struct-by-value register args
        for (i, arg) in args.iter().enumerate() {
            if let CallArgClass::StructByValReg { base_reg_idx, size } = arg_classes[i] {
                let regs_needed = if size <= 8 { 1 } else { 2 };
                match arg {
                    Operand::Value(v) => {
                        if let Some(&reg) = self.reg_assignments.get(&v.0) {
                            let reg_name = callee_saved_name(reg);
                            self.state.emit_fmt(format_args!("    mv t0, {}", reg_name));
                        } else if let Some(slot) = self.state.get_slot(v.0) {
                            if self.state.is_alloca(v.0) {
                                self.emit_alloca_addr("t0", v.0, slot.0);
                            } else {
                                self.emit_load_from_s0("t0", slot.0, "ld");
                            }
                        } else {
                            self.state.emit("    li t0, 0");
                        }
                    }
                    Operand::Const(_) => {
                        self.operand_to_t0(arg);
                    }
                }
                self.state.emit_fmt(format_args!(
                    "    ld {}, 0(t0)",
                    RISCV_ARG_REGS[base_reg_idx]
                ));
                if regs_needed > 1 {
                    self.state.emit_fmt(format_args!(
                        "    ld {}, 8(t0)",
                        RISCV_ARG_REGS[base_reg_idx + 1]
                    ));
                }
            }
        }

        // Load split struct register args
        for (i, arg) in args.iter().enumerate() {
            if let CallArgClass::StructSplitRegStack { reg_idx, .. } = arg_classes[i] {
                if Self::is_wide_scalar_arg(arg_types, i) {
                    // The low XLEN bits of a 2×XLEN scalar (the high ones
                    // went to the stack in emit_call_stack_args_impl).
                    self.emit_wide_scalar_half(
                        RISCV_ARG_REGS[reg_idx],
                        arg,
                        args,
                        arg_classes,
                        arg_types,
                        i,
                        0,
                        stack_arg_space as i64,
                    );
                    continue;
                }
                match arg {
                    Operand::Value(v) => {
                        if let Some(&reg) = self.reg_assignments.get(&v.0) {
                            let reg_name = callee_saved_name(reg);
                            self.state.emit_fmt(format_args!("    mv t0, {}", reg_name));
                        } else if let Some(slot) = self.state.get_slot(v.0) {
                            if self.state.is_alloca(v.0) {
                                self.emit_alloca_addr("t0", v.0, slot.0);
                            } else {
                                self.emit_load_from_s0("t0", slot.0, "ld");
                            }
                        } else {
                            self.state.emit("    li t0, 0");
                        }
                    }
                    Operand::Const(_) => {
                        self.operand_to_t0(arg);
                    }
                }
                self.state
                    .emit_fmt(format_args!("    ld {}, 0(t0)", RISCV_ARG_REGS[reg_idx]));
            }
        }

        // Load RISC-V LP64D float struct args into FP registers
        for (i, arg) in args.iter().enumerate() {
            let needs_ptr = matches!(
                arg_classes[i],
                CallArgClass::StructSseReg { .. }
                    | CallArgClass::StructMixedIntSseReg { .. }
                    | CallArgClass::StructMixedSseIntReg { .. }
            );
            if !needs_ptr {
                continue;
            }

            let rv_class = struct_arg_fp_agg_classes.get(i).copied().flatten();

            match arg {
                Operand::Value(v) => {
                    if let Some(&reg) = self.reg_assignments.get(&v.0) {
                        let reg_name = callee_saved_name(reg);
                        self.state.emit_fmt(format_args!("    mv t0, {}", reg_name));
                    } else if let Some(slot) = self.state.get_slot(v.0) {
                        if self.state.is_alloca(v.0) {
                            // An over-aligned record (a member aligned past
                            // 16 bytes, which a flattened record of any size
                            // may have) lives at a realigned address.
                            self.emit_alloca_addr("t0", v.0, slot.0);
                        } else {
                            self.emit_load_from_s0("t0", slot.0, "ld");
                        }
                    } else {
                        self.state.emit("    li t0, 0");
                    }
                }
                Operand::Const(_) => {
                    self.operand_to_t0(arg);
                }
            }

            match arg_classes[i] {
                CallArgClass::StructSseReg {
                    lo_fp_idx,
                    hi_fp_idx,
                    size,
                } => {
                    let (lo_sz, hi_sz, hi_offset) = match rv_class {
                        Some(crate::common::types::FpAggClass::OneFloat { size, .. }) => {
                            (size, 4, 0)
                        }
                        Some(crate::common::types::FpAggClass::TwoFloats {
                            lo_size,
                            hi_size,
                            hi_offset,
                            ..
                        }) => (lo_size, hi_size, hi_offset),
                        _ => (
                            if size > 4 { 8 } else { 4 },
                            8,
                            if size > 4 { 8 } else { 4 },
                        ),
                    };
                    self.emit_load_riscv_fp_field(float_arg_regs[lo_fp_idx], 0, lo_sz);
                    if let Some(hi_idx) = hi_fp_idx {
                        self.emit_load_riscv_fp_field(float_arg_regs[hi_idx], hi_offset, hi_sz);
                    }
                }
                CallArgClass::StructMixedIntSseReg {
                    int_reg_idx,
                    fp_reg_idx,
                    size: _,
                } => {
                    let (float_sz, int_size, float_offset) = match rv_class {
                        Some(crate::common::types::FpAggClass::IntAndFloat {
                            float_size,
                            int_size,
                            float_offset,
                            ..
                        }) => (float_size, int_size, float_offset),
                        _ => (8, 8, 8),
                    };
                    self.state.emit_fmt(format_args!(
                        "    {} {}, 0(t0)",
                        super::fp_agg::int_member_load(int_size),
                        RISCV_ARG_REGS[int_reg_idx]
                    ));
                    self.emit_load_riscv_fp_field(
                        float_arg_regs[fp_reg_idx],
                        float_offset,
                        float_sz,
                    );
                }
                CallArgClass::StructMixedSseIntReg {
                    fp_reg_idx,
                    int_reg_idx,
                    size: _,
                } => {
                    let (float_sz, int_offset, int_size) = match rv_class {
                        Some(crate::common::types::FpAggClass::FloatAndInt {
                            float_size,
                            int_offset,
                            int_size,
                            ..
                        }) => (float_size, int_offset, int_size),
                        _ => (8, 8, 8),
                    };
                    self.emit_load_riscv_fp_field(float_arg_regs[fp_reg_idx], 0, float_sz);
                    self.state.emit_fmt(format_args!(
                        "    {} {}, {}(t0)",
                        super::fp_agg::int_member_load(int_size),
                        RISCV_ARG_REGS[int_reg_idx],
                        int_offset
                    ));
                }
                _ => {}
            }
        }
    }

    pub(super) fn emit_call_instruction_impl(
        &mut self,
        direct_name: Option<&str>,
        func_ptr: Option<&Operand>,
        _indirect: bool,
        _stack_arg_space: usize,
    ) {
        // Taken on every path: a stale set must never reach a later call.
        // The static chain armed by the `SetStaticChain` just before this
        // call (its IR position) is t2, bit 8 of the annotation.
        let chain = std::mem::take(&mut self.static_chain_armed);
        let reads = self.call_gp_reads.take().map(|mask| {
            u16::from(mask)
                | if chain {
                    super::peephole::CALL_READS_CHAIN as u16
                } else {
                    0
                }
        });
        // Sibling call: the argument setup ran unchanged; the call itself is
        // replaced by the block's `Return` emitting `<epilogue>; tail target`
        // (or `jr t2`: the epilogue touches only t0 and t6 besides the
        // registers it restores).  The call's read annotation travels to the
        // tail line.
        if matches!(
            self.state.pending_sibcall,
            Some(crate::backend::state::SibcallPending::Armed { .. })
        ) {
            use crate::backend::state::{SibcallPending, SibcallTarget};
            let target = if let Some(name) = direct_name {
                Some(SibcallTarget::Direct(name.to_string()))
            } else if let Some(ptr) = func_ptr {
                self.operand_to_t0(ptr);
                self.state.emit("    mv t2, t0");
                Some(SibcallTarget::Reg("t2"))
            } else {
                None
            };
            if let Some(target) = target {
                self.sibcall_reads = reads;
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
        // call ever be emitted anyway, ra is preserved around it rather than
        // returning through a clobbered link register.  (Such a call has no
        // stack arguments -- `sibcall_abi_admits` passed it -- so moving sp
        // by the 16 bytes the psABI alignment keeps cannot misplace any.)
        let guard_link = self.frameless_leaf;
        if guard_link {
            self.state.emit("    addi sp, sp, -16");
            self.state.emit("    sd ra, 8(sp)");
        }
        if let Some(name) = direct_name {
            match reads {
                Some(mask) => self.state.emit_fmt(format_args!(
                    "    call {}{}{}",
                    name,
                    super::peephole::CALL_READS_TAG,
                    mask
                )),
                None => self.state.emit_fmt(format_args!("    call {}", name)),
            }
        } else if let Some(ptr) = func_ptr {
            self.operand_to_t0(ptr);
            self.state.emit("    mv t2, t0");
            self.state.emit("    jalr ra, t2, 0");
        }
        if guard_link {
            self.state.emit("    ld ra, 8(sp)");
            self.state.emit("    addi sp, sp, 16");
        }
    }

    pub(super) fn emit_call_cleanup_impl(
        &mut self,
        stack_arg_space: usize,
        f128_temp_space: usize,
        _indirect: bool,
    ) {
        if f128_temp_space > 0 && stack_arg_space == 0 {
            self.emit_addi_sp(f128_temp_space as i64);
        }
        if stack_arg_space > 0 {
            let cleanup = stack_arg_space as i64 + f128_temp_space as i64;
            self.emit_addi_sp(cleanup);
        }
    }

    /// Load the float member `size` bytes wide at `offset(t0)` of a struct
    /// argument into `freg`.  A half (no Zfh in the baseline) is NaN-boxed
    /// through t1/t3: this runs after the GP staging, so the t3-t5 staging
    /// temporaries are free, while t2 may hold the static chain a
    /// `SetStaticChain` armed for this very call (bit 8 of its read
    /// annotation) -- `lui t2, 0xffff0` here destroyed the chain of a direct
    /// nested-function call passing a `struct { _Float16 h; float f; }`.
    fn emit_load_riscv_fp_field(&mut self, freg: &str, offset: usize, size: u8) {
        match size {
            8 => {
                self.state
                    .emit_fmt(format_args!("    fld {}, {}(t0)", freg, offset));
            }
            2 => {
                self.state
                    .emit_fmt(format_args!("    lhu t1, {}(t0)", offset));
                self.state.emit("    lui t3, 0xffff0");
                self.state.emit("    or t1, t1, t3");
                self.state
                    .emit_fmt(format_args!("    fmv.w.x {}, t1", freg));
            }
            _ => {
                self.state
                    .emit_fmt(format_args!("    flw {}, {}(t0)", freg, offset));
            }
        }
    }

    pub(super) fn emit_call_store_result_impl(&mut self, dest: &Value, return_type: IrType) {
        // Sibling call: the result is never materialised -- the tail
        // callee's exit is this function's, its result registers already
        // hold what our `Return` hands over.
        if return_type == IrType::Void || self.state.pending_sibcall.is_some() {
            return;
        }
        if self.call_ret_is_f128_sse && return_type == IrType::U16 {
            self.state.emit("    fmv.x.w t0, fa0");
            self.state.emit("    slli t0, t0, 48");
            self.state.emit("    srli t0, t0, 48");
            self.store_t0_to(dest);
            return;
        }
        if is_i128_type(return_type) {
            if let Some(slot) = self.state.get_slot(dest.0) {
                self.emit_store_to_s0("a0", slot.0, "sd");
                self.emit_store_to_s0("a1", slot.0 + 8, "sd");
            }
        } else if return_type.is_long_double() {
            // binary128 comes back in a0:a1, the soft-float arg1 pair: the
            // value is published like any F128 libcall result -- into its
            // slot, with no `__trunctfdf2` approximation beside it.
            crate::backend::f128_softfloat::F128SoftFloat::f128_publish_result(self, dest);
        } else if return_type == IrType::F32 {
            if let Some(slot) = self.state.get_slot(dest.0) {
                self.state.emit("    fmv.x.w t0, fa0");
                self.emit_store_to_s0("t0", slot.0, "sd");
            }
        } else if return_type.is_float() {
            if let Some(slot) = self.state.get_slot(dest.0) {
                self.state.emit("    fmv.x.d t0, fa0");
                self.emit_store_to_s0("t0", slot.0, "sd");
            }
        } else if let Some(&reg) = self.reg_assignments.get(&dest.0) {
            let reg_name = callee_saved_name(reg);
            self.state.emit_fmt(format_args!("    mv {}, a0", reg_name));
        } else if let Some(slot) = self.state.get_slot(dest.0) {
            self.emit_store_to_s0("a0", slot.0, "sd");
        }
    }
}
