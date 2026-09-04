//! X86Codegen: prologue, epilogue, parameter storage.

use super::emit::{
    collect_inline_asm_callee_saved_x86, is_xmm_reg, phys_reg_name, X86Codegen, X86_ARG_REGS,
    X86_CALLEE_SAVED, X86_CALLEE_SAVED_WITH_RBP, X86_CALLER_SAVED,
};
use crate::backend::call_abi::{classify_params, ParamClass};
use crate::backend::generation::{calculate_stack_space_common, find_param_alloca};
use crate::backend::liveness::{for_each_operand_in_instruction, for_each_operand_in_terminator};
use crate::backend::regalloc::PhysReg;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::{AddressSpace, EightbyteClass, IrType};
use crate::ir::reexports::{
    Instruction, IntrinsicOp, IrBinOp, IrCmpOp, IrFunction, Operand, Terminator, Value,
};

/// Def-chain stability classification for a va_list root (see the analysis
/// in `calculate_stack_space_impl`): `true` only when the chain bottoms out
/// in an Alloca, ParamRef, Call result, or a constant — i.e. an id that can
/// never be re-derived under a different SSA name. Copy/Cast are walked
/// through (pure renames); everything else — Load, GEP, Select, Phi — can be
/// re-emitted per source mention with a fresh id and is reported unstable.
/// The walk is bounded; a cycle (phi loops) classifies unstable.
fn va_root_is_stable(func: &IrFunction, root: u32) -> bool {
    let mut cur = root;
    for _ in 0..16 {
        let mut next: Option<u32> = None;
        let mut stable = false;
        let mut decided = false;
        for block in &func.blocks {
            for inst in &block.instructions {
                match inst {
                    Instruction::Alloca { dest, .. } | Instruction::ParamRef { dest, .. } => {
                        if dest.0 == cur {
                            stable = true;
                            decided = true;
                        }
                    }
                    Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
                        if let Some(dest) = info.dest.as_ref() {
                            if dest.0 == cur {
                                stable = true;
                                decided = true;
                            }
                        }
                    }
                    Instruction::Copy { dest, src } | Instruction::Cast { dest, src, .. } => {
                        if dest.0 == cur {
                            match src {
                                Operand::Value(v) => next = Some(v.0),
                                Operand::Const(_) => {
                                    stable = true;
                                    decided = true;
                                }
                            }
                        }
                    }
                    // Load/GEP/Select/Phi and every other def form: the value
                    // is memory- or control-derived; a second source mention
                    // re-derives it under a fresh id. Unstable.
                    _ => {}
                }
                if decided {
                    break;
                }
            }
            if decided {
                break;
            }
        }
        if decided {
            return stable;
        }
        match next {
            Some(n) if n != cur => cur = n,
            _ => return false, // no def found (should not happen) or a cycle
        }
    }
    false
}

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

        // Same-block div/rem pair fusion table (one divq/divl serves a
        // URem+UDiv couple with identical operands). Constant-RHS pairs
        // never fuse (compute_i686_divrem_pairs excludes them
        // unconditionally): the magic-number path may claim them, and the
        // RA model must stay exact. The X86_64 target pairs 64-bit ops in
        // addition to the 32-bit classes.
        let pairs = crate::backend::regalloc::compute_i686_divrem_pairs(
            func,
            crate::backend::regalloc::DivRemTarget::X86_64,
        );
        self.divrem_tail_dests = pairs.tail_dests;
        self.divrem_head_partners = pairs.head_partners;
        self.divrem_broken_tails.clear();

        // Analyze IVSR patterns for Phase 9b indexed addressing optimization
        self.analyze_ivsr_pointers(func);

        // Track variadic function info
        self.is_variadic = func.is_variadic;
        if func.is_variadic {
            // Dead-save elimination (GCC's `sum_int`/`sum_dbl` behavior): the
            // register save area only needs to preserve a class (GP / SSE) if
            // some va_arg in THIS body actually reads it, or the va_list
            // escapes to a callee that may read any class. This skips all 8
            // 16-byte XMM saves in integer-only varargs (printf/printk
            // wrappers that never re-consume FP), and the 6 GP saves in
            // FP-only varargs.
            let (mut needs_gp, mut needs_fp) = (false, false);
            // va_list-aliased values (VaStart root + every va_copy src/dest),
            // closed transitively over Copy so `va_list ap2 = ap; f(ap2)` is
            // still detected as an escape. The raw seed values are kept for
            // the stability classification below.
            let mut seeds: Vec<Value> = Vec::new();
            let mut va_ids: FxHashSet<u32> = FxHashSet::default();
            for block in &func.blocks {
                for inst in &block.instructions {
                    match inst {
                        Instruction::VaStart { va_list_ptr } => {
                            va_ids.insert(va_list_ptr.0);
                            seeds.push(*va_list_ptr);
                        }
                        Instruction::VaCopy { dest_ptr, src_ptr } => {
                            va_ids.insert(dest_ptr.0);
                            va_ids.insert(src_ptr.0);
                            seeds.push(*dest_ptr);
                            seeds.push(*src_ptr);
                        }
                        Instruction::VaArg { result_ty, .. } => {
                            if result_ty.is_128bit() || !result_ty.is_float() {
                                needs_gp = true;
                            } else if !result_ty.is_long_double() {
                                // F32/F64 (SSE class). long double is MEMORY.
                                needs_fp = true;
                            }
                        }
                        Instruction::VaArgStruct {
                            eightbyte_classes, ..
                        } => {
                            for c in eightbyte_classes {
                                match c {
                                    EightbyteClass::Integer | EightbyteClass::NoClass => {
                                        needs_gp = true
                                    }
                                    EightbyteClass::Sse => needs_fp = true,
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            // Alias closure: optimization can represent a source-level
            // `cond ? &ap : NULL` as Select, and CFG form uses Phi.  A pointer
            // derived from the va_list root by any of these value-only forms
            // still exposes the register-save area to a callee.  Tracking only
            // Copy made FP-save elimination phase-order dependent: if-conversion
            // changed the escaping `&ap` into a Select, all XMM saves vanished,
            // and a callee's `va_arg(double)` read uninitialized stack bytes.
            let mut changed = true;
            while changed {
                changed = false;
                for block in &func.blocks {
                    for inst in &block.instructions {
                        let (dest, aliases_va_list) = match inst {
                            Instruction::Copy {
                                dest,
                                src: Operand::Value(src),
                            }
                            | Instruction::Cast {
                                dest,
                                src: Operand::Value(src),
                                ..
                            } => (*dest, va_ids.contains(&src.0)),
                            Instruction::GetElementPtr { dest, base, .. } => {
                                (*dest, va_ids.contains(&base.0))
                            }
                            Instruction::Select {
                                dest,
                                true_val,
                                false_val,
                                ..
                            } => (
                                *dest,
                                [true_val, false_val].iter().any(|op| {
                                    matches!(op, Operand::Value(v) if va_ids.contains(&v.0))
                                }),
                            ),
                            Instruction::Phi { dest, incoming, .. } => (
                                *dest,
                                incoming.iter().any(|(op, _)| {
                                    matches!(op, Operand::Value(v) if va_ids.contains(&v.0))
                                }),
                            ),
                            _ => continue,
                        };
                        if aliases_va_list && va_ids.insert(dest.0) {
                            changed = true;
                        }
                    }
                }
            }
            // A va_list rooted in static storage, or whose address is stored
            // anywhere, escapes without appearing as a call argument. Global
            // `va_start(gap, ...)` followed by `bar()` is a canonical GCC
            // torture shape: bar reads gap and may consume both GP and SSE
            // classes. Preserve the complete register-save area.
            for block in &func.blocks {
                for inst in &block.instructions {
                    let escapes = match inst {
                        Instruction::GlobalAddr { dest, .. } => va_ids.contains(&dest.0),
                        Instruction::Store {
                            val: Operand::Value(value),
                            ..
                        } => va_ids.contains(&value.0),
                        _ => false,
                    };
                    if escapes {
                        needs_gp = true;
                        needs_fp = true;
                    }
                }
            }

            // Escape: any call argument that references a va_list value.
            for block in &func.blocks {
                for inst in &block.instructions {
                    let info = match inst {
                        Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
                            info
                        }
                        _ => continue,
                    };
                    for arg in &info.args {
                        if let Operand::Value(v) = arg {
                            if va_ids.contains(&v.0) {
                                needs_gp = true;
                                needs_fp = true;
                                break;
                            }
                        }
                    }
                }
            }
            // Fail-closed root classification. The value-alias closure can
            // only see SAME-ID aliases; a va_list whose address arrives
            // through memory (a Load — gcc.c-torture va-arg-21's
            // `va_list *ap_array[]` keeps the pointer in an array element, so
            // every textual mention re-loads it under a fresh SSA id) or
            // through address arithmetic (a GEP — `va_list a[2]`'s `a[0]`
            // re-emits the GEP per mention) can hand the SAME va_list to a
            // callee under an id the closure never matches, hiding the
            // escape. The dead-save elimination would then skip the
            // register-save prologue and the callee's va_arg reads
            // uninitialized save-area bytes (observed: "hello (null)").
            //
            // A root is *stable* only when its def chain bottoms out in an
            // Alloca (the canonical local `va_list ap;`, where every mention
            // decays to the alloca's own id), a ParamRef, a fresh Call
            // result (unaliased until stored, which the store-escape rule
            // above already catches), or a constant. Copy/Cast are walked
            // through. Every other root — Load, GEP, Select, Phi, unknown —
            // conservatively requires BOTH save classes. This costs nothing
            // on the canonical shapes (the dead-save optimization's measured
            // corpus is local-va_list printf wrappers) and closes the whole
            // re-load blind spot at once. Kill switch for bisection:
            // CCC_NO_VA_ROOT_GUARD=1 restores the pre-guard analysis.
            if std::env::var("CCC_NO_VA_ROOT_GUARD").is_err()
                && seeds.iter().any(|s| !va_root_is_stable(func, s.0))
            {
                needs_gp = true;
                needs_fp = true;
            }
            self.vararg_gp_save = needs_gp;
            self.vararg_fp_save = needs_fp;
            if std::env::var("CCC_DEBUG_VARARG").is_ok() {
                eprintln!(
                    "debug vararg {}: gp={} fp={} stable={} va_ids={:?}",
                    func.name,
                    needs_gp,
                    needs_fp,
                    seeds.iter().all(|s| va_root_is_stable(func, s.0)),
                    va_ids
                );
            }
        }
        // Count named params using the shared ABI classification, so this
        // stays in sync with classify_call_args (caller side) automatically.
        {
            let config = self.call_abi_config_impl();
            let classification = crate::backend::call_abi::classify_params_full(func, &config);
            let mut named_gp = 0usize;
            let mut named_fp = 0usize;
            for class in &classification.classes {
                named_gp += class.gp_reg_count();
                named_fp += class.fp_reg_count();
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
        let mut available_regs =
            crate::backend::generation::filter_available_regs(callee_base, &asm_clobbered_regs);
        // A GNU non-local goto bypasses this nested function's epilogue, so a
        // value assigned to rbx/r12..r15 would overwrite the parent's live
        // callee-saved value permanently. Keep this rare function class out of
        // the callee-saved pool; caller-saved homes and slots remain valid.
        let has_nonlocal_goto = func.blocks.iter().any(|block| {
            block
                .instructions
                .iter()
                .any(|inst| matches!(inst, Instruction::NonlocalGoto { .. }))
        });
        if has_nonlocal_goto {
            available_regs.clear();
        }

        let mut caller_saved_regs = X86_CALLER_SAVED.to_vec();
        let mut has_indirect_call = false;
        let mut has_calls = false;
        let mut has_i128_ops = false;
        let mut has_atomic_rmw = false;
        // Track rdx-clobbering patterns for conditional rdx allocation
        let mut has_div_rem = false;
        let mut has_gep = false; // GEP → indirect stores → emit_save_acc uses rdx
        let mut has_switch = false; // Switch → jump tables use rdx
        let mut has_select = false; // Select → cmov path uses rdx
        let mut has_rdx_intrinsic = false; // Fixed-scratch intrinsic paths overwrite rdx
        let mut has_i32_widening = false; // Cast from I32/U32 to I64/pointer → needs sign-ext
        for block in &func.blocks {
            for inst in &block.instructions {
                match inst {
                    Instruction::Call { .. } => {
                        has_calls = true;
                    }
                    Instruction::CallIndirect { .. } => {
                        has_calls = true;
                        has_indirect_call = true;
                    }
                    Instruction::BinOp { op, ty, .. } => {
                        if matches!(ty, IrType::I128 | IrType::U128) {
                            has_i128_ops = true;
                        }
                        if matches!(
                            op,
                            IrBinOp::SDiv | IrBinOp::UDiv | IrBinOp::SRem | IrBinOp::URem
                        ) {
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
                            || matches!(to_ty, IrType::I128 | IrType::U128)
                        {
                            has_i128_ops = true;
                        }
                        // Detect I32/U32 widening to 64-bit: requires sign-extension.
                        if matches!(from_ty, IrType::I32 | IrType::U32)
                            && matches!(to_ty, IrType::I64 | IrType::U64 | IrType::Ptr)
                        {
                            has_i32_widening = true;
                        }
                    }
                    Instruction::Cmp { ty, .. } | Instruction::Store { ty, .. } => {
                        if matches!(ty, IrType::I128 | IrType::U128) {
                            has_i128_ops = true;
                        }
                    }
                    Instruction::AtomicRmw { .. } => {
                        has_atomic_rmw = true;
                    }
                    Instruction::GetElementPtr { .. } => {
                        has_gep = true;
                    }
                    Instruction::Select { .. } => {
                        has_select = true;
                    }
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
                                | IntrinsicOp::FmaF64x4HoistedSIB
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
            caller_saved_regs.retain(|r| r.0 != 12 && r.0 != 13 && r.0 != 14 && r.0 != 15);
            // r8, r9, rdi, rsi
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
        if crate::backend::regalloc::x86_param_caller_homes_safe(func) {
            let preferred = [14u8, 15, 16, 12, 13, 10, 11];
            caller_saved_regs.sort_by_key(|reg| {
                preferred
                    .iter()
                    .position(|&id| id == reg.0)
                    .unwrap_or(preferred.len())
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
        let is_64 =
            |t: &IrType| matches!(t, IrType::I64 | IrType::U64 | IrType::Ptr | IrType::F128);
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
                        mark(lhs, &mut needs_sext_set);
                        mark(rhs, &mut needs_sext_set);
                    }
                    Instruction::Cmp { ty, lhs, rhs, .. } if is_64(ty) => {
                        mark(lhs, &mut needs_sext_set);
                        mark(rhs, &mut needs_sext_set);
                    }
                    Instruction::Store { ty, val, .. } if is_64(ty) => {
                        mark(val, &mut needs_sext_set);
                    }
                    Instruction::Cast {
                        src,
                        from_ty,
                        to_ty,
                        ..
                    } if matches!(from_ty, IrType::I32 | IrType::U32) && is_64(to_ty) => {
                        mark(src, &mut needs_sext_set);
                    }
                    Instruction::GetElementPtr { offset, .. } => {
                        mark(offset, &mut needs_sext_set);
                    }
                    Instruction::Select {
                        ty,
                        true_val,
                        false_val,
                        ..
                    } if is_64(ty) => {
                        mark(true_val, &mut needs_sext_set);
                        mark(false_val, &mut needs_sext_set);
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
                if let Instruction::AtomicCmpxchg {
                    expected,
                    desired,
                    ty,
                    ..
                } = inst
                {
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

        // PF-16b: Clz/Ctz/Popcount on <=32-bit types produce results in
        // [0, bitwidth] — bit 31 is provably zero, so their widening casts
        // can take the unsigned (movl) path (see bitop_nonneg_values).
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
            for block in &func.blocks {
                for inst in &block.instructions {
                    for_each_operand_in_instruction(inst, |op| {
                        if let Operand::Value(v) = op {
                            *use_counts.entry(v.0).or_insert(0) += 1;
                        }
                    });
                }
                for_each_operand_in_terminator(&block.terminator, |op| {
                    if let Operand::Value(v) = op {
                        *use_counts.entry(v.0).or_insert(0) += 1;
                    }
                });
            }
            // Spill/reload WIDTH comes from the shared type map, so the stack
            // layout's slot sizing and the emitters' access width provably
            // agree: a value whose Copy/Phi-propagated type is 64-bit must not
            // be narrowed to a 4-byte slot that a `movq` reload would read
            // past (the -O2 preboot-ZSTD "compressed data is corrupt"
            // miscompile).  See `backend::common::compute_value_type_map`.
            let value_types = crate::backend::common::compute_value_type_map(func);

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
                    if let Instruction::Cmp { dest, ty, op, .. } = inst {
                        if use_counts.get(&dest.0).copied().unwrap_or(0) != 1 {
                            continue;
                        }
                        // Wide ints (I128/U128) route to emit_i128_cmp and
                        // never participate in pending_cmp. Float compares
                        // now fuse for the RELATIONAL operators only
                        // (Sgt/Sge/Slt/Sle and unsigned peers): ucomisd sets
                        // CF/ZF/PF such that `ja`/`jae`/`jb`/`jbe` give the
                        // correct ordered result (NaN → unordered → false on
                        // all of them). Eq/Ne need the parity bit, so they
                        // stay materialized (setnp+sete / setp+setne) and are
                        // NOT fused here.
                        if crate::backend::generation::is_wide_int_type(*ty) {
                            continue;
                        }
                        let is_fp = matches!(
                            ty,
                            crate::common::types::IrType::F32 | crate::common::types::IrType::F64
                        );
                        if is_fp {
                            let relational = matches!(
                                op,
                                crate::ir::reexports::IrCmpOp::Sgt
                                    | crate::ir::reexports::IrCmpOp::Sge
                                    | crate::ir::reexports::IrCmpOp::Slt
                                    | crate::ir::reexports::IrCmpOp::Sle
                                    | crate::ir::reexports::IrCmpOp::Ugt
                                    | crate::ir::reexports::IrCmpOp::Uge
                                    | crate::ir::reexports::IrCmpOp::Ult
                                    | crate::ir::reexports::IrCmpOp::Ule
                            );
                            if !relational {
                                continue;
                            }
                        } else if !ty.is_integer() {
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
                                Instruction::Copy {
                                    dest: cd,
                                    src: Operand::Value(sv),
                                } if sv.0 == cur => {
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
                                Instruction::Cast {
                                    dest: cd,
                                    src: Operand::Value(sv),
                                    from_ty,
                                    to_ty,
                                } if sv.0 == cur && from_ty.is_integer() && to_ty.is_integer() => {
                                    if use_counts.get(&cd.0).copied().unwrap_or(0) != 1 {
                                        chain_single_use = false;
                                        break;
                                    }
                                    forward_dests.push(cd.0);
                                    cur = cd.0;
                                    k += 1;
                                }
                                // IS-07: a flag-neutral, side-effect-free,
                                // boolean-independent data movement between
                                // the Cmp and its Select consumer does not
                                // disturb the fusion — the flags survive
                                // mov/movz/movs/lea untouched, and the Cmp's
                                // boolean is still consumed by the SAME
                                // select (use_counts==1 on the Cmp dest is
                                // checked before the walk starts). This is
                                // the `limit` clamp shape:
                                //   cmp x>100; movslq x, x64; select(100, x64)
                                // which previously lost the fusion to the
                                // intervening widening move and fell back to
                                // setcc+movzbl+testq+cmovne (4 extra
                                // instructions). These instructions are NOT
                                // boolean forwarding (their values must and
                                // do emit normally) — they are transparent
                                // to the handshake, so they are neither
                                // chained nor added to fused_forward.
                                // Accepted kind: a widening integer cast of
                                // an UNRELATED value — its emission (any
                                // path: mature direct move, accumulator
                                // staging, or a MachInst-queued movslq) is
                                // pure data movement and leaves EFLAGS
                                // untouched. Deliberately NOT accepted:
                                // UnaryOp (Neg/Not write flags), BinOp
                                // (writes flags), GEP (scaled indices may
                                // emit imul — writes flags), anything that
                                // touches memory or the pending boolean.
                                Instruction::Cast {
                                    src: Operand::Value(sv),
                                    from_ty: ft,
                                    to_ty: tt,
                                    ..
                                } if sv.0 != cur
                                    && ft.is_integer()
                                    && tt.is_integer()
                                    && tt.size() > ft.size() =>
                                {
                                    k += 1;
                                }
                                Instruction::Select {
                                    cond: Operand::Value(v),
                                    ..
                                } if v.0 == cur => {
                                    // The copy chain must terminate here (the
                                    // final value has exactly one use: this
                                    // select). FP relational Cmps may NOT
                                    // fuse into a Select: the Select consumer
                                    // reads `pending_cmp` (integer jcc), not
                                    // `pending_fp_cmp`, so fusing would skip
                                    // the boolean materialization while the
                                    // Select still expects to read it — a
                                    // miscompile. FP fusion is CondBranch-only.
                                    if !is_fp && use_counts.get(&cur).copied().unwrap_or(0) == 1 {
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
                        } else if !is_consumer
                            && chain_single_use
                            && k == insts.len()
                            && insts.len() >= 1
                        {
                            // Check the block terminator as the consumer.
                            if let Terminator::CondBranch {
                                cond: Operand::Value(v),
                                ..
                            } = &block.terminator
                            {
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
            // Candidate selection lives in
            // comparison::compute_cmp_replay_scan — the SAME function the
            // RA-link builder consumes below, so allocator and emitter can
            // never disagree about which operands are read at the consumer
            // position.
            //
            // Home soundness is decided POST-RA (prune_replay_entries): a
            // register-homed operand is safe because its interval was
            // extended to the consumer position through RegAllocConfig::
            // folded_index_uses; a slot-homed operand is always safe; an
            // operand with no home at all (never-materialized) prunes the
            // entry (the Cmp then materializes its boolean normally).
            let replay_scan = super::comparison::compute_cmp_replay_scan(func, &use_counts, &fused);
            self.cmp_replay_operand_links = replay_scan.operand_links;
            self.cmp_replay = replay_scan.replay;
            // FP-SELECT (S05): float Cmps whose boolean feeds only Selects.
            // The Cmp emitter skips the ucomisd/setcc boolean entirely; every
            // select re-derives a vcmpsd/vcmpss mask from the recorded
            // operands and blends the FP arms with vblendvpd/vblendvps (gcc
            // emits exactly this). Pruned post-RA like cmp_replay: a select
            // whose operands became unreadable falls back to re-materializing
            // the boolean (emit_fp_cmp_boolean) and the ordinary cmov path.
            self.fp_select_cmps = replay_scan.fp_select;

            self.fused_cmp_dests = fused;
            self.fused_forward_dests = fused_forward;
            self.value_use_counts = use_counts;

            self.value_types = value_types;

            // Values whose 16-byte i128 slot home is written EAGERLY at
            // their definition: the ParamRef of an i128 parameter (the
            // prologue's param pre-store spills the incoming pair) and the
            // result of a call returning a 16-bit... a 128-bit aggregate
            // (the call-result machinery writes the pair to the slot).
            // These are the only slot-homed i128 values the typed store
            // route may READ: a value computed into the accumulator pair
            // by arithmetic or an inlined vector builtin has no slot
            // writer of its own — its slot is stale until a mature-path
            // store materializes it (builtin_ia32_vector_value regression:
            // the inlined maxps/minps result read zeros through the
            // un-written slot).
            let mut coherent_i128_homes = crate::common::fx_hash::FxHashSet::default();
            // (dest, source-value) edges for the Copy/Load propagation pass.
            let mut home_writes: Vec<(u32, Option<u32>)> = Vec::new();
            for block in &func.blocks {
                for inst in &block.instructions {
                    match inst {
                        Instruction::ParamRef {
                            dest, param_idx, ..
                        } => {
                            let is_i128_param =
                                self.state.param_classes.get(*param_idx).is_some_and(|c| {
                                    matches!(
                                        c,
                                        crate::backend::call_abi::ParamClass::I128RegPair { .. }
                                    )
                                });
                            if is_i128_param {
                                coherent_i128_homes.insert(dest.0);
                            }
                        }
                        Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
                            if let Some(dest) = info.dest {
                                if matches!(info.return_type, IrType::I128 | IrType::U128) {
                                    coherent_i128_homes.insert(dest.0);
                                }
                            }
                        }
                        // A Copy from a coherent source eagerly writes the
                        // dest's home (the mature i128 copy materializes
                        // the pair into the dest slot); a load's dest home
                        // is written by the load itself. Propagate to a
                        // fixpoint so Copy chains stay coherent.
                        Instruction::Copy { dest, src } => {
                            if let Operand::Value(v) = src {
                                home_writes.push((dest.0, Some(v.0)));
                            }
                        }
                        Instruction::Load {
                            dest,
                            ty,
                            seg_override: _,
                            ..
                        } if matches!(ty, IrType::I128 | IrType::U128) => {
                            home_writes.push((dest.0, None));
                        }
                        _ => {}
                    }
                }
            }
            loop {
                let mut grew = false;
                for (dest, src) in &home_writes {
                    let coherent = match src {
                        Some(v) => coherent_i128_homes.contains(v),
                        None => true,
                    };
                    if coherent && coherent_i128_homes.insert(*dest) {
                        grew = true;
                    }
                }
                if !grew {
                    break;
                }
            }
            self.coherent_i128_homes = coherent_i128_homes;
        }

        // Direct scalar global accesses and safely reconstructible global
        // address roots never need a register or stack home. The former become
        // `sym(%rip)` Load/Store operands; the latter are recreated only at
        // audited Add/Sub/GEP uses as `leaq sym(%rip)` plus the live offset.
        // Filtering GOT/TLS identities before either analysis is the critical
        // fail-closed contract: full PIC and weak PIE externs still materialize.
        let never_materialized = {
            let mut gmap = crate::backend::generation::build_global_addr_map_for(
                func,
                &self.state.tls_symbols,
            );
            gmap.retain(|_, sym| {
                // Strip a composed constant displacement. Symbol names emitted
                // by C cannot contain '+'; '-' here is only the map's suffix.
                let base = sym.split(['+', '-']).next().unwrap_or(sym);
                !self.state.needs_got_for_addr(base)
                    && !self.state.tls_symbols.contains(base)
                    && !self.state.absolute_symbols.contains(base)
            });
            let mut set =
                crate::backend::generation::build_foldable_global_addr_set_for(func, &gmap);
            set.extend(
                crate::backend::generation::build_rematerializable_global_addr_set_for(func, &gmap),
            );

            // PF-07: rematerializing an indexed symbol base re-emits
            // `leaq sym(%rip), %rcx` at EVERY access. Non-PIC code pays
            // nothing (the absolute `sym(,%idx,4)` SIB form needs no base
            // register), but PIC code pays one lea per iteration of any loop
            // containing the access. When the GlobalAddr is defined at a
            // STRICTLY SHALLOWER loop depth than an indexed consumer
            // (the hoisted-address, table-driven-loop shape: CRC-32 tables,
            // expat's name lookup, hash tables), keep it RA-eligible so it
            // gets a register home and the SIB load reads the home directly
            // — exactly what GCC hoists for this shape.
            let mut promoted: FxHashSet<u32> = FxHashSet::default();
            if self.state.pic_mode && !set.is_empty() {
                let depths = global_addr_loop_depths(func, &set);
                let mut promote: crate::common::fx_hash::FxHashSet<u32> =
                    crate::common::fx_hash::FxHashSet::default();
                for (block_idx, block) in func.blocks.iter().enumerate() {
                    let consumer_depth =
                        depths.get(block_idx).copied().unwrap_or_else(|| usize::MAX);
                    for inst in &block.instructions {
                        let Instruction::GetElementPtr {
                            base,
                            offset: Operand::Value(_),
                            ..
                        } = inst
                        else {
                            continue;
                        };
                        if !set.contains(&base.0) {
                            continue;
                        }
                        // The base's def block depth: find where the
                        // GlobalAddr (or its chain root) is defined.
                        let def_depth = gaddr_def_block(func, base.0, &gmap)
                            .and_then(|b| depths.get(b).copied())
                            .unwrap_or(consumer_depth);
                        if consumer_depth > def_depth {
                            promote.insert(base.0);
                        }
                    }
                }
                for v in &promote {
                    set.remove(v);
                }
                promoted = promote;
            }

            self.state.promoted_global_addr_homes = promoted;
            self.state.never_materialized_values = set.clone();
            set
        };

        // Lever-1 port from i686 (session 23, extended session 24):
        // immediately-consumed values flow through the accumulator WITHOUT a
        // home. x86-64's store_rax_to has the immediately_consumed skip
        // branch and operand_to_rax / operand_to_callee_reg consult the acc
        // cache, so denying the home removes the `movq %rax,%reg`
        // materialisation + the consumer's home read, freeing a register.
        //
        // SOUNDNESS (why this is scoped per consumer class): unlike i686,
        // x86-64 codegen is NOT uniformly accumulator-centric — BinOps take a
        // register-direct path, and the acc cache holds only ONE value, so a
        // consumer that reads a home register instead of the accumulator
        // would read garbage for a home-less value (session 23: the full set
        // corrupted live pointers, simd_sse2_arith SIGSEGV).  Each consumer
        // class is therefore enabled only after its emission path is proven
        // to read through the accumulator.  Classes are selected via
        // CCC_X64_NOHOME_CLASSES (comma-separated subset of
        // ret,store,copy,cast,unary,binop,cmp); default "ret" (the proven
        // class).  "all" enables every class (for auditing only).
        //
        // DEFAULT (session-24 per-class audit on the 352-test battery):
        //   ret, store, copy, cast, unary, binop  — each 352+6 clean.
        //   cmp  — EXCLUDED: the COMPARE-REPLAY mechanism re-emits a skipped
        //          Cmp at its (possibly later) consumer; a home-less LHS only
        //          lives in %rax at the original point, so a replayed compare
        //          reads a clobbered accumulator (simd_sse2_arith SIGSEGV).
        let never_materialized = if std::env::var_os("CCC_NO_X64_IMMED_NOHOME").is_some() {
            never_materialized
        } else {
            let classes = std::env::var("CCC_X64_NOHOME_CLASSES")
                .unwrap_or_else(|_| "ret,store,copy,cast,unary,binop".into());
            let has = |c: &str| classes == "all" || classes.split(',').any(|x| x.trim() == c);
            let skip: crate::common::fx_hash::FxHashSet<u32> =
                crate::backend::regalloc::analyze_accumulator_assignments(
                    func,
                    crate::backend::regalloc::AccumulatorPolicy {
                        operand_order: crate::backend::regalloc::AccumulatorOperandOrder::LhsFirst,
                        return_consumes_accumulator: false,
                    },
                )
                .into_iter()
                .map(|a| a.value_id)
                .collect();
            if skip.is_empty() {
                never_materialized
            } else {
                use crate::ir::reexports::{Instruction, Operand, Terminator};
                // Classify every value by its (single) consumer instruction.
                let mut cls_of: crate::common::fx_hash::FxHashMap<u32, &'static str> =
                    crate::common::fx_hash::FxHashMap::default();
                for block in &func.blocks {
                    for inst in &block.instructions {
                        let mut mark = |op: &Operand, cls: &'static str| {
                            if let Operand::Value(v) = op {
                                cls_of.insert(v.0, cls);
                            }
                        };
                        match inst {
                            Instruction::Store { val, .. } => mark(val, "store"),
                            Instruction::Cast { src, .. } => mark(src, "cast"),
                            Instruction::UnaryOp { src, .. } => mark(src, "unary"),
                            Instruction::Copy { src, .. } => mark(src, "copy"),
                            Instruction::BinOp { lhs, .. } => mark(lhs, "binop"),
                            Instruction::Cmp { lhs, .. } => mark(lhs, "cmp"),
                            _ => {}
                        }
                    }
                    if let Terminator::Return(Some(Operand::Value(v))) = &block.terminator {
                        cls_of.insert(v.0, "ret");
                    }
                }
                // A narrowing cast is a semantic operation even when its only
                // consumer immediately widens the result again. Its destination
                // must retain a home: otherwise the cast->cast no-home handoff
                // can expose the pre-truncation register value to the widening
                // cast. zlib-ng's bi_reverse hit exactly U32->U8->U64 and used
                // 8188 instead of (uint8_t)8188, corrupting Huffman codes.
                let mut narrowing_cast_values = crate::common::fx_hash::FxHashSet::default();
                for block in &func.blocks {
                    for inst in &block.instructions {
                        if let Instruction::Cast {
                            dest,
                            src,
                            from_ty,
                            to_ty,
                            ..
                        } = inst
                        {
                            if from_ty.is_integer()
                                && to_ty.is_integer()
                                && to_ty.size() < from_ty.size()
                            {
                                // Protect both sides. Keeping only the result
                                // homed fixes optimized register-direct chains;
                                // keeping the input homed also prevents the O0
                                // load/cast handoff from redirecting a full-width
                                // load into the narrow result without emitting
                                // the truncation.
                                narrowing_cast_values.insert(dest.0);
                                if let Operand::Value(src) = src {
                                    narrowing_cast_values.insert(src.0);
                                }
                            }
                        }
                    }
                }
                // A Cmp dest consumed by an integer Cast/Copy is classified
                // as "cast"/"copy" and would otherwise lose its home. The
                // Cmp emitter then skips setcc (nothing to write) and the
                // Cast reads the compare's *operand* register — typically
                // the size/level being tested — instead of 0/1.
                // zlib-ng `zng_deflateSetParams`: `int32_t buf_error =
                // param->size < min_size` became `buf_error = size`, so a
                // 4-byte buffer was reported as Z_BUF_ERROR.
                // Flag fusion (`fused_cmp_dests`) is the only legal way to
                // skip materializing a Cmp; nohome must not take that
                // decision for a compare result.
                let mut cmp_dests = crate::common::fx_hash::FxHashSet::default();
                for block in &func.blocks {
                    for inst in &block.instructions {
                        if let Instruction::Cmp { dest, .. } = inst {
                            cmp_dests.insert(dest.0);
                        }
                    }
                }
                let mut set = never_materialized;
                set.extend(skip.into_iter().filter(|v| {
                    !cmp_dests.contains(v)
                        && !narrowing_cast_values.contains(v)
                        && cls_of.get(v).is_some_and(|c| has(c))
                }));
                set
            }
        };

        // SysV AMD64 argument registers present in the caller-saved pool
        // (r8, r9, rdi, rsi and — when enabled — rdx). A value consumed as a
        // call argument must not be homed in one of these: the staging writes
        // them in order before reading the value (`printf("%d %d", add(3,4),
        // mul(3,4))` read the mul result out of the format-string register).
        if self.state.disable_regalloc {
            available_regs.clear();
            caller_saved_regs.clear();
        }
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
        let (reg_assigned, cached_liveness, caller_save_spans, accumulator_assignments) =
            crate::backend::stack_layout::run_regalloc_and_merge_clobbers_ex(
                func,
                available_regs,
                caller_saved_regs,
                &asm_clobbered_regs,
                &mut self.reg_assignments,
                &mut self.used_callee_saved,
                false,
                Some(never_materialized),
                call_arg_regs,
                indirect_target_regs,
                // Session 28: x86-64 emits SIB indexed addressing directly at
                // the Load/Store (emit_load_indexed/emit_store_indexed) AND
                // folds constant-offset GEPs with register bases
                // (const_offset_fold_reg_base_ok): BOTH forms consume address
                // registers RA-invisibly at the access position.
                // collect_folded_gep_links_all extends const-fold base intervals
                // plus indexed-fold base AND index intervals to their consumers,
                // so every address register survives intervening calls and value
                // staging (zlib-ng gz_reset NULL-store crash class).
                {
                    // CMP-REPLAY operand reads (IS-09): the consumer re-emits
                    // the comparison at the SELECT/CondBranch position, so a
                    // REGISTER-homed operand's interval must extend to the
                    // consumer — otherwise the allocator frees the register
                    // at the original Cmp position and a later-defined value
                    // reuses it (sqlite3 yy_shift compared state+415 instead
                    // of state). The links map each operand to its Cmp dest,
                    // whose single use marks the replay position; the
                    // extension then treats the replayed read exactly like
                    // the folded-GEP address reads this map already covers.
                    // Links are a SUPERSET of what the emitter finally
                    // accepts (built with an empty fused set is not needed:
                    // the scan skipped fused dests, and those operands keep
                    // their normal adjacency-synchronized ranges) — a
                    // retained link for a pruned entry only over-constrains
                    // the allocator, never under-constrains.
                    let mut links = crate::backend::generation::collect_folded_gep_links_all(func);
                    for (operand, dests) in &self.cmp_replay_operand_links {
                        links
                            .entry(*operand)
                            .or_default()
                            .extend(dests.iter().copied());
                    }
                    links
                },
            );

        // ── CMP-REPLAY post-RA home pruning (IS-09) ─────────────────────────
        // The scan accepted every Value operand optimistically; now that
        // homes are final, keep only entries whose operands are readable at
        // the replay point:
        //   * pre-existing stack slot (param/alloca)  — always safe: the
        //     slot is written at the definition and never reused while the
        //     SSA value is live;
        //   * register assignment — safe because the operand's interval was
        //     extended to the consumer position via the folded_index links
        //     (unless the extension is disabled: CCC_NO_FOLDED_INDEX_LIVENESS
        //     removes the guarantee, so reg-homed operands revert to
        //     refused);
        //   * accumulator-located (immediately-consumed) values — NOT
        //     readable: the producer leaves the value in %rax for the
        //     ADJACENT consumer only (store_rax_to skips the home store and
        //     slot assignment skips the slot), so at the replay distance
        //     there is no register, no slot and (after the replay's
        //     invalidate_acc) no cache entry — reading one would ICE or, if
        //     a slot was speculatively assigned, silently load a
        //     never-written slot. The canonical case: a widening Cast
        //     feeding a Cmp whose Select consumer is not adjacent
        //     (io_uring kbuf io_ring_buffers_peek: `Cast U16→U64` → `Cmp
        //     Ugt` → intervening `Cast U64→U16` → `Select`) — the Cmp is
        //     replay-deferred and its zext operand becomes unreachable.
        //     Prune the replay: the Cmp then emits at its own position,
        //     where the adjacency contract holds and the acc-cache read is
        //     valid;
        //   * neither — safe only when the allocator will hand the value a
        //     spill slot (every non-never-materialized value gets one).
        //     never-materialized values have no home at all: reading one at
        //     the replay would consume stale register/slot state — prune.
        {
            let ext_active = std::env::var_os("CCC_NO_FOLDED_INDEX_LIVENESS").is_none();
            // The RA-verified accumulator assignments: single-use values
            // whose consumer is the immediately following instruction.
            // These are exactly the values stack layout will give an
            // `ExplicitLocation::Accumulator` (no home store). A register
            // home still wins (checked first below), so only the
            // register-less subset is refused.
            let acc_no_home: crate::common::fx_hash::FxHashSet<u32> =
                accumulator_assignments.iter().map(|a| a.value_id).collect();
            let mut prune: Vec<u32> = Vec::new();
            for (cdest, (_op, lhs, rhs, _ty)) in self.cmp_replay.iter() {
                let readable = |op: &Operand| -> bool {
                    match op {
                        Operand::Const(_) => true,
                        Operand::Value(v) => {
                            if self.state.get_slot(v.0).is_some() {
                                return true;
                            }
                            if self.reg_assignments.contains_key(&v.0) {
                                return ext_active;
                            }
                            if acc_no_home.contains(&v.0) {
                                return false;
                            }
                            !self.state.never_materialized_values.contains(&v.0)
                        }
                    }
                };
                if !readable(lhs) || !readable(rhs) {
                    prune.push(*cdest);
                }
            }
            for d in prune {
                self.cmp_replay.remove(&d);
            }
            // ── FP-SELECT post-RA prune (S05) ───────────────────────────────
            // Same readability contract as the replay prune above: a select
            // can re-derive the blend mask only if both operands are still
            // readable at the select position (Const, pre-existing slot,
            // register home with the folded-index extension active, or a
            // materialized slot; accumulator-only and never-materialized
            // operands are out). Non-readable → the entry is dropped: the
            // Cmp then materializes its boolean at its own position and the
            // select uses the ordinary materialized path (sound, no blend).
            {
                let ext_active = std::env::var_os("CCC_NO_FOLDED_INDEX_LIVENESS").is_none();
                let acc_no_home: crate::common::fx_hash::FxHashSet<u32> =
                    accumulator_assignments.iter().map(|a| a.value_id).collect();
                let mut prune_fp: Vec<u32> = Vec::new();
                for (cdest, (_op, lhs, rhs, _ty)) in self.fp_select_cmps.iter() {
                    let readable = |op: &Operand| -> bool {
                        match op {
                            Operand::Const(_) => true,
                            Operand::Value(v) => {
                                if self.state.get_slot(v.0).is_some() {
                                    return true;
                                }
                                if self.reg_assignments.contains_key(&v.0) {
                                    return ext_active;
                                }
                                if acc_no_home.contains(&v.0) {
                                    return false;
                                }
                                !self.state.never_materialized_values.contains(&v.0)
                            }
                        }
                    };
                    if !readable(lhs) || !readable(rhs) {
                        prune_fp.push(*cdest);
                    }
                }
                for d in prune_fp {
                    self.fp_select_cmps.remove(&d);
                }
            }
        }

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
        // Diagnostic knob (2026-09-02, kernel-boot zstd hunt): comma-separated
        // name substrings; functions matching any of them are forced onto the
        // classic backend. Lets a miscompiled function be bisected without
        // touching the source (source edits perturb layout and flip layout-
        // sensitive miscompiles on/off).
        let fn_disable = std::env::var("CCC_MI_FN_DISABLE").unwrap_or_default();
        let fn_disabled = !fn_disable.is_empty()
            && fn_disable
                .split(',')
                .any(|pat| !pat.is_empty() && func.name.contains(pat));
        // Inverse diagnostic knob: force MachInst ON for matching functions
        // even under CCC_NO_MACHINST=1. Combined with an otherwise-classic
        // build, a FAIL introduced by forcing exactly one function proves that
        // function's own MachInst codegen is the miscompile (layout
        // coincidences cannot create a bug when only one function changes).
        // CCC_MI_ALL_CLASSIC gives an all-classic baseline WITHOUT the
        // CCC_NO_MACHINST side effects (it only zeroes the per-function gate,
        // so a CCC_MI_FN_FORCE'd function still flows through the MachInst
        // emitter, and the MachInst-incompatible peephole phase stays off).
        let all_classic = std::env::var("CCC_MI_ALL_CLASSIC").is_ok();
        let fn_force = std::env::var("CCC_MI_FN_FORCE").unwrap_or_default();
        let fn_forced = !fn_force.is_empty()
            && fn_force
                .split(',')
                .any(|pat| !pat.is_empty() && func.name.contains(pat));
        self.machinst_function_enabled = (self.machinst_enabled
            && !all_classic
            && (loop_insts <= max_loop_insts || std::env::var("CCC_MI_FORCE_LOOPS").is_ok())
            && !fn_disabled)
            || fn_forced;
        if std::env::var("CCC_MI_DEBUG").is_ok() {
            eprintln!(
                "[MI-PROFIT] fn={} loop_insts={} limit={} enabled={} fn_disabled={} fn_forced={}",
                func.name,
                loop_insts,
                max_loop_insts,
                self.machinst_function_enabled,
                fn_disabled,
                fn_forced
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
        let mut cast_by_src: FxHashMap<u32, (Value, IrType, IrType)> = FxHashMap::default();
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Instruction::Cast {
                    dest,
                    src: Operand::Value(sv),
                    from_ty,
                    to_ty,
                } = inst
                {
                    cast_by_src.entry(sv.0).or_insert((*dest, *from_ty, *to_ty));
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
                        let ok_ty = matches!(ty, IrType::U8 | IrType::U16 | IrType::U32)
                            && *seg_override == AddressSpace::Default;
                        let single_use =
                            self.value_use_counts.get(&dest.0).copied().unwrap_or(0) == 1;
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
                                    if let Some(reg) = self.reg_assignments.get(&cd.0).copied() {
                                        if !is_xmm_reg(reg) {
                                            // Register-free-at-load guard (RA-05:
                                            // segment-aware). Folding loads the
                                            // cast dest's register at `load_pp`;
                                            // any OTHER value that is genuinely
                                            // live there and shares that register
                                            // would be clobbered. Segment-shared
                                            // partners are dead at `load_pp` by
                                            // construction (that is why the scan
                                            // let them share), so the test uses
                                            // hole-aware pieces: a fat-interval
                                            // test refused the fold whenever ANY
                                            // holder's hull covered the load,
                                            // re-introducing the load→rax→reg
                                            // staging the fold exists to delete
                                            // (adler32 DO8: `movzbl (%rbp),%eax;
                                            // movq %rax,%r12` per byte). Values
                                            // WITHOUT segment data keep the
                                            // conservative fat test.
                                            let mut seg_conflict = false;
                                            'outer: for seg in cached_liveness
                                                .iter()
                                                .flat_map(|l| l.segments.iter())
                                            {
                                                if seg.value_id == cd.0
                                                    || !(seg.start <= load_pp && load_pp <= seg.end)
                                                {
                                                    continue;
                                                }
                                                if let Some(&r) =
                                                    self.reg_assignments.get(&seg.value_id)
                                                {
                                                    if r == reg {
                                                        seg_conflict = true;
                                                        break 'outer;
                                                    }
                                                }
                                            }
                                            // Conservative fallback: any assigned
                                            // value whose FAT hull covers the
                                            // load and that never appears in the
                                            // segment list still blocks the fold.
                                            let mut fat_conflict = false;
                                            for &(vid, s, e) in &intervals {
                                                if vid != cd.0 && s <= load_pp && load_pp <= e {
                                                    if let Some(&r) = self.reg_assignments.get(&vid)
                                                    {
                                                        if r == reg
                                                            && !cached_liveness
                                                                .as_ref()
                                                                .is_some_and(|l| {
                                                                    l.segments.iter().any(|sg| {
                                                                        sg.value_id == vid
                                                                    })
                                                                })
                                                        {
                                                            fat_conflict = true;
                                                            break;
                                                        }
                                                    }
                                                }
                                            }
                                            if !seg_conflict && !fat_conflict {
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
        // Capture before `self.state` is mutably borrowed by the layout closure.
        let fpo = self.state.omit_frame_pointer;
        self.state.ra_accumulator_values =
            accumulator_assignments.iter().map(|a| a.value_id).collect();
        let mut space = calculate_stack_space_common(
            &mut self.state,
            func,
            callee_save_reserve,
            |space, alloc_size, align| {
                // 4-byte granularity for 4-byte spill slots (small values):
                // rounding them to 8 would defeat the frame-size halving that
                // fixes pcre2-style stack overflow in deep recursion. Slots
                // wider than 4 bytes keep the 8-byte rounding; alignment
                // requests of 16+ (over-aligned allocas, vectors) are honoured
                // exactly as before.
                let is_small_alloc = alloc_size <= 4;
                let effective_align = if align > 0 {
                    align.max(8)
                } else if is_small_alloc {
                    4
                } else {
                    8
                };
                let alloc = if is_small_alloc {
                    4
                } else {
                    (alloc_size + 7) & !7
                };
                let mut new_space =
                    ((space + alloc + effective_align - 1) / effective_align) * effective_align;
                // FPO: the virtual frame base (rsp + frame_size) is entry %rsp,
                // which is ≡ 8 (mod 16) because the caller's CALL pushed the return
                // address. A slot assigned here at offset ≡ 0 (mod A) therefore
                // lands at ≡ 8 (mod 16) — misaligned for any A >= 16. Shifting the
                // slot by 8 makes the final address (entry_rsp + offset) ≡
                // (8 + A-8) = 0 (mod A), exactly right for every power-of-two
                // alignment >= 16 (16, 32, 64, ...). (rbp mode is unaffected:
                // %rbp = entry_rsp - 8 is already 16-aligned.)
                if fpo && effective_align >= 16 {
                    new_space += 8;
                }
                (-new_space, new_space)
            },
            &reg_assigned,
            &X86_CALLEE_SAVED,
            cached_liveness,
        );

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

        // Callee-saved registers in a call-free leaf are already represented
        // by push/pop. If layout consumed no bytes beyond the conservative
        // collision reserve, there are no local slots and no call-alignment
        // obligation, so a second local frame is pure overhead. Keep the reserve
        // for calls, varargs, and whenever any real slot was allocated.
        if !func.is_variadic
            && !self.func_has_calls
            && space == callee_save_reserve
            && std::env::var_os("CCC_NO_EMPTY_LOCAL_FRAME_ELISION").is_none()
        {
            0
        } else {
            space
        }
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
            if aligned % 16 == 0 {
                aligned + 8
            } else {
                aligned
            }
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
        self.func_set_second_ret = false;
        self.func_ret_is_f128_sse = func.ret_is_f128_sse;
        if self.state.cf_protection_branch {
            self.state.emit("    endbr64");
        }

        // Variadic functions no longer force a frame pointer: va_start/va_arg
        // address the register save area and overflow area %rsp-relative via
        // the same slot_ref / emit_instr_*_rbp machinery (which adds the frame
        // size in FPO mode). The 16-byte reg-save slots only ever use
        // unaligned moves (movdqu/movsd), so no extra alignment is required.
        let omit_fp = self.state.omit_frame_pointer;
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
                    self.state
                        .out
                        .emit_instr_imm_reg("    subq", local_size, "rsp");
                }
            } else {
                if frame_size > 0 {
                    self.state
                        .out
                        .emit_instr_imm_reg("    subq", frame_size, "rsp");
                }
                for (i, &reg) in used_regs.iter().enumerate() {
                    let reg_name = phys_reg_name(reg);
                    let rsp_offset = frame_size - (i as i64 + 1) * 8;
                    self.state
                        .emit_fmt(format_args!("    movq %{}, {}(%rsp)", reg_name, rsp_offset));
                }
            }
            let actual_stack = frame_size.max(n_saves * 8);
            if self.state.emit_cfi {
                self.state
                    .emit_fmt(format_args!("    .cfi_def_cfa_offset {}", actual_stack + 8));
            }
            self.state.out.use_rsp_addressing = true;
            // RSP-relative addressing models a virtual frame base at the
            // caller's entry %rsp.  When the empty-local-frame elision keeps a
            // leaf function at `frame_size == 0` but the register allocator
            // still saved callee-saved homes with push/pop, the live %rsp is
            // below entry by `n_saves * 8`.  Positive references to stack
            // parameters therefore need the save area in the displacement.
            // Keeping `rsp_frame_size == 0` made the 7th+ integer parameters
            // reload saved registers instead of caller arguments
            // (gcc.c-torture/execute/20010518-1.c returned 187 instead of 91).
            self.state.out.rsp_frame_size = actual_stack;
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

            // Classic mcount site (-pg without -mfentry/-mnop-mcount): the
            // call belongs AFTER the frame is established — the mcount ABI
            // reads the parent PC through the frame, and GCC measures to the
            // identical shape (frame first, then `call mcount`; GCC rejects
            // -pg together with -fomit-frame-pointer). The label is recorded
            // in __mcount_loc when -mrecord-mcount is active.
            if let Some(lbl) = self.state.pending_classic_mcount_label.take() {
                if self.state.mcount.map(|m| m.record).unwrap_or(false) {
                    self.state.emit_fmt(format_args!("{}:", lbl));
                }
                self.state.emit("    call mcount");
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
                    self.state
                        .out
                        .emit_instr_imm_reg("    movq", local_size, "r11");
                    self.state.out.emit_named_label(&probe_label);
                    self.state
                        .out
                        .emit_instr_imm_reg("    subq", PAGE_SIZE, "rsp");
                    self.state.emit("    orl $0, (%rsp)");
                    self.state
                        .out
                        .emit_instr_imm_reg("    subq", PAGE_SIZE, "r11");
                    self.state
                        .out
                        .emit_instr_imm_reg("    cmpq", PAGE_SIZE, "r11");
                    self.state.out.emit_jcc_label("    ja", &probe_label);
                    self.state.emit("    subq %r11, %rsp");
                    self.state.emit("    orl $0, (%rsp)");
                } else {
                    self.state
                        .out
                        .emit_instr_imm_reg("    subq", local_size, "rsp");
                }
            }
        }

        // Preserve source-level volatile semantics through the late text
        // peephole.  The marker is a GNU-as comment and is consumed only by
        // LCCC's peephole pre-scan; it names the final direct slot address.
        // This avoids treating a volatile local as an ordinary dead stack
        // temporary after register/FP shuttle rewrites.
        let mut volatile_ids: Vec<u32> =
            self.state.volatile_alloca_values.iter().copied().collect();
        volatile_ids.sort_unstable();
        for id in volatile_ids {
            if let Some(slot) = self.state.get_slot(id) {
                if self.state.out.use_rsp_addressing {
                    let off = self.state.out.rsp_frame_size + slot.0;
                    self.state
                        .emit_fmt(format_args!("    # LCCC_VOLATILE_SLOT {}(%rsp)", off));
                } else {
                    self.state
                        .emit_fmt(format_args!("    # LCCC_VOLATILE_SLOT {}(%rbp)", slot.0));
                }
            }
        }

        if func.is_variadic {
            let base = self.reg_save_area_offset;

            // Save only the GP registers that can hold VARIADIC arguments: the
            // first `num_named_int_params` registers carried named params, and
            // va_start sets gp_offset past them, so va_arg can never read their
            // save slots. (GCC/Clang skip them the same way; `sum3(a,b,c,...)`
            // saves only %rcx/%r8/%r9.) When no va_arg in the body reads the
            // INTEGER class AND the va_list never escapes (vararg_gp_save is
            // false), the whole GP block is dead and skipped — FP-only varargs
            // save nothing in the GP area (GCC's `sum_dbl` behavior).
            if self.vararg_gp_save {
                let gp_start = self.num_named_int_params.min(6);
                let gp_regs = ["rdi", "rsi", "rdx", "rcx", "r8", "r9"];
                for i in gp_start..6 {
                    self.state.out.emit_instr_reg_rbp(
                        "    movq",
                        gp_regs[i],
                        base + (i as i64) * 8,
                    );
                }
            }

            // Save only the XMM registers that can hold variadic FP arguments
            // (xmm[num_named_fp_params..7]), and only when the caller actually
            // placed any argument in an SSE register: %al holds that count
            // (0-8) for variadic calls (SysV AMD64 §3.5.7). When no va_arg in
            // the body reads the SSE class AND the va_list never escapes
            // (vararg_fp_save is false), the whole XMM block is skipped —
            // integer-only varargs (printf/printk wrappers) save nothing here,
            // not even behind the %al gate.
            if !self.no_sse && self.vararg_fp_save {
                let fp_start = self.num_named_fp_params.min(8);
                if fp_start < 8 {
                    let skip = self.state.fresh_label("no_fp_save");
                    self.state.emit("    testb %al, %al");
                    self.state.out.emit_jcc_label("    je", &skip);
                    for i in fp_start..8usize {
                        // rsp-aware emitter: emits `offset(%rbp)` in rbp mode
                        // and `(frame_size + offset)(%rsp)` in FPO mode.
                        let xmm = format!("xmm{}", i);
                        self.state.out.emit_instr_reg_rbp(
                            "    movdqu",
                            &xmm,
                            base + 48 + (i as i64) * 16,
                        );
                    }
                    self.state.out.emit_named_label(&skip);
                }
            }
        }
    }

    pub(super) fn emit_epilogue_impl(&mut self, frame_size: i64) {
        let used_regs = self.used_callee_saved.clone();
        let num_saved = used_regs.len() as i64;
        let omit_fp = self.state.omit_frame_pointer;

        if omit_fp {
            let use_push_pop = num_saved > 0;

            if use_push_pop {
                let local_size = frame_size - num_saved * 8;
                if local_size > 0 {
                    self.state
                        .out
                        .emit_instr_imm_reg("    addq", local_size, "rsp");
                }
                for &reg in used_regs.iter().rev() {
                    let reg_name = phys_reg_name(reg);
                    self.state.emit_fmt(format_args!("    popq %{}", reg_name));
                }
            } else {
                for (i, &reg) in used_regs.iter().enumerate() {
                    let reg_name = phys_reg_name(reg);
                    let offset = frame_size - (i as i64 + 1) * 8;
                    self.state
                        .emit_fmt(format_args!("    movq {}(%rsp), %{}", offset, reg_name));
                }
                if frame_size > 0 {
                    self.state
                        .out
                        .emit_instr_imm_reg("    addq", frame_size, "rsp");
                }
            }
        } else {
            // Traditional epilogue: restore from pushes, then popq %rbp
            if num_saved > 0 {
                self.state
                    .emit_fmt(format_args!("    leaq {}(%rbp), %rsp", -(num_saved * 8)));
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
        let xmm_regs = [
            "xmm0", "xmm1", "xmm2", "xmm3", "xmm4", "xmm5", "xmm6", "xmm7",
        ];
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

        // Build a map of param_idx -> ParamRef dest Value for fast lookup.
        // This is used to optimize parameter storage: when the ParamRef dest
        // is register-allocated, we can store the ABI arg register directly
        // to the callee-saved register, skipping the alloca slot entirely.
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

        // In RBP mode: stack args start at 16(%rbp) = return_addr(8) + saved_rbp(8).
        // In FPO mode: no saved rbp, so stack args start at 8 from the virtual rbp
        // (which is entry_rsp = return_addr).
        let stack_base: i64 = if self.state.omit_frame_pointer { 8 } else { 16 };

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
        let mut gpr_prestores: Vec<(usize, &'static str, &'static str, &'static str, PhysReg)> =
            Vec::new();

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
                    eprintln!(
                        "[PRE-STORE] param {} paramref_dest={} has_slot={} reg={:?}",
                        i, paramref_dest.0, has_slot, reg
                    );
                }
                if !has_slot {
                    if let Some(&phys_reg) = self.reg_assignments.get(&paramref_dest.0) {
                        // Callee-saved GPRs can be copied immediately.  x86-64
                        // caller-saved GPR homes (PhysReg 10..16) are legal for
                        // values proven not to span a call, but their moves are
                        // deferred below because a destination may still hold a
                        // different incoming ABI argument. XMM homes use their
                        // own ordered parallel-copy set for the same reason.
                        // The GPR pool is X86_CALLEE_SAVED_WITH_RBP (1..=6)
                        // whenever the frame pointer is omitted; rbp(6) is a
                        // legal callee-saved home then. The old 1..=5 range
                        // left rbp-homed params unclassified, so no pre-store
                        // was emitted and the ParamRef fell back to an ABI
                        // read that copy-propagation could corrupt
                        // (sqlite_vdbe_peephole: `op` read from %dil).
                        let is_callee_saved = phys_reg.0 >= 1 && phys_reg.0 <= 6;
                        let is_caller_saved_gpr = (10..=16).contains(&phys_reg.0);
                        let is_xmm = super::emit::is_xmm_reg(phys_reg);
                        if std::env::var("CCC_DEBUG_PARAM_STORE").is_ok() {
                            let shared =
                                reg_to_params.get(&phys_reg.0).is_some_and(|u| u.len() > 1);
                            eprintln!(
                                "[PRE-STORE]   callee={} caller={} shared={} class={:?}",
                                is_callee_saved, is_caller_saved_gpr, shared, class
                            );
                        }
                        if is_callee_saved || is_caller_saved_gpr || is_xmm {
                            let shared = reg_to_params
                                .get(&phys_reg.0)
                                .is_some_and(|users| users.len() > 1);
                            if !shared {
                                let dest_reg = phys_reg_name(phys_reg);
                                if let ParamClass::IntReg { reg_idx } = class {
                                    let source = X86_ARG_REGS[reg_idx];
                                    if is_caller_saved_gpr {
                                        gpr_prestores.push((i, source, source, dest_reg, phys_reg));
                                    } else {
                                        self.state
                                            .out
                                            .emit_instr_reg_reg("    movq", source, dest_reg);
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
                        eprintln!(
                            "[PARAM-STORE] param {} has alloca dest={}, no ParamRef",
                            i, alloca_dest.0
                        );
                        eprintln!(
                            "[PARAM-STORE] has_slot={}",
                            self.state.get_slot(alloca_dest.0).is_some()
                        );
                        for (bi, block) in func.blocks.iter().enumerate() {
                            for inst in &block.instructions {
                                match inst {
                                    Instruction::Store { ptr, val, .. }
                                        if ptr.0 == alloca_dest.0 =>
                                    {
                                        eprintln!(
                                            "[PARAM-STORE]   block[{}] Store to alloca: val={:?}",
                                            bi, val
                                        )
                                    }
                                    Instruction::Load { ptr, dest, .. }
                                        if ptr.0 == alloca_dest.0 =>
                                    {
                                        eprintln!(
                                            "[PARAM-STORE]   block[{}] Load from alloca: dest={}",
                                            bi, dest.0
                                        )
                                    }
                                    Instruction::Copy { dest, src } => eprintln!(
                                        "[PARAM-STORE]   block[{}] Copy dest={} src={:?}",
                                        bi, dest.0, src
                                    ),
                                    Instruction::ParamRef {
                                        dest, param_idx, ..
                                    } => eprintln!(
                                        "[PARAM-STORE]   block[{}] ParamRef dest={} idx={}",
                                        bi, dest.0, param_idx
                                    ),
                                    _ => {}
                                }
                            }
                        }
                        for (&vid, &reg) in self.reg_assignments.iter() {
                            eprintln!(
                                "[PARAM-STORE]   reg_assign: val={} -> PhysReg({})",
                                vid, reg.0
                            );
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
                        let mut all_vals: crate::common::fx_hash::FxHashSet<u32> =
                            param_vals.iter().copied().collect();
                        let mut changed_prop = true;
                        while changed_prop {
                            changed_prop = false;
                            for block in &func.blocks {
                                for inst in &block.instructions {
                                    if let Instruction::Copy {
                                        dest,
                                        src: crate::ir::instruction::Operand::Value(v),
                                    } = inst
                                    {
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
                                            "    movq",
                                            X86_ARG_REGS[reg_idx],
                                            dest_reg,
                                        );
                                        self.state.param_pre_stored.insert(i);
                                        self.param_source_regs
                                            .insert(phys_reg.0, X86_ARG_REGS[reg_idx]);
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
                    self.state
                        .out
                        .emit_instr_reg_rbp("    movq", X86_ARG_REGS[reg_idx], slot.0);
                }
                ParamClass::FloatReg { reg_idx } => {
                    if ty == IrType::F32 || ty == IrType::D32 {
                        self.state
                            .out
                            .emit_instr_reg_reg("    movd", xmm_regs[reg_idx], "eax");
                        self.state.out.emit_instr_reg_rbp("    movq", "rax", slot.0);
                    } else {
                        self.state
                            .out
                            .emit_instr_reg_rbp("    movq", xmm_regs[reg_idx], slot.0);
                    }
                }
                ParamClass::I128RegPair { base_reg_idx } => {
                    self.state.out.emit_instr_reg_rbp(
                        "    movq",
                        X86_ARG_REGS[base_reg_idx],
                        slot.0,
                    );
                    self.state.out.emit_instr_reg_rbp(
                        "    movq",
                        X86_ARG_REGS[base_reg_idx + 1],
                        slot.0 + 8,
                    );
                }
                ParamClass::StructByValReg { base_reg_idx, size } => {
                    self.state.out.emit_instr_reg_rbp(
                        "    movq",
                        X86_ARG_REGS[base_reg_idx],
                        slot.0,
                    );
                    if size > 8 {
                        self.state.out.emit_instr_reg_rbp(
                            "    movq",
                            X86_ARG_REGS[base_reg_idx + 1],
                            slot.0 + 8,
                        );
                    }
                }
                ParamClass::StructSseReg {
                    lo_fp_idx,
                    hi_fp_idx,
                    ..
                } => {
                    self.state
                        .out
                        .emit_instr_reg_rbp("    movq", xmm_regs[lo_fp_idx], slot.0);
                    if let Some(hi) = hi_fp_idx {
                        self.state
                            .out
                            .emit_instr_reg_rbp("    movq", xmm_regs[hi], slot.0 + 8);
                    }
                }
                ParamClass::F128SseReg { reg_idx } => {
                    // _Float128: the full 16 bytes arrive in ONE XMM register.
                    self.state
                        .out
                        .emit_instr_reg_rbp("    movdqu", xmm_regs[reg_idx], slot.0);
                }
                ParamClass::StructMixedIntSseReg {
                    int_reg_idx,
                    fp_reg_idx,
                    ..
                } => {
                    self.state.out.emit_instr_reg_rbp(
                        "    movq",
                        X86_ARG_REGS[int_reg_idx],
                        slot.0,
                    );
                    self.state
                        .out
                        .emit_instr_reg_rbp("    movq", xmm_regs[fp_reg_idx], slot.0 + 8);
                }
                ParamClass::StructMixedSseIntReg {
                    fp_reg_idx,
                    int_reg_idx,
                    ..
                } => {
                    self.state
                        .out
                        .emit_instr_reg_rbp("    movq", xmm_regs[fp_reg_idx], slot.0);
                    self.state.out.emit_instr_reg_rbp(
                        "    movq",
                        X86_ARG_REGS[int_reg_idx],
                        slot.0 + 8,
                    );
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
                    self.state
                        .out
                        .emit_instr_rbp_reg("    movq", src + 8, "rax");
                    self.state
                        .out
                        .emit_instr_reg_rbp("    movq", "rax", slot.0 + 8);
                }
                ParamClass::StackScalar { offset } => {
                    // Load from caller's stack frame and store full 8 bytes to ensure
                    // the entire slot is initialized (see IntReg comment above).
                    let src = stack_base + offset;
                    self.state.out.emit_instr_rbp_reg("    movq", src, "rax");
                    self.state.out.emit_instr_reg_rbp("    movq", "rax", slot.0);
                }
                ParamClass::StructStack { offset, size }
                | ParamClass::LargeStructStack { offset, size } => {
                    let src = stack_base + offset;
                    let n_qwords = size.div_ceil(8);
                    // Over-aligned (>16) parameter allocas have their slot
                    // oversized by (align-1); the EFFECTIVE address is
                    // align_up(slot, align). `&s` (via value_to_reg) resolves
                    // that same aligned address, so the copy must target it too
                    // — writing the raw slot desyncs the two by the alignment
                    // pad (the _Alignas(32) struct param regression).
                    let over_align = find_param_alloca(func, i)
                        .and_then(|(dest, _)| self.state.alloca_over_align(dest.0))
                        .filter(|&a| a > 16);
                    if let Some(a) = over_align {
                        self.state.out.emit_instr_rbp_reg("    leaq", slot.0, "rcx");
                        self.state
                            .out
                            .emit_instr_imm_reg("    addq", (a - 1) as i64, "rcx");
                        self.state
                            .out
                            .emit_instr_imm_reg("    andq", -(a as i64), "rcx");
                        for qi in 0..n_qwords {
                            let src_off = src + (qi as i64 * 8);
                            self.state
                                .out
                                .emit_instr_rbp_reg("    movq", src_off, "rax");
                            self.state
                                .emit_fmt(format_args!("    movq %rax, {}(%rcx)", qi * 8));
                        }
                    } else {
                        for qi in 0..n_qwords {
                            let src_off = src + (qi as i64 * 8);
                            let dst_off = slot.0 + (qi as i64 * 8);
                            self.state
                                .out
                                .emit_instr_rbp_reg("    movq", src_off, "rax");
                            self.state
                                .out
                                .emit_instr_reg_rbp("    movq", "rax", dst_off);
                        }
                    }
                }
                ParamClass::F128FpReg { .. }
                | ParamClass::F128GpPair { .. }
                | ParamClass::F128Stack { .. }
                | ParamClass::LargeStructByRefReg { .. }
                | ParamClass::LargeStructByRefStack { .. }
                | ParamClass::StructSplitRegStack { .. }
                | ParamClass::ZeroSizeSkip => {}
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
                    let clobbers_other_source = pending
                        .iter()
                        .enumerate()
                        .any(|(j, other)| j != idx && other.1 == dest);
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
                    let clobbers = pending
                        .iter()
                        .any(|&(j, j_abi, _, _, _)| j != i && dest == xmm_regs[j_abi]);
                    if !clobbers {
                        let mnemonic = if is_f32 { "    movss" } else { "    movsd" };
                        self.state
                            .out
                            .emit_instr_reg_reg(mnemonic, xmm_regs[abi_idx], dest);
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
                        self.state
                            .out
                            .emit_instr_reg_reg(mnemonic, xmm_regs[abi_idx], scratch);
                        self.state.param_pre_stored.insert(i);
                        scratch_saves.push((dest, is_f32, scratch));
                        let _ = home;
                        pending.remove(0);
                    } else {
                        // Degenerate (every XMM is a pending home): emit directly.
                        let mnemonic = if is_f32 { "    movss" } else { "    movsd" };
                        self.state
                            .out
                            .emit_instr_reg_reg(mnemonic, xmm_regs[abi_idx], dest);
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
                self.state
                    .emit_fmt(format_args!("    {} {}, {}", load_instr, sr, reg));
                self.store_rax_to(dest);
                return;
            }
        }

        let xmm_regs = [
            "xmm0", "xmm1", "xmm2", "xmm3", "xmm4", "xmm5", "xmm6", "xmm7",
        ];
        let class = self.state.param_classes[param_idx];
        let stack_base: i64 = if self.state.omit_frame_pointer { 8 } else { 16 };

        match class {
            ParamClass::IntReg { reg_idx } => {
                let src_reg = Self::reg_for_type(X86_ARG_REGS[reg_idx], ty);
                let load_instr = Self::mov_load_for_type(ty);
                let dest_reg = Self::load_dest_reg(ty);
                if std::env::var_os("CCC_DEBUG_PARAMREF").is_some() {
                    eprintln!(
                        "[PARAMREF] dest=v{} param_idx={} reg_idx={} src={} ty={:?}",
                        dest.0, param_idx, reg_idx, src_reg, ty
                    );
                }
                // The fallback ABI read is a pinned contract: the operand is
                // the parameter's *incoming* ABI register. A caller-saved
                // pre-store of a different parameter may have copied another
                // value into the same register name (`movq %rdi, %rsi` for
                // param 0 while param 1 arrives in %sil), and copy propagation
                // must never rewrite this read through that copy — it would
                // silently substitute the wrong parameter. The marker is
                // consumed by `pin_param_abi_reads` in the text peephole.
                self.state
                    .emit_fmt(format_args!("    # LCCC_PARAM_ABI_READ {}", src_reg));
                self.state.emit_fmt(format_args!(
                    "    {} %{}, {}",
                    load_instr, src_reg, dest_reg
                ));
                self.store_rax_to(dest);
            }
            ParamClass::FloatReg { reg_idx } => {
                if ty == IrType::F32 {
                    self.state
                        .out
                        .emit_instr_reg_reg("    movd", xmm_regs[reg_idx], "eax");
                    self.store_rax_to(dest);
                } else {
                    self.state
                        .out
                        .emit_instr_reg_reg("    movq", xmm_regs[reg_idx], "rax");
                    self.store_rax_to(dest);
                }
            }
            ParamClass::StackScalar { offset } => {
                let src = stack_base + offset;
                let load_instr = Self::mov_load_for_type(ty);
                let reg = Self::load_dest_reg(ty);
                let sr = self.slot_ref(src);
                self.state
                    .emit_fmt(format_args!("    {} {}, {}", load_instr, sr, reg));
                self.store_rax_to(dest);
            }
            _ => {}
        }
    }

    pub(super) fn emit_epilogue_and_ret_impl(&mut self, frame_size: i64) {
        // IS-20: a function that dirtied upper YMM halves must clean them
        // before returning, or every legacy-SSE instruction in the caller
        // pays the AVX-SSE transition penalty (~50-100 cycles on current
        // Intel). GCC/Clang emit the same vzeroupper-before-ret.
        if self.state.dirty_upper_ymm {
            self.state.emit("    vzeroupper");
        }
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

/// Per-block loop-nesting depth for the PF-07 indexed-symbol-base hoist
/// decision (cheap CFG pass: back-edge detection + header-body marking,
/// mirroring `liveness::compute_loop_depth` without dragging the whole
/// dataflow along).
fn global_addr_loop_depths(func: &IrFunction, _watched: &FxHashSet<u32>) -> Vec<usize> {
    let n = func.blocks.len();
    let mut depths = vec![0usize; n];
    if n == 0 {
        return depths;
    }
    // Successor lists by block index.
    let label_to_idx: FxHashMap<u32, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.label.0, i))
        .collect();
    let mut succs: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, b) in func.blocks.iter().enumerate() {
        match &b.terminator {
            Terminator::Branch(t) => {
                if let Some(&j) = label_to_idx.get(&t.0) {
                    succs[i].push(j);
                }
            }
            Terminator::CondBranch {
                true_label,
                false_label,
                ..
            } => {
                if let Some(&j) = label_to_idx.get(&true_label.0) {
                    succs[i].push(j);
                }
                if let Some(&j) = label_to_idx.get(&false_label.0) {
                    succs[i].push(j);
                }
            }
            Terminator::Switch { cases, default, .. } => {
                if let Some(&j) = label_to_idx.get(&default.0) {
                    succs[i].push(j);
                }
                for (_, l) in cases {
                    if let Some(&j) = label_to_idx.get(&l.0) {
                        succs[i].push(j);
                    }
                }
            }
            _ => {}
        }
    }
    // Back edges: t -> h where h dominates t. Compute dominators by the
    // simple iterative algorithm (blocks are few at this call site).
    let preds: Vec<Vec<usize>> = {
        let mut p: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (i, ss) in succs.iter().enumerate() {
            for &j in ss {
                p[j].push(i);
            }
        }
        p
    };
    let mut idom: Vec<usize> = vec![usize::MAX; n];
    if !preds[0].is_empty() {
        // Entry has predecessors (unreachable shape): fall back to depth 0.
        return depths;
    }
    idom[0] = 0;
    let mut changed = true;
    while changed {
        changed = false;
        for b in 1..n {
            let mut new_idom = usize::MAX;
            for &p in &preds[b] {
                if idom[p] == usize::MAX {
                    continue;
                }
                new_idom = if new_idom == usize::MAX {
                    p
                } else {
                    // Intersect.
                    let mut x = new_idom;
                    let mut y = p;
                    while x != y {
                        while x > y {
                            x = idom[x];
                        }
                        while y > x {
                            y = idom[y];
                        }
                    }
                    x
                };
            }
            if new_idom != usize::MAX && idom[b] != new_idom {
                idom[b] = new_idom;
                changed = true;
            }
        }
    }
    // Loop bodies: for each back edge t->h (h dominates t), every block that
    // can reach t without passing through h, plus h, is the loop body; bump.
    for t in 0..n {
        for &h in &succs[t] {
            // h dominates t?
            let mut x = t;
            let mut dom_ok = false;
            for _ in 0..n {
                if x == h {
                    dom_ok = true;
                    break;
                }
                if idom[x] == usize::MAX || idom[x] == x {
                    break;
                }
                x = idom[x];
            }
            if !dom_ok {
                continue;
            }
            // Reverse-reach from t stopping at h.
            let mut stack = vec![t];
            let mut body = vec![h];
            let mut in_body: FxHashSet<usize> = FxHashSet::default();
            in_body.insert(h);
            while let Some(b) = stack.pop() {
                if !in_body.insert(b) {
                    continue;
                }
                body.push(b);
                for &p in &preds[b] {
                    if !in_body.contains(&p) {
                        stack.push(p);
                    }
                }
            }
            for b in body {
                depths[b] += 1;
            }
        }
    }
    depths
}

/// Defining block of a GlobalAddr root (or of the const-offset chain that
/// ends in one), for the PF-07 depth comparison.
fn gaddr_def_block(func: &IrFunction, mut v: u32, gmap: &FxHashMap<u32, String>) -> Option<usize> {
    for _ in 0..8 {
        if gmap.contains_key(&v) {
            // Find its defining block.
            for (bi, b) in func.blocks.iter().enumerate() {
                for inst in &b.instructions {
                    if let Some(d) = inst.dest() {
                        if d.0 == v {
                            return Some(bi);
                        }
                    }
                }
            }
            return None;
        }
        // Walk the const-offset chain one step.
        let mut next: Option<u32> = None;
        for b in &func.blocks {
            for inst in &b.instructions {
                match inst {
                    Instruction::GetElementPtr { dest, base, .. } if dest.0 == v => {
                        next = Some(base.0);
                    }
                    Instruction::Copy { dest, src } if dest.0 == v => {
                        if let Operand::Value(sv) = src {
                            next = Some(sv.0);
                        }
                    }
                    _ => {}
                }
            }
        }
        v = next?;
    }
    None
}
