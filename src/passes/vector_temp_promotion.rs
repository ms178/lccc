//! Promote short-lived SIMD vector temporaries into their destination variable
//! slots, eliminating the compiler-introduced `Memcpy` + temp alloca produced by
//! vector intrinsic lowering.
//!
//! Every Vec128/Vec256 intrinsic lowers to:
//! ```text
//!   %tmp = Alloca(16|32)                       // temp result slot
//!   %d   = Intrinsic { op, dest_ptr: %tmp, .. } // backend writes %ymm0 -> *%tmp
//!   ...   the C expression value is the *pointer* %tmp ...
//!   %var = Alloca(16|32)                       // the C variable (over-aligned)
//!   Memcpy { dest: %var, src: %tmp, size }     // aggregate assignment
//! ```
//! The Memcpy forces a 16/32-byte copy plus a runtime-alignment dance, and the
//! temp alloca inflates the frame. When `%tmp` is used only by the Intrinsic
//! (as `dest_ptr`) and by exactly one Memcpy (as `src`), we point the
//! Intrinsic's `dest_ptr` at `%var`, drop the Memcpy and the temp alloca, and
//! let the backend write the result straight into the variable's slot.
//! Subsequent intrinsics then read the variable slot directly, which also
//! enables the backend's memory-operand folding (e.g. `vpcmpeqb slot,%ymm0,%ymm0`).
//!
//! Safety: the rewrite is only applied when (a) the Intrinsic and the Memcpy are
//! in the same block with the Intrinsic first, (b) `%tmp` has exactly two uses
//! (the Intrinsic `dest_ptr` and the one Memcpy `src`), so no other read/write
//! can observe the temp, and (c) the Intrinsic's args are evaluated before the
//! write in IR order, preserving `x = op(x, y)` semantics.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::instruction::Instruction;
use crate::ir::reexports::{IrFunction, IrModule, Value};

/// Run the vector-temp promotion across all functions.
pub(crate) fn promote_vector_temps(module: &mut IrModule) -> usize {
    let mut total = 0usize;
    for func in &mut module.functions {
        if func.is_declaration || func.blocks.is_empty() {
            continue;
        }
        total += promote_in_function(func);
    }
    total
}

struct Rewrite {
    block: usize,
    intrinsic: usize,
    memcpy: usize,
    tmp: u32,
    var: u32,
}

fn promote_in_function(func: &mut IrFunction) -> usize {
    // 1. Collect alloca sizes and use-counts for alloca values.
    let mut alloca_sizes: FxHashMap<u32, usize> = FxHashMap::default();
    let mut uses: FxHashMap<u32, usize> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca { dest, size, .. } = inst {
                alloca_sizes.insert(dest.0, *size);
            }
            for v in inst.used_values() {
                *uses.entry(v).or_insert(0) += 1;
            }
        }
        for v in block.terminator.used_values() {
            *uses.entry(v).or_insert(0) += 1;
        }
    }

    // 2. Find intrinsic(dest_ptr=%tmp) ... Memcpy(dest=%var, src=%tmp) pairs.
    let mut rewrites: Vec<Rewrite> = Vec::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        // tmp value -> index of the preceding Intrinsic that writes it.
        let mut intrinsic_at: FxHashMap<u32, usize> = FxHashMap::default();
        for (ii, inst) in block.instructions.iter().enumerate() {
            match inst {
                Instruction::Intrinsic { dest_ptr: Some(tmp), .. } => {
                    if alloca_sizes.contains_key(&tmp.0) {
                        intrinsic_at.insert(tmp.0, ii);
                    }
                }
                Instruction::Memcpy { dest, src, size } => {
                    if let Some(&intr_idx) = intrinsic_at.get(&src.0) {
                        let tmp = src.0;
                        // The temp must be used ONLY by the Intrinsic dest_ptr and
                        // this one Memcpy src (used_values counts both), and sizes
                        // must match so the assignment is a full copy.
                        if uses.get(&tmp) == Some(&2)
                            && alloca_sizes.get(&tmp) == Some(size)
                            && alloca_sizes.contains_key(&dest.0)
                        {
                            rewrites.push(Rewrite {
                                block: bi,
                                intrinsic: intr_idx,
                                memcpy: ii,
                                tmp,
                                var: dest.0,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    }

    if rewrites.is_empty() {
        return 0;
    }

    // 3. Apply. Rebuild each affected block once: patch the Intrinsic dest_ptr,
    // drop the Memcpy, and drop the now-unused temp Alloca.
    let mut by_block: FxHashMap<usize, Vec<&Rewrite>> = FxHashMap::default();
    for r in &rewrites {
        by_block.entry(r.block).or_default().push(r);
    }

    let mut promoted = 0usize;
    for (bi, rs) in by_block {
        let mut memcpy_drop: FxHashSet<usize> = FxHashSet::default();
        let mut tmp_patch: FxHashMap<u32, u32> = FxHashMap::default(); // tmp -> var
        let mut tmp_set: FxHashSet<u32> = FxHashSet::default();
        for r in &rs {
            memcpy_drop.insert(r.memcpy);
            tmp_patch.insert(r.tmp, r.var);
            tmp_set.insert(r.tmp);
            promoted += 1;
        }

        let block = &mut func.blocks[bi];
        let mut new_instrs = Vec::with_capacity(block.instructions.len());
        for (ii, mut inst) in std::mem::take(&mut block.instructions).into_iter().enumerate() {
            if memcpy_drop.contains(&ii) {
                continue;
            }
            if let Instruction::Alloca { dest, .. } = &inst {
                if tmp_set.contains(&dest.0) {
                    continue; // temp alloca is now dead
                }
            }
            if let Instruction::Intrinsic { dest_ptr, .. } = &mut inst {
                if let Some(ptr) = dest_ptr {
                    if let Some(&var) = tmp_patch.get(&ptr.0) {
                        *ptr = Value(var);
                    }
                }
            }
            new_instrs.push(inst);
        }
        block.instructions = new_instrs;
    }

    promoted
}

/// Fuse vector load intrinsics into their single consumer.
///
/// After temp promotion, the IR for `ymm = _mm256_loadu_si256(p); c = op(ymm, ..)`
/// is `Intrinsic{Loadu256, dest_ptr: %var, args:[p]}` followed by a consumer
/// intrinsic that reads `%var` as an operand. When `%var`'s most recent write is
/// that load and the consumer is its first subsequent read, the slot round-trip
/// is pure overhead: replace the consumer's operand with the load's source
/// pointer `p` (the backend loads `vmovdqu (%p)` for it) and delete the load.
/// Works across loop-carried variable reassignment because the check is
/// sequential ("last writer of %var is a load, no intervening reads").
pub(crate) fn fuse_vector_loads(module: &mut IrModule) -> usize {
    let mut total = 0usize;
    for func in &mut module.functions {
        if func.is_declaration || func.blocks.is_empty() {
            continue;
        }
        total += fuse_in_function(func);
    }
    total
}

fn fuse_in_function(func: &mut IrFunction) -> usize {
    use crate::ir::intrinsics::IntrinsicOp;
    use crate::ir::reexports::Operand;

    let is_vec_load = |op: &IntrinsicOp| {
        matches!(op,
            IntrinsicOp::Loadu256 | IntrinsicOp::Load256
            | IntrinsicOp::Loaddqu
            | IntrinsicOp::VecLoadF64x4 | IntrinsicOp::VecLoadF64x2
            | IntrinsicOp::VecLoadI32x8 | IntrinsicOp::VecLoadI32x4)
    };

    // Phase 1: per-block analysis. Track, for every alloca slot, whether its
    // most recent write was a vector load (and from which pointer), and whether
    // the slot has been read since that write.
    #[derive(Default)]
    struct BlockPlan {
        fused_vars: FxHashSet<u32>,
        patches: Vec<(usize, usize, Operand)>, // (consumer_idx, arg_idx, load_source)
    }
    let mut plans: Vec<BlockPlan> = Vec::with_capacity(func.blocks.len());
    for block in &func.blocks {
        let mut plan = BlockPlan::default();
        let mut last_load: FxHashMap<u32, (Operand, usize)> = FxHashMap::default();
        let mut last_read: FxHashMap<u32, usize> = FxHashMap::default();
        for (ii, inst) in block.instructions.iter().enumerate() {
            match inst {
                Instruction::Intrinsic { op, dest_ptr: Some(d), args, .. } if is_vec_load(op) => {
                    if let Some(src) = args.first() {
                        last_load.insert(d.0, (src.clone(), ii));
                    }
                    last_read.remove(&d.0); // the load overwrites d
                }
                Instruction::Intrinsic { args, .. } => {
                    for (ai, arg) in args.iter().enumerate() {
                        if let Operand::Value(v) = arg {
                            if let Some((src, w)) = last_load.get(&v.0) {
                                if last_read.get(&v.0).map_or(true, |&r| r < *w) {
                                    // First read after the load: fuse.
                                    plan.patches.push((ii, ai, src.clone()));
                                    plan.fused_vars.insert(v.0);
                                }
                            }
                        }
                    }
                    for arg in args {
                        if let Operand::Value(v) = arg {
                            last_read.insert(v.0, ii);
                        }
                    }
                }
                other => {
                    for v in other.used_values() {
                        if last_load.contains_key(&v) {
                            last_read.insert(v, ii);
                        }
                    }
                    if let Some(d) = other.dest() {
                        last_load.remove(&d.0);
                    }
                }
            }
        }
        plans.push(plan);
    }

    // Phase 2: apply. Patch consumer operands; keep the load intrinsics in place
    // (their slot may still be read by other, unfused consumers).
    let mut removed = 0usize;
    for (bi, plan) in plans.into_iter().enumerate() {
        if plan.patches.is_empty() {
            continue;
        }
        let block = &mut func.blocks[bi];
        for (ii, inst) in block.instructions.iter_mut().enumerate() {
            for &(ci, ai, ref src) in &plan.patches {
                if ci == ii {
                    if let Instruction::Intrinsic { args, .. } = inst {
                        args[ai] = src.clone();
                        removed += 1;
                    }
                }
            }
        }
    }

    // Phase 3: drop load intrinsics whose destination now has zero remaining
    // uses (every consumer was fused). A load with any surviving reader must
    // stay so the slot still receives its data.
    // Count REMAINING READS of each value. dest_ptr of an intrinsic is a write,
    // not a read — a load's own dest_ptr must not keep it alive.
    let mut uses_after: FxHashMap<u32, usize> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Intrinsic { dest_ptr: Some(_), .. } => {
                    // used_values() includes dest_ptr; count only args here.
                    if let Instruction::Intrinsic { args, .. } = inst {
                        for arg in args {
                            if let crate::ir::reexports::Operand::Value(v) = arg {
                                *uses_after.entry(v.0).or_insert(0) += 1;
                            }
                        }
                    }
                }
                _ => {
                    for v in inst.used_values() {
                        *uses_after.entry(v).or_insert(0) += 1;
                    }
                }
            }
        }
        for v in block.terminator.used_values() {
            *uses_after.entry(v).or_insert(0) += 1;
        }
    }
    for bi in 0..func.blocks.len() {
        let block = &mut func.blocks[bi];
        let mut new_instrs = Vec::with_capacity(block.instructions.len());
        for (ii, inst) in std::mem::take(&mut block.instructions).into_iter().enumerate() {
            let drop = if let Instruction::Intrinsic { op, dest_ptr: Some(d), .. } = &inst {
                is_vec_load(op) && uses_after.get(&d.0).copied().unwrap_or(0) == 0
            } else {
                false
            };
            if !drop {
                new_instrs.push(inst);
            }
        }
        block.instructions = new_instrs;
    }
    removed
}

/// Downgrade the alignment of non-escaping vector-sized allocas (16/32 bytes)
/// from >16 to 0.
///
/// Every vector access to a variable alloca in the x86-64 backend uses
/// UNALIGNED moves (vmovdqu/movdqu — the "aligned" intrinsic forms are
/// reg-to-reg no-ops over an unaligned load). Keeping `align > 16` on such an
/// alloca makes every access pay the runtime `lea/add/and` alignment dance
/// (3 instructions per 32-byte access in zlib-ng's compare256_avx2 inner
/// loop) for an alignment nothing observes: if the address never escapes the
/// function, no `_Alignas` guarantee is observable and no aligned instruction
/// can fault. If the address DOES escape (address-taken, call arg, intrinsic
/// arg — conservative: aligned intrinsic forms like `_mm256_load_si256` count
/// as escapes), the alignment is preserved.
///
/// Adopted from Agent B's fold_vec_temp (their escape list is identical in
/// spirit; this version additionally treats Memcpy/InlineAsm/terminator uses
/// as escapes for extra safety). Sound because the emitter never emits
/// vmovdqa on variable allocas.
pub(crate) fn downgrade_nonescaping_vector_align(module: &mut IrModule) -> usize {
    let mut total = 0usize;
    for func in &mut module.functions {
        if func.is_declaration || func.blocks.is_empty() {
            continue;
        }
        total += downgrade_in_function(func);
    }
    total
}

fn downgrade_in_function(func: &mut IrFunction) -> usize {
    use crate::ir::reexports::Operand;

    // Conservative escape set: any use that could observe the alloca's ADDRESS
    // (or pass it to code that requires alignment).
    let mut escape: FxHashSet<u32> = FxHashSet::default();
    let mut mark = |inst: &Instruction, escape: &mut FxHashSet<u32>| {
        match inst {
            Instruction::Store { val, ptr, .. } => {
                if let Operand::Value(v) = val {
                    escape.insert(v.0);
                }
                escape.insert(ptr.0);
            }
            Instruction::Load { ptr, .. } => {
                escape.insert(ptr.0);
            }
            Instruction::GetElementPtr { base, .. } => {
                escape.insert(base.0);
            }
            Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
                for a in &info.args {
                    if let Operand::Value(v) = a {
                        escape.insert(v.0);
                    }
                }
            }
            // Memcpy does NOT escape: it is byte-wise / unaligned-safe by
            // definition, so copying to/from an unaligned slot is always
            // correct, and compiler-generated aggregate copies of vector
            // values are exactly the accesses this pass optimizes. The
            // alignment-observing cases (address-of, aligned vector loads,
            // call args) are handled by the other arms.
            Instruction::Memcpy { .. } => {}
            Instruction::Intrinsic { op, args, .. } => {
                // Only MEMORY-LOAD intrinsics take their args as ADDRESSES
                // (e.g. `_mm256_loadu_si256(&v)`), so only those can observe an
                // alloca's alignment. Every other intrinsic reads its vector
                // args as VALUES via unaligned moves — the alignment of a
                // non-escaping local is unobservable, so those do NOT escape.
                use crate::ir::intrinsics::IntrinsicOp;
                let is_address_op = matches!(
                    op,
                    IntrinsicOp::Loadu256 | IntrinsicOp::Load256 | IntrinsicOp::Loaddqu
                        | IntrinsicOp::LoadF64x4 | IntrinsicOp::LoadF64x2
                        | IntrinsicOp::LoadI32x8 | IntrinsicOp::LoadI32x4
                        | IntrinsicOp::VecLoadF64x4 | IntrinsicOp::VecLoadF64x2
                        | IntrinsicOp::VecLoadI32x8 | IntrinsicOp::VecLoadI32x4
                );
                if is_address_op {
                    for a in args {
                        if let Operand::Value(v) = a {
                            escape.insert(v.0);
                        }
                    }
                }
            }
            Instruction::InlineAsm { outputs, inputs, .. } => {
                for (_, val, _) in outputs {
                    escape.insert(val.0);
                }
                for (_, op, _) in inputs {
                    if let Operand::Value(v) = op {
                        escape.insert(v.0);
                    }
                }
            }
            _ => {}
        }
    };
    for block in &func.blocks {
        for inst in &block.instructions {
            mark(inst, &mut escape);
        }
        for v in block.terminator.used_values() {
            escape.insert(v);
        }
    }

    // Vector-sized allocas with over-alignment that never escape: downgrade.
    let mut candidates: Vec<u32> = Vec::new();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca {
                dest,
                size,
                align,
                ..
            } = inst
            {
                if (*size == 16 || *size == 32) && *align > 16 && !escape.contains(&dest.0) {
                    candidates.push(dest.0);
                }
            }
        }
    }
    if candidates.is_empty() {
        return 0;
    }
    let mut changed = 0usize;
    for block in &mut func.blocks {
        for inst in &mut block.instructions {
            if let Instruction::Alloca {
                dest, align, ..
            } = inst
            {
                if candidates.contains(&dest.0) {
                    *align = 0;
                    changed += 1;
                }
            }
        }
    }
    changed
}
