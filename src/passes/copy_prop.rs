//! Copy propagation optimization pass.
//!
//! This pass eliminates redundant Copy instructions by replacing uses of a
//! Copy's destination with the Copy's source operand. This is particularly
//! important because:
//! - Phi elimination generates many Copy instructions
//! - Mem2reg creates Copy instructions when replacing loads
//! - Other optimization passes (simplify, GVN) create Copy instructions
//!
//! Without copy propagation, each Copy becomes a load-to-accumulator then
//! store-to-new-slot in codegen, wasting both instructions and stack space.
//!
//! After this pass runs, the dead Copy instructions are cleaned up by DCE.
//!
//! Performance: Uses a flat `Vec<Option<Operand>>` indexed by Value ID instead of
//! FxHashMap, since Value IDs are dense sequential u32s. This eliminates hashing
//! overhead and gives O(1) lookups with better cache locality.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::reexports::{
    Instruction, IrConst, IrFunction, IrModule, Operand, Terminator, Value,
};

/// Run copy propagation on the entire module.
/// Returns the number of operand replacements made.
pub fn run(module: &mut IrModule) -> usize {
    module.for_each_function(propagate_copies)
}

/// Forward a purely intermediate memcpy source:
///
/// ```text
///     memcpy tmp, src
///     memcpy dst, tmp
/// ```
///
/// becomes `memcpy dst, src` when `tmp` has no other use. This is a local
/// copy-chain rewrite; unlike general aggregate forwarding it does not remove
/// the final copy or change any load/store aliasing.
pub(crate) fn forward_memcpy_chains(module: &mut IrModule) -> usize {
    let mut total = 0;
    for func in &mut module.functions {
        if func.is_declaration {
            continue;
        }

        // Function-wide use counts. The intermediate `tmp` of a chain
        //   memcpy tmp, src
        //   memcpy dst, tmp
        // must be referenced by NOTHING except those two memcpys (dest of the
        // first, src of the second). In particular, uses of `tmp` in OTHER
        // blocks (loop bodies reading the variable whose slot `tmp` holds)
        // make the rewrite unsound: dropping the first memcpy would leave
        // `tmp`'s slot stale for those readers (adler32 dual-loop
        // miscompile: vs1's slot was never initialized/updated while the
        // wide/narrow loops kept reading it).
        let mut uses: FxHashMap<u32, u32> = FxHashMap::default();
        let allocas: FxHashSet<u32> = func
            .blocks
            .iter()
            .flat_map(|b| b.instructions.iter())
            .filter_map(|i| match i {
                Instruction::Alloca { dest, .. } => Some(dest.0),
                _ => None,
            })
            .collect();
        for block in &func.blocks {
            for inst in &block.instructions {
                for v in inst.used_values() {
                    *uses.entry(v).or_insert(0) += 1;
                }
            }
            for v in block.terminator.used_values() {
                *uses.entry(v).or_insert(0) += 1;
            }
        }

        for block in &mut func.blocks {
            let mut i = 0;
            while i < block.instructions.len() {
                let (tmp, src) = match block.instructions[i] {
                    Instruction::Memcpy { dest, src, .. } if dest != src => (dest, src),
                    _ => {
                        i += 1;
                        continue;
                    }
                };
                // Soundness: `tmp` may be referenced only by this memcpy's
                // dest and the chain consumer's src (2 uses total), and must be an alloca.
                if !allocas.contains(&tmp.0) || uses.get(&tmp.0).copied().unwrap_or(0) != 2 {
                    i += 1;
                    continue;
                }
                let mut consumer = None;
                let mut safe = true;
                for j in (i + 1)..block.instructions.len() {
                    let inst = &block.instructions[j];
                    match inst {
                        Instruction::Memcpy { src: copy_src, .. } if *copy_src == tmp => {
                            if consumer.is_some() {
                                safe = false;
                                break;
                            }
                            consumer = Some(j);
                        }
                        _ if inst.used_values().into_iter().any(|value| value == tmp.0) => {
                            safe = false;
                            break;
                        }
                        _ => {}
                    }
                    // SOUNDNESS: if the source location `src` is modified between
                    // the two memcpys, forwarding would read stale data.  Check
                    // for stores/memcpys/intrinsics that write to `src`.
                    // Without this, zlib-ng's fold_1 pattern
                    //   x_tmp3 = *c3; *c3 = *c0; ... *c2 = x_tmp3;
                    // was miscompiled: x_tmp3 was forwarded to re-read *c3,
                    // but *c3 had already been overwritten by *c0.
                    match inst {
                        Instruction::Store { ptr, .. } if *ptr == src => {
                            safe = false;
                            break;
                        }
                        Instruction::Memcpy { dest, .. } if *dest == src => {
                            safe = false;
                            break;
                        }
                        Instruction::Intrinsic {
                            dest_ptr: Some(dp), ..
                        } if *dp == src => {
                            safe = false;
                            break;
                        }
                        _ => {}
                    }
                }
                let Some(consumer) = consumer else {
                    i += 1;
                    continue;
                };
                if !safe {
                    i += 1;
                    continue;
                }
                if let Instruction::Memcpy { src: copy_src, .. } = &mut block.instructions[consumer]
                {
                    *copy_src = src;
                }
                block.instructions.remove(i);
                if block.source_spans.len() > i {
                    block.source_spans.remove(i);
                }
                total += 1;
            }
        }
    }
    total
}

/// Remove a large local aggregate copy when every observation is a scalar load
/// from the destination after the copy. Each load gets a new source-rooted GEP,
/// so no pointer is rewritten before its source alloca dominates it.
pub(crate) fn forward_large_memcpy_loads(module: &mut IrModule) -> usize {
    let mut total = 0;
    for func in &mut module.functions {
        if func.is_declaration {
            continue;
        }
        'restart: loop {
            for block_idx in 0..func.blocks.len() {
                for copy_idx in 0..func.blocks[block_idx].instructions.len() {
                    let (dest, src, size) = match func.blocks[block_idx].instructions[copy_idx] {
                        Instruction::Memcpy { dest, src, size } => (dest, src, size),
                        _ => continue,
                    };
                    if size < 128
                        || dest == src
                        || !is_local_alloca(func, dest)
                        || !is_local_alloca(func, src)
                    {
                        continue;
                    }

                    let dest_derived = collect_derived_pointers(func, dest);
                    let mut loads = Vec::with_capacity(16);
                    let mut safe = true;
                    for (bi, block) in func.blocks.iter().enumerate() {
                        for (ii, inst) in block.instructions.iter().enumerate() {
                            if bi == block_idx && ii == copy_idx {
                                continue;
                            }
                            let uses = inst
                                .used_values()
                                .into_iter()
                                .any(|v| dest_derived.contains(&v));
                            if !uses {
                                continue;
                            }
                            match inst {
                                Instruction::GetElementPtr { base, .. }
                                    if dest_derived.contains(&base.0) => {}
                                Instruction::Load { ptr, .. }
                                    if dest_derived.contains(&ptr.0)
                                        && bi == block_idx
                                        && ii > copy_idx =>
                                {
                                    loads.push((bi, ii, *ptr));
                                }
                                _ => {
                                    safe = false;
                                    break;
                                }
                            }
                        }
                        if !safe {
                            break;
                        }
                        if block
                            .terminator
                            .used_values()
                            .into_iter()
                            .any(|v| dest_derived.contains(&v))
                        {
                            safe = false;
                            break;
                        }
                    }
                    if !safe || loads.is_empty() {
                        continue;
                    }

                    // No source write, call, or opaque operation may occur
                    // between the copy and any forwarded load on this path.
                    for &(_, load_idx, _) in &loads {
                        for inst in &func.blocks[block_idx].instructions[copy_idx + 1..load_idx] {
                            if matches!(
                                inst,
                                Instruction::Store { .. }
                                    | Instruction::Memcpy { .. }
                                    | Instruction::Call { .. }
                                    | Instruction::CallIndirect { .. }
                                    | Instruction::InlineAsm { .. }
                                    | Instruction::Intrinsic { .. }
                            ) {
                                safe = false;
                                break;
                            }
                        }
                        if !safe {
                            break;
                        }
                    }
                    if !safe {
                        continue;
                    }

                    // Resolve each destination-rooted pointer to a constant
                    // byte offset. Dynamic GEPs are rejected rather than
                    // guessing an alias relation.
                    let mut gep_defs: FxHashMap<u32, (u32, i64, crate::common::types::IrType)> =
                        FxHashMap::default();
                    for block in &func.blocks {
                        for inst in &block.instructions {
                            if let Instruction::GetElementPtr {
                                dest,
                                base,
                                offset: Operand::Const(c),
                                ty,
                            } = inst
                            {
                                if let Some(offset) = c.to_i64() {
                                    gep_defs.insert(dest.0, (base.0, offset, *ty));
                                }
                            }
                        }
                    }
                    let resolve = |ptr: Value| -> Option<(i64, crate::common::types::IrType)> {
                        let mut current = ptr.0;
                        let mut offset = 0i64;
                        let mut ty = crate::common::types::IrType::Ptr;
                        let mut seen = FxHashSet::default();
                        loop {
                            if !seen.insert(current) {
                                return None;
                            }
                            if current == dest.0 {
                                return Some((offset, ty));
                            }
                            let &(base, delta, gep_ty) = gep_defs.get(&current)?;
                            offset = offset.checked_add(delta)?;
                            ty = gep_ty;
                            current = base;
                        }
                    };
                    let mut resolved = Vec::with_capacity(loads.len());
                    for &(bi, ii, ptr) in &loads {
                        let Some((offset, ty)) = resolve(ptr) else {
                            safe = false;
                            break;
                        };
                        resolved.push((bi, ii, offset, ty));
                    }
                    if !safe {
                        continue;
                    }

                    // Insert source-rooted GEPs immediately before loads, in
                    // reverse order to keep indices stable.
                    for &(_, load_idx, offset, ty) in resolved.iter().rev() {
                        let new_ptr = Value(func.next_value_id);
                        func.next_value_id += 1;
                        let gep = Instruction::GetElementPtr {
                            dest: new_ptr,
                            base: src,
                            offset: Operand::Const(IrConst::I64(offset)),
                            ty,
                        };
                        func.blocks[block_idx].instructions.insert(load_idx, gep);
                        if func.blocks[block_idx].source_spans.len() >= load_idx {
                            func.blocks[block_idx]
                                .source_spans
                                .insert(load_idx, crate::common::source::Span::dummy());
                        }
                        // The load shifted by one; locate the first load at or
                        // after the insertion point with the old destination-rooted ptr.
                        for inst in &mut func.blocks[block_idx].instructions[load_idx + 1..] {
                            if let Instruction::Load { ptr, .. } = inst {
                                if dest_derived.contains(&ptr.0) {
                                    *ptr = new_ptr;
                                    break;
                                }
                            }
                        }
                    }
                    func.blocks[block_idx].instructions.remove(copy_idx);
                    if func.blocks[block_idx].source_spans.len() > copy_idx {
                        func.blocks[block_idx].source_spans.remove(copy_idx);
                    }
                    total += 1;
                    continue 'restart;
                }
            }
            break;
        }
    }
    total
}

fn is_local_alloca(func: &IrFunction, value: Value) -> bool {
    func.blocks.iter().any(|block| {
        block.instructions.iter().any(|inst| {
        matches!(inst, Instruction::Alloca { dest, volatile: false, .. } if *dest == value)
    })
    })
}

fn collect_derived_pointers(func: &IrFunction, root: Value) -> FxHashSet<u32> {
    let mut derived = FxHashSet::default();
    derived.insert(root.0);
    let mut changed = true;
    while changed {
        changed = false;
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Instruction::GetElementPtr { dest, base, .. } = inst {
                    if derived.contains(&base.0) && derived.insert(dest.0) {
                        changed = true;
                    }
                }
            }
        }
    }
    derived
}

/// Propagate copies within a single function.
pub(crate) fn propagate_copies(func: &mut IrFunction) -> usize {
    let max_id = func.max_value_id() as usize;

    // Phase 1: Build the copy map as a flat lookup table (dest -> resolved source)
    let (copy_map, has_copies) = build_copy_map(func, max_id);

    // Early exit if no copies found (avoids scanning the entire copy_map Vec)
    if !has_copies {
        return 0;
    }

    // Phase 2: Replace all uses of copied values
    let mut replacements = 0;

    for block in &mut func.blocks {
        for inst in &mut block.instructions {
            replacements += replace_operands_in_instruction(inst, &copy_map);
        }
        replacements += replace_operands_in_terminator(&mut block.terminator, &copy_map);
    }

    replacements
}

/// Build a flat lookup table from Copy destinations to their ultimate sources.
/// Follows chains: if %a = Copy %b and %b = Copy %c, resolves %a -> %c.
/// Returns (copy_map, has_any_entries) to avoid scanning the map for emptiness.
///
/// Uses path compression: when resolving a chain, all intermediate entries are
/// updated to point directly to the final resolved value. This makes
/// resolution amortized O(1) per entry instead of O(chain_length).
/// Also avoids allocating a separate `resolved` vector by resolving in-place.
fn build_copy_map(func: &IrFunction, max_id: usize) -> (Vec<Option<Operand>>, bool) {
    // First pass: collect direct copy relationships into flat table.
    // If a Value has multiple Copy definitions (e.g. from single-phi elimination),
    // we mark it as multi-def using a sentinel (self-referencing copy) and skip it.
    let mut direct: Vec<Option<Operand>> = vec![None; max_id + 1];
    let mut has_copies = false;

    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Copy { dest, src } = inst {
                let id = dest.0 as usize;
                if id < direct.len() {
                    if direct[id].is_some() {
                        // Multi-def: mark with self-referencing sentinel
                        direct[id] = Some(Operand::Value(Value(id as u32)));
                    } else {
                        direct[id] = Some(*src);
                        has_copies = true;
                    }
                }
            }
        }
    }

    if !has_copies {
        return (direct, false);
    }

    // Second pass: resolve chains in-place with path compression.
    // We resolve each entry to its ultimate source and memoize the result
    // back into `direct`, so subsequent lookups of intermediate entries
    // are O(1). Uses iterative chain walking to avoid stack overflow.
    let mut any_resolved = false;
    for i in 0..=max_id {
        if direct[i].is_some() {
            resolve_chain_with_compression(&mut direct, i as u32);
            // After compression, check if this is a valid (non-self-ref) entry
            if let Some(Operand::Value(v)) = direct[i] {
                if v.0 == i as u32 {
                    // Self-referencing (multi-def sentinel or cycle) - clear it
                    direct[i] = None;
                    continue;
                }
            }
            if direct[i].is_some() {
                any_resolved = true;
            }
        }
    }

    (direct, any_resolved)
}

/// Resolve a copy chain starting at `start` with path compression.
/// Walks the chain to find the ultimate source, then updates all intermediate
/// entries to point directly to it (path compression / union-find style).
fn resolve_chain_with_compression(copies: &mut [Option<Operand>], start: u32) {
    // First, find the ultimate source by walking the chain.
    let mut current = start;
    let mut depth = 0;
    const MAX_DEPTH: usize = 64;

    let ultimate = loop {
        if depth >= MAX_DEPTH {
            break Operand::Value(Value(current));
        }

        let idx = current as usize;
        match if idx < copies.len() {
            copies[idx]
        } else {
            None
        } {
            Some(Operand::Value(v)) => {
                if v.0 == current {
                    // Self-reference (multi-def or cycle)
                    break Operand::Value(Value(current));
                }
                current = v.0;
                depth += 1;
            }
            Some(Operand::Const(c)) => {
                break Operand::Const(c);
            }
            None => {
                break Operand::Value(Value(current));
            }
        }
    };

    // Path compression: walk the chain from start and update every
    // intermediate entry to point directly to `ultimate`.
    // For short chains (depth <= 1), we only need to update start itself.
    let start_idx = start as usize;
    if depth <= 1 {
        if start_idx < copies.len() && copies[start_idx].is_some() {
            copies[start_idx] = Some(ultimate);
        }
        return;
    }

    // For longer chains, walk from start and compress each hop.
    // We track the ultimate value ID (if it's a Value) to know when to stop.
    let ultimate_id = match ultimate {
        Operand::Value(v) => Some(v.0),
        Operand::Const(_) => None,
    };
    let mut current = start;
    for _ in 0..depth {
        let idx = current as usize;
        if idx >= copies.len() {
            break;
        }
        match copies[idx] {
            Some(Operand::Value(v)) if v.0 != current => {
                let next = v.0;
                copies[idx] = Some(ultimate);
                // Stop if next is the ultimate target (nothing more to compress)
                if ultimate_id == Some(next) {
                    break;
                }
                current = next;
            }
            _ => break,
        }
    }
}

/// Replace operands in an instruction that reference copied values.
/// Returns the number of replacements made.
fn replace_operands_in_instruction(inst: &mut Instruction, copy_map: &[Option<Operand>]) -> usize {
    let mut count = 0;

    match inst {
        Instruction::Alloca { .. } | Instruction::PgoCounterInc { .. } => {}
        Instruction::GetStaticChain { .. } => {}
        Instruction::SetStaticChain { src } => {
            count += replace_operand(src, copy_map);
        }
        Instruction::InitTrampoline { buffer, chain, .. } => {
            count += replace_value_in_place(buffer, copy_map);
            count += replace_operand(chain, copy_map);
        }
        Instruction::NonlocalGotoSave { frame, .. } => {
            count += replace_value_in_place(frame, copy_map);
        }
        Instruction::NonlocalGoto { chain, .. } => {
            count += replace_operand(chain, copy_map);
        }
        Instruction::DynAlloca { size, .. } => {
            count += replace_operand(size, copy_map);
        }
        Instruction::Store { val, ptr, .. } => {
            count += replace_operand(val, copy_map);
            count += replace_value_in_place(ptr, copy_map);
        }
        Instruction::Load { ptr, .. } => {
            count += replace_value_in_place(ptr, copy_map);
        }
        Instruction::BinOp { lhs, rhs, .. } => {
            count += replace_operand(lhs, copy_map);
            count += replace_operand(rhs, copy_map);
        }
        Instruction::UnaryOp { src, .. } => {
            count += replace_operand(src, copy_map);
        }
        Instruction::Cmp { lhs, rhs, .. } => {
            count += replace_operand(lhs, copy_map);
            count += replace_operand(rhs, copy_map);
        }
        Instruction::Call { info, .. } => {
            for arg in info.args.iter_mut() {
                count += replace_operand(arg, copy_map);
            }
        }
        Instruction::CallIndirect { func_ptr, info } => {
            count += replace_operand(func_ptr, copy_map);
            for arg in info.args.iter_mut() {
                count += replace_operand(arg, copy_map);
            }
        }
        Instruction::GetElementPtr { base, offset, .. } => {
            count += replace_value_in_place(base, copy_map);
            count += replace_operand(offset, copy_map);
        }
        Instruction::Cast { src, .. } => {
            count += replace_operand(src, copy_map);
        }
        Instruction::Copy { src, .. } => {
            count += replace_operand(src, copy_map);
        }
        Instruction::GlobalAddr { .. } => {}
        Instruction::Memcpy { dest, src, .. } => {
            count += replace_value_in_place(dest, copy_map);
            count += replace_value_in_place(src, copy_map);
        }
        Instruction::VaArg { va_list_ptr, .. } => {
            count += replace_value_in_place(va_list_ptr, copy_map);
        }
        Instruction::VaStart { va_list_ptr } => {
            count += replace_value_in_place(va_list_ptr, copy_map);
        }
        Instruction::VaEnd { va_list_ptr } => {
            count += replace_value_in_place(va_list_ptr, copy_map);
        }
        Instruction::VaCopy { dest_ptr, src_ptr } => {
            count += replace_value_in_place(dest_ptr, copy_map);
            count += replace_value_in_place(src_ptr, copy_map);
        }
        Instruction::VaArgStruct {
            dest_ptr,
            va_list_ptr,
            ..
        } => {
            count += replace_value_in_place(dest_ptr, copy_map);
            count += replace_value_in_place(va_list_ptr, copy_map);
        }
        Instruction::AtomicRmw { ptr, val, .. } => {
            count += replace_operand(ptr, copy_map);
            count += replace_operand(val, copy_map);
        }
        Instruction::AtomicInc { ptr, .. } => {
            count += replace_operand(ptr, copy_map);
        }
        Instruction::AtomicCmpxchg {
            ptr,
            expected,
            desired,
            ..
        } => {
            count += replace_operand(ptr, copy_map);
            count += replace_operand(expected, copy_map);
            count += replace_operand(desired, copy_map);
        }
        Instruction::AtomicLoad { ptr, .. } => {
            count += replace_operand(ptr, copy_map);
        }
        Instruction::AtomicStore { ptr, val, .. } => {
            count += replace_operand(ptr, copy_map);
            count += replace_operand(val, copy_map);
        }
        Instruction::Fence { .. } => {}
        Instruction::Phi { incoming, .. } => {
            for (op, _label) in incoming.iter_mut() {
                count += replace_operand(op, copy_map);
            }
        }
        Instruction::LabelAddr { .. } => {}
        Instruction::GetReturnF64Second { .. } => {}
        Instruction::GetReturnF32Second { .. } => {}
        Instruction::GetReturnF128Second { .. } => {}
        Instruction::SetReturnF64Second { src } => {
            count += replace_operand(src, copy_map);
        }
        Instruction::SetReturnF32Second { src } => {
            count += replace_operand(src, copy_map);
        }
        Instruction::SetReturnF128Second { src } => {
            count += replace_operand(src, copy_map);
        }
        Instruction::InlineAsm {
            outputs, inputs, ..
        } => {
            // Output pointers are address operands (e.g. "=r"(*p) derefs): they
            // must participate in copy propagation like Intrinsic dest_ptr
            // below. Without this, a Copy/Load that materialized the pointer
            // becomes the asm output's only def; DCE then removes it (the asm
            // output is treated as a use, but propagation never rewrote it),
            // leaving a dangling output that slot_assignment promotes to a
            // direct slot, so the asm result is stored into the wrong slot
            // instead of through the pointer (wrong code, e.g. cpuid.h).
            for (_constraint, ptr, _name) in outputs.iter_mut() {
                count += replace_value_in_place(ptr, copy_map);
            }
            for (_constraint, op, _name) in inputs.iter_mut() {
                count += replace_operand(op, copy_map);
            }
        }
        Instruction::Intrinsic { dest_ptr, args, .. } => {
            // dest_ptr is an address operand (e.g. Storedqu's target); it must
            // participate in copy propagation or a surviving `Copy p = q`
            // leaves p with no register and no slot, and the backend emits a
            // store through a stale register (wrong-code).
            if let Some(dp) = dest_ptr.as_mut() {
                count += replace_value_in_place(dp, copy_map);
            }
            for arg in args.iter_mut() {
                count += replace_operand(arg, copy_map);
            }
        }
        Instruction::Select {
            cond,
            true_val,
            false_val,
            ..
        } => {
            count += replace_operand(cond, copy_map);
            count += replace_operand(true_val, copy_map);
            count += replace_operand(false_val, copy_map);
        }
        Instruction::StackSave { .. } => {}
        Instruction::StackRestore { ptr } => {
            count += replace_value_in_place(ptr, copy_map);
        }
        Instruction::ParamRef { .. } => {}
    }

    count
}

/// Replace operands in a terminator.
fn replace_operands_in_terminator(term: &mut Terminator, copy_map: &[Option<Operand>]) -> usize {
    let mut count = 0;
    match term {
        Terminator::Return(Some(val)) => {
            count += replace_operand(val, copy_map);
        }
        Terminator::Return(None) => {}
        Terminator::Branch(_) => {}
        Terminator::CondBranch { cond, .. } => {
            count += replace_operand(cond, copy_map);
        }
        Terminator::IndirectBranch { target, .. } => {
            count += replace_operand(target, copy_map);
        }
        Terminator::Switch { val, .. } => {
            count += replace_operand(val, copy_map);
        }
        Terminator::Unreachable => {}
    }
    count
}

/// Replace an Operand if it references a copied value.
/// Returns 1 if a replacement was made, 0 otherwise.
#[inline]
fn replace_operand(op: &mut Operand, copy_map: &[Option<Operand>]) -> usize {
    if let Operand::Value(v) = op {
        let idx = v.0 as usize;
        if let Some(Some(replacement)) = copy_map.get(idx) {
            *op = *replacement;
            return 1;
        }
    }
    0
}

/// Replace a Value in-place if it references a copied value.
/// Only replaces if the resolved source is also a Value (not a Const).
/// Returns 1 if a replacement was made, 0 otherwise.
#[inline]
fn replace_value_in_place(val: &mut Value, copy_map: &[Option<Operand>]) -> usize {
    let idx = val.0 as usize;
    if let Some(Some(Operand::Value(new_val))) = copy_map.get(idx) {
        *val = *new_val;
        return 1;
    }
    0
}

/// ms178: Post-phi-elimination copy cleanup.
///
/// `eliminate_phis` runs LAST in the pipeline, so the copy-backs it introduces
/// are never optimized: every phi copy-back becomes a store→load→store relay in
/// the emitted asm (huge spill churn in hot loops; gzip's `longest_match` had
/// 120 copies surviving to codegen).
///
/// We cannot naively run `propagate_copies` here — the IR is no longer in SSA
/// form, and a copy dest may be used before its copy executes in a loop (that
/// is precisely the phi semantics). Instead we apply two provably sound
/// transforms, in a fixpoint loop:
///
/// 1. **Dead-copy elimination**: remove `Copy(dest, src)` when `dest` has zero
///    uses anywhere. No use, no semantics.
///
/// 2. **Same-block, after-copy propagation through single-def values**: for
///    `Copy(dest, src)` at instruction index `i` in block B, if every use of
///    `dest` is an instruction operand in B at an index > `i` (never in a
///    terminator, never in another block) AND `src` is single-def (defined by
///    exactly one instruction, or a constant/parameter — hence never modified
///    between the copy and any use), then replacing those uses with `src` is
///    equivalent. Removes the copy.
///
/// The single-def requirement on `src` restores the SSA guarantee that the
/// source's value is invariant, which is what makes the substitution sound
/// despite the IR no longer being SSA. Loop-carried phi copy-backs (src
/// multi-def) are conservatively kept.
///
/// Returns the number of copies removed.
pub fn propagate_copies_post_phi(func: &mut IrFunction) -> usize {
    let mut total = 0usize;
    // Fixpoint: chains (Copy(a,b); Copy(b,c)) collapse one link per round, and
    // dead copies cascade. Chains are short; cap at 8 rounds.
    for _ in 0..8 {
        let removed = one_round_post_phi(func);
        total += removed;
        if removed == 0 {
            break;
        }
    }
    total
}

fn one_round_post_phi(func: &mut IrFunction) -> usize {
    let max_id = func.max_value_id() as usize;
    let mut def_count = vec![0u32; max_id + 1];
    let mut def_position: Vec<Option<(usize, usize)>> = vec![None; max_id + 1];
    let mut use_count = vec![0u32; max_id + 1];
    let mut use_positions: Vec<Vec<(usize, usize)>> = vec![Vec::new(); max_id + 1];
    // Uses in *Value-typed* instruction fields (Load.ptr, Store.ptr, GEP.base,
    // Memcpy dest/src, va_list pointers, InlineAsm output pointers, ...).
    // These fields structurally CANNOT hold a constant, so a copy whose
    // resolved source is a Const must never be removed while such a use
    // exists: `replace_value_in_place` cannot rewrite it and the use dangles.
    // glibc __libc_start_main_impl hit exactly this — `v = Copy(Const 16);
    // Load %fs:(v)` (TLS stack-guard) lost its only def and the backend's
    // no-home hard gate fired at the SegFs load.
    let mut value_pos_uses = vec![0u32; max_id + 1];
    let mut copies: Vec<(usize, usize, Value, Operand)> = Vec::new(); // (bi, ii, dest, src)

    // Count definitions (single-def test) and uses.
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            match inst {
                Instruction::Copy { dest, src } => {
                    if (dest.0 as usize) <= max_id {
                        def_count[dest.0 as usize] += 1;
                        def_position[dest.0 as usize] = Some((bi, ii));
                    }
                    if let Operand::Value(v) = src {
                        if (v.0 as usize) <= max_id {
                            use_count[v.0 as usize] += 1;
                            use_positions[v.0 as usize].push((bi, ii));
                        }
                    }
                    copies.push((bi, ii, *dest, *src));
                }
                _ => {
                    if let Some(d) = inst.dest() {
                        if (d.0 as usize) <= max_id {
                            def_count[d.0 as usize] += 1;
                            def_position[d.0 as usize] = Some((bi, ii));
                        }
                    }
                    // Operand uses (BinOp/Cmp/Call args/Cast src/GEP offset/...)
                    crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                        if let Operand::Value(v) = op {
                            if (v.0 as usize) <= max_id {
                                use_count[v.0 as usize] += 1;
                                use_positions[v.0 as usize].push((bi, ii));
                            }
                        }
                    });
                    // Value-only uses (Load.ptr, Store.ptr, GEP.base, Memcpy,
                    // CallIndirect func_ptr, va_* pointers, atomic ptrs). These
                    // are uses too — missing them made the pass think a copy
                    // dest was dead and remove a copy that a Store/Load still
                    // depended on (miscompiled inflate/deflate).
                    crate::backend::liveness::for_each_value_use_in_instruction(inst, |v| {
                        if (v.0 as usize) <= max_id {
                            use_count[v.0 as usize] += 1;
                            use_positions[v.0 as usize].push((bi, ii));
                            value_pos_uses[v.0 as usize] += 1;
                        }
                    });
                }
            }
        }
        // Terminator uses (Return value, CondBranch cond, Switch val,
        // IndirectBranch target) — recorded at (bi, usize::MAX) so they count
        // as "after any instruction in the block".
        crate::backend::liveness::for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                if (v.0 as usize) <= max_id {
                    use_count[v.0 as usize] += 1;
                    use_positions[v.0 as usize].push((bi, usize::MAX));
                }
            }
        });
    }

    // Decide which copies to remove.
    let mut to_remove: Vec<(usize, usize, Value, Operand)> = Vec::new();
    let mut dbg_kept = [0usize; 4]; // [other_block, use_before, src_multidef, would_change]
    for &(bi, ii, dest, ref src) in &copies {
        let id = dest.0 as usize;
        let empty: &[(usize, usize)] = &[];
        let uses: &[(usize, usize)] = if id < use_positions.len() {
            use_positions[id].as_slice()
        } else {
            empty
        };
        // Case 1: dead copy.
        if uses.is_empty() {
            to_remove.push((bi, ii, dest, *src));
            continue;
        }
        // ms178 debug: restrict to dead-copy-only mode when asked.
        if std::env::var("CCC_PHICLEANUP_DEADONLY").is_ok() {
            continue;
        }
        // Case 2: same-block-after AND src provably available before the copy
        // AND dest single-def.
        //
        // SOUNDNESS: the IR is not SSA here. A loop-carried src may be defined
        // AFTER the copy in program order (its def executes at the end of the
        // previous iteration) — substituting it would read an undefined value on
        // the first iteration. So we require src to be a constant, a parameter,
        // or defined in the SAME block at an index BEFORE the copy. That
        // guarantees src holds the same value at every rewritten use.
        let src_available = match src {
            Operand::Const(_) => true,
            Operand::Value(v) => {
                if (v.0 as usize) > max_id {
                    false
                } else {
                    match def_position[v.0 as usize] {
                        Some((db, di)) => db == bi && di < ii,
                        None => false, // undefined value
                    }
                }
            }
        };
        if !src_available {
            dbg_kept[2] += 1;
            continue;
        }
        if def_count[id] != 1 {
            continue;
        }
        let mut all_after = true;
        let mut reason = 0usize; // 0=other block, 1=use before
        for &(ub, ui) in uses {
            if ub != bi {
                all_after = false;
                reason = 0;
                break;
            }
            if ui <= ii {
                all_after = false;
                reason = 1;
                break;
            }
        }
        if all_after {
            to_remove.push((bi, ii, dest, *src));
        } else {
            dbg_kept[reason] += 1;
        }
    }
    if std::env::var("CCC_DEBUG_PHICLEANUP").is_ok() {
        eprintln!(
            "[PHICLEANUP] fn={} copies={} removed={} kept_other_block={} kept_use_before={} kept_src_multidef={}",
            func.name,
            copies.len(),
            to_remove.len(),
            dbg_kept[0],
            dbg_kept[1],
            dbg_kept[2]
        );
    }

    if to_remove.is_empty() {
        return 0;
    }

    // Apply: replace same-block-after uses of removed copies with their
    // RESOLVED source. Chained copies (d1=Copy(s1); d2=Copy(d1)) must resolve
    // d2's source to s1 — substituting the intermediate d1 would leave a
    // dangling reference once d1's own copy is removed.
    //
    // Resolution is a single memoised sweep in program order.  A removable
    // copy's src is (by the `src_available` rule above) a constant, or a value
    // defined in the SAME block at an EARLIER index; and its dest is
    // single-def.  Hence every chain link points at a strictly earlier entry
    // of `to_remove` (which is in program order), so `resolved[k]` only ever
    // needs `resolved[j]` for j < k — O(n) total, exact for chains of any
    // length.  The previous implementation walked each chain with a linear
    // `find` and a hard `depth < 32` cap: a 35-link chain (the `fails`
    // accumulator of a stress-lab `main` after every check block folded to
    // `Copy`) stopped at an intermediate dest that was itself removed, and
    // the backend then met `Copy v393 = v359` with v359 undefined (-O1 ICE
    // "operand_to_rax: value has no register home" in divmod/shifts/builtins
    // seeds; tests/regression/post_phi_copy_chain_long.c).
    let resolved_src: Vec<Operand> = {
        let mut dest_to_idx: crate::common::fx_hash::FxHashMap<u32, usize> =
            crate::common::fx_hash::FxHashMap::default();
        let mut resolved: Vec<Operand> = Vec::with_capacity(to_remove.len());
        for (k, &(_, _, dest, src)) in to_remove.iter().enumerate() {
            let r = match src {
                Operand::Value(v) => match dest_to_idx.get(&v.0) {
                    Some(&j) => resolved[j],
                    None => src,
                },
                Operand::Const(_) => src,
            };
            resolved.push(r);
            dest_to_idx.insert(dest.0, k);
        }
        // Invariant: no resolved source may name a dest that is being removed
        // (that is exactly the dangling-reference failure mode above).
        debug_assert!(resolved.iter().all(|r| match r {
            Operand::Value(v) => !dest_to_idx.contains_key(&v.0),
            Operand::Const(_) => true,
        }));
        resolved
    };
    // Const-resolved removals must not orphan Value-position uses (Load.ptr,
    // Store.ptr, GEP.base, ...): those fields cannot hold a constant, so
    // `replace_value_in_place` would silently skip them and the removed copy's
    // dest would dangle. This covers both direct `Copy(Const)` and chains that
    // RESOLVE to a Const through other removed copies. Dropping an entry is
    // always sound: its copy simply stays, and chains that resolved through it
    // still substitute the same resolved value into their (operand) uses.
    let (to_remove, resolved_src): (Vec<_>, Vec<_>) = to_remove
        .into_iter()
        .zip(resolved_src)
        .filter(|((_, _, dest, _), rsrc)| {
            !(matches!(rsrc, Operand::Const(_))
                && value_pos_uses.get(dest.0 as usize).copied().unwrap_or(0) > 0)
        })
        .unzip();
    if to_remove.is_empty() {
        return 0;
    }
    let removed = to_remove.len();
    for (idx, (bi, ii, dest, _)) in to_remove.iter().enumerate() {
        // Replace uses of dest in block bi at indices > ii — INCLUDING the
        // block's terminator (a terminator use is recorded at (bi, usize::MAX),
        // passes the "after" check, and executes after all instructions in the
        // block; leaving it unreplaced would dangle after the copy is removed).
        let map = copy_single_map(*dest, resolved_src[idx]);
        let block = &mut func.blocks[*bi];
        for inst in block.instructions.iter_mut().skip(ii + 1) {
            replace_operands_in_instruction(inst, &map);
        }
        let term = &mut func.blocks[*bi].terminator;
        replace_operands_in_terminator(term, &map);
    }

    // Remove the copies themselves (rebuild each block's instruction list,
    // dropping the marked copy indices).
    {
        use crate::common::fx_hash::FxHashSet;
        let mut per_block: Vec<FxHashSet<usize>> = vec![FxHashSet::default(); func.blocks.len()];
        for &(bi, ii, _, _) in &to_remove {
            if bi < per_block.len() {
                per_block[bi].insert(ii);
            }
        }
        for (bi, block) in func.blocks.iter_mut().enumerate() {
            let to_drop = &per_block[bi];
            let mut new_list: Vec<Instruction> = Vec::with_capacity(block.instructions.len());
            for (ii, inst) in block.instructions.drain(..).enumerate() {
                if !to_drop.contains(&ii) {
                    new_list.push(inst);
                }
            }
            block.instructions = new_list;
        }
    }

    removed
}

/// A tiny single-entry copy map used by the post-phi substitution.
fn copy_single_map(dest: Value, src: Operand) -> Vec<Option<Operand>> {
    let mut m = Vec::with_capacity(16);
    let cap = dest.0 as usize + 1;
    m.resize(cap, None);
    m[dest.0 as usize] = Some(src);
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::IrType;
    use crate::ir::reexports::{BasicBlock, BlockId, IrBinOp, IrConst};

    /// Post-phi cleanup must resolve copy chains of ANY length exactly.  The
    /// old resolver capped the walk at 32 links; a longer chain resolved to
    /// an intermediate dest that was itself removed and the survivor read an
    /// undefined value (backend hard gate ICE at -O1).  Chain of 40 links,
    /// last link consumed by a Cmp in another block (so it is kept) and by a
    /// Return: after cleanup every remaining operand must be defined.
    #[test]
    fn test_post_phi_long_chain_no_dangling_use() {
        const N: u32 = 40;
        let mut func = IrFunction::new("test".to_string(), IrType::I32, vec![], false);
        let mut insts = vec![Instruction::Copy {
            dest: Value(1),
            src: Operand::Const(IrConst::I32(0)),
        }];
        for i in 1..N {
            insts.push(Instruction::Copy {
                dest: Value(i + 1),
                src: Operand::Value(Value(i)),
            });
        }
        // Last link feeds a compare in the same block AND a return in
        // another block, so it must survive while the chain collapses.
        insts.push(Instruction::Cmp {
            dest: Value(N + 1),
            op: crate::ir::reexports::IrCmpOp::Eq,
            lhs: Operand::Value(Value(N)),
            rhs: Operand::Const(IrConst::I32(0)),
            ty: IrType::I32,
        });
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: insts,
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(N + 1)),
                true_label: BlockId(1),
                false_label: BlockId(1),
            },
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![],
            terminator: Terminator::Return(Some(Operand::Value(Value(N)))),
            source_spans: Vec::new(),
        });

        let removed = propagate_copies_post_phi(&mut func);
        assert!(removed >= (N - 1) as usize, "chain should collapse, removed={removed}");

        // Every remaining Value operand must have a definition.
        let mut defined = std::collections::HashSet::new();
        for b in &func.blocks {
            for inst in &b.instructions {
                if let Some(d) = inst.dest() {
                    defined.insert(d.0);
                }
            }
        }
        for b in &func.blocks {
            for inst in &b.instructions {
                crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                    if let Operand::Value(v) = op {
                        assert!(defined.contains(&v.0), "dangling use of v{} in {:?}", v.0, inst);
                    }
                });
            }
            crate::backend::liveness::for_each_operand_in_terminator(&b.terminator, |op| {
                if let Operand::Value(v) = op {
                    assert!(defined.contains(&v.0), "dangling use of v{} in terminator", v.0);
                }
            });
        }
        // The surviving definition of the last link must be the constant.
        let last = func.blocks[0]
            .instructions
            .iter()
            .find(|i| matches!(i, Instruction::Copy { dest, .. } if dest.0 == N))
            .expect("last link kept");
        assert!(matches!(last, Instruction::Copy { src: Operand::Const(IrConst::I32(0)), .. }), "{last:?}");
    }

    #[test]
    fn test_simple_copy_propagation() {
        // %1 = Copy %0
        // %2 = Add %1, const(1)
        // Should become:
        // %1 = Copy %0 (dead, will be removed by DCE)
        // %2 = Add %0, const(1)
        let mut func = IrFunction::new("test".to_string(), IrType::I32, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Value(Value(0)),
                },
                Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(2)))),
            source_spans: Vec::new(),
        });

        let replacements = propagate_copies(&mut func);
        assert!(replacements > 0);

        // The BinOp should now reference %0 directly
        match &func.blocks[0].instructions[1] {
            Instruction::BinOp {
                lhs: Operand::Value(v),
                ..
            } => {
                assert_eq!(v.0, 0, "Should reference original value %0");
            }
            other => panic!("Expected BinOp, got {:?}", other),
        }
    }

    #[test]
    fn test_chain_copy_propagation() {
        // %1 = Copy %0
        // %2 = Copy %1
        // %3 = Add %2, const(1)
        // Should resolve %2 -> %0
        let mut func = IrFunction::new("test".to_string(), IrType::I32, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Value(Value(0)),
                },
                Instruction::Copy {
                    dest: Value(2),
                    src: Operand::Value(Value(1)),
                },
                Instruction::BinOp {
                    dest: Value(3),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(3)))),
            source_spans: Vec::new(),
        });

        let replacements = propagate_copies(&mut func);
        assert!(replacements > 0);

        // The BinOp should now reference %0 directly
        match &func.blocks[0].instructions[2] {
            Instruction::BinOp {
                lhs: Operand::Value(v),
                ..
            } => {
                assert_eq!(v.0, 0, "Should resolve chain to original value %0");
            }
            other => panic!("Expected BinOp, got {:?}", other),
        }
    }

    #[test]
    fn test_const_copy_propagation() {
        // %0 = Copy const(42)
        // %1 = Add %0, const(1)
        // Should propagate const(42) into the Add
        let mut func = IrFunction::new("test".to_string(), IrType::I32, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Copy {
                    dest: Value(0),
                    src: Operand::Const(IrConst::I32(42)),
                },
                Instruction::BinOp {
                    dest: Value(1),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(1)))),
            source_spans: Vec::new(),
        });

        let replacements = propagate_copies(&mut func);
        assert!(replacements > 0);

        // The BinOp should now have const(42) as lhs
        match &func.blocks[0].instructions[1] {
            Instruction::BinOp {
                lhs: Operand::Const(IrConst::I32(42)),
                ..
            } => {}
            other => panic!("Expected BinOp with const 42, got {:?}", other),
        }
    }

    #[test]
    fn test_terminator_propagation() {
        // %1 = Copy %0
        // return %1
        // Should become return %0
        let mut func = IrFunction::new("test".to_string(), IrType::I32, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::Copy {
                dest: Value(1),
                src: Operand::Value(Value(0)),
            }],
            terminator: Terminator::Return(Some(Operand::Value(Value(1)))),
            source_spans: Vec::new(),
        });

        let replacements = propagate_copies(&mut func);
        assert!(replacements > 0);

        match &func.blocks[0].terminator {
            Terminator::Return(Some(Operand::Value(v))) => {
                assert_eq!(v.0, 0, "Return should reference %0 directly");
            }
            other => panic!("Expected Return with %0, got {:?}", other),
        }
    }

    #[test]
    fn test_no_propagation_when_no_copies() {
        let mut func = IrFunction::new("test".to_string(), IrType::I32, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::BinOp {
                dest: Value(0),
                op: IrBinOp::Add,
                lhs: Operand::Const(IrConst::I32(1)),
                rhs: Operand::Const(IrConst::I32(2)),
                ty: IrType::I32,
            }],
            terminator: Terminator::Return(Some(Operand::Value(Value(0)))),
            source_spans: Vec::new(),
        });

        let replacements = propagate_copies(&mut func);
        assert_eq!(replacements, 0);
    }
}
