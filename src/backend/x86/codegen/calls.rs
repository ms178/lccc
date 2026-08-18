//! X86Codegen: function call operations.

use crate::ir::reexports::{IrConst, Operand, Value};
use crate::common::types::IrType;
use crate::backend::call_abi::{CallAbiConfig, CallArgClass, compute_stack_push_bytes};
use crate::backend::generation::is_i128_type;
use super::emit::{X86Codegen, X86_ARG_REGS};

impl X86Codegen {
    pub(super) fn call_abi_config_impl(&self) -> CallAbiConfig {
        CallAbiConfig {
            max_int_regs: 6, max_float_regs: 8,
            align_i128_pairs: false,
            f128_in_fp_regs: false, f128_in_gp_pairs: false,
            variadic_floats_in_gp: false,
            large_struct_by_ref: false,
            use_sysv_struct_classification: true,
            use_riscv_float_struct_classification: false,
            allow_struct_split_reg_stack: false,
            align_struct_pairs: false,
            sret_uses_dedicated_reg: false,
            gcc_regparm_mode: false,
        }
    }

    pub(super) fn emit_call_compute_stack_space_impl(&self, arg_classes: &[CallArgClass], _arg_types: &[IrType]) -> usize {
        compute_stack_push_bytes(arg_classes)
    }

    pub(super) fn emit_call_stack_args_impl(&mut self, args: &[Operand], arg_classes: &[CallArgClass],
                            _arg_types: &[IrType], stack_arg_space: usize, _fptr_spill: usize, _f128_temp_space: usize) -> i64 {
        let need_align_pad = stack_arg_space % 16 != 0;
        let mut sp_adjust: i64 = 0;
        if need_align_pad {
            self.state.emit("    subq $8, %rsp");
            sp_adjust += 8;
            // Adjust RSP frame size so operand_to_rax slot conversions
            // are correct after the alignment subq.
            if self.state.out.use_rsp_addressing {
                self.state.out.rsp_frame_size += 8;
            }
        }
        let arg_padding = crate::backend::call_abi::compute_stack_arg_padding(arg_classes);
        let stack_indices: Vec<usize> = (0..args.len())
            .filter(|&i| arg_classes[i].is_stack())
            .collect();
        for &si in stack_indices.iter().rev() {
            match arg_classes[si] {
                CallArgClass::F128Stack => {
                    match &args[si] {
                        Operand::Const(ref c) => {
                            let x87_bytes: [u8; 10] = match c {
                                IrConst::LongDouble(_, f128_bytes) => {
                                    let x87 = crate::common::long_double::f128_bytes_to_x87_bytes(f128_bytes);
                                    let mut b = [0u8; 10];
                                    b.copy_from_slice(&x87[..10]);
                                    b
                                }
                                _ => {
                                    let f64_val = c.to_f64().unwrap_or(0.0);
                                    crate::ir::reexports::f64_to_x87_bytes(f64_val)
                                }
                            };
                            // SysV x86-64 passes long double in a 16-byte stack
                            // slot (the low 10 bytes are x87 extended precision).
                            // Account for the reservation before emitting any
                            // RSP-relative operand, otherwise `slot_ref`/`fldt`
                            // address the pre-subtraction stack location.
                            self.state.emit("    subq $16, %rsp");
                            sp_adjust += 16;
                            if self.state.out.use_rsp_addressing {
                                self.state.out.rsp_frame_size += 16;
                            }
                            let lo = u64::from_le_bytes(x87_bytes[0..8].try_into().unwrap());
                            let hi_2bytes = u16::from_le_bytes(x87_bytes[8..10].try_into().unwrap());
                            self.state.out.emit_instr_imm_reg("    movabsq", lo as i64, "rax");
                            self.state.emit("    movq %rax, (%rsp)");
                            self.state.emit_fmt(format_args!("    movw ${}, 8(%rsp)", hi_2bytes));
                            self.state.reg_cache.invalidate_all();
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
                        }
                        Operand::Value(ref v) => {
                            if self.state.f128_direct_slots.contains(&v.0) {
                                self.state.emit("    subq $16, %rsp");
                                sp_adjust += 16;
                                if self.state.out.use_rsp_addressing {
                                    self.state.out.rsp_frame_size += 16;
                                }
                                if let Some(slot) = self.state.get_slot(v.0) {
                                    self.state.out.emit_instr_rbp("    fldt", slot.0);
                                    self.state.emit("    fstpt (%rsp)");
                                }
                            } else if let Some(slot) = self.state.get_slot(v.0) {
                                if self.state.is_alloca(v.0) {
                                    self.state.emit("    subq $16, %rsp");
                                    sp_adjust += 16;
                                    if self.state.out.use_rsp_addressing {
                                        self.state.out.rsp_frame_size += 16;
                                    }
                                    self.state.out.emit_instr_rbp("    fldt", slot.0);
                                    self.state.emit("    fstpt (%rsp)");
                                } else {
                                    // The value is represented as a scalar f64
                                    // bit-pattern. Read it before reserving the
                                    // outgoing slot, then widen in that slot;
                                    // this avoids an untracked push/pop window.
                                    self.state.out.emit_instr_rbp_reg("    movq", slot.0, "rax");
                                    self.state.emit("    subq $16, %rsp");
                                    sp_adjust += 16;
                                    if self.state.out.use_rsp_addressing {
                                        self.state.out.rsp_frame_size += 16;
                                    }
                                    self.state.emit("    movq %rax, (%rsp)");
                                    self.state.emit("    fldl (%rsp)");
                                    self.state.emit("    fstpt (%rsp)");
                                }
                            } else {
                                self.state.emit("    subq $16, %rsp");
                                sp_adjust += 16;
                                if self.state.out.use_rsp_addressing {
                                    self.state.out.rsp_frame_size += 16;
                                }
                            }
                            self.state.reg_cache.invalidate_all();
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
                        }
                    }
                }
                CallArgClass::I128Stack => {
                    // Read both halves before changing RSP.  The old code
                    // advanced rsp_frame_size before these slot_ref calls, so
                    // a source i128 was read from the outgoing argument area
                    // and its halves were swapped/corrupted.
                    match &args[si] {
                        Operand::Value(v) => {
                            if let Some(slot) = self.state.get_slot(v.0) {
                                let sr1 = self.slot_ref(slot.0 + 8);
                                let sr0 = self.slot_ref(slot.0);
                                self.state.emit_fmt(format_args!("    movq {}, %r11", sr1));
                                self.state.emit_fmt(format_args!("    movq {}, %rax", sr0));
                                self.state.emit("    pushq %r11");
                                self.state.emit("    pushq %rax");
                            } else {
                                self.state.emit("    pushq $0");
                                self.state.emit("    pushq $0");
                            }
                        }
                        Operand::Const(c) => {
                            if let IrConst::I128(v) = c {
                                let low = *v as u64;
                                let high = (*v >> 64) as u64;
                                self.state.emit_fmt(format_args!("    pushq ${}", high as i64));
                                self.state.emit_fmt(format_args!("    pushq ${}", low as i64));
                            } else {
                                // Smaller constant or zero
                                if let Operand::Value(_) = &args[si] {} // can't happen
                                self.operand_to_rax(&args[si]);
                                self.state.emit("    pushq $0");
                                self.state.emit("    pushq %rax");
                            }
                        }
                    }
                    // Subsequent stack arguments and register arguments see
                    // the two pushed qwords at their new RSP-relative offsets.
                    sp_adjust += 16;
                    if self.state.out.use_rsp_addressing {
                        self.state.out.rsp_frame_size += 16;
                    }
                }
                CallArgClass::StructByValStack { size } | CallArgClass::LargeStructStack { size } => {
                    self.operand_to_rax(&args[si]);
                    let n_qwords = size.div_ceil(8);
                    for qi in (0..n_qwords).rev() {
                        let offset = qi * 8;
                        if offset + 8 <= size {
                            self.state.emit_fmt(format_args!("    pushq {}(%rax)", offset));
                        } else {
                            self.state.out.emit_instr_mem_reg("    movq", offset as i64, "rax", "rcx");
                            self.state.emit("    pushq %rcx");
                        }
                    }
                    let push_bytes = (n_qwords * 8) as i64;
                    if self.state.out.use_rsp_addressing {
                        self.state.out.rsp_frame_size += push_bytes;
                    }
                    sp_adjust += push_bytes;
                }
                CallArgClass::Stack => {
                    self.operand_to_rax(&args[si]);
                    self.state.emit("    pushq %rax");
                    if self.state.out.use_rsp_addressing {
                        self.state.out.rsp_frame_size += 8;
                    }
                    sp_adjust += 8;
                }
                _ => {}
            }
            let pad = arg_padding[si];
            if pad > 0 {
                self.state.out.emit_instr_imm_reg("    subq", pad as i64, "rsp");
                sp_adjust += pad as i64;
                if self.state.out.use_rsp_addressing {
                    self.state.out.rsp_frame_size += pad as i64;
                }
            }
        }
        // Restore the original RSP frame size. The total adjustment is
        // tracked in sp_adjust and returned to emit_call_reg_args.
        if self.state.out.use_rsp_addressing {
            self.state.out.rsp_frame_size -= sp_adjust;
        }
        // Return the total RSP adjustment so emit_call_reg_args can
        // compensate stack slot offsets when loading register arguments.
        sp_adjust
    }

    /// Spill an indirect function pointer before stack argument setup.
    ///
    /// The shared call pipeline pushes stack arguments before emitting the call.
    /// If we reload the function pointer from an RSP-addressed stack slot after
    /// those pushes, the slot offset is wrong and the indirect call may jump to
    /// NULL.  Keep the target in r10 instead: r10 is not an ABI argument register
    /// on SysV x86-64, and caller-saved live values have already been spilled by
    /// `emit_pre_call_save_caller_regs` before this hook runs.
    pub(super) fn emit_call_spill_fptr_impl(&mut self, func_ptr: &Operand) {
        self.operand_to_rax(func_ptr);
        self.state.emit("    movq %rax, %r10");
        self.state.reg_cache.invalidate_all();
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
    }

    pub(super) fn emit_call_reg_args_impl(&mut self, args: &[Operand], arg_classes: &[CallArgClass],
                          _arg_types: &[IrType], total_sp_adjust: i64, _f128_temp_space: usize, _stack_arg_space: usize,
                          _struct_arg_riscv_float_classes: &[Option<crate::common::types::RiscvFloatClass>]) {
        // Stack args (Phase 2) may have adjusted rsp. Temporarily increase
        // the RSP frame size so rbp-to-rsp offset conversion is correct
        // when loading register arguments from stack slots.
        if total_sp_adjust != 0 && self.state.out.use_rsp_addressing {
            self.state.out.rsp_frame_size += total_sp_adjust;
        }
        let xmm_regs = ["xmm0", "xmm1", "xmm2", "xmm3", "xmm4", "xmm5", "xmm6", "xmm7"];
        let mut float_count = 0usize;
        for (i, arg) in args.iter().enumerate() {
            match arg_classes[i] {
                CallArgClass::I128RegPair { base_reg_idx } => {
                    let lo_reg = X86_ARG_REGS[base_reg_idx];
                    let hi_reg = X86_ARG_REGS[base_reg_idx + 1];
                    // Load 128-bit value directly into the target register pair,
                    // avoiding operand_to_rax_rdx which clobbers rax and rdx
                    // (potentially overwriting previously-assigned arguments).
                    match arg {
                        Operand::Value(v) => {
                            if let Some(slot) = self.state.get_slot(v.0) {
                                // Load both halves directly from the stack slot
                                self.state.out.emit_instr_rbp_reg("    movq", slot.0, lo_reg);
                                self.state.out.emit_instr_rbp_reg("    movq", slot.0 + 8, hi_reg);
                            } else {
                                // No slot: zero both halves
                                self.state.emit_fmt(format_args!("    xorq %{}, %{}", lo_reg, lo_reg));
                                self.state.emit_fmt(format_args!("    xorq %{}, %{}", hi_reg, hi_reg));
                            }
                        }
                        Operand::Const(c) => {
                            match c {
                                IrConst::I128(v) => {
                                    let low = *v as u64;
                                    let high = (*v >> 64) as u64;
                                    self.state.emit_fmt(format_args!("    movabsq ${}, %{}", low as i64, lo_reg));
                                    self.state.emit_fmt(format_args!("    movabsq ${}, %{}", high as i64, hi_reg));
                                }
                                IrConst::Zero => {
                                    self.state.emit_fmt(format_args!("    xorq %{}, %{}", lo_reg, lo_reg));
                                    self.state.emit_fmt(format_args!("    xorq %{}, %{}", hi_reg, hi_reg));
                                }
                                _ => {
                                    // Smaller constant: load into lo_reg via rax, zero hi_reg
                                    self.operand_to_rax(arg);
                                    self.state.out.emit_instr_reg_reg("    movq", "rax", lo_reg);
                                    self.state.emit_fmt(format_args!("    xorq %{}, %{}", hi_reg, hi_reg));
                                }
                            }
                        }
                    }
                }
                CallArgClass::StructByValReg { base_reg_idx, size } => {
                    self.operand_to_rax(arg);
                    let lo_reg = X86_ARG_REGS[base_reg_idx];
                    self.state.out.emit_instr_mem_reg("    movq", 0, "rax", lo_reg);
                    if size > 8 {
                        let hi_reg = X86_ARG_REGS[base_reg_idx + 1];
                        self.state.out.emit_instr_mem_reg("    movq", 8, "rax", hi_reg);
                    }
                }
                CallArgClass::StructSseReg { lo_fp_idx, hi_fp_idx, .. } => {
                    self.operand_to_rax(arg);
                    self.state.out.emit_instr_mem_reg("    movq", 0, "rax", xmm_regs[lo_fp_idx]);
                    float_count += 1;
                    if let Some(hi) = hi_fp_idx {
                        self.state.out.emit_instr_mem_reg("    movq", 8, "rax", xmm_regs[hi]);
                        float_count += 1;
                    }
                }
                CallArgClass::F128SseReg { reg_idx } => {
                    // _Float128: the full 16 bytes go in ONE XMM register (SysV psABI).
                    // The IR value is the 16-byte DATA (in a slot), not a pointer:
                    // load the slot directly. Constants are materialized bit-exact.
                    match arg {
                        Operand::Value(v) => {
                            if let Some(slot) = self.state.get_slot(v.0) {
                                self.state.out.emit_instr_rbp_reg("    movdqu", slot.0, xmm_regs[reg_idx]);
                            } else {
                                self.state.emit_fmt(format_args!("    pxor %{}, %{}", xmm_regs[reg_idx], xmm_regs[reg_idx]));
                            }
                        }
                        Operand::Const(IrConst::I128(c)) => {
                            let bytes = (*c as u128).to_le_bytes();
                            let lo = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
                            let hi = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
                            // Register-agnostic materialization via a 16-byte
                            // stack scratch: the old xmm0/xmm1 construction
                            // clobbered earlier arguments when reg_idx != 0
                            // (the second _Float128 constant argument
                            // overwrote the first).
                            self.state.emit("    subq $16, %rsp");
                            self.state.out.emit_instr_imm_reg("    movabsq", lo as i64, "rax");
                            self.state.emit("    movq %rax, (%rsp)");
                            self.state.out.emit_instr_imm_reg("    movabsq", hi as i64, "rax");
                            self.state.emit("    movq %rax, 8(%rsp)");
                            self.state.emit_fmt(format_args!("    movdqu (%rsp), %{}", xmm_regs[reg_idx]));
                            self.state.emit("    addq $16, %rsp");
                        }
                        Operand::Const(IrConst::LongDouble(_, f128_bytes)) => {
                            // long double (x87 80-bit): the constant's payload
                            // is binary128; narrow to x87 bytes first, then
                            // materialize the 16-byte slot (10 significant
                            // bytes) into the XMM argument register.
                            let x87 = crate::common::long_double::f128_bytes_to_x87_bytes(f128_bytes);
                            let lo = u64::from_le_bytes(x87[0..8].try_into().unwrap());
                            let hi = u64::from_le_bytes([x87[8], x87[9], 0, 0, 0, 0, 0, 0]);
                            self.state.emit("    subq $16, %rsp");
                            self.state.out.emit_instr_imm_reg("    movabsq", lo as i64, "rax");
                            self.state.emit("    movq %rax, (%rsp)");
                            self.state.out.emit_instr_imm_reg("    movabsq", hi as i64, "rax");
                            self.state.emit("    movq %rax, 8(%rsp)");
                            self.state.emit_fmt(format_args!("    movdqu (%rsp), %{}", xmm_regs[reg_idx]));
                            self.state.emit("    addq $16, %rsp");
                        }
                        _ => {
                            self.operand_to_rax(arg);
                            self.state.emit_fmt(format_args!("    movdqu 0(%rax), %{}", xmm_regs[reg_idx]));
                        }
                    }
                    float_count += 1;
                }
                CallArgClass::StructMixedIntSseReg { int_reg_idx, fp_reg_idx, .. } => {
                    self.operand_to_rax(arg);
                    self.state.out.emit_instr_mem_reg("    movq", 8, "rax", xmm_regs[fp_reg_idx]);
                    float_count += 1;
                    self.state.out.emit_instr_mem_reg("    movq", 0, "rax", X86_ARG_REGS[int_reg_idx]);
                }
                CallArgClass::StructMixedSseIntReg { fp_reg_idx, int_reg_idx, .. } => {
                    self.operand_to_rax(arg);
                    self.state.out.emit_instr_mem_reg("    movq", 8, "rax", X86_ARG_REGS[int_reg_idx]);
                    self.state.out.emit_instr_mem_reg("    movq", 0, "rax", xmm_regs[fp_reg_idx]);
                    float_count += 1;
                }
                CallArgClass::FloatReg { reg_idx } => {
                    match arg {
                        Operand::Const(IrConst::F64(_)) => {
                            self.emit_fp_operand_to_xmm(arg, IrType::F64, xmm_regs[reg_idx]);
                        }
                        Operand::Const(IrConst::F32(_)) => {
                            self.emit_fp_operand_to_xmm(arg, IrType::F32, xmm_regs[reg_idx]);
                        }
                        _ => {
                            self.operand_to_rax(arg);
                            self.state.out.emit_instr_reg_reg("    movq", "rax", xmm_regs[reg_idx]);
                        }
                    }
                    float_count += 1;
                }
                CallArgClass::IntReg { reg_idx } => {
                    let target_reg = X86_ARG_REGS[reg_idx];
                    // Register-direct: callee-saved regs and constants bypass rax.
                    // Skip when it would create a round-trip (e.g., rdi→rbx→rdi)
                    // that the peephole would eliminate along with the param store.
                    let did_direct = if let Operand::Value(v) = arg {
                        if let Some(&phys) = self.reg_assignments.get(&v.0) {
                            if phys.0 >= 1 && phys.0 <= 6 {
                                // Check for round-trip: if this callee-saved reg was
                                // loaded from the SAME arg reg we're targeting, skip
                                let is_round_trip = self.param_source_regs.get(&phys.0)
                                    .map_or(false, |&src| src == target_reg);
                                if !is_round_trip {
                                    let src = super::emit::phys_reg_name(phys);
                                    self.state.out.emit_instr_reg_reg("    movq", src, target_reg);
                                    true
                                } else { false }
                            } else { false }
                        } else { false }
                    } else if let Operand::Const(c) = arg {
                        if let Some(imm) = c.to_i64() {
                            if imm == 0 {
                                let target_32 = match target_reg {
                                    "rdi" => "edi", "rsi" => "esi", "rdx" => "edx",
                                    "rcx" => "ecx", "r8" => "r8d", "r9" => "r9d",
                                    _ => target_reg,
                                };
                                self.state.emit_fmt(format_args!("    xorl %{}, %{}", target_32, target_32));
                            } else {
                                self.state.out.emit_instr_imm_reg("    movq", imm, target_reg);
                            }
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    if !did_direct {
                        self.operand_to_rax(arg);
                        self.state.out.emit_instr_reg_reg("    movq", "rax", target_reg);
                    }
                }
                _ => {}
            }
        }
        if float_count > 0 {
            self.state.out.emit_instr_imm_reg("    movb", float_count as i64, "al");
        } else if self.state.call_is_variadic && !(self.skip_rax_setup && self.no_sse) {
            // %al reports the number of live SSE argument registers to a
            // VARIADIC callee (SysV AMD64 3.5.7). It is meaningless for a
            // prototyped non-variadic callee, and GCC emits nothing there --
            // lccc used to zero %eax before every indirect call, costing an
            // instruction and a false dependency on %rax at each site.
            //
            // -mskip-rax-setup additionally drops it for variadic callees,
            // sound only when no SSE argument register can be live, which
            // -mno-sse guarantees. A real float argument always wins (the
            // `float_count > 0` arm above).
            self.state.emit("    xorl %eax, %eax");
        }
        self.state.reg_cache.invalidate_all();
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        // Restore the original RSP frame size.
        if total_sp_adjust != 0 && self.state.out.use_rsp_addressing {
            self.state.out.rsp_frame_size -= total_sp_adjust;
        }
    }

    pub(super) fn emit_call_instruction_impl(&mut self, direct_name: Option<&str>, func_ptr: Option<&Operand>, _indirect: bool, _stack_arg_space: usize) {
        // Lazy flush: the call clobbers all XMM registers — a pending
        // deferred vector-result store must hit its slot first.
        self.flush_pending_vec_store_impl();
        if let Some(name) = direct_name {
            if self.state.needs_plt(name) {
                // Versioned symbols (`printf@GLIBC_2.2.5`, bound by .symver)
                // must use the base name in @PLT references — GAS 2.47 rejects
                // `sym@ver@PLT` (GAS-oracle). The linker resolves the version.
                let n = name.split('@').next().unwrap_or(name);
                self.state.emit_fmt(format_args!("    call {}@PLT", n));
            } else {
                self.state.out.emit_call(name);
            }
        } else if func_ptr.is_some() {
            // Indirect function pointers are preloaded into r10 by
            // emit_call_spill_fptr_impl before stack arguments are pushed.  Do
            // not reload the Operand here: stack argument setup changes %rsp,
            // so RSP-relative slots would be addressed incorrectly.
            self.emit_retpoline_call("r10");
        }
        self.state.reg_cache.invalidate_all();
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
    }

    pub(super) fn emit_call_cleanup_impl(&mut self, stack_arg_space: usize, _f128_temp_space: usize, _indirect: bool) {
        let need_align_pad = stack_arg_space % 16 != 0;
        let total_cleanup = stack_arg_space + if need_align_pad { 8 } else { 0 };
        if total_cleanup > 0 {
            self.state.out.emit_instr_imm_reg("    addq", total_cleanup as i64, "rsp");
        }
    }

    pub(super) fn set_call_ret_eightbyte_classes_impl(&mut self, classes: &[crate::common::types::EightbyteClass]) {
        self.call_ret_classes = classes.to_vec();
    }

    pub(super) fn set_call_ret_is_f128_sse_impl(&mut self, is_f128: bool) {
        self.call_ret_is_f128_sse = is_f128;
    }

    pub(super) fn emit_call_store_result_impl(&mut self, dest: &Value, return_type: IrType) {
        // Void functions have no meaningful return value — skip the store.
        // Without this, store_rax_to writes garbage %rax to a stack slot that
        // may overlap with callee-saved register save area, corrupting them.
        if return_type == IrType::Void {
            return;
        }
        if self.call_ret_is_f128_sse {
            // _Float128 returns come back in ONE XMM register (xmm0, 16 bytes).
            // Mark the dest as a 16-byte payload value: its slot holds the
            // DATA directly (resolve_slot_addr must answer Direct, and
            // copies/loads/stores must treat it as full 128-bit).
            if let Some(slot) = self.state.get_slot(dest.0) {
                self.state.out.emit_instr_reg_rbp("    movdqu", "xmm0", slot.0);
                self.state.i128_values.insert(dest.0);
                self.state.f128_direct_slots.insert(dest.0);
            }
            self.state.reg_cache.invalidate_all();
            self.flush_pending_vec_store_impl();
            self.state.invalidate_vec_peephole();
            return;
        }
        if is_i128_type(return_type) {
            use crate::common::types::EightbyteClass;
            if self.call_ret_classes.len() == 2 {
                let (c0, c1) = (self.call_ret_classes[0], self.call_ret_classes[1]);
                match (c0, c1) {
                    (EightbyteClass::Integer, EightbyteClass::Sse) => {
                        if let Some(slot) = self.state.get_slot(dest.0) {
                            self.state.out.emit_instr_reg_rbp("    movq", "rax", slot.0);
                            self.state.emit("    movq %xmm0, %rdx");
                            self.state.out.emit_instr_reg_rbp("    movq", "rdx", slot.0 + 8);
                        }
                        self.state.reg_cache.invalidate_all();
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
                    }
                    (EightbyteClass::Sse, EightbyteClass::Integer) => {
                        if let Some(slot) = self.state.get_slot(dest.0) {
                            self.state.emit("    movq %xmm0, %rdx");
                            self.state.out.emit_instr_reg_rbp("    movq", "rdx", slot.0);
                            self.state.out.emit_instr_reg_rbp("    movq", "rax", slot.0 + 8);
                        }
                        self.state.reg_cache.invalidate_all();
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
                    }
                    (EightbyteClass::Sse, EightbyteClass::Sse) => {
                        if let Some(slot) = self.state.get_slot(dest.0) {
                            self.state.emit("    movq %xmm0, %rax");
                            self.state.out.emit_instr_reg_rbp("    movq", "rax", slot.0);
                            self.state.emit("    movq %xmm1, %rax");
                            self.state.out.emit_instr_reg_rbp("    movq", "rax", slot.0 + 8);
                        }
                        self.state.reg_cache.invalidate_all();
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
                    }
                    _ => {
                        self.store_rax_rdx_to(dest);
                    }
                }
            } else {
                self.store_rax_rdx_to(dest);
            }
        } else if return_type == IrType::F32 {
            self.store_xmm_to(dest, "xmm0", IrType::F32);
        } else if return_type == IrType::F128 {
            if let Some(slot) = self.state.get_slot(dest.0) {
                self.state.out.emit_instr_rbp("    fstpt", slot.0);
                self.state.out.emit_instr_rbp("    fldt", slot.0);
                self.state.emit("    subq $8, %rsp");
                self.state.emit("    fstpl (%rsp)");
                self.state.emit("    popq %rax");
                self.state.reg_cache.set_acc(dest.0, false);
                self.state.f128_direct_slots.insert(dest.0);
            } else {
                self.state.emit("    subq $8, %rsp");
                self.state.emit("    fstpl (%rsp)");
                self.state.emit("    popq %rax");
                self.store_rax_to(dest);
            }
        } else if return_type == IrType::F64 {
            self.store_xmm_to(dest, "xmm0", IrType::F64);
        } else {
            self.store_rax_to(dest);
        }
    }

    pub(super) fn emit_call_store_i128_result_impl(&mut self, dest: &Value) {
        self.store_rax_rdx_to(dest);
    }

    pub(super) fn emit_call_move_f32_to_acc_impl(&mut self) {
        self.state.emit("    movd %xmm0, %eax");
    }

    pub(super) fn emit_call_move_f64_to_acc_impl(&mut self) {
        self.state.emit("    movq %xmm0, %rax");
    }
}
