//! X86Codegen: function call operations.

use super::emit::{X86_ARG_REGS, X86Codegen};
use crate::backend::call_abi::{CallAbiConfig, CallArgClass, compute_stack_push_bytes};
use crate::backend::generation::is_i128_type;
use crate::common::types::IrType;
use crate::ir::reexports::{Instruction, IrBinOp, IrConst, Operand, Value};

/// How a resolved global call target must be emitted.
enum ResolvedIndirectCall {
    /// func_ptr was `Load(ptr)` with ptr = GlobalAddr(sym)+off: the loaded
    /// VALUE is the target, so fold load+call into the patchable
    /// memory-indirect form `call *sym+off(%rip)` (ff 15).
    MemorySlot(String, i64),
    /// func_ptr's VALUE itself resolves to sym+off (a GlobalAddr chain with
    /// no intervening Load): the value IS the target, so it must be a
    /// DIRECT `call sym+off`. Emitting `call *sym(%rip)` here would read
    /// sym's first 8 code/data bytes as the target (pic_extern_addr
    /// SIGSEGV: `call *add7(%rip)` jumped into `movq %rdi+7, %rax`).
    Direct(String, i64),
}

impl X86Codegen {
    pub(super) fn call_abi_config_impl(&self) -> CallAbiConfig {
        CallAbiConfig {
            max_int_regs: 6,
            max_float_regs: 8,
            align_i128_pairs: false,
            f128_in_fp_regs: false,
            f128_in_gp_pairs: false,
            variadic_floats_in_gp: false,
            large_struct_by_ref: false,
            use_sysv_struct_classification: true,
            use_riscv_float_struct_classification: false,
            allow_struct_split_reg_stack: false,
            align_struct_pairs: false,
            sret_uses_dedicated_reg: false,
            gcc_regparm_mode: false,
            // GCC >= 4.6 honors the full natural alignment of MEMORY-class
            // stack arguments (SysV AMD64); the >16 case is completed by the
            // caller's dynamic %rsp realignment (emit_call_stack_args_impl).
            stack_arg_align_cap: usize::MAX,
        }
    }

    /// Extract a constant i64 from an Operand, following Copy chains.
    fn operand_const_i64(&self, op: &Operand) -> Option<i64> {
        match op {
            Operand::Const(c) => c.to_i64(),
            Operand::Value(v) => {
                let mut cur = v.0;
                let mut visited = 0;
                loop {
                    if visited > 16 {
                        return None;
                    }
                    visited += 1;
                    let inst = self.get_defining_instruction(cur)?;
                    match inst {
                        Instruction::Copy { src, .. } => match src {
                            Operand::Const(c) => return c.to_i64(),
                            Operand::Value(nv) => cur = nv.0,
                        },
                        Instruction::Cast { src, .. } => match src {
                            Operand::Const(c) => return c.to_i64(),
                            Operand::Value(nv) => cur = nv.0,
                        },
                        _ => return None,
                    }
                }
            }
        }
    }

    /// Resolve a Value chain that ends in GlobalAddr, accumulating GEP/Add/Sub/Cast offsets.
    /// Returns (symbol_name, total_offset) if the chain is GlobalAddr (+ GEPs/Copies/Adds/Subs/Casts).
    /// This is the paravirt fast-path: `pv_ops` is a struct of function pointers, so
    /// `Load(GEP(GlobalAddr pv_ops, off))` must become `call *pv_ops+off(%rip)` (ff 15)
    /// for `apply_alternatives` to patch. The resolver is intentionally generous:
    /// it follows Cast (bitcast/ptrtoint/inttoptr), Copy, Add (Value+Const), Sub (Value-Const),
    /// and GEP with const or Copy-of-const offsets, with checked arithmetic and 64-hop bound.
    /// The symbol name is returned VERBATIM (version suffix intact) — folding policy
    /// (`.symver` rejection, GOT/TLS/absolute guards) lives in the caller.
    fn resolve_global_addr_chain(&self, mut val_id: u32, mut acc: i64) -> Option<(String, i64)> {
        let mut visited = 0;
        loop {
            if visited > 64 {
                return None;
            }
            visited += 1;
            let inst = self.get_defining_instruction(val_id)?;
            match inst {
                Instruction::GlobalAddr { name, .. } => {
                    return Some((name.clone(), acc));
                }
                Instruction::GetElementPtr { base, offset, .. } => {
                    let off = self.operand_const_i64(offset)?;
                    acc = acc.checked_add(off)?;
                    val_id = base.0;
                }
                Instruction::Copy { src, .. } => match src {
                    Operand::Value(v) => val_id = v.0,
                    Operand::Const(_) => return None, // Copy of const is not a global chain
                },
                Instruction::Cast { src, .. } => match src {
                    Operand::Value(v) => val_id = v.0,
                    Operand::Const(_) => return None,
                },
                Instruction::BinOp { op, lhs, rhs, .. } => match op {
                    IrBinOp::Add => {
                        let (base_val, const_off) = match (lhs, rhs) {
                            (Operand::Value(v), c) => {
                                let co = self.operand_const_i64(c)?;
                                (v.0, co)
                            }
                            (c, Operand::Value(v)) => {
                                let co = self.operand_const_i64(c)?;
                                (v.0, co)
                            }
                            _ => return None,
                        };
                        acc = acc.checked_add(const_off)?;
                        val_id = base_val;
                    }
                    IrBinOp::Sub => {
                        // Only Value - Const is representable as sym+off (off decreases)
                        let (base_val, const_off) = match (lhs, rhs) {
                            (Operand::Value(v), c) => {
                                let co = self.operand_const_i64(c)?;
                                (v.0, co)
                            }
                            _ => return None,
                        };
                        acc = acc.checked_sub(const_off)?;
                        val_id = base_val;
                    }
                    _ => return None,
                },
                _ => return None,
            }
        }
    }

    /// Try to resolve an indirect call's func_ptr Operand to a global+offset.
    /// The func_ptr is expected to be Value(Load(ptr)), where ptr is GEP chain
    /// ending in GlobalAddr (e.g. pv_ops+72). Returns Some((sym, off)) if so.
    /// Also handles the case where Load was folded or func_ptr itself is the GEP chain.
    ///
    /// Versioned symbols (`sym@VER`, bound by a top-level `.symver real, sym@VER`
    /// directive) are rejected outright: the memory-indirect form
    /// `call *sym@VER+off(%rip)` is rejected by GAS 2.47, the stripped base
    /// name would silently re-bind the call to the DEFAULT version (a
    /// .symver semantic change, not a fold), and the GOT/TLS/absolute guards
    /// below are keyed by the full symbol name. The value-based fallback
    /// (load the pointer, call through it) is correct for every binding, so
    /// rejecting the fold is strictly safe — and versioned function-pointer
    /// slots are glibc-internal compat shapes, so the cost is nil.
    fn try_resolve_indirect_call_global(&self, op: &Operand) -> Option<ResolvedIndirectCall> {
        let v = match op {
            Operand::Value(v) => v,
            _ => return None,
        };
        let inst = self.get_defining_instruction(v.0)?;
        match inst {
            Instruction::Load { ptr, .. } => {
                // If the loaded pointer's defining chain is global+off, we can fold into
                // the memory-indirect form. Guard against GOT/TLS/absolute symbols that
                // need different addressing.
                let (sym, off) = self.resolve_global_addr_chain(ptr.0, 0)?;
                if sym.contains('@') {
                    return None;
                }
                // Don't use RIP-relative call for symbols that need GOT or are TLS/absolute —
                // those would need GOTPCREL or fs: addressing, not plain *sym(%rip).
                if self.state.needs_got_for_addr(&sym)
                    || self.state.tls_symbols.contains(&sym)
                    || self.state.absolute_symbols.contains(&sym)
                {
                    return None;
                }
                Some(ResolvedIndirectCall::MemorySlot(sym, off))
            }
            // Defensive: func_ptr itself might be the GlobalAddr/GEP chain (if the
            // Load was optimized away). The resolved symbol+offset is then the call
            // target value itself, so only a DIRECT call is semantically correct.
            _ => {
                let (sym, off) = self.resolve_global_addr_chain(v.0, 0)?;
                if sym.contains('@') {
                    return None;
                }
                if self.state.needs_got_for_addr(&sym)
                    || self.state.tls_symbols.contains(&sym)
                    || self.state.absolute_symbols.contains(&sym)
                {
                    return None;
                }
                Some(ResolvedIndirectCall::Direct(sym, off))
            }
        }
    }

    pub(super) fn emit_call_compute_stack_space_impl(
        &self,
        arg_classes: &[CallArgClass],
        _arg_types: &[IrType],
        struct_arg_aligns: &[Option<usize>],
    ) -> usize {
        // x86-64: uncapped per-arg alignment padding (GCC ≥ 4.6 honors the
        // full natural alignment of MEMORY-class stack arguments; the >16
        // case is completed by the dynamic %rsp realignment in
        // emit_call_stack_args_impl).
        compute_stack_push_bytes(arg_classes, struct_arg_aligns, usize::MAX)
    }

    pub(super) fn emit_call_stack_args_impl(
        &mut self,
        args: &[Operand],
        arg_classes: &[CallArgClass],
        _arg_types: &[IrType],
        stack_arg_space: usize,
        _fptr_spill: usize,
        _f128_temp_space: usize,
        struct_arg_aligns: &[Option<usize>],
    ) -> i64 {
        // Per-call-site transient state (defensive reset: the pipeline
        // always consumes the flag, but a stale value from an aborted
        // emission path must never leak into the next call).
        self.dyn_align_cleanup = false;

        // SysV AMD64 (GCC ≥ 4.6 ABI change, flagged by GCC as "the ABI for
        // passing parameters with N-byte alignment has changed in GCC 4.6"):
        // a MEMORY-class argument whose natural alignment exceeds 16 bytes is
        // placed at a slot aligned to its FULL alignment. The callee's va_arg
        // overflow walk aligns dynamically (addq $A-1; andq $-A — see
        // emit_va_arg_struct_overflow_body), anchored at the FIRST stack
        // argument's address (= (%rsp) at the call, where the callee finds it
        // at entry_rsp+8). A purely static layout cannot guarantee that
        // anchor ≡ 0 mod A — the pre-push %rsp is only known to be 16-aligned,
        // so %rsp mod 32/64 is runtime-dependent. When any stack argument
        // needs alignment A > 16, the outgoing area is built GCC-style:
        //   (1) %rsp is realigned BEFORE any push, by D = (R0 − TOTAL) mod A
        //       where TOTAL = save slot + parity pad + argument area is the
        //       static byte count the pushes will consume — so the FINAL
        //       (%rsp) at the call (the anchor) lands exactly on an A
        //       boundary. A is a multiple of 16 and TOTAL is 16-rounded, so D
        //       is a multiple of 16 and %rsp stays 16-aligned at the call;
        //   (2) a 16-byte slot above every argument (the callee's overflow
        //       walk never reads past the last argument it consumes) saves
        //       the pre-realignment %rsp for the exact cleanup restore —
        //       correct even when a DynAlloca moved %rsp between prologue and
        //       this call;
        //   (3) the cleanup skips the static argument area and restores %rsp
        //       exactly from the save slot.
        // The dynamic %rsp delta forbids %rsp-relative slot addressing, so
        // generation.rs pins such functions to an rbp frame (exactly GCC's
        // answer: it realigns the whole frame). %rcx/%rax are free here —
        // register arguments are loaded in Phase 3, after this function.
        let dyn_align_a =
            crate::backend::call_abi::max_stack_arg_alignment(arg_classes, struct_arg_aligns);
        // generation.rs pins functions with >16-aligned stack args to an rbp
        // frame (the dynamic %rsp delta forbids %rsp-relative addressing),
        // so the realign path cannot be reached in FPO mode; the fallback
        // keeps the emission self-consistent if that invariant is ever broken.
        debug_assert!(dyn_align_a <= 16 || !self.state.out.use_rsp_addressing);
        let dyn_align = dyn_align_a > 16 && !self.state.out.use_rsp_addressing;

        let mut sp_adjust: i64 = 0;
        if dyn_align {
            let area = stack_arg_space as i64;
            let parity_pad = if stack_arg_space % 16 != 0 { 8 } else { 0 };
            let total = 16 + parity_pad + area; // 16-rounded by construction
            let a = dyn_align_a as i64;
            // (1) realign: %rsp -= (R0 - TOTAL) mod A  → anchor ≡ 0 (mod A)
            self.state.emit("    movq %rsp, %rcx"); // %rcx = R0 (scratch only)
            self.state.emit("    movq %rcx, %rax");
            self.state
                .emit_fmt(format_args!("    subq ${}, %rax", total));
            self.state.emit_fmt(format_args!("    andq ${}, %rax", a - 1));
            self.state.emit("    subq %rax, %rsp");
            // (2) save the pre-realignment %rsp above every argument
            self.state.emit("    subq $16, %rsp");
            self.state.emit("    movq %rcx, (%rsp)");
            sp_adjust += 16;
            if parity_pad > 0 {
                self.state
                    .out
                    .emit_instr_imm_reg("    subq", parity_pad, "rsp");
                sp_adjust += parity_pad;
            }
            self.dyn_align_cleanup = true;
        } else if stack_arg_space % 16 != 0 {
            self.state.emit("    subq $8, %rsp");
            sp_adjust += 8;
            // Adjust RSP frame size so operand_to_rax slot conversions
            // are correct after the alignment subq.
            if self.state.out.use_rsp_addressing {
                self.state.out.rsp_frame_size += 8;
            }
        }
        let arg_padding = crate::backend::call_abi::compute_stack_arg_padding(
            arg_classes,
            struct_arg_aligns,
            usize::MAX,
        );
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
                                    let x87 = crate::common::long_double::f128_bytes_to_x87_bytes(
                                        f128_bytes,
                                    );
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
                            let hi_2bytes =
                                u16::from_le_bytes(x87_bytes[8..10].try_into().unwrap());
                            self.state
                                .out
                                .emit_instr_imm_reg("    movabsq", lo as i64, "rax");
                            self.state.emit("    movq %rax, (%rsp)");
                            self.state
                                .emit_fmt(format_args!("    movw ${}, 8(%rsp)", hi_2bytes));
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
                                self.state
                                    .emit_fmt(format_args!("    pushq ${}", high as i64));
                                self.state
                                    .emit_fmt(format_args!("    pushq ${}", low as i64));
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
                CallArgClass::StructByValStack { size }
                | CallArgClass::LargeStructStack { size } => {
                    self.operand_to_rax(&args[si]);
                    let n_qwords = size.div_ceil(8);
                    for qi in (0..n_qwords).rev() {
                        let offset = qi * 8;
                        if offset + 8 <= size {
                            self.state
                                .emit_fmt(format_args!("    pushq {}(%rax)", offset));
                        } else {
                            self.state.out.emit_instr_mem_reg(
                                "    movq",
                                offset as i64,
                                "rax",
                                "rcx",
                            );
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
                self.state
                    .out
                    .emit_instr_imm_reg("    subq", pad as i64, "rsp");
                sp_adjust += pad as i64;
                if self.state.out.use_rsp_addressing {
                    self.state.out.rsp_frame_size += pad as i64;
                }
            }
        }
        // Restore the original RSP frame size. The total adjustment is
        // tracked in sp_adjust and returned to emit_call_reg_args. (The
        // dynamic realignment delta is NOT tracked — it is invisible to the
        // rbp-relative addressing that such functions are pinned to.)
        if self.state.out.use_rsp_addressing {
            self.state.out.rsp_frame_size -= sp_adjust;
        }
        // Return the total STATIC RSP adjustment so emit_call_reg_args can
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
        // Statically resolved targets (a paravirt data cell, or a direct
        // `&symbol` value) need no r10 spill: emit_call_instruction_impl
        // emits the resolved call form directly.
        if self.try_resolve_indirect_call_global(func_ptr).is_some() {
            self.state.reg_cache.invalidate_all();
            self.flush_pending_vec_store_impl();
            self.state.invalidate_vec_peephole();
            return;
        }
        self.operand_to_rax(func_ptr);
        self.state.emit("    movq %rax, %r10");
        self.state.reg_cache.invalidate_all();
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
    }

    pub(super) fn emit_call_reg_args_impl(
        &mut self,
        args: &[Operand],
        arg_classes: &[CallArgClass],
        _arg_types: &[IrType],
        total_sp_adjust: i64,
        _f128_temp_space: usize,
        _stack_arg_space: usize,
        _struct_arg_riscv_float_classes: &[Option<crate::common::types::RiscvFloatClass>],
    ) {
        // Stack args (Phase 2) may have adjusted rsp. Temporarily increase
        // the RSP frame size so rbp-to-rsp offset conversion is correct
        // when loading register arguments from stack slots.
        if total_sp_adjust != 0 && self.state.out.use_rsp_addressing {
            self.state.out.rsp_frame_size += total_sp_adjust;
        }
        let xmm_regs = [
            "xmm0", "xmm1", "xmm2", "xmm3", "xmm4", "xmm5", "xmm6", "xmm7",
        ];
        let mut float_count = 0usize;

        // ---- Staging-hazard pre-spill (session 25) ----
        // Die-at-birth register coalescing can home a still-needed value in
        // an argument register that an EARLIER argument's staging overwrites:
        // fp_die_at_birth's `cd` accumulator was homed in xmm3 (sharing with
        // the dying `cn`), and staging emitted `movsd %xmm7,%xmm3` (cn, arg4)
        // BEFORE reading cd for arg5 — arg5 then read cn's value ("chain_div
        // returned chain_neg's value").  Detect every argument whose source
        // home is an argument register already written by an earlier-staged
        // argument, and pre-spill those values to a transient stack area
        // BEFORE any staging write.  Symmetric for GPR argument registers.
        let mut written_xmm: [bool; 8] = [false; 8];
        let mut written_gpr: [bool; 6] = [false; 6];
        // (arg index, phys home, is_fp)
        let mut hazards: Vec<(usize, crate::backend::regalloc::PhysReg, bool)> = Vec::new();
        for (i, arg) in args.iter().enumerate() {
            if let Operand::Value(v) = arg {
                if let Some(&phys) = self.reg_assignments.get(&v.0) {
                    if super::emit::is_xmm_reg(phys) {
                        // Allocator XMM homes are PhysReg 20..33 = xmm2..xmm15.
                        let src_idx = phys.0 as i64 - 18;
                        if (0..=7).contains(&src_idx) && written_xmm[src_idx as usize] {
                            hazards.push((i, phys, true));
                        }
                    } else {
                        // Allocatable GPR homes that double as SysV arg regs:
                        // rdi=14→arg0, rsi=15→arg1, rdx=16→arg2, r8=12→arg4,
                        // r9=13→arg5 (rcx/rax are not allocatable homes).
                        let src_idx: Option<usize> = match phys.0 {
                            14 => Some(0),
                            15 => Some(1),
                            16 => Some(2),
                            12 => Some(4),
                            13 => Some(5),
                            _ => None,
                        };
                        if let Some(si) = src_idx {
                            if written_gpr[si] {
                                hazards.push((i, phys, false));
                            }
                        }
                    }
                }
            }
            // Mark the targets THIS argument writes (after its own check).
            match &arg_classes[i] {
                CallArgClass::FloatReg { reg_idx } => {
                    if *reg_idx < 8 {
                        written_xmm[*reg_idx] = true;
                    }
                }
                CallArgClass::IntReg { reg_idx } => {
                    if *reg_idx < 6 {
                        written_gpr[*reg_idx] = true;
                    }
                }
                CallArgClass::StructSseReg {
                    lo_fp_idx,
                    hi_fp_idx,
                    ..
                } => {
                    if *lo_fp_idx < 8 {
                        written_xmm[*lo_fp_idx] = true;
                    }
                    if let Some(hi) = hi_fp_idx {
                        if *hi < 8 {
                            written_xmm[*hi] = true;
                        }
                    }
                }
                CallArgClass::F128SseReg { reg_idx } => {
                    if *reg_idx < 8 {
                        written_xmm[*reg_idx] = true;
                    }
                }
                CallArgClass::StructMixedIntSseReg {
                    int_reg_idx,
                    fp_reg_idx,
                    ..
                }
                | CallArgClass::StructMixedSseIntReg {
                    int_reg_idx,
                    fp_reg_idx,
                    ..
                } => {
                    if *fp_reg_idx < 8 {
                        written_xmm[*fp_reg_idx] = true;
                    }
                    if *int_reg_idx < 6 {
                        written_gpr[*int_reg_idx] = true;
                    }
                }
                _ => {}
            }
        }
        let mut hazard_slot: crate::common::fx_hash::FxHashMap<usize, i64> =
            crate::common::fx_hash::FxHashMap::default();
        let hazard_area = ((hazards.len() * 8 + 15) / 16) * 16;
        if hazard_area > 0 {
            self.state
                .emit_fmt(format_args!("    subq ${}, %rsp", hazard_area));
            // The pre-spill area moves rsp: every rsp-relative slot read
            // emitted during the staging loop below (emit_fp_operand_to_xmm,
            // operand_to_rax, ... all go through slot_ref) must see the
            // enlarged frame, or they read hazard_area bytes BELOW their
            // slot. Under rsp addressing this returned stale/garbage args:
            // printf("%f %f %f %f", s1, s2, t1, t2) printed t1's value as
            // arg1 and stack garbage for args 2-3. rbp-addressed frames are
            // unaffected (slot_ref ignores rsp there), which is why the
            // session-25 hazard tests missed it.
            if self.state.out.use_rsp_addressing {
                self.state.out.rsp_frame_size += hazard_area as i64;
            }
            for (k, (arg_i, phys, is_fp)) in hazards.iter().enumerate() {
                let off = (k * 8) as i64;
                hazard_slot.insert(*arg_i, off);
                let name = super::emit::phys_reg_name(*phys);
                if *is_fp {
                    self.state
                        .emit_fmt(format_args!("    movsd %{}, {}(%rsp)", name, off));
                } else {
                    self.state
                        .emit_fmt(format_args!("    movq %{}, {}(%rsp)", name, off));
                }
            }
        }

        for (i, arg) in args.iter().enumerate() {
            // Hazard arguments read their pre-spilled slot instead of the
            // (already-overwritten) argument-register home.
            if let Some(&off) = hazard_slot.get(&i) {
                match &arg_classes[i] {
                    CallArgClass::FloatReg { reg_idx } => {
                        self.state.emit_fmt(format_args!(
                            "    movsd {}(%rsp), %{}",
                            off, xmm_regs[*reg_idx]
                        ));
                        float_count += 1;
                        continue;
                    }
                    CallArgClass::IntReg { reg_idx } => {
                        self.state.emit_fmt(format_args!(
                            "    movq {}(%rsp), %{}",
                            off, X86_ARG_REGS[*reg_idx]
                        ));
                        continue;
                    }
                    _ => {}
                }
            }
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
                                self.state
                                    .out
                                    .emit_instr_rbp_reg("    movq", slot.0, lo_reg);
                                self.state
                                    .out
                                    .emit_instr_rbp_reg("    movq", slot.0 + 8, hi_reg);
                            } else {
                                // No slot: zero both halves
                                self.state
                                    .emit_fmt(format_args!("    xorq %{}, %{}", lo_reg, lo_reg));
                                self.state
                                    .emit_fmt(format_args!("    xorq %{}, %{}", hi_reg, hi_reg));
                            }
                        }
                        Operand::Const(c) => {
                            match c {
                                IrConst::I128(v) => {
                                    let low = *v as u64;
                                    let high = (*v >> 64) as u64;
                                    self.state.emit_fmt(format_args!(
                                        "    movabsq ${}, %{}",
                                        low as i64, lo_reg
                                    ));
                                    self.state.emit_fmt(format_args!(
                                        "    movabsq ${}, %{}",
                                        high as i64, hi_reg
                                    ));
                                }
                                IrConst::Zero => {
                                    self.state.emit_fmt(format_args!(
                                        "    xorq %{}, %{}",
                                        lo_reg, lo_reg
                                    ));
                                    self.state.emit_fmt(format_args!(
                                        "    xorq %{}, %{}",
                                        hi_reg, hi_reg
                                    ));
                                }
                                _ => {
                                    // Smaller constant: load into lo_reg via rax, zero hi_reg
                                    self.operand_to_rax(arg);
                                    self.state.out.emit_instr_reg_reg("    movq", "rax", lo_reg);
                                    self.state.emit_fmt(format_args!(
                                        "    xorq %{}, %{}",
                                        hi_reg, hi_reg
                                    ));
                                }
                            }
                        }
                    }
                }
                CallArgClass::StructByValReg { base_reg_idx, size } => {
                    self.operand_to_rax(arg);
                    let lo_reg = X86_ARG_REGS[base_reg_idx];
                    self.state
                        .out
                        .emit_instr_mem_reg("    movq", 0, "rax", lo_reg);
                    if size > 8 {
                        let hi_reg = X86_ARG_REGS[base_reg_idx + 1];
                        self.state
                            .out
                            .emit_instr_mem_reg("    movq", 8, "rax", hi_reg);
                    }
                }
                CallArgClass::StructSseReg {
                    lo_fp_idx,
                    hi_fp_idx,
                    ..
                } => {
                    self.operand_to_rax(arg);
                    self.state
                        .out
                        .emit_instr_mem_reg("    movq", 0, "rax", xmm_regs[lo_fp_idx]);
                    float_count += 1;
                    if let Some(hi) = hi_fp_idx {
                        self.state
                            .out
                            .emit_instr_mem_reg("    movq", 8, "rax", xmm_regs[hi]);
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
                                self.state.out.emit_instr_rbp_reg(
                                    "    movdqu",
                                    slot.0,
                                    xmm_regs[reg_idx],
                                );
                            } else {
                                self.state.emit_fmt(format_args!(
                                    "    pxor %{}, %{}",
                                    xmm_regs[reg_idx], xmm_regs[reg_idx]
                                ));
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
                            self.state
                                .out
                                .emit_instr_imm_reg("    movabsq", lo as i64, "rax");
                            self.state.emit("    movq %rax, (%rsp)");
                            self.state
                                .out
                                .emit_instr_imm_reg("    movabsq", hi as i64, "rax");
                            self.state.emit("    movq %rax, 8(%rsp)");
                            self.state.emit_fmt(format_args!(
                                "    movdqu (%rsp), %{}",
                                xmm_regs[reg_idx]
                            ));
                            self.state.emit("    addq $16, %rsp");
                        }
                        Operand::Const(IrConst::LongDouble(_, f128_bytes)) => {
                            // long double (x87 80-bit): the constant's payload
                            // is binary128; narrow to x87 bytes first, then
                            // materialize the 16-byte slot (10 significant
                            // bytes) into the XMM argument register.
                            let x87 =
                                crate::common::long_double::f128_bytes_to_x87_bytes(f128_bytes);
                            let lo = u64::from_le_bytes(x87[0..8].try_into().unwrap());
                            let hi = u64::from_le_bytes([x87[8], x87[9], 0, 0, 0, 0, 0, 0]);
                            self.state.emit("    subq $16, %rsp");
                            self.state
                                .out
                                .emit_instr_imm_reg("    movabsq", lo as i64, "rax");
                            self.state.emit("    movq %rax, (%rsp)");
                            self.state
                                .out
                                .emit_instr_imm_reg("    movabsq", hi as i64, "rax");
                            self.state.emit("    movq %rax, 8(%rsp)");
                            self.state.emit_fmt(format_args!(
                                "    movdqu (%rsp), %{}",
                                xmm_regs[reg_idx]
                            ));
                            self.state.emit("    addq $16, %rsp");
                        }
                        _ => {
                            self.operand_to_rax(arg);
                            self.state.emit_fmt(format_args!(
                                "    movdqu 0(%rax), %{}",
                                xmm_regs[reg_idx]
                            ));
                        }
                    }
                    float_count += 1;
                }
                CallArgClass::StructMixedIntSseReg {
                    int_reg_idx,
                    fp_reg_idx,
                    ..
                } => {
                    self.operand_to_rax(arg);
                    self.state
                        .out
                        .emit_instr_mem_reg("    movq", 8, "rax", xmm_regs[fp_reg_idx]);
                    float_count += 1;
                    self.state.out.emit_instr_mem_reg(
                        "    movq",
                        0,
                        "rax",
                        X86_ARG_REGS[int_reg_idx],
                    );
                }
                CallArgClass::StructMixedSseIntReg {
                    fp_reg_idx,
                    int_reg_idx,
                    ..
                } => {
                    self.operand_to_rax(arg);
                    self.state.out.emit_instr_mem_reg(
                        "    movq",
                        8,
                        "rax",
                        X86_ARG_REGS[int_reg_idx],
                    );
                    self.state
                        .out
                        .emit_instr_mem_reg("    movq", 0, "rax", xmm_regs[fp_reg_idx]);
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
                            self.state
                                .out
                                .emit_instr_reg_reg("    movq", "rax", xmm_regs[reg_idx]);
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
                                let is_round_trip = self
                                    .param_source_regs
                                    .get(&phys.0)
                                    .map_or(false, |&src| src == target_reg);
                                if !is_round_trip {
                                    let src = super::emit::phys_reg_name(phys);
                                    self.state
                                        .out
                                        .emit_instr_reg_reg("    movq", src, target_reg);
                                    true
                                } else {
                                    false
                                }
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    } else if let Operand::Const(c) = arg {
                        if let Some(imm) = c.to_i64() {
                            if imm == 0 {
                                let target_32 = match target_reg {
                                    "rdi" => "edi",
                                    "rsi" => "esi",
                                    "rdx" => "edx",
                                    "rcx" => "ecx",
                                    "r8" => "r8d",
                                    "r9" => "r9d",
                                    _ => target_reg,
                                };
                                self.state.emit_fmt(format_args!(
                                    "    xorl %{}, %{}",
                                    target_32, target_32
                                ));
                            } else {
                                self.state
                                    .out
                                    .emit_instr_imm_reg("    movq", imm, target_reg);
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
                        self.state
                            .out
                            .emit_instr_reg_reg("    movq", "rax", target_reg);
                    }
                }
                _ => {}
            }
        }
        if float_count > 0 {
            self.state
                .out
                .emit_instr_imm_reg("    movb", float_count as i64, "al");
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
        // Release the staging-hazard pre-spill area (all hazard args have
        // been staged from it; nothing beyond this point reads it).
        if hazard_area > 0 {
            self.state
                .emit_fmt(format_args!("    addq ${}, %rsp", hazard_area));
            if self.state.out.use_rsp_addressing {
                self.state.out.rsp_frame_size -= hazard_area as i64;
            }
        }
        // Restore the original RSP frame size.
        if total_sp_adjust != 0 && self.state.out.use_rsp_addressing {
            self.state.out.rsp_frame_size -= total_sp_adjust;
        }
    }

    pub(super) fn emit_call_instruction_impl(
        &mut self,
        direct_name: Option<&str>,
        func_ptr: Option<&Operand>,
        _indirect: bool,
        _stack_arg_space: usize,
    ) {
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
        } else if let Some(fp) = func_ptr {
            // Paravirt/global fast-path: if func_ptr resolves to a global+offset
            // (e.g. pv_ops+72), emit `call *sym+off(%rip)` directly. This produces
            // the `ff 15 disp32` encoding with R_X86_64_PC32 reloc that
            // arch/x86/kernel/paravirt.c:apply_alternatives expects to patch.
            // Without this, we would emit `mov $sym, %r15; call *off(%r15)`
            // (41 ff 57) which is not patchable and breaks boot.
            if let Some(target) = self.try_resolve_indirect_call_global(fp) {
                // Construct `sym+off` string for GAS. Use `sym` alone when off==0.
                let (sym, off) = match &target {
                    ResolvedIndirectCall::MemorySlot(sym, off)
                    | ResolvedIndirectCall::Direct(sym, off) => (sym.clone(), *off),
                };
                let sym_str = if off == 0 {
                    sym.clone()
                } else if off > 0 {
                    format!("{}+{}", sym, off)
                } else {
                    format!("{}{}", sym, off) // off negative includes '-'
                };
                match target {
                    // Memory-indirect through the slot: GAS produces ff 15 with a
                    // PC32 reloc (the paravirt patchable form). Raw emit avoids
                    // any PLT/GOT logic.
                    ResolvedIndirectCall::MemorySlot(..) => {
                        self.state
                            .emit_fmt(format_args!("    call *{}({})", sym_str, "%rip"));
                    }
                    // The resolved value IS the target: direct call, with the
                    // same PLT handling as a syntactically direct call. An
                    // offset into the callee (`&func + off`, computed-goto
                    // style block addresses) is never interposed — the PLT
                    // entry is only defined for offset 0 — so the offset is
                    // preserved and @PLT is used only for it. (Versioned
                    // symbols never reach this arm: they are rejected in
                    // try_resolve_indirect_call_global and fall back to the
                    // value-based call, which preserves the .symver binding.)
                    ResolvedIndirectCall::Direct(..) => {
                        if off == 0 && self.state.needs_plt(&sym) {
                            self.state
                                .emit_fmt(format_args!("    call {}@PLT", sym));
                        } else {
                            self.state.out.emit_call(&sym_str);
                        }
                    }
                }
            } else {
                // Indirect function pointers are preloaded into r10 by
                // emit_call_spill_fptr_impl before stack arguments are pushed.  Do
                // not reload the Operand here: stack argument setup changes %rsp,
                // so RSP-relative slots would be addressed incorrectly.
                self.emit_retpoline_call("r10");
            }
        }
        self.state.reg_cache.invalidate_all();
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
    }

    pub(super) fn emit_call_cleanup_impl(
        &mut self,
        stack_arg_space: usize,
        _f128_temp_space: usize,
        _indirect: bool,
    ) {
        // Dynamically realigned outgoing area: after the call %rsp points at
        // the first stack argument (the anchor). Skip the statically-pushed
        // argument area (16-rounded) to the save slot written by
        // emit_call_stack_args_impl, then restore %rsp EXACTLY to its
        // pre-realignment value — correct even under DynAlloca. %r11 is the
        // scratch — %rax may still carry the callee's return value for Phase
        // 6's result store, which runs after this cleanup.
        if self.dyn_align_cleanup {
            self.dyn_align_cleanup = false;
            let area = (stack_arg_space + 15) & !15;
            if area > 0 {
                self.state
                    .out
                    .emit_instr_imm_reg("    addq", area as i64, "rsp");
            }
            self.state.emit("    movq (%rsp), %r11");
            self.state.emit("    movq %r11, %rsp");
            return;
        }
        let need_align_pad = stack_arg_space % 16 != 0;
        let total_cleanup = stack_arg_space + if need_align_pad { 8 } else { 0 };
        if total_cleanup > 0 {
            self.state
                .out
                .emit_instr_imm_reg("    addq", total_cleanup as i64, "rsp");
        }
    }

    pub(super) fn set_call_ret_eightbyte_classes_impl(
        &mut self,
        classes: &[crate::common::types::EightbyteClass],
    ) {
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
                self.state
                    .out
                    .emit_instr_reg_rbp("    movdqu", "xmm0", slot.0);
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
                            // SSE eightbyte: store %xmm0 directly (movq
                            // xmm→m64 is baseline SSE2); the old %rdx relay
                            // was an FP-domain round-trip for no reason.
                            self.state
                                .out
                                .emit_instr_reg_rbp("    movq", "xmm0", slot.0 + 8);
                        }
                        self.state.reg_cache.invalidate_all();
                        self.flush_pending_vec_store_impl();
                        self.state.invalidate_vec_peephole();
                    }
                    (EightbyteClass::Sse, EightbyteClass::Integer) => {
                        if let Some(slot) = self.state.get_slot(dest.0) {
                            // Direct xmm0→slot store (baseline SSE2 movq);
                            // no GPR relay.
                            self.state
                                .out
                                .emit_instr_reg_rbp("    movq", "xmm0", slot.0);
                            self.state
                                .out
                                .emit_instr_reg_rbp("    movq", "rax", slot.0 + 8);
                        }
                        self.state.reg_cache.invalidate_all();
                        self.flush_pending_vec_store_impl();
                        self.state.invalidate_vec_peephole();
                    }
                    (EightbyteClass::Sse, EightbyteClass::Sse) => {
                        if let Some(slot) = self.state.get_slot(dest.0) {
                            // Both halves are SSE: two direct xmm stores,
                            // replacing the double movq→rax relay (2 extra
                            // insns and two false GPR dependencies).
                            self.state
                                .out
                                .emit_instr_reg_rbp("    movq", "xmm0", slot.0);
                            self.state
                                .out
                                .emit_instr_reg_rbp("    movq", "xmm1", slot.0 + 8);
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
