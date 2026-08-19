//! X86Codegen: prologue, epilogue, parameter storage.

use crate::ir::reexports::{IntrinsicOp, IrBinOp, IrCmpOp, IrFunction, Instruction, Operand, Terminator, Value};
use crate::common::types::{AddressSpace, IrType};
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::backend::call_abi::{ParamClass, classify_params};
use crate::backend::generation::{calculate_stack_space_common, find_param_alloca};
use crate::backend::liveness::{for_each_operand_in_instruction, for_each_operand_in_terminator};
use crate::backend::regalloc::PhysReg;
use super::emit::{X86Codegen, X86_CALLEE_SAVED, X86_CALLEE_SAVED_WITH_RBP, X86_CALLER_SAVED,
                     phys_reg_name, collect_inline_asm_callee_saved_x86, X86_ARG_REGS, is_xmm_reg};

impl X86Codegen {
    pub(super) fn calculate_stack_space_impl(&mut self, func: &IrFunction) -> i64 {
        // ms178 debug: dump IR per function
        if std::env::var("CCC_DUMP_IR").is_ok() {
            eprintln!("===== IR for {} =====", func.name);
            eprintln!("{:#?}", func);
            eprintln!("===== end IR =====");
        }
        // Store function pointer for indexed addressing detection
        self.current_func = Some(func as *const IrFunction);

        // Analyze IVSR patterns for Phase 9b indexed addressing optimization
        self.analyze_ivsr_pointers(func);

        // Track variadic function info
        self.is_variadic = func.is_variadic;
        // Count named params using the shared ABI classification, so this
        // stays in sync with classify_call_args (caller side) automatically.
        {
            let config = self.call_abi_config_impl();
            let classification = crate::backend::call_abi::classify_params_full(func, &config);
            let mut named_gp = 0usize;
            let mut named_fp = 0usize;
            for class in &classification.classes {
                named_gp += class.gp_reg_count();
                if matches!(class, crate::backend::call_abi::ParamClass::FloatReg { .. }) {
                    named_fp += 1;
                }
            }
            self.num_named_int_params = named_gp;
            self.num_named_fp_params = named_fp;
            self.num_named_stack_bytes =
                crate::backend::call_abi::named_params_stack_bytes(&classification.classes);
        }

        // Run register allocator BEFORE stack space computation so we can
        // skip allocating stack slots for values assigned to registers.
        let mut asm_clobbered_regs: Vec<PhysReg> = Vec::new();
        collect_inline_asm_callee_saved_x86(func, &mut asm_clobbered_regs);
        let callee_base: &[PhysReg] = if self.state.omit_frame_pointer {
            &X86_CALLEE_SAVED_WITH_RBP
        } else {
            &X86_CALLEE_SAVED
        };
        let mut available_regs = crate::backend::generation::filter_available_regs(callee_base, &asm_clobbered_regs);

        let mut caller_saved_regs = X86_CALLER_SAVED.to_vec();
        let mut has_indirect_call = false;
        let mut has_calls = false;
        let mut has_i128_ops = false;
        let mut has_atomic_rmw = false;
        // Track rdx-clobbering patterns for conditional rdx allocation
        let mut has_div_rem = false;
        let mut has_gep = false;       // GEP → indirect stores → emit_save_acc uses rdx
        let mut has_switch = false;    // Switch → jump tables use rdx
        let mut has_select = false;    // Select → cmov path uses rdx
        let mut has_rdx_intrinsic = false; // Fixed-scratch intrinsic paths overwrite rdx
        let mut has_i32_widening = false; // Cast from I32/U32 to I64/pointer → needs sign-ext
        for block in &func.blocks {
            for inst in &block.instructions {
                match inst {
                    Instruction::Call { .. } => { has_calls = true; }
                    Instruction::CallIndirect { .. } => {
                        has_calls = true;
                        has_indirect_call = true;
                    }
                    Instruction::BinOp { op, ty, .. } => {
                        if matches!(ty, IrType::I128 | IrType::U128) {
                            has_i128_ops = true;
                        }
                        if matches!(op, IrBinOp::SDiv | IrBinOp::UDiv | IrBinOp::SRem | IrBinOp::URem) {
                            has_div_rem = true;
                        }
                    }
                    Instruction::UnaryOp { ty, .. } => {
                        if matches!(ty, IrType::I128 | IrType::U128) {
                            has_i128_ops = true;
                        }
                    }
                    Instruction::Cast { from_ty, to_ty, .. } => {
                        if matches!(from_ty, IrType::I128 | IrType::U128)
                            || matches!(to_ty, IrType::I128 | IrType::U128) {
                            has_i128_ops = true;
                        }
                        // Detect I32/U32 widening to 64-bit: requires sign-extension.
                        if matches!(from_ty, IrType::I32 | IrType::U32)
                            && matches!(to_ty, IrType::I64 | IrType::U64 | IrType::Ptr) {
                            has_i32_widening = true;
                        }
                    }
                    Instruction::Cmp { ty, .. }
                    | Instruction::Store { ty, .. } => {
                        if matches!(ty, IrType::I128 | IrType::U128) {
                            has_i128_ops = true;
                        }
                    }
                    Instruction::AtomicRmw { .. } => { has_atomic_rmw = true; }
                    Instruction::GetElementPtr { .. } => { has_gep = true; }
                    Instruction::Select { .. } => { has_select = true; }
                    Instruction::Intrinsic { op, .. } => {
                        // These x86 emitters use rdx as an unmodeled fixed
                        // scratch (or architectural output for rdtsc).  Keep
                        // allocator-owned values out of rdx across them.  A
                        // late vectorizer SDiv used to hide this bug by
                        // disabling rdx allocation for the whole function.
                        has_rdx_intrinsic |= matches!(
                            op,
                            IntrinsicOp::Rdtsc
                                | IntrinsicOp::Rdtscp
                                | IntrinsicOp::F128Copysign
                                | IntrinsicOp::FmaF64x2
                                | IntrinsicOp::FmaF64x4
                                | IntrinsicOp::FmaF64x4Hoisted
                                | IntrinsicOp::FmaF64x4SIB
                                | IntrinsicOp::LoadF64x4
                                | IntrinsicOp::LoadF64x2
                                | IntrinsicOp::LoadI32x8
                                | IntrinsicOp::LoadI32x4
                                | IntrinsicOp::VecZeroI32x8
                                | IntrinsicOp::VecZeroI32x4
                        );
                    }
                    _ => {}
                }
            }
            if matches!(block.terminator, Terminator::Switch { .. }) {
                has_switch = true;
            }
        }
        self.func_has_calls = has_calls;
        // r10: use call *%rax instead of call *%r10 (frees r10 for non-call-spanning).
        // Exception: indirect branch thunks still use r10 (rare).
        if has_i128_ops {
            caller_saved_regs.retain(|r| r.0 != 12 && r.0 != 13 && r.0 != 14 && r.0 != 15); // r8, r9, rdi, rsi
        }
        // r8: atomic RMW uses rdi instead of r8 (frees r8 unless i128 excludes it).
        // rdx (PhysReg 16) is available as caller-saved when no instruction
        // implicitly clobbers it: division (rdx:rax), i128 ops (rax:rdx pair),
        // switch jump tables (rdx as dispatch), or fixed-scratch intrinsics.
        // GEP indirect stores and Select cmov paths use %r11 when rdx is
        // allocated.
        if !has_div_rem && !has_i128_ops && !has_switch && !has_rdx_intrinsic {
            caller_saved_regs.push(PhysReg(16)); // rdx
        }

        // For a one-block leaf, ParamRefs are eligible for caller-saved homes.
        // Prefer their incoming ABI registers in argument order so the common
        // case needs no entry copy at all (rdi->rdi, rsi->rsi, rdx->rdx).
        // RCX remains reserved scratch; remaining homes follow ABI order and
        // the ordered parallel-copy emitter handles overlaps such as rcx->r8
        // while a later argument still arrives in r8.
        if func.blocks.len() == 1 && std::env::var("CCC_NO_LEAF_PARAM_GPR").is_err() {
            let preferred = [14u8, 15, 16, 12, 13, 10, 11];
            caller_saved_regs.sort_by_key(|reg| {
                preferred.iter().position(|&id| id == reg.0).unwrap_or(preferred.len())
            });
        }

        // Note: promoting caller-saved registers to callee-saved does NOT work
        // because the x86-64 SysV ABI defines the callee-saved set. When we
        // promote r11 to callee-saved in function A and A calls function B,
        // B (following the ABI) freely clobbers r11. A's "callee-saved" r11
        // value is destroyed. The fix requires per-call save/restore, not
        // promotion. See binary size investigation in project memory.

        // Compute the exact set of I32 values that are consumed in a 64-bit
        // context and therefore need `movslq` after a 32-bit ALU op.
        //
        // Pass 1 (direct): mark every value that appears as an operand of a
        // 64-bit-consuming position — 64-bit BinOp/Cmp/Store, Cast I32->I64,
        // GEP offset, 64-bit Select, 64-bit call arg, 64-bit atomic op. This
        // supersedes the original unsound Cast+GEP-only set.
        //
        // Pass 2 (transitive closure through Copy and Phi): if `w` needs sext
        // and `w = Copy(v)` or `w = Phi(..., v, ...)`, then `v`'s 64-bit value
        // flows into `w`'s register, so `v` needs sext too. Without this, a
        // 32-bit ALU result `v` that is only copied to a `w` used in a 64-bit
        // context would be wrongly skipped: the `movq %v, %w` copies the stale
        // upper bits and the 64-bit use of `w` (when register-allocated) would
        // read garbage. (Sound: adding values to this set only ever emits more
        // movslq, never removes one, so it cannot miscompile.)
        let is_64 = |t: &IrType| matches!(t,
            IrType::I64 | IrType::U64 | IrType::Ptr | IrType::F128);
        let mut needs_sext_set: FxHashSet<u32> = FxHashSet::default();
        // copy_phi_srcs[dest] = list of value sources (from Copy/Phi) whose
        // 64-bit value flows into `dest`. Built in pass 1, used in pass 2.
        let mut copy_phi_srcs: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
        let mut mark = |op: &Operand, set: &mut FxHashSet<u32>| {
            if let Operand::Value(v) = op {
                set.insert(v.0);
            }
        };
        for block in &func.blocks {
            for inst in &block.instructions {
                match inst {
                    Instruction::BinOp { ty, lhs, rhs, .. } if is_64(ty) => {
                        mark(lhs, &mut needs_sext_set); mark(rhs, &mut needs_sext_set);
                    }
                    Instruction::Cmp { ty, lhs, rhs, .. } if is_64(ty) => {
                        mark(lhs, &mut needs_sext_set); mark(rhs, &mut needs_sext_set);
                    }
                    Instruction::Store { ty, val, .. } if is_64(ty) => {
                        mark(val, &mut needs_sext_set);
                    }
                    Instruction::Cast { src, from_ty, to_ty, .. }
                        if matches!(from_ty, IrType::I32 | IrType::U32) && is_64(to_ty) => {
                        mark(src, &mut needs_sext_set);
                    }
                    Instruction::GetElementPtr { offset, .. } => {
                        mark(offset, &mut needs_sext_set);
                    }
                    Instruction::Select { ty, true_val, false_val, .. } if is_64(ty) => {
                        mark(true_val, &mut needs_sext_set); mark(false_val, &mut needs_sext_set);
                    }
                    Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
                        for (arg, at) in info.args.iter().zip(info.arg_types.iter()) {
                            if is_64(at) {
                                mark(arg, &mut needs_sext_set);
                            }
                        }
                    }
                    Instruction::AtomicRmw { ty, val, .. } if is_64(ty) => {
                        mark(val, &mut needs_sext_set);
                    }
                    Instruction::AtomicStore { ty, val, .. } if is_64(ty) => {
                        mark(val, &mut needs_sext_set);
                    }
                    // Record Copy/Phi value-flow edges for the transitive closure.
                    Instruction::Copy { dest, src } => {
                        if let Operand::Value(v) = src {
                            copy_phi_srcs.entry(dest.0).or_default().push(v.0);
                        }
                    }
                    Instruction::Phi { dest, incoming, .. } => {
                        let e = copy_phi_srcs.entry(dest.0).or_default();
                        for (op, _) in incoming {
                            if let Operand::Value(v) = op {
                                e.push(v.0);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        // Pass 1b: 64-bit consumers that live in block terminators and
        // instructions the pass-1 match missed. All conservative (only ever
        // ADDS values to the sext set, which only emits more movslq):
        //   * Return operands of 64-bit-returning functions. The lowering's
        //     emit_implicit_cast skips Ptr targets entirely (no Cast is
        //     inserted for `return (char*)x;`), so a raw I32 value can flow
        //     straight into a 64-bit return — it must be sign-extended.
        //   * AtomicCmpxchg expected/desired operands of 64-bit width.
        for block in &func.blocks {
            if let Terminator::Return(Some(op)) = &block.terminator {
                if is_64(&func.return_type) {
                    mark(op, &mut needs_sext_set);
                }
            }
            for inst in &block.instructions {
                if let Instruction::AtomicCmpxchg { expected, desired, ty, .. } = inst {
                    if is_64(ty) {
                        mark(expected, &mut needs_sext_set);
                        mark(desired, &mut needs_sext_set);
                    }
                }
            }
        }

        // Pass 2: fixed-point transitive closure over Copy/Phi sources.
        if !copy_phi_srcs.is_empty() {
            let mut worklist: Vec<u32> = needs_sext_set.iter().copied().collect();
            while let Some(vid) = worklist.pop() {
                if let Some(srcs) = copy_phi_srcs.get(&vid) {
                    for &s in srcs {
                        if needs_sext_set.insert(s) {
                            worklist.push(s);
                        }
                    }
                }
            }
        }
        self.skip_i32_sext = needs_sext_set.is_empty();
        self.needs_sext_values = needs_sext_set;

        // ── Value type map, use counts and Cmp→consumer flag fusion ──────
        //
        // The codegen emits IR instructions strictly in order. When a Cmp's
        // result is consumed ONLY by the immediately-following Select or
        // CondBranch (same block, no instruction in between), the boolean
        // materialization (setcc/movzbl/store) is redundant: the flags set by
        // the cmp/test are still live. We record such Cmp destinations in
        // `fused_cmp_dests`; the Cmp emitter then skips setcc and the
        // consumer emits `jcc`/`cmovcc` directly. This is the single biggest
        // instruction-count win in branch-heavy code (gzip's longest_match).
        //
        // Soundness: the Cmp handler only skips setcc when the map proves the
        // consumer is next, and the dispatch loop emits that pair adjacently.
        // MachInst explicitly declines fused Cmp and Select candidates so it
        // cannot bypass this pending-flags handshake.
        {
            let mut use_counts: FxHashMap<u32, u32> = FxHashMap::default();
            let mut value_types: FxHashMap<u32, IrType> = FxHashMap::default();
            for block in &func.blocks {
                for inst in &block.instructions {
                    // Map every producing instruction to its result type.
                    if let Some(ty) = inst.result_type() {
                        if let Some(dest) = inst.dest() {
                            value_types.entry(dest.0).or_insert(ty);
                        }
                    }
                    for_each_operand_in_instruction(inst, |op| {
                        if let Operand::Value(v) = op {
                            *use_counts.entry(v.0).or_insert(0) += 1;
                        }
                    });
                    // Copy/Phi: propagate the source type to the dest.
                    match inst {
                        Instruction::Copy { dest, src } => {
                            if let Operand::Value(v) = src {
                                if let Some(&t) = value_types.get(&v.0) {
                                    value_types.insert(dest.0, t);
                                }
                            }
                        }
                        Instruction::Phi { dest, incoming, .. } => {
                            for (op, _) in incoming {
                                if let Operand::Value(v) = op {
                                    if let Some(&t) = value_types.get(&v.0) {
                                        value_types.insert(dest.0, t);
                                        break;
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                for_each_operand_in_terminator(&block.terminator, |op| {
                    if let Operand::Value(v) = op {
                        *use_counts.entry(v.0).or_insert(0) += 1;
                    }
                });
            }
            // ParamRef/Alloca/Load/… dest types come from result_type(); also
            // map function parameters (their ParamRef carries the type).
            for block in &func.blocks {
                for inst in &block.instructions {
                    if let Instruction::ParamRef { dest, ty, .. } = inst {
                        value_types.entry(dest.0).or_insert(*ty);
                    }
                }
            }

            // Find fused Cmp → Select/CondBranch pairs: the consumer must be
            // the immediately-following instruction (or the block terminator)
            // and the Cmp dest must have exactly one use.
            //
            // CHAIN EXTENSION: a run of `Copy` instructions between the Cmp
            // and the consumer is also fuseable — a Copy emits a plain
            // flag-neutral `movq` (and is often coalesced away entirely), so
            // the flags set by the Cmp survive to the consumer. Only Copy is
            // allowed in the chain (anything else could clobber flags or be a
            // different consumer).
            // Maps a fused Cmp's dest value to the FINAL value of the copy
            // chain that feeds the consumer (== the cmp dest when the
            // consumer is adjacent). The Cmp emitter records pending flags
            // under the chain-end value so the consumer matches.
            let mut fused: FxHashMap<u32, u32> = FxHashMap::default();
            // Forwarding Copy/Cast destinations in fused chains: their value is
            // never materialized, so the emitters must skip them (see
            // fused_forward_dests on the emitter state).
            let mut fused_forward: crate::common::fx_hash::FxHashSet<u32> =
                crate::common::fx_hash::FxHashSet::default();
            for block in &func.blocks {
                let insts = &block.instructions;
                for (ii, inst) in insts.iter().enumerate() {
                    if let Instruction::Cmp { dest, .. } = inst {
                        if use_counts.get(&dest.0).copied().unwrap_or(0) != 1 {
                            continue;
                        }
                        // Walk forward over Copies that forward the cmp value.
                        let mut cur = dest.0;
                        let mut k = ii + 1;
                        let mut is_consumer = false;
                        let mut chain_single_use = true;
                        let mut forward_dests: Vec<u32> = Vec::new();
                        while k < insts.len() {
                            match &insts[k] {
                                Instruction::Copy { dest: cd, src: Operand::Value(sv) }
                                    if sv.0 == cur => {
                                    // Flag-neutral forwarding copy.
                                    //
                                    // SOUNDNESS: with flag fusion, the Cmp
                                    // SKIPS materializing the boolean, so `dest`
                                    // and every intermediate copy destination are
                                    // NEVER WRITTEN (the copies just forward a
                                    // stale register). It is therefore only safe
                                    // to fuse if EVERY value in the chain is used
                                    // EXACTLY ONCE (only by the forwarding copy /
                                    // the final select-or-branch). If any
                                    // intermediate value is read by any other
                                    // instruction (a store, a call argument, a
                                    // return, an ALU...), that read sees the
                                    // never-materialized stale bool — a
                                    // miscompile. (The old code only checked the
                                    // FINAL value's use count, missing reads of
                                    // intermediate copies → expat runtests
                                    // segfault in the accounting tests.)
                                    if use_counts.get(&cd.0).copied().unwrap_or(0) != 1 {
                                        chain_single_use = false;
                                        break;
                                    }
                                    forward_dests.push(cd.0);
                                    cur = cd.0;
                                    k += 1;
                                }
                                // A widening/shrinking integer Cast of the
                                // boolean is also flag-neutral: it emits a
                                // plain mov/movzx/movsx that leaves the Cmp's
                                // flags intact, and (like the Copy above) its
                                // destination is never read by anything other
                                // than the next chain link or the consumer.
                                // This is what lets the range-check fold's
                                // `Cmp -> Cast(bool, I8->I64) -> CondBranch`
                                // fuse into `cmp; jcc` (Expat's name scanner).
                                Instruction::Cast { dest: cd, src: Operand::Value(sv), from_ty, to_ty }
                                    if sv.0 == cur && from_ty.is_integer() && to_ty.is_integer() =>
                                {
                                    if use_counts.get(&cd.0).copied().unwrap_or(0) != 1 {
                                        chain_single_use = false;
                                        break;
                                    }
                                    forward_dests.push(cd.0);
                                    cur = cd.0;
                                    k += 1;
                                }
                                Instruction::Select { cond: Operand::Value(v), .. } if v.0 == cur => {
                                    // The copy chain must terminate here (the
                                    // final value has exactly one use: this
                                    // select).
                                    if use_counts.get(&cur).copied().unwrap_or(0) == 1 {
                                        is_consumer = true;
                                    }
                                    break;
                                }
                                _ => break,
                            }
                        }
                        if is_consumer && chain_single_use {
                            fused.insert(dest.0, cur);
                            fused_forward.extend(forward_dests.iter().copied());
                        } else if !is_consumer && chain_single_use && k == insts.len() && insts.len() >= 1 {
                            // Check the block terminator as the consumer.
                            if let Terminator::CondBranch { cond: Operand::Value(v), .. } = &block.terminator {
                                if v.0 == cur && use_counts.get(&cur).copied().unwrap_or(0) == 1 {
                                    fused.insert(dest.0, cur);
                                    fused_forward.extend(forward_dests.iter().copied());
                                }
                            }
                        }
                    }
                }
            }
            // ── Compare-replay set ───────────────────────────────────────────
            // Cmps whose single use is a Select (or the block-terminator
            // CondBranch) but NOT adjacent to the Cmp cannot use the pending-
            // flag handshake (an intervening ALU instruction — e.g. the
            // `m - WSIZE` sub in fill_window's prev[] loop — clobbers the
            // flags). For those, the Cmp skips setcc/movzbl entirely and the
            // consumer re-emits `cmp` from the recorded operands, then uses
            // cmovcc/jcc directly. This removes the
            // setcc/movzbl + testq (3 instructions) per select in hot loops.
            //
            // Soundness: the Cmp dest has exactly one use (the consumer), the
            // operands are pure SSA values (side-effect-free re-materialization
            // at the consumer; the accumulator cache is invalidated first so a
            // value that only lived in %rax is reloaded from its slot), and the
            // replayed cmp is emitted immediately before the cmov/jcc with no
            // intervening flag mutation.
            let mut cmp_replay: FxHashMap<u32, (IrCmpOp, Operand, Operand, IrType)> =
                FxHashMap::default();
            for block in &func.blocks {
                let insts = &block.instructions;
                for (ii, inst) in insts.iter().enumerate() {
                    let (cdest, cop, clhs, crhs, cty) = match inst {
                        Instruction::Cmp { dest, op, lhs, rhs, ty } => {
                            (dest.0, *op, lhs.clone(), rhs.clone(), *ty)
                        }
                        _ => continue,
                    };
                    if use_counts.get(&cdest).copied().unwrap_or(0) != 1 {
                        continue;
                    }
                    // Integer comparisons only: float comparisons use ucomiss/
                    // ucomisd with a different flag contract (PF for NaN, the
                    // setnp/sete dance) — replaying them as integer cmps
                    // produces wrong selects (simd_sse2_arith regression).
                    if !cty.is_integer() {
                        continue;
                    }
                    // Replay soundness: the operands are re-materialized at the
                    // consumer via a STACK-SLOT load. A register assignment is
                    // NOT a stable location here: the allocator sized the
                    // operand's live range against the ORIGINAL Cmp position,
                    // but the replay executes LATER (after intervening
                    // instructions), and a later-defined value may share the
                    // register — the replay would compare the wrong value
                    // (sqlite3 yy_shift: `state > 599 ? state+415 : state`
                    // compared state+415 instead of state because the Cmp lhs
                    // register had been reused by the +415 add). Slot-only
                    // operands are always safe: store_rax_to writes the slot
                    // for every non-register-allocated value.
                    let op_has_location = |op: &Operand| -> bool {
                        match op {
                            Operand::Const(_) => true,
                            Operand::Value(v) => self.state.get_slot(v.0).is_some(),
                        }
                    };
                    if !op_has_location(&clhs) || !op_has_location(&crhs) {
                        continue;
                    }
                    // Already handled by the (better) adjacent fusion.
                    if fused.contains_key(&cdest) {
                        continue;
                    }
                    // The single use must be a Select in this block or the
                    // block-terminator CondBranch.
                    let mut used_by_select = false;
                    for (jj, other) in insts.iter().enumerate() {
                        if jj == ii {
                            continue;
                        }
                        if let Instruction::Select { cond: Operand::Value(v), .. } = other {
                            if v.0 == cdest {
                                used_by_select = true;
                                break;
                            }
                        }
                    }
                    if !used_by_select {
                        if let Terminator::CondBranch { cond: Operand::Value(v), .. } =
                            &block.terminator
                        {
                            if v.0 == cdest {
                                used_by_select = true;
                            }
                        }
                    }
                    if used_by_select {
                        cmp_replay.insert(cdest, (cop, clhs, crhs, cty));
                    }
                }
            }
            self.cmp_replay = cmp_replay;

            self.fused_cmp_dests = fused;
            self.fused_forward_dests = fused_forward;
            self.value_use_counts = use_counts;


            self.value_types = value_types;
        }

        // GlobalAddr values whose EVERY use is a foldable Load/Store pointer
        // never materialize (they become direct sym(%rip) accesses in
        // generate_load/store). Keep them out of the allocator AND slot
        // assignment: a dead address value otherwise consumes a register a
        // live value needs (forcing a callee-saved push/pop or a spill) and
        // an 8-byte frame slot. Exact same fold-preview the i686 backend
        // uses; the preconditions mirror generate_load/store (non-TLS,
        // non-wide type, default address space; GOT-needing symbols are
        // filtered inside the fold itself via needs_got_for_addr, which the
        // preview conservatively respects by only listing GlobalAddr dests
        // whose uses ALL satisfy the foldable-shape test).
        let never_materialized = {
            let gmap = crate::backend::generation::build_global_addr_map_for(
                func, &self.state.tls_symbols);
            let mut set = crate::backend::generation::build_foldable_global_addr_set_for(func, &gmap);
            // x86-64 refinement: the fold additionally requires
            // !needs_got_for_addr(sym) at emission time. Drop candidates
            // whose symbol would go through the GOT — those DO materialize.
            set.retain(|vid| {
                gmap.get(vid).map(|sym| {
                    // strip "+off" suffixes the GEP-merge may have added
                    let base = sym.split(['+', '-']).next().unwrap_or(sym);
                    !self.state.needs_got_for_addr(base)
                        && !self.state.tls_symbols.contains(base)
                }).unwrap_or(false)
            });
            self.state.never_materialized_values = set.clone();
            set
        };

        // SysV AMD64 argument registers present in the caller-saved pool
        // (r8, r9, rdi, rsi and — when enabled — rdx). A value consumed as a
        // call argument must not be homed in one of these: the staging writes
        // them in order before reading the value (`printf("%d %d", add(3,4),
        // mul(3,4))` read the mul result out of the format-string register).
        let call_arg_regs = vec![
            crate::backend::regalloc::PhysReg(12), // r8
            crate::backend::regalloc::PhysReg(13), // r9
            crate::backend::regalloc::PhysReg(14), // rdi
            crate::backend::regalloc::PhysReg(15), // rsi
            crate::backend::regalloc::PhysReg(16), // rdx
        ];
        // The indirect-call staging keeps the callee address in %r10 from
        // emit_call_spill_fptr until the `call *%r10`. NOTE the historical
        // x86-64 regalloc numbering swap in phys_reg_name(): PhysReg(10) is
        // named "r11" and PhysReg(11) is named "r10". The indirect target is
        // the literal "%r10", i.e. PhysReg(11).
        let indirect_target_regs = vec![crate::backend::regalloc::PhysReg(11)];
        let (reg_assigned, cached_liveness, caller_save_spans) =
            crate::backend::stack_layout::run_regalloc_and_merge_clobbers_ex(
            func, available_regs, caller_saved_regs, &asm_clobbered_regs,
            &mut self.reg_assignments, &mut self.used_callee_saved,
            false, Some(never_materialized), call_arg_regs, indirect_target_regs,
        );

        // MachInst is profitable on straight-line and modest-CFG code, but its
        // current local scheduler regressed gzip's large hot loops by ~3% even
        // while shrinking them. Keep it default-on selectively: functions with
        // a large static loop body use the mature backend. This is a target-
        // independent cost decision, not a function-name exception.
        let max_loop_insts = std::env::var("CCC_MI_MAX_LOOP_INSTS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(32);
        let loop_insts = cached_liveness
            .as_ref()
            .map(|liveness| {
                func.blocks
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| {
                        liveness.block_loop_depth.get(*index).copied().unwrap_or(0) > 0
                    })
                    .map(|(_, block)| block.instructions.len() + 1)
                    .sum::<usize>()
            })
            .unwrap_or(0);
        self.machinst_function_enabled = self.machinst_enabled
            && (loop_insts <= max_loop_insts
                || std::env::var("CCC_MI_FORCE_LOOPS").is_ok());
        if std::env::var("CCC_MI_DEBUG").is_ok() {
            eprintln!(
                "[MI-PROFIT] fn={} loop_insts={} limit={} enabled={}",
                func.name, loop_insts, max_loop_insts, self.machinst_function_enabled
            );
        }


            // W2 Load->Cast fold (2026-08-10): when a Load's result is used
            // EXACTLY ONCE and that sole use is an ADJACENT Cast whose
            // zero-extension is already provided by the load opcode
            // (movzbl/movzwl write a fully zero-extended 32-bit register;
            // movl zero-extends to 64), AND the cast's dest holds a GPR
            // register, the load targets that register directly and the cast
            // emits nothing. Removes the `movzbl (%p),%rX; mov %rX,%rYd`
            // staging pair — 33% of gzip-9 cycles sat in that pattern in
            // longest_match.
            //
            // SOUNDNESS (debugged 2026-08-10 on simd_crc_adler):
            // (a) runs AFTER regalloc so reg_assignments is THIS function's;
            // (b) ADJACENCY: the cast must immediately follow the load, so no
            //     program point exists between them;
            // (c) REGISTER-FREE-AT-LOAD: no OTHER value assigned to the cast
            //     dest's register may have a live interval covering the load
            //     point — otherwise the early load clobbers a value still
            //     needed (the simd_crc_adler corruption: fold loaded into r13
            //     which still held a value consumed by the next instruction).
            // use_counts includes terminator uses, so "exactly one use" is
            // faithful; the load's dest keeps its own (now never-written)
            // home, which nothing reads.
            let mut cast_by_src: FxHashMap<u32, (Value, IrType, IrType)> =
                FxHashMap::default();
            for block in &func.blocks {
                for inst in &block.instructions {
                    if let Instruction::Cast {
                        dest,
                        src: Operand::Value(sv),
                        from_ty,
                        to_ty,
                    } = inst
                    {
                        cast_by_src
                            .entry(sv.0)
                            .or_insert((*dest, *from_ty, *to_ty));
                    }
                }
            }
            let mut lcf: FxHashMap<u32, (PhysReg, u32)> = FxHashMap::default();
            let mut fcd: FxHashSet<u32> = FxHashSet::default();
            if std::env::var("CCC_NO_LOAD_CAST_FOLD").is_err() {
                // Program-point numbering matching liveness.rs (1 per
                // instruction + 1 per terminator, in block order).
                let intervals: Vec<(u32, u32, u32)> = cached_liveness
                    .as_ref()
                    .map(|l| {
                        l.intervals
                            .iter()
                            .map(|iv| (iv.value_id, iv.start, iv.end))
                            .collect()
                    })
                    .unwrap_or_default();
                let mut pp: u32 = 0;
                for block in &func.blocks {
                    let insts = &block.instructions;
                    for ii in 0..insts.len() {
                        let load_pp = pp;
                        if let Instruction::Load {
                            dest,
                            ty,
                            seg_override,
                            ..
                        } = &insts[ii]
                        {
                            let ok_ty = matches!(
                                ty,
                                IrType::U8 | IrType::U16 | IrType::U32
                            ) && *seg_override == AddressSpace::Default;
                            let single_use = self
                                .value_use_counts
                                .get(&dest.0)
                                .copied()
                                .unwrap_or(0)
                                == 1;
                            let adjacent_cast = matches!(
                                insts.get(ii + 1),
                                Some(Instruction::Cast {
                                    src: Operand::Value(sv),
                                    ..
                                }) if sv.0 == dest.0
                            );
                            if ok_ty && single_use && adjacent_cast {
                                if let Some((cd, from, to)) = cast_by_src.get(&dest.0) {
                                    let width_ok = *from == *ty
                                        && matches!(
                                            (ty, to),
                                            (IrType::U8, IrType::I32)
                                                | (IrType::U8, IrType::U32)
                                                | (IrType::U8, IrType::I64)
                                                | (IrType::U8, IrType::U64)
                                                | (IrType::U16, IrType::I32)
                                                | (IrType::U16, IrType::U32)
                                                | (IrType::U16, IrType::I64)
                                                | (IrType::U16, IrType::U64)
                                                | (IrType::U32, IrType::I32)
                                                | (IrType::U32, IrType::U32)
                                                | (IrType::U32, IrType::I64)
                                                | (IrType::U32, IrType::U64)
                                        );
                                    if width_ok {
                                        if let Some(reg) =
                                            self.reg_assignments.get(&cd.0).copied()
                                        {
                                            if !is_xmm_reg(reg) {
                                                // Register-free-at-load guard.
                                                let mut conflict = false;
                                                for &(vid, s, e) in &intervals {
                                                    if vid != cd.0
                                                        && s <= load_pp
                                                        && load_pp <= e
                                                    {
                                                        if let Some(&r) = self
                                                            .reg_assignments
                                                            .get(&vid)
                                                        {
                                                            if r == reg {
                                                                conflict = true;
                                                                break;
                                                            }
                                                        }
                                                    }
                                                }
                                                if !conflict {
                                                    lcf.insert(dest.0, (reg, cd.0));
                                                    fcd.insert(cd.0);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        pp += 1;
                    }
                    pp += 1; // terminator
                }
                if std::env::var("CCC_DEBUG_LOAD_CAST_FOLD").is_ok() && !lcf.is_empty() {
                    eprintln!(
                        "[LOAD-CAST-FOLD] fn={} folds={} (candidates; each fires only if its load takes a redirecting path)",
                        func.name,
                        lcf.len()
                    );
                }
                if std::env::var("CCC_NO_LOAD_CAST_FOLD").is_ok() {
                    lcf.clear();
                    fcd.clear();
                }
            }
            self.load_cast_fold = lcf;
            self.folded_cast_dests = fcd;
        // FPO (RSP mode): callee saves are movq'd into the frame at offsets -8..-N*8
        // from the virtual rbp. callee_save_reserve shifts local slots below them.
        // RBP (push mode): pushes go BEFORE subq and are at -8(%rbp)..-N*8(%rbp).
        // Local slots are within the subq frame starting at -(N*8+first_slot)(%rbp).
        // For non-variadic RBP functions, no reserve is needed because the subq frame
        // starts below the push area. For variadic functions, the register save area
        // is added to `space` separately.
        // Reserve space for callee-saved register saves PLUS 8 bytes of padding.
        // The saves occupy offsets -8, -16, ..., -(N*8) from the virtual rbp.
        // Without the +8 padding, block-local slot reuse (Tier 3) can assign
        // a variable to offset -(N*8) which COLLIDES with the last callee-save
        // (typically %rbp). Adding 8 bytes ensures the reuse pool starts at
        // -(N*8 + 8), safely below the callee-save area.
        let n_callee = self.used_callee_saved.len() as i64;
        let callee_save_reserve = if n_callee > 0 { n_callee * 8 + 8 } else { 0 };
        let mut space = calculate_stack_space_common(&mut self.state, func, callee_save_reserve, |space, alloc_size, align| {
            let effective_align = if align > 0 { align.max(8) } else { 8 };
            let alloc = (alloc_size + 7) & !7;
            let new_space = ((space + alloc + effective_align - 1) / effective_align) * effective_align;
            (-new_space, new_space)
        }, &reg_assigned, &X86_CALLEE_SAVED, cached_liveness, true);

        // Allocate spill slots for Phase 2b caller-saved-spanning registers.
        self.caller_save_spill_slots.clear();
        self.caller_save_intervals.clear();
        for (&reg_id, spans) in &caller_save_spans {
            if !spans.is_empty() {
                space += 8;
                let slot = crate::backend::state::StackSlot(-space);
                self.caller_save_spill_slots.insert(reg_id, slot);
                self.caller_save_intervals.insert(reg_id, spans.clone());
            }
        }

        if func.is_variadic {
            if self.no_sse {
                space += 48;
            } else {
                space += 176;
            }
            self.reg_save_area_offset = -space;
        }

        // `space` includes callee_save_reserve for the save area — return as-is.
        space
    }

    pub(super) fn aligned_frame_size_impl(&self, raw_space: i64) -> i64 {
        if self.state.omit_frame_pointer {
            // With frame pointer omission, there's no `push %rbp` to absorb the
            // 8-byte return address misalignment. The frame size must be ≡ 8 (mod 16)
            // so that RSP is 16-byte aligned at subsequent CALL instructions.
            // At function entry: RSP ≡ 8 (mod 16) due to the caller's CALL.
            // subq $(8 mod 16), %rsp → RSP ≡ 8 - 8 = 0 (mod 16) ✓
            if raw_space <= 0 {
                // Frame-less function: without any pushes the %rsp stays ≡ 8
                // (mod 16). A call from such a function would push a misaligned
                // %rsp (movaps in the callee faults — float_special/main segv).
                // Emit an 8-byte pad iff the function actually calls out.
                return if self.func_has_calls { 8 } else { 0 };
            }
            let aligned = (raw_space + 15) & !15;
            if aligned % 16 == 0 { aligned + 8 } else { aligned }
        } else {
            if raw_space <= 0 {
                // Frame-pointer path: `push %rbp` already realigns entry
                // %rsp ≡ 8 → ≡ 0 (mod 16); no pad needed.
                return 0;
            }
            (raw_space + 15) & !15
        }
    }

    pub(super) fn emit_prologue_impl(&mut self, func: &IrFunction, frame_size: i64) {
        self.current_return_type = func.return_type;
        self.func_ret_classes = func.ret_eightbyte_classes.clone();
        self.func_ret_is_f128_sse = func.ret_is_f128_sse;
        if self.state.cf_protection_branch {
            self.state.emit("    endbr64");
        }

        // Variadic functions need the frame pointer for va_start/va_arg to
        // compute register save area addresses relative to %rbp. Override FPO.
        let omit_fp = self.state.omit_frame_pointer && !func.is_variadic;
        let used_regs = self.used_callee_saved.clone();

        if omit_fp {
            // Frame-pointer-less prologue.
            // Use compact push/pop encoding for all saved registers (only
            // callee-saved registers appear here; Phase 2 caller-saved
            // allocations need no prologue save, and Phase 2b caller-saved
            // spans are saved selectively at call sites). pushq is 1-2 bytes
            // vs movq's 7-8 bytes. The Phase 2b spill slots (for selective
            // save/restore at call sites) are in the subq area, not the push
            // area, so they don't interfere.
            let n_saves = used_regs.len() as i64;
            let use_push_pop = n_saves > 0;

            if use_push_pop {
                for &reg in &used_regs {
                    let reg_name = phys_reg_name(reg);
                    self.state.emit_fmt(format_args!("    pushq %{}", reg_name));
                }
                let local_size = frame_size - n_saves * 8;
                if local_size > 0 {
                    self.state.out.emit_instr_imm_reg("    subq", local_size, "rsp");
                }
            } else {
                if frame_size > 0 {
                    self.state.out.emit_instr_imm_reg("    subq", frame_size, "rsp");
                }
                for (i, &reg) in used_regs.iter().enumerate() {
                    let reg_name = phys_reg_name(reg);
                    let rsp_offset = frame_size - (i as i64 + 1) * 8;
                    self.state.emit_fmt(format_args!("    movq %{}, {}(%rsp)", reg_name, rsp_offset));
                }
            }
            if self.state.emit_cfi {
                self.state.emit_fmt(format_args!("    .cfi_def_cfa_offset {}", frame_size + 8));
            }
            self.state.out.use_rsp_addressing = true;
            self.state.out.rsp_frame_size = frame_size;
        } else {
            // Traditional frame-pointer prologue.
            // Reset RSP-relative addressing flag: a previous FPO function may have
            // set it, and the AsmOutput is shared across all functions in the module.
            self.state.out.use_rsp_addressing = false;
            self.state.emit("    pushq %rbp");
            if self.state.emit_cfi {
                self.state.emit("    .cfi_def_cfa_offset 16");
                self.state.emit("    .cfi_offset %rbp, -16");
            }
            self.state.emit("    movq %rsp, %rbp");
            if self.state.emit_cfi {
                self.state.emit("    .cfi_def_cfa_register %rbp");
            }

            // Save callee-saved registers with pushq
            for &reg in &used_regs {
                let reg_name = phys_reg_name(reg);
                self.state.emit_fmt(format_args!("    pushq %{}", reg_name));
            }

            // Allocate remaining stack space for local variables
            let local_size = frame_size - (used_regs.len() as i64 * 8);
            if local_size > 0 {
                const PAGE_SIZE: i64 = 4096;
                if local_size > PAGE_SIZE {
                    let probe_label = self.state.fresh_label("stack_probe");
                    self.state.out.emit_instr_imm_reg("    movq", local_size, "r11");
                    self.state.out.emit_named_label(&probe_label);
                    self.state.out.emit_instr_imm_reg("    subq", PAGE_SIZE, "rsp");
                    self.state.emit("    orl $0, (%rsp)");
                    self.state.out.emit_instr_imm_reg("    subq", PAGE_SIZE, "r11");
                    self.state.out.emit_instr_imm_reg("    cmpq", PAGE_SIZE, "r11");
                    self.state.out.emit_jcc_label("    ja", &probe_label);
                    self.state.emit("    subq %r11, %rsp");
                    self.state.emit("    orl $0, (%rsp)");
                } else {
                    self.state.out.emit_instr_imm_reg("    subq", local_size, "rsp");
                }
            }
        }

        // Preserve source-level volatile semantics through the late text
        // peephole.  The marker is a GNU-as comment and is consumed only by
        // LCCC's peephole pre-scan; it names the final direct slot address.
        // This avoids treating a volatile local as an ordinary dead stack
        // temporary after register/FP shuttle rewrites.
        let mut volatile_ids: Vec<u32> = self.state.volatile_alloca_values.iter().copied().collect();
        volatile_ids.sort_unstable();
        for id in volatile_ids {
            if let Some(slot) = self.state.get_slot(id) {
                if self.state.out.use_rsp_addressing {
                    let off = self.state.out.rsp_frame_size + slot.0;
                    self.state.emit_fmt(format_args!("    # LCCC_VOLATILE_SLOT {}(%rsp)", off));
                } else {
                    self.state.emit_fmt(format_args!("    # LCCC_VOLATILE_SLOT {}(%rbp)", slot.0));
                }
            }
        }

        if func.is_variadic {
            let base = self.reg_save_area_offset;
            self.state.out.emit_instr_reg_rbp("    movq", "rdi", base);
            self.state.out.emit_instr_reg_rbp("    movq", "rsi", base + 8);
            self.state.out.emit_instr_reg_rbp("    movq", "rdx", base + 16);
            self.state.out.emit_instr_reg_rbp("    movq", "rcx", base + 24);
            self.state.out.emit_instr_reg_rbp("    movq", "r8", base + 32);
            self.state.out.emit_instr_reg_rbp("    movq", "r9", base + 40);
            if !self.no_sse {
                for i in 0..8i64 {
                    self.state.emit_fmt(format_args!("    movdqu %xmm{}, {}(%rbp)", i, base + 48 + i * 16));
                }
            }
        }
    }

    pub(super) fn emit_epilogue_impl(&mut self, frame_size: i64) {
        let used_regs = self.used_callee_saved.clone();
        let num_saved = used_regs.len() as i64;
        let omit_fp = self.state.omit_frame_pointer && !self.state.func_is_variadic;

        if omit_fp {
            let use_push_pop = num_saved > 0;

            if use_push_pop {
                let local_size = frame_size - num_saved * 8;
                if local_size > 0 {
                    self.state.out.emit_instr_imm_reg("    addq", local_size, "rsp");
                }
                for &reg in used_regs.iter().rev() {
                    let reg_name = phys_reg_name(reg);
                    self.state.emit_fmt(format_args!("    popq %{}", reg_name));
                }
            } else {
                for (i, &reg) in used_regs.iter().enumerate() {
                    let reg_name = phys_reg_name(reg);
                    let offset = frame_size - (i as i64 + 1) * 8;
                    self.state.emit_fmt(format_args!("    movq {}(%rsp), %{}", offset, reg_name));
                }
                if frame_size > 0 {
                    self.state.out.emit_instr_imm_reg("    addq", frame_size, "rsp");
                }
            }
        } else {
            // Traditional epilogue: restore from pushes, then popq %rbp
            if num_saved > 0 {
                self.state.emit_fmt(format_args!("    leaq {}(%rbp), %rsp", -(num_saved * 8)));
            } else {
                self.state.emit("    movq %rbp, %rsp");
            }
            for &reg in used_regs.iter().rev() {
                let reg_name = phys_reg_name(reg);
                self.state.emit_fmt(format_args!("    popq %{}", reg_name));
            }
            self.state.emit("    popq %rbp");
        }
        let _ = frame_size;
    }

    pub(super) fn emit_store_params_impl(&mut self, func: &IrFunction) {
        let xmm_regs = ["xmm0", "xmm1", "xmm2", "xmm3", "xmm4", "xmm5", "xmm6", "xmm7"];
        let config = self.call_abi_config_impl();
        let param_classes = classify_params(func, &config);
        self.state.param_classes = param_classes.clone();
        self.state.num_params = func.params.len();
        self.state.func_is_variadic = func.is_variadic;

        self.state.param_alloca_slots = (0..func.params.len()).map(|i| {
            find_param_alloca(func, i).and_then(|(dest, ty)| {
                self.state.get_slot(dest.0).map(|slot| (slot, ty))
            })
        }).collect();

        // Build a map of param_idx -> ParamRef dest Value for fast lookup.
        // This is used to optimize parameter storage: when the ParamRef dest
        // is register-allocated, we can store the ABI arg register directly
        // to the callee-saved register, skipping the alloca slot entirely.
        let mut paramref_dests: Vec<Option<Value>> = vec![None; func.params.len()];
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Instruction::ParamRef { dest, param_idx, .. } = inst {
                    if *param_idx < paramref_dests.len() {
                        paramref_dests[*param_idx] = Some(*dest);
                    }
                }
            }
        }

        // In RBP mode: stack args start at 16(%rbp) = return_addr(8) + saved_rbp(8).
        // In FPO mode: no saved rbp, so stack args start at 8 from the virtual rbp
        // (which is entry_rsp = return_addr).
        let stack_base: i64 = if self.state.omit_frame_pointer { 8 } else { 16 };

        // Build a map from physical register -> list of param indices that use it,
        // so we can detect when two params share the same callee-saved register.
        let mut reg_to_params: crate::common::fx_hash::FxHashMap<u8, Vec<usize>> = crate::common::fx_hash::FxHashMap::default();
        for (i, _) in func.params.iter().enumerate() {
            if let Some(paramref_dest) = paramref_dests[i] {
                if let Some(&phys_reg) = self.reg_assignments.get(&paramref_dest.0) {
                    reg_to_params.entry(phys_reg.0).or_default().push(i);
                }
            }
        }

        // Deferred FP pre-stores: (param_idx, abi_xmm_idx, home_phys, dest_name, is_f32).
        // They are emitted AFTER the main loop in an order where a home register
        // never clobbers an un-consumed ABI argument register — a home may be
        // xmm2..xmm7, which double as ABI arg registers for other params, and
        // pre-storing in param order then destroys a later param's incoming
        // value (observed miscompile: constp(a, b, scale) computed (a+b)*a
        // because a's home xmm2 clobbered scale's ABI xmm2).
        let mut fp_prestores: Vec<(usize, usize, PhysReg, &'static str, bool)> = Vec::new();
        // Deferred integer/pointer pre-stores form a parallel-copy problem:
        // caller-saved homes such as r8/r9/rdi/rsi can still contain a later
        // incoming argument.  Emit them after collection in a dependency-safe
        // order and break cycles through reserved scratch %rax.
        // (param_idx, current source, original ABI source, destination, home)
        let mut gpr_prestores: Vec<(
            usize, &'static str, &'static str, &'static str, PhysReg
        )> = Vec::new();

        for (i, _param) in func.params.iter().enumerate() {
            let class = param_classes[i];

            // Pre-store optimization: when a param's alloca is dead (eliminated by
            // dead param alloca analysis) but the ParamRef dest is register-assigned,
            // store the ABI arg register directly to the assigned physical register
            // in the prologue. This is critical because:
            // 1. Dead alloca means no stack slot exists for this param
            // 2. The ABI register (rdi, rsi, etc.) is caller-saved and will be clobbered
            // 3. We must save the value NOW, before any other code runs
            // 4. emit_param_ref will see param_pre_stored and skip code generation
            if let Some(paramref_dest) = paramref_dests[i] {
                let has_slot = find_param_alloca(func, i)
                    .and_then(|(dest, _)| self.state.get_slot(dest.0))
                    .is_some();
                if std::env::var("CCC_DEBUG_PARAM_STORE").is_ok() {
                    let reg = self.reg_assignments.get(&paramref_dest.0);
                    eprintln!("[PRE-STORE] param {} paramref_dest={} has_slot={} reg={:?}", i, paramref_dest.0, has_slot, reg);
                }
                if !has_slot {
                    if let Some(&phys_reg) = self.reg_assignments.get(&paramref_dest.0) {
                        // Callee-saved GPRs can be copied immediately.  x86-64
                        // caller-saved GPR homes (PhysReg 10..16) are legal for
                        // values proven not to span a call, but their moves are
                        // deferred below because a destination may still hold a
                        // different incoming ABI argument. XMM homes use their
                        // own ordered parallel-copy set for the same reason.
                        let is_callee_saved = phys_reg.0 >= 1 && phys_reg.0 <= 5;
                        let is_caller_saved_gpr = (10..=16).contains(&phys_reg.0);
                        let is_xmm = super::emit::is_xmm_reg(phys_reg);
                        if std::env::var("CCC_DEBUG_PARAM_STORE").is_ok() {
                            let shared = reg_to_params.get(&phys_reg.0).is_some_and(|u| u.len() > 1);
                            eprintln!("[PRE-STORE]   callee={} caller={} shared={} class={:?}",
                                is_callee_saved, is_caller_saved_gpr, shared, class);
                        }
                        if is_callee_saved || is_caller_saved_gpr || is_xmm {
                            let shared = reg_to_params.get(&phys_reg.0)
                                .is_some_and(|users| users.len() > 1);
                            if !shared {
                                let dest_reg = phys_reg_name(phys_reg);
                                if let ParamClass::IntReg { reg_idx } = class {
                                    let source = X86_ARG_REGS[reg_idx];
                                    if is_caller_saved_gpr {
                                        gpr_prestores.push((i, source, source, dest_reg, phys_reg));
                                    } else {
                                        self.state.out.emit_instr_reg_reg(
                                            "    movq", source, dest_reg);
                                        self.state.param_pre_stored.insert(i);
                                        self.param_source_regs.insert(phys_reg.0, source);
                                    }
                                } else if let ParamClass::FloatReg { reg_idx } = class {
                                    // FP pre-store: DEFERRED (see above).
                                    // Record the ABI register and home so the
                                    // deferred pass can order them safely.
                                    fp_prestores.push((
                                        i,
                                        reg_idx,
                                        phys_reg,
                                        dest_reg,
                                        func.params[i].ty == IrType::F32,
                                    ));
                                }
                            }
                        }
                    }
                    continue;
                }
            } else {
                // DEBUG: dump entry block instructions for this param
                if std::env::var("CCC_DEBUG_PARAM_STORE").is_ok() {
                    if let Some((alloca_dest, _)) = find_param_alloca(func, i) {
                        eprintln!("[PARAM-STORE] param {} has alloca dest={}, no ParamRef", i, alloca_dest.0);
                        eprintln!("[PARAM-STORE] has_slot={}", self.state.get_slot(alloca_dest.0).is_some());
                        for (bi, block) in func.blocks.iter().enumerate() {
                            for inst in &block.instructions {
                                match inst {
                                    Instruction::Store { ptr, val, .. } if ptr.0 == alloca_dest.0 =>
                                        eprintln!("[PARAM-STORE]   block[{}] Store to alloca: val={:?}", bi, val),
                                    Instruction::Load { ptr, dest, .. } if ptr.0 == alloca_dest.0 =>
                                        eprintln!("[PARAM-STORE]   block[{}] Load from alloca: dest={}", bi, dest.0),
                                    Instruction::Copy { dest, src } =>
                                        eprintln!("[PARAM-STORE]   block[{}] Copy dest={} src={:?}", bi, dest.0, src),
                                    Instruction::ParamRef { dest, param_idx, .. } =>
                                        eprintln!("[PARAM-STORE]   block[{}] ParamRef dest={} idx={}", bi, dest.0, param_idx),
                                    _ => {}
                                }
                            }
                        }
                        for (&vid, &reg) in self.reg_assignments.iter() {
                            eprintln!("[PARAM-STORE]   reg_assign: val={} -> PhysReg({})", vid, reg.0);
                        }
                    }
                }
                // No ParamRef for this param. The alloca may have been promoted
                // by mem2reg, converting Store/Load chains to direct SSA references.
                // After promotion, the param value flows through Copy/Cast chains.
                //
                // The register allocator may have assigned a promoted value to a
                // callee-saved register. We must copy the ABI arg register to it
                // in the prologue, because ABI registers get clobbered by calls.
                //
                // Strategy: find the alloca for this param, then search for Store
                // instructions that write TO that alloca. The Store's source value
                // (which may be a Copy from the original param) tells us which SSA
                // value represents the parameter after store-to-load forwarding.
                let has_slot = find_param_alloca(func, i)
                    .and_then(|(dest, _)| self.state.get_slot(dest.0))
                    .is_some();
                if !has_slot {
                    if let Some((alloca_dest, _)) = find_param_alloca(func, i) {
                        let alloca_id = alloca_dest.0;
                        // Collect all SSA values stored to this alloca, then
                        // propagate through Copy chains to find all derived values.
                        // Any of these that are register-assigned need the ABI arg
                        // saved in the prologue.
                        let mut param_vals: Vec<u32> = Vec::new();
                        for block in &func.blocks {
                            for inst in &block.instructions {
                                if let Instruction::Store { ptr, val, .. } = inst {
                                    if ptr.0 == alloca_id {
                                        if let crate::ir::instruction::Operand::Value(v) = val {
                                            param_vals.push(v.0);
                                        }
                                    }
                                }
                            }
                        }
                        // Propagate through Copy chains
                        let mut all_vals: crate::common::fx_hash::FxHashSet<u32> = param_vals.iter().copied().collect();
                        let mut changed_prop = true;
                        while changed_prop {
                            changed_prop = false;
                            for block in &func.blocks {
                                for inst in &block.instructions {
                                    if let Instruction::Copy { dest, src: crate::ir::instruction::Operand::Value(v) } = inst {
                                        if all_vals.contains(&v.0) && all_vals.insert(dest.0) {
                                            changed_prop = true;
                                        }
                                    }
                                }
                            }
                        }
                        // Check if any derived value is callee-saved register-assigned
                        for &vid in &all_vals {
                            if let Some(&phys_reg) = self.reg_assignments.get(&vid) {
                                let is_callee_saved = phys_reg.0 >= 1 && phys_reg.0 <= 5;
                                if is_callee_saved {
                                    let dest_reg = phys_reg_name(phys_reg);
                                    if let ParamClass::IntReg { reg_idx } = class {
                                        self.state.out.emit_instr_reg_reg(
                                            "    movq", X86_ARG_REGS[reg_idx], dest_reg);
                                        self.state.param_pre_stored.insert(i);
                                        self.param_source_regs.insert(phys_reg.0, X86_ARG_REGS[reg_idx]);
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    continue;
                }
            }

            let (slot, ty) = if let Some((dest, ty)) = find_param_alloca(func, i) {
                if let Some(slot) = self.state.get_slot(dest.0) {
                    (slot, ty)
                } else {
                    continue;
                }
            } else {
                continue;
            };

            match class {
                // I64RegPair exists only under i686 gcc_regparm_mode; this
                // backend never sets that config flag.
                ParamClass::I64RegPair { .. } => unreachable!("I64RegPair is i686-regparm-only"),
                ParamClass::IntReg { reg_idx } => {
                    // Always store the full 64-bit register to ensure the entire 8-byte
                    // slot is initialized. Using a typed store (e.g., movl for I32) would
                    // only write 4 bytes, leaving the upper bytes uninitialized. Later
                    // untyped loads via value_to_reg use movq (8 bytes), which would read
                    // uninitialized memory and trigger valgrind errors.
                    // The typed load in emit_param_ref_impl correctly extracts only the
                    // meaningful bytes (e.g., movslq for I32).
                    self.state.out.emit_instr_reg_rbp("    movq", X86_ARG_REGS[reg_idx], slot.0);
                }
                ParamClass::FloatReg { reg_idx } => {
                    if ty == IrType::F32 {
                        self.state.out.emit_instr_reg_reg("    movd", xmm_regs[reg_idx], "eax");
                        self.state.out.emit_instr_reg_rbp("    movq", "rax", slot.0);
                    } else {
                        self.state.out.emit_instr_reg_rbp("    movq", xmm_regs[reg_idx], slot.0);
                    }
                }
                ParamClass::I128RegPair { base_reg_idx } => {
                    self.state.out.emit_instr_reg_rbp("    movq", X86_ARG_REGS[base_reg_idx], slot.0);
                    self.state.out.emit_instr_reg_rbp("    movq", X86_ARG_REGS[base_reg_idx + 1], slot.0 + 8);
                }
                ParamClass::StructByValReg { base_reg_idx, size } => {
                    self.state.out.emit_instr_reg_rbp("    movq", X86_ARG_REGS[base_reg_idx], slot.0);
                    if size > 8 {
                        self.state.out.emit_instr_reg_rbp("    movq", X86_ARG_REGS[base_reg_idx + 1], slot.0 + 8);
                    }
                }
                ParamClass::StructSseReg { lo_fp_idx, hi_fp_idx, .. } => {
                    self.state.out.emit_instr_reg_rbp("    movq", xmm_regs[lo_fp_idx], slot.0);
                    if let Some(hi) = hi_fp_idx {
                        self.state.out.emit_instr_reg_rbp("    movq", xmm_regs[hi], slot.0 + 8);
                    }
                }
                ParamClass::F128SseReg { reg_idx } => {
                    // _Float128: the full 16 bytes arrive in ONE XMM register.
                    self.state.out.emit_instr_reg_rbp("    movdqu", xmm_regs[reg_idx], slot.0);
                }
                ParamClass::StructMixedIntSseReg { int_reg_idx, fp_reg_idx, .. } => {
                    self.state.out.emit_instr_reg_rbp("    movq", X86_ARG_REGS[int_reg_idx], slot.0);
                    self.state.out.emit_instr_reg_rbp("    movq", xmm_regs[fp_reg_idx], slot.0 + 8);
                }
                ParamClass::StructMixedSseIntReg { fp_reg_idx, int_reg_idx, .. } => {
                    self.state.out.emit_instr_reg_rbp("    movq", xmm_regs[fp_reg_idx], slot.0);
                    self.state.out.emit_instr_reg_rbp("    movq", X86_ARG_REGS[int_reg_idx], slot.0 + 8);
                }
                ParamClass::F128AlwaysStack { offset } => {
                    let src = stack_base + offset;
                    self.state.out.emit_instr_rbp("    fldt", src);
                    self.state.out.emit_instr_rbp("    fstpt", slot.0);
                }
                ParamClass::I128Stack { offset } => {
                    let src = stack_base + offset;
                    self.state.out.emit_instr_rbp_reg("    movq", src, "rax");
                    self.state.out.emit_instr_reg_rbp("    movq", "rax", slot.0);
                    self.state.out.emit_instr_rbp_reg("    movq", src + 8, "rax");
                    self.state.out.emit_instr_reg_rbp("    movq", "rax", slot.0 + 8);
                }
                ParamClass::StackScalar { offset } => {
                    // Load from caller's stack frame and store full 8 bytes to ensure
                    // the entire slot is initialized (see IntReg comment above).
                    let src = stack_base + offset;
                    self.state.out.emit_instr_rbp_reg("    movq", src, "rax");
                    self.state.out.emit_instr_reg_rbp("    movq", "rax", slot.0);
                }
                ParamClass::StructStack { offset, size } | ParamClass::LargeStructStack { offset, size } => {
                    let src = stack_base + offset;
                    let n_qwords = size.div_ceil(8);
                    for qi in 0..n_qwords {
                        let src_off = src + (qi as i64 * 8);
                        let dst_off = slot.0 + (qi as i64 * 8);
                        self.state.out.emit_instr_rbp_reg("    movq", src_off, "rax");
                        self.state.out.emit_instr_reg_rbp("    movq", "rax", dst_off);
                    }
                }
                ParamClass::F128FpReg { .. } | ParamClass::F128GpPair { .. } | ParamClass::F128Stack { .. } |
                ParamClass::LargeStructByRefReg { .. } | ParamClass::LargeStructByRefStack { .. } |
                ParamClass::StructSplitRegStack { .. } |
                ParamClass::ZeroSizeSkip => {}
            }
        }

        // Emit integer/pointer parameter moves as a parallel copy.  A move is
        // safe when its destination is not the current source of another
        // pending move.  If all remaining moves form a cycle, preserve one
        // source in reserved scratch %rax and continue.  This is what makes it
        // sound for register allocation to use r8/r9/rdi/rsi as parameter homes.
        if !gpr_prestores.is_empty() {
            let mut pending = gpr_prestores;
            while !pending.is_empty() {
                let ready = pending.iter().enumerate().find_map(|(idx, cand)| {
                    let dest = cand.3;
                    let clobbers_other_source = pending.iter().enumerate().any(
                        |(j, other)| j != idx && other.1 == dest
                    );
                    (!clobbers_other_source).then_some(idx)
                });
                if let Some(idx) = ready {
                    let (param_idx, source, original_source, dest, home) = pending.remove(idx);
                    if source != dest {
                        self.state.out.emit_instr_reg_reg("    movq", source, dest);
                    }
                    self.state.param_pre_stored.insert(param_idx);
                    self.param_source_regs.insert(home.0, original_source);
                } else {
                    // Parallel-copy cycle. RAX is excluded from allocation and
                    // is not an incoming integer argument register.
                    let source = pending[0].1;
                    self.state.out.emit_instr_reg_reg("    movq", source, "rax");
                    pending[0].1 = "rax";
                }
            }
        }

        // Emit the deferred FP pre-stores in a safe order (see the collection
        // above). Greedy: a param may be pre-stored only when its home register
        // is not the ABI argument register of any still-pending param — reading
        // that ABI register later would yield the overwritten home value. Cycles
        // (home(i) == ABI(j) AND home(j) == ABI(i), only possible with 3+ FP
        // params whose homes are each other's ABI registers) are broken with a
        // scratch XMM register that is no pending param's home.
        if !fp_prestores.is_empty() {
            // (param_idx, abi_idx, home_phys, dest_name, is_f32)
            let mut pending: Vec<(usize, usize, PhysReg, &'static str, bool)> = fp_prestores;
            // (dest_name, is_f32, scratch_name)
            let mut scratch_saves: Vec<(&'static str, bool, &'static str)> = Vec::new();
            while !pending.is_empty() {
                let mut progressed = false;
                let mut remaining: Vec<(usize, usize, PhysReg, &'static str, bool)> = Vec::new();
                for cand in &pending {
                    let (i, abi_idx, _home, dest, is_f32) = *cand;
                    // Unsafe if the home register IS the ABI argument register
                    // of a DIFFERENT pending param (reading that ABI register
                    // later would yield the overwritten home value). Compare
                    // names: the home is xmm2..xmm15, ABI args are xmm0..xmm7.
                    let clobbers = pending.iter().any(|&(j, j_abi, _, _, _)| {
                        j != i && dest == xmm_regs[j_abi]
                    });
                    if !clobbers {
                        let mnemonic = if is_f32 { "    movss" } else { "    movsd" };
                        self.state.out.emit_instr_reg_reg(mnemonic, xmm_regs[abi_idx], dest);
                        self.state.param_pre_stored.insert(i);
                        progressed = true;
                    } else {
                        remaining.push(*cand);
                    }
                }
                if !progressed {
                    // Cycle: save the first pending param's ABI value to a
                    // scratch XMM register (no pending param's home), so its
                    // ABI register is free for the others; the scratch value
                    // moves to its home at the very end.
                    let (i, abi_idx, home, dest, is_f32) = pending[0];
                    let scratch = {
                        let mut s: Option<&'static str> = None;
                        for r in (26u8..=33).rev() {
                            let name = phys_reg_name(PhysReg(r));
                            if pending.iter().all(|&(_, _, h, _, _)| h.0 != r) {
                                s = Some(name);
                                break;
                            }
                        }
                        s
                    };
                    if let Some(scratch) = scratch {
                        let mnemonic = if is_f32 { "    movss" } else { "    movsd" };
                        self.state.out.emit_instr_reg_reg(mnemonic, xmm_regs[abi_idx], scratch);
                        self.state.param_pre_stored.insert(i);
                        scratch_saves.push((dest, is_f32, scratch));
                        let _ = home;
                        pending.remove(0);
                    } else {
                        // Degenerate (every XMM is a pending home): emit directly.
                        let mnemonic = if is_f32 { "    movss" } else { "    movsd" };
                        self.state.out.emit_instr_reg_reg(mnemonic, xmm_regs[abi_idx], dest);
                        self.state.param_pre_stored.insert(i);
                        pending.remove(0);
                    }
                } else {
                    pending = remaining;
                }
            }
            for (dest, is_f32, scratch) in scratch_saves {
                let mnemonic = if is_f32 { "    movss" } else { "    movsd" };
                self.state.out.emit_instr_reg_reg(mnemonic, scratch, dest);
            }
        }
    }

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
                let load_instr = Self::mov_load_for_type(alloca_ty);
                let reg = Self::load_dest_reg(alloca_ty);
                let sr = self.slot_ref(slot.0);
                self.state.emit_fmt(format_args!("    {} {}, {}", load_instr, sr, reg));
                self.store_rax_to(dest);
                return;
            }
        }

        let xmm_regs = ["xmm0", "xmm1", "xmm2", "xmm3", "xmm4", "xmm5", "xmm6", "xmm7"];
        let class = self.state.param_classes[param_idx];
        let stack_base: i64 = if self.state.omit_frame_pointer { 8 } else { 16 };

        match class {
            ParamClass::IntReg { reg_idx } => {
                let src_reg = Self::reg_for_type(X86_ARG_REGS[reg_idx], ty);
                let load_instr = Self::mov_load_for_type(ty);
                let dest_reg = Self::load_dest_reg(ty);
                self.state.emit_fmt(format_args!("    {} %{}, {}", load_instr, src_reg, dest_reg));
                self.store_rax_to(dest);
            }
            ParamClass::FloatReg { reg_idx } => {
                if ty == IrType::F32 {
                    self.state.out.emit_instr_reg_reg("    movd", xmm_regs[reg_idx], "eax");
                    self.store_rax_to(dest);
                } else {
                    self.state.out.emit_instr_reg_reg("    movq", xmm_regs[reg_idx], "rax");
                    self.store_rax_to(dest);
                }
            }
            ParamClass::StackScalar { offset } => {
                let src = stack_base + offset;
                let load_instr = Self::mov_load_for_type(ty);
                let reg = Self::load_dest_reg(ty);
                let sr = self.slot_ref(src);
                self.state.emit_fmt(format_args!("    {} {}, {}", load_instr, sr, reg));
                self.store_rax_to(dest);
            }
            _ => {}
        }
    }

    pub(super) fn emit_epilogue_and_ret_impl(&mut self, frame_size: i64) {
        self.emit_epilogue_impl(frame_size);
        if self.state.function_return_thunk {
            self.state.emit("    jmp __x86_return_thunk");
        } else {
            self.state.emit("    ret");
        }
    }

    pub(super) fn store_instr_for_type_impl(&self, ty: IrType) -> &'static str {
        Self::mov_store_for_type(ty)
    }

    pub(super) fn load_instr_for_type_impl(&self, ty: IrType) -> &'static str {
        Self::mov_load_for_type(ty)
    }
}
