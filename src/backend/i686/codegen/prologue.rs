//! I686Codegen: prologue/epilogue and stack frame operations.

use super::emit::{
    i686_clobber_to_phys, i686_constraint_to_phys, phys_reg_name, I686Codegen, I686_CALLEE_SAVED,
    I686_CALLEE_SAVED_WITH_EBP, I686_CALLER_SAVED,
};
use crate::backend::call_abi::{classify_params, ParamClass};
use crate::backend::generation::{
    calculate_stack_space_common, filter_available_regs, find_param_alloca, is_i128_type,
    run_regalloc_and_merge_clobbers,
};
use crate::backend::regalloc::PhysReg;
use crate::backend::traits::ArchCodegen;
use crate::common::types::IrType;
use crate::emit;
use crate::ir::reexports::{Instruction, IrFunction, Value};

impl I686Codegen {
    /// Does this function need the PIC GOT base in %ebx?
    ///
    /// Exactly the constructs that expand to a `@GOT`/`@GOTOFF` reference:
    ///   * `GlobalAddr`  -> `movl sym@GOT(%ebx)` / `leal sym@GOTOFF(%ebx)`
    ///   * `LabelAddr`   -> `leal L@GOTOFF(%ebx)`   (computed goto, `&&label`)
    ///   * TLS addresses -> `movl sym@GOTNTPOFF(%ebx)`
    ///   * a `Switch` lowered to a jump table -> `leal jt@GOTOFF(%ebx)`
    ///   * inline asm, which may name any symbol in text we do not parse
    ///
    /// Deliberately an ALLOWLIST with a conservative default: anything not
    /// recognised here keeps the GOT. Getting this wrong in the permissive
    /// direction produces a wild %ebx dereference, so the failure mode must be
    /// "one wasted register", never "wrong address".
    fn function_needs_got(&self, func: &IrFunction) -> bool {
        use crate::ir::reexports::{Instruction, Terminator};
        if !self.state.pic_mode {
            return false;
        }
        for block in &func.blocks {
            // A Switch may be lowered to a GOTOFF-relative jump table. The
            // decision happens later in emit_switch_jump_table, so assume the
            // table form whenever a Switch is present.
            if matches!(block.terminator, Terminator::Switch { .. }) {
                return true;
            }
            if matches!(block.terminator, Terminator::IndirectBranch { .. }) {
                return true;
            }
            for inst in &block.instructions {
                match inst {
                    Instruction::GlobalAddr { .. }
                    | Instruction::LabelAddr { .. }
                    | Instruction::InlineAsm { .. }
                    | Instruction::InitTrampoline { .. } => return true,
                    // A `call sym@PLT` is the i386 PSABI's OTHER consumer of
                    // %ebx: the lazy-binding PLT stub is
                    //     jmp *name@GOT(%ebx); push $reloc; jmp .plt0
                    // and .plt0 itself does `push 4(%ebx); jmp *8(%ebx)`.
                    // Without the GOT base at the call site the very first
                    // call through the stub jumps through garbage. This is
                    // why GCC sets up the thunk even in functions whose only
                    // PIC-relevant construct is an external call (verified:
                    // gcc -m32 -fPIC on `return helper(x)+1;` emits
                    // get_pc_thunk.bx + call helper@PLT).
                    Instruction::Call { func: callee, .. } if self.state.needs_plt(callee) => {
                        return true;
                    }
                    // 64-bit div/mod lowers to __{u}divdi3/__{u}moddi3 calls
                    // behind the backend's back (emit_i128_divmod). Those are
                    // PLT calls under the same rule when the weak stubs end
                    // up resolved externally, so keep the GOT alive for them.
                    Instruction::BinOp { op, ty, .. }
                        if matches!(
                            op,
                            crate::ir::reexports::IrBinOp::SDiv
                                | crate::ir::reexports::IrBinOp::UDiv
                                | crate::ir::reexports::IrBinOp::SRem
                                | crate::ir::reexports::IrBinOp::URem
                        ) && matches!(ty, IrType::I64 | IrType::U64) =>
                    {
                        return true;
                    }
                    _ => {}
                }
            }
        }
        false
    }

    // ---- calculate_stack_space ----

    pub(super) fn calculate_stack_space_impl(&mut self, func: &IrFunction) -> i64 {
        self.is_variadic = func.is_variadic;
        self.is_fastcall = func.is_fastcall;
        self.current_return_type = func.return_type;

        // Same-block div/rem pair fusion table (one divl serves a
        // URem+UDiv couple with identical operands). Constant-RHS pairs
        // fuse as well: the head runs ONE magic-number sequence producing
        // q in %eax and r in %edx (emit_divrem_const_in_eax_edx), or the
        // staged `divl` at -Os: the same clobber set the RA model charges
        // any constant-divisor division with, so the model stays exact.
        let pairs = crate::backend::regalloc::compute_i686_divrem_pairs(
            func,
            crate::backend::regalloc::DivRemTarget::I686,
        );
        self.divrem_tail_dests = pairs.tail_dests;
        self.divrem_head_partners = pairs.head_partners;
        self.divrem_broken_tails.clear();

        // Zero-extending wide casts (u8/u16/u32/ptr -> i64/u64): their high
        // half is provably zero — the 64-bit ALU fast paths use this to drop
        // the alo*bhi cross product and to carry with an immediate 0.
        self.zext_wide_values.clear();
        for block in &func.blocks {
            for inst in &block.instructions {
                if let crate::ir::reexports::Instruction::Cast {
                    dest,
                    src: crate::ir::reexports::Operand::Value(_),
                    from_ty,
                    to_ty,
                } = inst
                {
                    let zext = matches!(
                        (from_ty, to_ty),
                        (
                            crate::common::types::IrType::U8
                                | crate::common::types::IrType::U16
                                | crate::common::types::IrType::U32
                                | crate::common::types::IrType::Ptr,
                            crate::common::types::IrType::I64 | crate::common::types::IrType::U64
                        )
                    );
                    if zext {
                        self.zext_wide_values.insert(dest.0);
                    }
                }
            }
        }

        // Dynamic alloca (VLAs) requires the frame pointer to track the stack,
        // since ESP changes by runtime-computed amounts.
        if self.state.has_dyn_alloca {
            self.omit_frame_pointer = false;
        }

        // __builtin_frame_address(0) and __builtin_setjmp read %ebp directly;
        // under -fomit-frame-pointer the register would hold a stale
        // caller-owned value.  Mirror the has_frame_address veto the x86-64
        // pipeline applies in backend/generation.rs.
        // gcc.c-torture execute/frame-address.c caught this: at -O1/-O2 the
        // noipa helper containing the intrinsic lost its frame pointer while
        // its caller (holding a dynamic alloca) kept one, so the stack-address
        // containment check failed and the test aborted.
        if func.blocks.iter().any(|block| {
            block.instructions.iter().any(|inst| {
                matches!(
                    inst,
                    crate::ir::reexports::Instruction::Intrinsic {
                        op: crate::ir::intrinsics::IntrinsicOp::FrameAddress
                            | crate::ir::intrinsics::IntrinsicOp::BuiltinSetjmp,
                        ..
                    }
                )
            })
        }) {
            self.omit_frame_pointer = false;
        }

        // Compute named parameter stack bytes for va_start (variadic functions).
        {
            let config = self.call_abi_config();
            let classification = crate::backend::call_abi::classify_params_full(func, &config);
            self.incoming_stack_arg_bytes = classification.total_stack_bytes as i64;
            if func.is_variadic {
                self.va_named_stack_bytes = classification.total_stack_bytes;
            }
        }

        // Run register allocator before stack space computation.
        // Use the _with_generic variant to conservatively mark all callee-saved
        // registers as clobbered when generic register constraints (r, q, g) are
        // present. On i686, the scratch allocator may pick esi/edi/ebx for generic
        // constraints, which would clobber values the register allocator placed there.
        let mut asm_clobbered_regs: Vec<PhysReg> = Vec::new();

        // When omitting the frame pointer, EBP is available as a callee-saved
        // register, so use the extended set that includes EBP.
        let callee_saved_set = if self.omit_frame_pointer {
            I686_CALLEE_SAVED_WITH_EBP
        } else {
            I686_CALLEE_SAVED
        };

        // Precision variant: the blanket "generic constraint clobbers every
        // callee-saved register" rule is skipped when a per-block demand scan
        // proves the scratch allocator can satisfy the block from %ecx/%edx
        // (the head of its fixed pool). arch/x86/boot is full of one-operand
        // "=rm"/"r" segment reads that were paying 3 push/pop pairs plus
        // total register-allocator starvation for a guarantee they never
        // needed. Falls back to the conservative marking when any block's
        // demand exceeds the caller-saved prefix.
        crate::backend::stack_layout::collect_inline_asm_callee_saved_i686(
            func,
            &mut asm_clobbered_regs,
            i686_constraint_to_phys,
            i686_clobber_to_phys,
            callee_saved_set,
        );
        // In PIC mode, %ebx (PhysReg(0)) is reserved as the GOT base pointer --
        // but ONLY for functions that actually reference the GOT.
        //
        // Unconditionally reserving it cost every leaf function 12 bytes it
        // could not use: `push %ebx` + `call __x86.get_pc_thunk.bx` +
        // `addl $_GLOBAL_OFFSET_TABLE_,%ebx` + `pop %ebx`, plus the loss of a
        // register that then forced spills elsewhere. On the 32 KiB-limited
        // kernel setup binary that is pure waste, because the overwhelming
        // majority of those functions never form a global address at all.
        // GCC makes the same call: it materializes the GOT base lazily.
        let needs_got = self.function_needs_got(func);
        self.pic_got_live = needs_got;
        if needs_got && !asm_clobbered_regs.contains(&PhysReg(0)) {
            asm_clobbered_regs.push(PhysReg(0));
        }
        let mut available_regs = filter_available_regs(callee_saved_set, &asm_clobbered_regs);

        let mut caller_saved_regs = I686_CALLER_SAVED.to_vec();
        if self.state.disable_regalloc {
            available_regs.clear();
            caller_saved_regs.clear();
        }

        // GlobalAddr values whose EVERY use is a foldable Load/Store pointer
        // never materialize (non-PIC absolute fold in generate_load/store);
        // keep them out of the allocator so a dead address cannot occupy a
        // register the live loaded value needs. Mirrors the fold conditions:
        // non-PIC, non-TLS, non-wide type, default address space.
        let mut never_materialized = if !self.state.pic_mode {
            let gmap = crate::backend::generation::build_global_addr_map_for(
                func,
                &self.state.tls_symbols,
            );
            let mut set =
                crate::backend::generation::build_foldable_global_addr_set_for(func, &gmap);
            // Loads folded into `cmp{b,w,l} $imm,(mem)` and fused
            // compare-and-branch booleans emit no code and write no value, so
            // they must not grab a caller-saved register that the live loop
            // pointer needs (strlen/strchr/strstr used to park the dead byte
            // or boolean in %edx, forcing the pointer into a push/pop-paying
            // callee-saved home).
            set.extend(crate::backend::generation::build_folded_value_set(
                func,
                &self.state.tls_symbols,
                &self.state.absolute_symbols,
            ));
            // Also deny stack slots: these values produce no code at all.
            self.state.never_materialized_values = set.clone();
            Some(set)
        } else {
            None
        };

        // Return-consumed accumulator flow: a value whose SOLE use is the
        // Return terminator and whose producer preserves the accumulator
        // (compute_immediately_consumed enforces both, plus producer/terminator
        // adjacency) is consumed by emit_return_default via operand_to_eax —
        // a cache hit, zero instructions.  Give it NO home: the producer
        // leaves the result in %eax (store_eax_to emits nothing without a
        // home and keeps the cache entry), and the ret reads it there.
        // strlen's `p - s` tail stops paying `movl %edi,%eax` + a register
        // home.  Kept out of the PIC-only set above because it applies
        // regardless of PIC mode; state.never_materialized_values (the
        // emitter's fold set) is deliberately NOT extended — these values DO
        // emit their producer.
        {
            let mut ret_operands: crate::common::fx_hash::FxHashSet<u32> =
                crate::common::fx_hash::FxHashSet::default();
            for block in &func.blocks {
                if let crate::ir::reexports::Terminator::Return(Some(op)) = &block.terminator {
                    if let crate::ir::reexports::Operand::Value(v) = op {
                        ret_operands.insert(v.0);
                    }
                }
            }
            // On i686 EVERY immediately-consumed value is read through the
            // accumulator (operand_to_eax checks the acc cache first): Store
            // val, Cast/UnaryOp/Copy src, BinOp/Cmp lhs, and fused-consume
            // terminators all stage via %eax.  Giving them a register home
            // only adds a `movl %R,%eax` relay (and a callee-save push for
            // the home).  Deny them registers as well as slots; the
            // direct-dest binop path then declines (no home) and the
            // accumulator path takes over, which is the shape the consumer
            // expects.  compute_immediately_consumed already proves the
            // consumer is adjacent (no cache-invalidation window) and the
            // producer accumulator-preserving.
            //
            // EXCEPTION — CondBranch conditions: the truthy test of a
            // register-resident condition emits `testl %R,%R` IN PLACE, but
            // a home-less condition pays `movl %src,%eax; testl %eax,%eax`
            // (the no-op-coalesced producer sets no acc-cache entry).  Keep
            // their homes (check_load_widen_cast_no_relay regression).
            let mut cond_values: crate::common::fx_hash::FxHashSet<u32> =
                crate::common::fx_hash::FxHashSet::default();
            for block in &func.blocks {
                if let crate::ir::reexports::Terminator::CondBranch { cond, .. } = &block.terminator
                {
                    if let crate::ir::reexports::Operand::Value(v) = cond {
                        cond_values.insert(v.0);
                    }
                }
            }
            let _ = &ret_operands; // (ret_operands kept for the PIC-free path)
            let skip = crate::backend::regalloc::analyze_accumulator_assignments(
                func,
                crate::backend::regalloc::AccumulatorPolicy {
                    operand_order:
                        crate::backend::regalloc::AccumulatorOperandOrder::AccumulatorCentric,
                    return_consumes_accumulator: true,
                },
            )
            .into_iter()
            .map(|a| a.value_id)
            .collect::<crate::common::fx_hash::FxHashSet<_>>();
            let extra: Vec<u32> = skip
                .iter()
                .copied()
                .filter(|v| !cond_values.contains(v))
                .collect();
            if !extra.is_empty() {
                match never_materialized.as_mut() {
                    Some(set) => set.extend(extra),
                    None => {
                        never_materialized = Some(extra.into_iter().collect());
                    }
                }
            }
        }

        let (reg_assigned, cached_liveness, _caller_save_spans, accumulator_assignments) =
            crate::backend::stack_layout::run_regalloc_and_merge_clobbers_ex(
                func,
                available_regs,
                caller_saved_regs,
                &asm_clobbered_regs,
                &mut self.reg_assignments,
                &mut self.used_callee_saved,
                false,
                never_materialized,
                Vec::new(),
                Vec::new(),
                // i686 emits SIB indexed addressing directly at the Load/Store
                // (session 27) AND folds constant-offset GEPs with register
                // bases (const_offset_fold_reg_base_ok): BOTH forms consume
                // address registers RA-invisibly at the access position.
                // collect_folded_gep_links_all extends the base intervals of
                // const-offset folds plus base AND index intervals of indexed
                // folds to their consumers, so the address registers survive
                // intervening calls and accumulator staging.
                crate::backend::generation::collect_folded_gep_links_all(func),
            );

        // %ebx must be saved/restored only when it really holds the GOT base.
        if needs_got && !self.used_callee_saved.contains(&PhysReg(0)) {
            self.used_callee_saved.insert(0, PhysReg(0));
        }

        let callee_saved_bytes = self.used_callee_saved.len() as i64 * 4;

        // The bias ensures that slots requiring >= 16-byte alignment land on
        // 16-byte boundaries at runtime. The correct value depends on the
        // stack overhead between the 16-byte-aligned call-site ESP and the
        // reference point for slot addressing:
        //
        //   With frame pointer:   return addr (4) + saved ebp (4) = 8
        //     Address of slot -X = EBP - X = (16n - 8) - X, aligned when X ≡ 8 mod 16
        //
        //   Without frame pointer: return addr (4) only
        //     Address of slot = 16n - 4 - space, aligned when space ≡ 12 mod 16
        let omit_fp = self.omit_frame_pointer;
        let alignment_bias: i64 = if omit_fp { 12 } else { 8 };

        self.state.ra_accumulator_values =
            accumulator_assignments.iter().map(|a| a.value_id).collect();
        let space = calculate_stack_space_common(
            &mut self.state,
            func,
            callee_saved_bytes,
            |space, alloc_size, align| {
                let effective_align = if align > 0 { align.max(4) } else { 4 };
                let alloc = (alloc_size + 3) & !3;
                let required = space + alloc;
                let new_space = if effective_align >= 16 {
                    let bias = alignment_bias;
                    let a = effective_align;
                    let rem = ((required % a) + a) % a;
                    let needed = if rem <= bias {
                        bias - rem
                    } else {
                        a - rem + bias
                    };
                    required + needed
                } else {
                    ((required + effective_align - 1) / effective_align) * effective_align
                };
                (-new_space, new_space)
            },
            &reg_assigned,
            callee_saved_set,
            cached_liveness,
        );
        // Mul-acc chain plans resolve AFTER the RA and slot assignment:
        // fusibility depends on the final register homes and stack slots
        // (see compute_i686_mulacc_chains / resolve_mulacc_plans).
        self.resolve_mulacc_plans(func);
        space
    }

    // ---- aligned_frame_size ----

    pub(super) fn aligned_frame_size_impl(&self, raw_space: i64) -> i64 {
        let callee_saved_bytes = self.used_callee_saved.len() as i64 * 4;
        let raw_locals = raw_space - callee_saved_bytes;
        // With frame pointer: overhead = callee_saved + 8 (saved ebp + return addr)
        // Without frame pointer: overhead = callee_saved + 4 (return addr only)
        let fixed_overhead = if self.omit_frame_pointer {
            callee_saved_bytes + 4
        } else {
            callee_saved_bytes + 8
        };
        let needed = raw_locals + fixed_overhead;
        let b = self.stack_boundary.max(4);
        let aligned = (needed + (b - 1)) & !(b - 1);
        aligned - fixed_overhead
    }

    // ---- emit_prologue ----

    pub(super) fn emit_prologue_impl(&mut self, func: &IrFunction, frame_size: i64) {
        // Private codegen metadata consumed (and removed) by the i686 assembly
        // peephole.  Without the return type it must conservatively keep EDX
        // live at every ret for the i64 ABI, which preserves hundreds of dead
        // `mov ..., %edx` relays in ordinary 32-bit-returning functions.
        if matches!(func.return_type, IrType::I64 | IrType::U64) || is_i128_type(func.return_type) {
            self.state.emit("# lccc-i686-return-uses-edx");
        }
        if self.omit_frame_pointer {
            // No frame pointer setup; use ESP-relative addressing.
            // frame_base_offset and esp_adjust will be set after callee-saved pushes.
            // TODO: Emit ESP-relative CFI directives (.cfi_def_cfa_offset after each
            // push/sub) for proper unwinding when frame pointer is omitted. Currently
            // the default .cfi_startproc CFA (ESP+4) is used, which is only valid at
            // function entry. This is acceptable for now since -fomit-frame-pointer on
            // i686 is primarily used by the Linux kernel boot code, which disables
            // unwind tables via -fno-asynchronous-unwind-tables.
        } else {
            self.state.emit("    pushl %ebp");
            if self.state.emit_cfi {
                self.state.emit("    .cfi_def_cfa_offset 8");
                self.state.emit("    .cfi_offset %ebp, -8");
            }
            self.state.emit("    movl %esp, %ebp");
            if self.state.emit_cfi {
                self.state.emit("    .cfi_def_cfa_register %ebp");
            }
        }

        for &reg in self.used_callee_saved.iter() {
            let name = phys_reg_name(reg);
            emit!(self.state, "    pushl %{}", name);
        }

        if self.pic_got_live {
            debug_assert!(
                self.used_callee_saved.contains(&PhysReg(0)),
                "GOT-using function requires ebx in used_callee_saved"
            );
            self.state.emit("    call __x86.get_pc_thunk.bx");
            self.state.emit("    addl $_GLOBAL_OFFSET_TABLE_, %ebx");
            self.needs_pc_thunk_bx = true;
        }

        if frame_size > 0 {
            emit!(self.state, "    subl ${}, %esp", frame_size);
        }

        // Post-prologue %esp baseline (ebp-relative), valid in both frame
        // pointer modes: %esp at esp_adjust==0 equals %ebp minus this value.
        // DoBuiltinApply uses it to restore %esp after its untracked staging.
        self.esp_baseline_offset = self.used_callee_saved.len() as i64 * 4 + frame_size;

        if self.omit_frame_pointer {
            let callee_saved_bytes = self.used_callee_saved.len() as i64 * 4;
            self.frame_base_offset = callee_saved_bytes + frame_size;
            self.esp_adjust = 0;
        }
    }

    // ---- emit_epilogue ----

    pub(super) fn emit_epilogue_impl(&mut self, _frame_size: i64) {
        if self.omit_frame_pointer {
            let callee_saved_bytes = self.used_callee_saved.len() as i64 * 4;
            let total = self.frame_base_offset - callee_saved_bytes;
            if total > 0 {
                emit!(self.state, "    addl ${}, %esp", total);
            }
        } else {
            let callee_saved_bytes = self.used_callee_saved.len() as i64 * 4;
            if callee_saved_bytes > 0 {
                emit!(self.state, "    leal -{}(%ebp), %esp", callee_saved_bytes);
            } else {
                self.state.emit("    movl %ebp, %esp");
            }
        }

        for &reg in self.used_callee_saved.iter().rev() {
            let name = phys_reg_name(reg);
            emit!(self.state, "    popl %{}", name);
        }

        if !self.omit_frame_pointer {
            self.state.emit("    popl %ebp");
        }
    }

    // ---- emit_store_params ----

    pub(super) fn emit_store_params_impl(&mut self, func: &IrFunction) {
        let config = self.call_abi_config();
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

        let fastcall_reg_count = if self.is_fastcall {
            self.count_fastcall_reg_params(func)
        } else {
            0
        };
        self.fastcall_reg_param_count = fastcall_reg_count;

        if self.is_fastcall {
            let mut total_stack_bytes: usize = 0;
            for (i, _p) in func.params.iter().enumerate() {
                if i < fastcall_reg_count {
                    continue;
                }
                let ty = func.params[i].ty;
                let size = match ty {
                    IrType::I64 | IrType::U64 | IrType::F64 => 8,
                    IrType::F128 => 12,
                    _ if is_i128_type(ty) => 16,
                    _ => 4,
                };
                total_stack_bytes += size;
            }
            self.fastcall_stack_cleanup = total_stack_bytes;
        } else {
            self.fastcall_stack_cleanup = 0;
        }

        // Build a map of param_idx -> ParamRef dest Value for fast lookup.
        // Used to handle the case where param alloca was eliminated by mem2reg
        // but the register allocator assigned a callee-saved register.
        let mut paramref_dests: Vec<Option<Value>> = vec![None; func.params.len()];
        if self.is_fastcall || self.regparm > 0 {
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
        }

        let stack_base: i64 = 8;
        let mut fastcall_reg_idx = 0usize;

        // ── regparm callee-side capture ─────────────────────────────────────
        // Register parameters (%eax/%edx/%ecx) are caller-saved: the ONLY safe
        // place to capture them is the prologue, before any body instruction
        // can clobber them. Three cases per register param:
        //   1. Param alloca has a slot  -> handled by the per-class arms in
        //      the main loop below (store reg -> alloca slot).
        //   2. Alloca gone (mem2reg) but the ParamRef dest is register-
        //      allocated -> move the ABI register to the assigned register
        //      NOW and mark param_pre_stored.
        //   3. Alloca gone but the ParamRef dest has a spill slot -> store
        //      the ABI register to that slot NOW and mark param_pre_stored.
        // Params are processed in DESCENDING ABI-register order (ecx, edx,
        // eax): %ecx/%edx are themselves allocatable (caller-saved phase), so
        // a pre-store target could be a later param's still-unread source.
        // Saving higher-numbered sources first makes that impossible.
        if self.regparm > 0 && !self.is_fastcall {
            let regparm_srcs = ["%eax", "%edx", "%ecx"];
            // Phase 1: memory-target captures. Stores never clobber another
            // param's source register, so they are safe in any order. This
            // MUST cover register params with live allocas too: the main
            // loop's stack-param copies stage through %eax (and wide copies
            // through %edx), which would destroy unsaved register args.
            let mut reg_moves: Vec<(usize, &'static str, crate::backend::regalloc::PhysReg)> =
                Vec::new(); // (param_idx, src, dst)
            for (i, class) in param_classes.iter().enumerate() {
                let alloca_slot = find_param_alloca(func, i)
                    .and_then(|(dest, _)| self.state.get_slot(dest.0).map(|s| (dest, s)));
                match *class {
                    ParamClass::IntReg { reg_idx } => {
                        let src_full = regparm_srcs[reg_idx];
                        if let Some((_, slot)) = alloca_slot {
                            // Typed capture into the alloca slot. Sub-int
                            // types are extended in-register first (the
                            // extension clobbers only this param's own
                            // source register, which is dead afterwards).
                            let ty = func.params[i].ty;
                            let regparm_regs_byte = ["%al", "%dl", "%cl"];
                            let regparm_regs_word = ["%ax", "%dx", "%cx"];
                            let slot_ref = self.slot_ref(slot);
                            match ty {
                                IrType::I8 => {
                                    emit!(
                                        self.state,
                                        "    movsbl {}, {}",
                                        regparm_regs_byte[reg_idx],
                                        src_full
                                    );
                                    emit!(self.state, "    movl {}, {}", src_full, slot_ref);
                                }
                                IrType::U8 => {
                                    emit!(
                                        self.state,
                                        "    movzbl {}, {}",
                                        regparm_regs_byte[reg_idx],
                                        src_full
                                    );
                                    emit!(self.state, "    movl {}, {}", src_full, slot_ref);
                                }
                                IrType::I16 => {
                                    emit!(
                                        self.state,
                                        "    movswl {}, {}",
                                        regparm_regs_word[reg_idx],
                                        src_full
                                    );
                                    emit!(self.state, "    movl {}, {}", src_full, slot_ref);
                                }
                                IrType::U16 => {
                                    emit!(
                                        self.state,
                                        "    movzwl {}, {}",
                                        regparm_regs_word[reg_idx],
                                        src_full
                                    );
                                    emit!(self.state, "    movl {}, {}", src_full, slot_ref);
                                }
                                _ => {
                                    emit!(self.state, "    movl {}, {}", src_full, slot_ref);
                                }
                            }
                        } else if let Some(dest) = paramref_dests[i] {
                            if let Some(&phys) = self.reg_assignments.get(&dest.0) {
                                reg_moves.push((i, src_full, phys));
                            } else if let Some(slot) = self.state.get_slot(dest.0) {
                                let sr = self.slot_ref(slot);
                                emit!(self.state, "    movl {}, {}", src_full, sr);
                                self.state.param_pre_stored.insert(i);
                            }
                            // Neither register nor slot: value is dead.
                        }
                    }
                    ParamClass::I64RegPair { base_reg_idx } => {
                        // Wide values always live in 8-byte slots on i686.
                        let slot = alloca_slot
                            .map(|(_, s)| s)
                            .or_else(|| paramref_dests[i].and_then(|d| self.state.get_slot(d.0)));
                        if let Some(slot) = slot {
                            let sr0 = self.slot_ref(slot);
                            let sr4 = self.slot_ref_offset(slot, 4);
                            emit!(
                                self.state,
                                "    movl {}, {}",
                                regparm_srcs[base_reg_idx],
                                sr0
                            );
                            emit!(
                                self.state,
                                "    movl {}, {}",
                                regparm_srcs[base_reg_idx + 1],
                                sr4
                            );
                            if alloca_slot.is_none() {
                                self.state.param_pre_stored.insert(i);
                            }
                        }
                    }
                    ParamClass::StructByValReg { base_reg_idx, size } => {
                        let slot = alloca_slot
                            .map(|(_, s)| s)
                            .or_else(|| paramref_dests[i].and_then(|d| self.state.get_slot(d.0)));
                        if let Some(slot) = slot {
                            let words = size.div_ceil(4);
                            for k in 0..words {
                                let sr = self.slot_ref_offset(slot, (k * 4) as i64);
                                emit!(
                                    self.state,
                                    "    movl {}, {}",
                                    regparm_srcs[base_reg_idx + k],
                                    sr
                                );
                            }
                            if alloca_slot.is_none() {
                                self.state.param_pre_stored.insert(i);
                            }
                        }
                    }
                    _ => {}
                }
            }
            // Phase 2: register-target captures form a parallel move: a target
            // (%ecx/%edx — and since Phase 2e also %eax — are allocatable on
            // i686) can be another move's still-unread source. Standard
            // resolution: repeatedly emit a move whose target is not a pending
            // source; a stuck state is a swap cycle, broken with xchg. (In
            // practice a param never lands an %eax home — any real use spans
            // an eax hazard — so cycles stay within {%ecx, %edx}.)
            while !reg_moves.is_empty() {
                let pending_srcs: Vec<&str> = reg_moves.iter().map(|m| m.1).collect();
                if let Some(pos) = reg_moves.iter().position(|&(_, src, dst)| {
                    let dst_name = format!("%{}", phys_reg_name(dst));
                    dst_name == src || !pending_srcs.contains(&dst_name.as_str())
                }) {
                    let (i, src, dst) = reg_moves.remove(pos);
                    let dst_name = phys_reg_name(dst);
                    if format!("%{}", dst_name) != src {
                        emit!(self.state, "    movl {}, %{}", src, dst_name);
                    }
                    self.state.param_pre_stored.insert(i);
                } else {
                    // Pure swap cycle: only possible between %ecx and %edx.
                    self.state.emit("    xchgl %ecx, %edx");
                    for m in reg_moves.drain(..) {
                        // After the swap both values sit in their targets.
                        self.state.param_pre_stored.insert(m.0);
                    }
                }
            }
            self.state.reg_cache.invalidate_acc();
        }

        // Build a map from physical register -> list of param indices that use it,
        // so we can detect when two params share the same callee-saved register.
        let mut reg_to_params: crate::common::fx_hash::FxHashMap<u8, Vec<usize>> =
            crate::common::fx_hash::FxHashMap::default();
        if self.is_fastcall {
            for (i, _) in func.params.iter().enumerate() {
                if let Some(paramref_dest) = paramref_dests[i] {
                    if let Some(&phys_reg) = self.reg_assignments.get(&paramref_dest.0) {
                        reg_to_params.entry(phys_reg.0).or_default().push(i);
                    }
                }
            }
        }

        for (i, _param) in func.params.iter().enumerate() {
            let class = param_classes[i];

            // mem2reg commonly removes the frontend's parameter alloca.  A
            // regparm value still arrives in EAX/EDX/ECX and must be captured
            // at function entry, before ordinary code can clobber those
            // caller-saved registers.  i686's global allocator assigns only
            // callee-saved EBX/ESI/EDI/(EBP), so these moves cannot form an
            // incoming-register cycle; spilled destinations are equally safe.
            if let ParamClass::IntReg { reg_idx } = class {
                // The phase-1/2 capture pass above already saved this param
                // (param_pre_stored). Re-capturing HERE is a miscompile, not
                // just redundancy: this loop runs interleaved with the
                // stack-param copies, which stage through %eax — a second
                // `movl %eax, %dest` after param 0's copy writes the copied
                // STACK value over the already-captured register argument
                // (observed: d_i(double, int) read b as garbage at -O1+).
                if self.state.param_pre_stored.contains(&i) {
                    continue;
                }
                let has_alloca_slot = find_param_alloca(func, i)
                    .and_then(|(dest, _)| self.state.get_slot(dest.0))
                    .is_some();
                if !has_alloca_slot {
                    if let Some(paramref_dest) = paramref_dests[i] {
                        let full = ["%eax", "%edx", "%ecx"][reg_idx];
                        let byte = ["%al", "%dl", "%cl"][reg_idx];
                        let word = ["%ax", "%dx", "%cx"][reg_idx];
                        let ty = func.params[i].ty;
                        if let Some(&phys_reg) = self.reg_assignments.get(&paramref_dest.0) {
                            let dest_reg = phys_reg_name(phys_reg);
                            match ty {
                                IrType::I8 => {
                                    emit!(self.state, "    movsbl {}, %{}", byte, dest_reg)
                                }
                                IrType::U8 => {
                                    emit!(self.state, "    movzbl {}, %{}", byte, dest_reg)
                                }
                                IrType::I16 => {
                                    emit!(self.state, "    movswl {}, %{}", word, dest_reg)
                                }
                                IrType::U16 => {
                                    emit!(self.state, "    movzwl {}, %{}", word, dest_reg)
                                }
                                _ => emit!(self.state, "    movl {}, %{}", full, dest_reg),
                            }
                            self.state.param_pre_stored.insert(i);
                        } else if let Some(slot) = self.state.get_slot(paramref_dest.0) {
                            let slot_ref = self.slot_ref(slot);
                            match ty {
                                IrType::I8 => emit!(self.state, "    movsbl {}, {}", byte, full),
                                IrType::U8 => emit!(self.state, "    movzbl {}, {}", byte, full),
                                IrType::I16 => emit!(self.state, "    movswl {}, {}", word, full),
                                IrType::U16 => emit!(self.state, "    movzwl {}, {}", word, full),
                                _ => {}
                            }
                            emit!(self.state, "    movl {}, {}", full, slot_ref);
                            self.state.param_pre_stored.insert(i);
                        }
                    }
                    continue;
                }
            }

            // Pre-store optimization for fastcall register params: when the param's
            // alloca was eliminated (by mem2reg) but the ParamRef dest is register-
            // allocated to a callee-saved register, store the fastcall ABI register
            // (%ecx/%edx) directly to the assigned physical register. This is critical
            // because:
            // 1. Dead alloca means no stack slot exists for this param
            // 2. %ecx/%edx are caller-saved and will be clobbered
            // 3. We must save the value NOW, before any other code runs
            // 4. emit_param_ref will see param_pre_stored and skip code generation
            if self.is_fastcall && fastcall_reg_idx < fastcall_reg_count {
                let param_ty = func.params[i].ty;
                if self.is_fastcall_reg_eligible(param_ty) {
                    let has_alloca_slot = find_param_alloca(func, i)
                        .and_then(|(dest, _)| self.state.get_slot(dest.0))
                        .is_some();
                    if !has_alloca_slot {
                        let src_reg = if fastcall_reg_idx == 0 {
                            "%ecx"
                        } else {
                            "%edx"
                        };
                        if let Some(paramref_dest) = paramref_dests[i] {
                            if let Some(&phys_reg) = self.reg_assignments.get(&paramref_dest.0) {
                                // Safety check: if another param's dest is also assigned
                                // to this register, skip pre-store to avoid conflicts.
                                let shared = reg_to_params
                                    .get(&phys_reg.0)
                                    .is_some_and(|users| users.len() > 1);
                                if !shared {
                                    // Store directly to the callee-saved register
                                    let dest_reg = phys_reg_name(phys_reg);
                                    emit!(self.state, "    movl {}, %{}", src_reg, dest_reg);
                                    self.state.param_pre_stored.insert(i);
                                }
                            } else if let Some(slot) = self.state.get_slot(paramref_dest.0) {
                                // Value was spilled to a stack slot - no register conflict
                                let slot_ref = self.slot_ref(slot);
                                emit!(self.state, "    movl {}, {}", src_reg, slot_ref);
                                self.state.param_pre_stored.insert(i);
                            }
                        }
                        fastcall_reg_idx += 1;
                        continue;
                    }
                }
            }

            let (slot, ty, dest_id) = if let Some((dest, ty)) = find_param_alloca(func, i) {
                if let Some(slot) = self.state.get_slot(dest.0) {
                    (slot, ty, dest.0)
                } else {
                    if self.is_fastcall
                        && fastcall_reg_idx < fastcall_reg_count
                        && i < func.params.len()
                        && self.is_fastcall_reg_eligible(ty)
                    {
                        fastcall_reg_idx += 1;
                    }
                    continue;
                }
            } else {
                if self.is_fastcall
                    && fastcall_reg_idx < fastcall_reg_count
                    && i < func.params.len()
                {
                    let param_ty = func.params[i].ty;
                    if self.is_fastcall_reg_eligible(param_ty) {
                        fastcall_reg_idx += 1;
                    }
                }
                continue;
            };

            if self.is_fastcall
                && fastcall_reg_idx < fastcall_reg_count
                && self.is_fastcall_reg_eligible(ty)
            {
                let src_reg_full = if fastcall_reg_idx == 0 {
                    "%ecx"
                } else {
                    "%edx"
                };
                let slot_ref = self.slot_ref(slot);
                // For sub-int types, sign/zero-extend to full 32-bit before
                // storing to the 4-byte SSA slot (avoids partial-write issues).
                match ty {
                    IrType::I8 => {
                        let src_byte = if fastcall_reg_idx == 0 { "%cl" } else { "%dl" };
                        emit!(self.state, "    movsbl {}, {}", src_byte, src_reg_full);
                        emit!(self.state, "    movl {}, {}", src_reg_full, slot_ref);
                    }
                    IrType::U8 => {
                        let src_byte = if fastcall_reg_idx == 0 { "%cl" } else { "%dl" };
                        emit!(self.state, "    movzbl {}, {}", src_byte, src_reg_full);
                        emit!(self.state, "    movl {}, {}", src_reg_full, slot_ref);
                    }
                    IrType::I16 => {
                        let src_word = if fastcall_reg_idx == 0 { "%cx" } else { "%dx" };
                        emit!(self.state, "    movswl {}, {}", src_word, src_reg_full);
                        emit!(self.state, "    movl {}, {}", src_reg_full, slot_ref);
                    }
                    IrType::U16 => {
                        let src_word = if fastcall_reg_idx == 0 { "%cx" } else { "%dx" };
                        emit!(self.state, "    movzwl {}, {}", src_word, src_reg_full);
                        emit!(self.state, "    movl {}, {}", src_reg_full, slot_ref);
                    }
                    _ => {
                        emit!(self.state, "    movl {}, {}", src_reg_full, slot_ref);
                    }
                }
                fastcall_reg_idx += 1;
                continue;
            }

            let stack_offset_adjust = if self.is_fastcall {
                fastcall_reg_count as i64 * 4
            } else {
                0
            };

            match class {
                ParamClass::StackScalar { offset } => {
                    let src_offset = stack_base + offset - stack_offset_adjust;
                    // D64: BID bit container — integer pair copy, no x87.
                    if ty == IrType::F64
                        || ty == IrType::I64
                        || ty == IrType::U64
                        || ty == IrType::D64
                    {
                        let src_ref = self.param_ref(src_offset);
                        let dst_ref = self.slot_ref(slot);
                        emit!(self.state, "    movl {}, %eax", src_ref);
                        emit!(self.state, "    movl %eax, {}", dst_ref);
                        let src_ref_hi = self.param_ref(src_offset + 4);
                        let dst_ref_hi = self.slot_ref_offset(slot, 4);
                        emit!(self.state, "    movl {}, %eax", src_ref_hi);
                        emit!(self.state, "    movl %eax, {}", dst_ref_hi);
                    } else {
                        let load_instr = self.mov_load_for_type(ty);
                        let src_ref = self.param_ref(src_offset);
                        let dst_ref = self.slot_ref(slot);
                        emit!(self.state, "    {} {}, %eax", load_instr, src_ref);
                        // Always store full 32-bit value to SSA slot. The load
                        // instruction above already sign/zero-extended sub-int
                        // types into the full eax register. Using movb/movw here
                        // would leave garbage in the upper bytes of the 4-byte
                        // slot, which gets read back later by movl.
                        emit!(self.state, "    movl %eax, {}", dst_ref);
                    }
                }
                ParamClass::StructStack { offset, size }
                | ParamClass::LargeStructStack { offset, size } => {
                    let src = stack_base + offset - stack_offset_adjust;
                    // Over-aligned (>16) parameter allocas: the slot is
                    // oversized by (align-1) and the EFFECTIVE address is
                    // align_up(slot, align) — every alloca-address user
                    // (emit_alloca_addr_to, memcpy paths) resolves that same
                    // aligned address, so the incoming-parameter copy must
                    // target it too. Writing the raw slot desyncs the two by
                    // the alignment pad (mirrors the x86-64 prologue's
                    // _Alignas(32) fix; i686 repro: over_aligned_struct_arg).
                    //
                    // %ecx holds the aligned destination for the whole copy.
                    // It is caller-saved and, in a plain cdecl prologue (no
                    // regparm, no fastcall), provably dead here: register
                    // parameters are captured through the same loop's
                    // earlier arms and the function body has not started.
                    // With regparm/fastcall, %ecx may still carry an
                    // uncaptured register argument (the capture arms run in
                    // parameter order), so it is saved/restored around the
                    // copy — esp_adjust is kept in sync so that esp-relative
                    // slot/param references below the push stay exact.
                    let over_align = self.state.alloca_over_align(dest_id);
                    let ecx_free = self.regparm == 0 && !self.is_fastcall;
                    if over_align.is_some() {
                        if ecx_free {
                            self.emit_alloca_addr_to("ecx", dest_id, slot);
                        } else {
                            self.state.emit("    pushl %ecx");
                            self.esp_adjust += 4;
                            self.emit_alloca_addr_to("ecx", dest_id, slot);
                        }
                        let mut copied = 0usize;
                        while copied + 4 <= size {
                            let src_ref = self.param_ref(src + copied as i64);
                            emit!(self.state, "    movl {}, %eax", src_ref);
                            emit!(self.state, "    movl %eax, {}(%ecx)", copied);
                            copied += 4;
                        }
                        while copied < size {
                            let src_ref = self.param_ref(src + copied as i64);
                            emit!(self.state, "    movb {}, %al", src_ref);
                            emit!(self.state, "    movb %al, {}(%ecx)", copied);
                            copied += 1;
                        }
                        if !ecx_free {
                            self.state.emit("    popl %ecx");
                            self.esp_adjust -= 4;
                        }
                    } else {
                        let mut copied = 0usize;
                        while copied + 4 <= size {
                            let src_ref = self.param_ref(src + copied as i64);
                            let dst_ref = self.slot_ref_offset(slot, copied as i64);
                            emit!(self.state, "    movl {}, %eax", src_ref);
                            emit!(self.state, "    movl %eax, {}", dst_ref);
                            copied += 4;
                        }
                        while copied < size {
                            let src_ref = self.param_ref(src + copied as i64);
                            let dst_ref = self.slot_ref_offset(slot, copied as i64);
                            emit!(self.state, "    movb {}, %al", src_ref);
                            emit!(self.state, "    movb %al, {}", dst_ref);
                            copied += 1;
                        }
                    }
                }
                ParamClass::F128AlwaysStack { offset } => {
                    let src = stack_base + offset - stack_offset_adjust;
                    let src_ref = self.param_ref(src);
                    let dst_ref = self.slot_ref(slot);
                    emit!(self.state, "    fldt {}", src_ref);
                    emit!(self.state, "    fstpt {}", dst_ref);
                    self.state.f128_direct_slots.insert(dest_id);
                }
                ParamClass::I128Stack { offset } => {
                    let src = stack_base + offset - stack_offset_adjust;
                    for j in (0..16).step_by(4) {
                        let src_ref = self.param_ref(src + j as i64);
                        let dst_ref = self.slot_ref_offset(slot, j as i64);
                        emit!(self.state, "    movl {}, %eax", src_ref);
                        emit!(self.state, "    movl %eax, {}", dst_ref);
                    }
                }
                ParamClass::F128Stack { offset } => {
                    let src = stack_base + offset - stack_offset_adjust;
                    let src_ref = self.param_ref(src);
                    let dst_ref = self.slot_ref(slot);
                    emit!(self.state, "    fldt {}", src_ref);
                    emit!(self.state, "    fstpt {}", dst_ref);
                    self.state.f128_direct_slots.insert(dest_id);
                }
                ParamClass::IntReg { .. }
                | ParamClass::I64RegPair { .. }
                | ParamClass::StructByValReg { .. } => {
                    // Captured by the regparm phase-1 pass above (which runs
                    // BEFORE any stack-param copy can clobber %eax/%edx/%ecx).
                }
                _ => {
                    // Remaining register classes (FloatReg, SSE structs, etc.)
                    // don't apply to i686's ABI classification.
                }
            }
        }
    }

    // ---- emit_param_ref ----

    pub(super) fn emit_param_ref_impl(&mut self, dest: &Value, param_idx: usize, ty: IrType) {
        use crate::backend::call_abi::ParamClass;

        // If this param was pre-stored in the prologue (fastcall register param
        // with eliminated alloca), the value is already in the correct physical
        // register or stack slot. No code generation needed.
        if self.state.param_pre_stored.contains(&param_idx) {
            return;
        }

        if param_idx < self.state.param_alloca_slots.len() {
            if let Some((alloca_slot, _alloca_ty)) = self.state.param_alloca_slots[param_idx] {
                if let Some(dest_slot) = self.state.get_slot(dest.0) {
                    if dest_slot.0 == alloca_slot.0 {
                        // The param value is already in the alloca slot (stored by
                        // emit_store_params). If dest also has a register assignment,
                        // we must initialize the register from the slot — otherwise
                        // the register contains garbage from the caller, and any
                        // subsequent read via operand_to_eax will use the register
                        // (uninitialized) instead of the slot.
                        if let Some(phys) = self.dest_reg(dest) {
                            let reg = phys_reg_name(phys);
                            let load_instr = self.mov_load_for_type(ty);
                            let src_ref = self.slot_ref(alloca_slot);
                            emit!(self.state, "    {} {}, %eax", load_instr, src_ref);
                            emit!(self.state, "    movl %eax, %{}", reg);
                            self.state.reg_cache.invalidate_acc();
                        }
                        return;
                    }
                }
                if self.state.get_slot(dest.0).is_none() {
                    // Dest is register-allocated (or dead) with no spill slot.
                    // The param value lives in the alloca slot — emit_store_params
                    // put it there regardless of whether it arrived in a register
                    // (regparm) or on the stack. Load it from there; falling
                    // through to the stack-offset fallback is wrong for register
                    // params (they have no stack home).
                    if let Some(phys) = self.dest_reg(dest) {
                        let reg = phys_reg_name(phys);
                        let load_instr = self.mov_load_for_type(ty);
                        let src_ref = self.slot_ref(alloca_slot);
                        emit!(self.state, "    {} {}, %eax", load_instr, src_ref);
                        emit!(self.state, "    movl %eax, %{}", reg);
                        self.state.reg_cache.invalidate_acc();
                    }
                    return;
                }
                if let Some(dest_slot) = self.state.get_slot(dest.0) {
                    if is_i128_type(ty) {
                        for i in (0..16).step_by(4) {
                            let src_ref = self.slot_ref_offset(alloca_slot, i as i64);
                            let dst_ref = self.slot_ref_offset(dest_slot, i as i64);
                            emit!(self.state, "    movl {}, %eax", src_ref);
                            emit!(self.state, "    movl %eax, {}", dst_ref);
                        }
                    } else if ty == IrType::F128 {
                        let src_ref = self.slot_ref(alloca_slot);
                        let dst_ref = self.slot_ref(dest_slot);
                        emit!(self.state, "    fldt {}", src_ref);
                        emit!(self.state, "    fstpt {}", dst_ref);
                        self.state.f128_direct_slots.insert(dest.0);
                    } else if ty == IrType::F64 || ty == IrType::I64 || ty == IrType::U64 {
                        let src_ref = self.slot_ref(alloca_slot);
                        let dst_ref = self.slot_ref(dest_slot);
                        emit!(self.state, "    movl {}, %eax", src_ref);
                        emit!(self.state, "    movl %eax, {}", dst_ref);
                        let src_ref_hi = self.slot_ref_offset(alloca_slot, 4);
                        let dst_ref_hi = self.slot_ref_offset(dest_slot, 4);
                        emit!(self.state, "    movl {}, %eax", src_ref_hi);
                        emit!(self.state, "    movl %eax, {}", dst_ref_hi);
                    } else {
                        let load_instr = self.mov_load_for_type(ty);
                        let src_ref = self.slot_ref(alloca_slot);
                        emit!(self.state, "    {} {}, %eax", load_instr, src_ref);
                        self.store_eax_to(dest);
                    }
                    return;
                }
            }
        }

        if self.is_fastcall && param_idx < self.fastcall_reg_param_count {
            if let Some(Some((slot, _slot_ty))) = self.state.param_alloca_slots.get(param_idx) {
                let load_instr = self.mov_load_for_type(ty);
                let slot_ref = self.slot_ref(*slot);
                emit!(self.state, "    {} {}, %eax", load_instr, slot_ref);
                self.store_eax_to(dest);
            }
            return;
        }

        let stack_base: i64 = 8;
        let stack_offset_adjust = if self.is_fastcall {
            self.fastcall_reg_param_count as i64 * 4
        } else {
            0
        };
        let param_offset = if param_idx < self.state.param_classes.len() {
            match self.state.param_classes[param_idx] {
                ParamClass::StackScalar { offset }
                | ParamClass::StructStack { offset, .. }
                | ParamClass::LargeStructStack { offset, .. }
                | ParamClass::F128AlwaysStack { offset }
                | ParamClass::I128Stack { offset }
                | ParamClass::F128Stack { offset }
                | ParamClass::LargeStructByRefStack { offset, .. } => {
                    stack_base + offset - stack_offset_adjust
                }
                ParamClass::IntReg { .. }
                | ParamClass::I64RegPair { .. }
                | ParamClass::StructByValReg { .. } => {
                    // Register param whose alloca was eliminated: the value was
                    // captured in the prologue (param_pre_stored) or is dead.
                    // There IS no stack home to read; falling through to a
                    // stack load would read the caller's frame garbage.
                    debug_assert!(
                        false,
                        "emit_param_ref: register param {} not pre-stored",
                        param_idx
                    );
                    return;
                }
                _ => stack_base + (param_idx as i64) * 4,
            }
        } else {
            stack_base + (param_idx as i64) * 4
        };

        if is_i128_type(ty) {
            if let Some(slot) = self.state.get_slot(dest.0) {
                for i in (0..16).step_by(4) {
                    let src_ref = self.param_ref(param_offset + i as i64);
                    let dst_ref = self.slot_ref_offset(slot, i as i64);
                    emit!(self.state, "    movl {}, %eax", src_ref);
                    emit!(self.state, "    movl %eax, {}", dst_ref);
                }
            }
        } else if ty == IrType::F128 {
            if let Some(dest_slot) = self.state.get_slot(dest.0) {
                let src_ref = self.param_ref(param_offset);
                let dst_ref = self.slot_ref(dest_slot);
                emit!(self.state, "    fldt {}", src_ref);
                emit!(self.state, "    fstpt {}", dst_ref);
                self.state.f128_direct_slots.insert(dest.0);
            }
        } else if ty == IrType::F64 || ty == IrType::I64 || ty == IrType::U64 {
            if let Some(slot) = self.state.get_slot(dest.0) {
                let src_ref = self.param_ref(param_offset);
                let dst_ref = self.slot_ref(slot);
                emit!(self.state, "    movl {}, %eax", src_ref);
                emit!(self.state, "    movl %eax, {}", dst_ref);
                let src_ref_hi = self.param_ref(param_offset + 4);
                let dst_ref_hi = self.slot_ref_offset(slot, 4);
                emit!(self.state, "    movl {}, %eax", src_ref_hi);
                emit!(self.state, "    movl %eax, {}", dst_ref_hi);
            }
        } else {
            let load_instr = self.mov_load_for_type(ty);
            let src_ref = self.param_ref(param_offset);
            emit!(self.state, "    {} {}, %eax", load_instr, src_ref);
            self.store_eax_to(dest);
        }
    }

    // ---- emit_epilogue_and_ret ----

    pub(super) fn emit_epilogue_and_ret_impl(&mut self, frame_size: i64) {
        self.emit_epilogue(frame_size);
        if self.state.uses_sret && self.regparm == 0 {
            // i386 SysV sret: callee pops the hidden pointer.
            // Under -mregparm>=1 the hidden pointer travels in %eax (GCC
            // function_value semantics) — nothing is on the stack to pop.
            self.state.emit("    ret $4");
        } else if self.is_fastcall && self.fastcall_stack_cleanup > 0 {
            emit!(self.state, "    ret ${}", self.fastcall_stack_cleanup);
        } else {
            self.state.emit("    ret");
        }
    }

    // ---- store/load instr for type ----

    pub(super) fn store_instr_for_type_impl(&self, ty: IrType) -> &'static str {
        self.mov_store_for_type(ty)
    }

    pub(super) fn load_instr_for_type_impl(&self, ty: IrType) -> &'static str {
        self.mov_load_for_type(ty)
    }
}
