//! Forward reads from short-lived aggregate copies to their original storage.
//!
//! Inlining structure-returning functions often leaves IR like:
//! `memcpy(tmp, object); load(tmp.field)`.  When `tmp` never escapes or receives
//! another write, copying the complete aggregate is unnecessary.  This pass is
//! deliberately conservative: the copy and every read must be in the same
//! block, with the copy preceding the reads.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::reexports::{Instruction, IrFunction, Operand, Value};

fn type_size(ty: crate::common::types::IrType) -> i64 {
    use crate::common::types::IrType::*;
    match ty {
        I8 | U8 => 1,
        I16 | U16 => 2,
        I32 | U32 | F32 => 4,
        I64 | U64 | F64 | Ptr => 8,
        _ => 16,
    }
}

/// Remove stores to fields of non-escaping stack aggregates that are never read.
fn eliminate_dead_aggregate_field_stores(func: &mut IrFunction) -> usize {
    // Track aggregate roots separately from precise paths. Loop pointer phis
    // commonly merge offsets 0 and 48; they still share the same allocation
    // root even though `pointer_paths` deliberately rejects the differing paths.
    // suffix None = variable/unknown offset: such a pointer can reach ANY
    // byte of the aggregate. Loads through it read the whole root; stores
    // through it are never dead. (The previous code mapped variable offsets
    // to 0, so a `exp[i]` loop-load only "read" bytes 0..size and every
    // store at offset >= size was deleted — simd_vhaddps lane corruption.)
    let mut root_suffix: FxHashMap<u32, (u32, Option<i64>)> = FxHashMap::default();
    // Values whose defs reference two DIFFERENT roots: not attributable to a
    // single allocation; permanently untracked (tombstone prevents the
    // insert/remove oscillation a plain remove would cause in the fixpoint).
    let mut mixed_root: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca { dest, .. } = inst {
                root_suffix.insert(dest.0, (dest.0, Some(0)));
            }
        }
    }
    loop {
        let mut changed = false;
        for block in &func.blocks {
            for inst in &block.instructions {
                let derived = match inst {
                    Instruction::GetElementPtr {
                        dest, base, offset, ..
                    } => root_suffix.get(&base.0).copied().map(|(root, suffix)| {
                        let next = match (offset, suffix) {
                            (Operand::Const(c), Some(s)) => c.to_i64().map(|o| s + o),
                            _ => None, // variable or already-unknown offset
                        };
                        (dest.0, (root, next))
                    }),
                    Instruction::Copy {
                        dest,
                        src: Operand::Value(src),
                    } => root_suffix.get(&src.0).copied().map(|p| (dest.0, p)),
                    Instruction::Phi { dest, incoming, .. } => {
                        let vals: Vec<(u32, Option<i64>)> = incoming
                            .iter()
                            .filter_map(|(op, _)| match op {
                                Operand::Value(v) => root_suffix.get(&v.0).copied(),
                                _ => None,
                            })
                            .collect();
                        if !vals.is_empty() && vals.iter().all(|p| p.0 == vals[0].0) {
                            // Diverging offsets across phi arms => unknown, NOT 0.
                            let suffix = if vals.iter().all(|p| p.1 == vals[0].1) {
                                vals[0].1
                            } else {
                                None
                            };
                            Some((dest.0, (vals[0].0, suffix)))
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if let Some((dest, path)) = derived {
                    if mixed_root.contains(&dest) {
                        continue;
                    }
                    match root_suffix.get(&dest) {
                        None => {
                            root_suffix.insert(dest, path);
                            changed = true;
                        }
                        // MULTI-DEF pointer (post-phi Copy web: `p = base; ...;
                        // p = p+12` in a loop): the same SSA id denotes different
                        // offsets at different times. Keeping the first-seen
                        // suffix understated the read set and initializing
                        // stores got deleted (structs_bitfields). Same root =>
                        // demote to unknown offset; different root => not
                        // attributable to one allocation — tombstone (fail closed).
                        Some(&(old_root, old_suffix)) => {
                            if old_root == path.0 {
                                if old_suffix != path.1 && old_suffix.is_some() {
                                    root_suffix.insert(dest, (old_root, None));
                                    changed = true;
                                }
                            } else {
                                root_suffix.remove(&dest);
                                mixed_root.insert(dest);
                                changed = true;
                            }
                        }
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    let volatile_roots: FxHashSet<u32> = func
        .blocks
        .iter()
        .flat_map(|b| &b.instructions)
        .filter_map(|inst| match inst {
            Instruction::Alloca {
                dest,
                volatile: true,
                ..
            } => Some(dest.0),
            _ => None,
        })
        .collect();
    let mut escaping = FxHashSet::default();
    let mut loaded: FxHashMap<u32, Vec<(i64, i64)>> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Load { ptr, ty, .. } = inst {
                if let Some((root, suffix)) = root_suffix.get(&ptr.0) {
                    match suffix {
                        Some(off) => loaded
                            .entry(*root)
                            .or_default()
                            .push((*off, type_size(*ty))),
                        // Unknown offset: conservatively reads everything.
                        None => {
                            loaded
                                .entry(*root)
                                .or_default()
                                .push((i64::MIN / 4, i64::MAX / 2));
                        }
                    }
                }
            }
            // DEFAULT-CLOSED escape analysis: only instructions whose
            // memory semantics this pass models precisely (Load's read range,
            // Store's written range, and the GEP/Copy/Phi derivations above)
            // may reference a tracked root without escaping it. EVERYTHING
            // else — calls, intrinsics (dest_ptr AND args are raw pointers
            // into aggregates!), memcpy, atomics, inline asm, va_arg, selects,
            // terminator operands — escapes the root. The previous allowlist
            // missed Intrinsic entirely, so SSE/AVX stores into __m128i
            // allocas were "never read" and every initializing store was
            // deleted (simd regression cluster: paddd on zeroed inputs).
            match inst {
                Instruction::Load { .. } => {}
                Instruction::Store { ptr: _, val, .. } => {
                    // The written-to pointer is modeled; a tracked root used
                    // as the *value* being stored escapes (pointer written to
                    // memory can be reloaded and read anywhere).
                    if let Operand::Value(v) = val {
                        if let Some((root, _)) = root_suffix.get(&v.0) {
                            escaping.insert(*root);
                        }
                    }
                }
                Instruction::GetElementPtr { .. } | Instruction::Copy { .. } => {
                    // Pure derivations, tracked in root_suffix above.
                }
                Instruction::Phi { incoming, .. } => {
                    // A phi that merges pointers to DIFFERENT allocation roots
                    // is not a precise aggregate-field derivation.  The fixpoint
                    // deliberately tombstones the phi result in that case, but
                    // the incoming roots still flow to an untracked pointer; if
                    // a later load uses the phi, it may read any of those roots.
                    // Treat all such incoming roots as escaped instead of
                    // deleting their stores as "unread".  This is the STORE-CCP
                    // family from gcc.c-torture/execute/20041019-1.c and
                    // 20070212-2.c (`p = k ? &i1 : &j1; i1 = 0; return *p`).
                    let mut value_incomings = 0usize;
                    let roots: Vec<u32> = incoming
                        .iter()
                        .filter_map(|(op, _)| match op {
                            Operand::Value(v) => {
                                value_incomings += 1;
                                root_suffix.get(&v.0).map(|(root, _)| *root)
                            }
                            _ => None,
                        })
                        .collect();
                    if !roots.is_empty()
                        && (roots.len() != value_incomings || roots.iter().any(|r| *r != roots[0]))
                    {
                        escaping.extend(roots);
                    }
                }
                _ => {
                    // Escape every tracked root referenced by any operand,
                    // value use (incl. Intrinsic dest_ptr/args, Memcpy
                    // endpoints, call args), fail-closed.
                    crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                        if let Operand::Value(v) = op {
                            if let Some((root, _)) = root_suffix.get(&v.0) {
                                escaping.insert(*root);
                            }
                        }
                    });
                    crate::backend::liveness::for_each_value_use_in_instruction(inst, |v| {
                        if let Some((root, _)) = root_suffix.get(&v.0) {
                            escaping.insert(*root);
                        }
                    });
                }
            }
        }
        crate::backend::liveness::for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                if let Some((root, _)) = root_suffix.get(&v.0) {
                    escaping.insert(*root);
                }
            }
        });
    }
    let mut changes = 0;
    for block in &mut func.blocks {
        let old = std::mem::take(&mut block.instructions);
        let old_spans = std::mem::take(&mut block.source_spans);
        // Spans are only trustworthy when they parallel the instruction list
        // 1:1 (upstream convention, see dce.rs). Other passes may insert
        // instructions without maintaining spans; indexing old_spans[ii]
        // with a mismatched length was an out-of-bounds ICE.
        let has_spans = old_spans.len() == old.len();
        let mut kept = Vec::with_capacity(old.len());
        let mut spans = Vec::with_capacity(old_spans.len());
        for (ii, inst) in old.into_iter().enumerate() {
            let dead = if let Instruction::Store { ptr, ty, .. } = &inst {
                if let Some((root, Some(off))) = root_suffix.get(&ptr.0) {
                    if !volatile_roots.contains(root) && !escaping.contains(root) {
                        let size = type_size(*ty);
                        !loaded.get(root).is_some_and(|ranges| {
                            ranges
                                .iter()
                                .any(|(lo, ls)| *off < lo + ls && *lo < *off + size)
                        })
                    } else {
                        false
                    }
                } else {
                    false
                } // unknown store offset: never dead
            } else {
                false
            };
            if dead {
                changes += 1;
                continue;
            }
            kept.push(inst);
            if has_spans {
                spans.push(old_spans[ii]);
            }
        }
        block.instructions = kept;
        if has_spans {
            block.source_spans = spans;
        }
    }
    changes
}

#[derive(Clone)]
struct CopyCandidate {
    block: usize,
    inst: usize,
    source: Value,
    source_root: u32,
}

/// Instructions that can write memory the pass cannot attribute to a tracked
/// pointer path: calls (including libc `memset`/`memcpy` lowered as plain
/// `Call`s), inline asm, atomics, va-machinery, and impure intrinsics. A
/// snapshot window that spans one of these may only survive when the source
/// is a provably unclobberable local (non-escaping alloca).
fn is_opaque_memory_write(inst: &Instruction) -> bool {
    match inst {
        Instruction::Call { .. }
        | Instruction::CallIndirect { .. }
        | Instruction::InlineAsm { .. }
        | Instruction::AtomicRmw { .. }
        | Instruction::AtomicInc { .. }
        | Instruction::AtomicCmpxchg { .. }
        | Instruction::AtomicStore { .. }
        | Instruction::VaStart { .. }
        | Instruction::VaEnd { .. }
        | Instruction::VaCopy { .. }
        | Instruction::VaArg { .. }
        | Instruction::VaArgStruct { .. }
        | Instruction::StackRestore { .. } => true,
        Instruction::Intrinsic { op, .. } => !op.is_pure(),
        _ => false,
    }
}

/// Return the alloca root and GEP path for pointer values derived from allocas.
fn pointer_paths(
    func: &IrFunction,
) -> FxHashMap<u32, (u32, Vec<(Operand, crate::common::types::IrType)>)> {
    let mut paths = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca { dest, .. } = inst {
                paths.insert(dest.0, (dest.0, Vec::new()));
            }
        }
    }
    let mut changed = true;
    while changed {
        changed = false;
        for block in &func.blocks {
            for inst in &block.instructions {
                match inst {
                    Instruction::GetElementPtr {
                        dest,
                        base,
                        offset,
                        ty,
                    } => {
                        if paths.contains_key(&dest.0) {
                            continue;
                        }
                        if let Some((root, parent)) = paths.get(&base.0).cloned() {
                            let mut path = parent;
                            path.push((offset.clone(), *ty));
                            paths.insert(dest.0, (root, path));
                            changed = true;
                        }
                    }
                    Instruction::Copy {
                        dest,
                        src: Operand::Value(src),
                    } => {
                        if !paths.contains_key(&dest.0) {
                            if let Some(path) = paths.get(&src.0).cloned() {
                                paths.insert(dest.0, path);
                                changed = true;
                            }
                        }
                    }
                    Instruction::Phi { dest, incoming, .. } => {
                        if paths.contains_key(&dest.0) {
                            continue;
                        }
                        let mut common = None;
                        let mut compatible = true;
                        for (op, _) in incoming {
                            match op {
                                Operand::Const(c) if c.to_i64() == Some(0) => {}
                                Operand::Value(v) => {
                                    if let Some(path) = paths.get(&v.0) {
                                        if common.as_ref().is_some_and(
                                            |p: &(
                                                u32,
                                                Vec<(Operand, crate::common::types::IrType)>,
                                            )| {
                                                p.0 != path.0 || p.1.len() != path.1.len()
                                            },
                                        ) {
                                            compatible = false;
                                            break;
                                        }
                                        common = Some(path.clone());
                                    } else {
                                        compatible = false;
                                        break;
                                    }
                                }
                                _ => {
                                    compatible = false;
                                    break;
                                }
                            }
                        }
                        if compatible {
                            if let Some(path) = common {
                                paths.insert(dest.0, path);
                                changed = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    paths
}

/// Redirect construction of a store-only temporary aggregate into the final
/// memcpy destination.  This turns `build(tmp); memcpy(dst, tmp)` into
/// `build(dst)` when every use of `tmp` is a same-block GEP/store or that copy.
fn forward_store_only_temporaries(func: &mut IrFunction) -> usize {
    let paths = pointer_paths(func);
    let roots: FxHashSet<u32> = paths
        .iter()
        .filter_map(|(&v, (root, path))| (v == *root && path.is_empty()).then_some(v))
        .collect();
    // Entry-block allocas homing register parameters are written by the
    // PROLOGUE, not by IR-visible stores (find_param_alloca maps param i to
    // the i-th entry-block alloca; the x86/arm prologues spill incoming
    // registers there). Forwarding such a root redirects only the IR stores
    // and deletes the copy — the implicit prologue write is lost and the
    // copy destination stays uninitialized (dump128(__m128 v): the
    // `memcpy tmp, v_home` snapshot was deleted outright because v_home has
    // ZERO IR stores; tmp then fed printf with stack garbage).
    //
    // The exclusion set is `func.param_alloca_values` — the authoritative
    // list recorded at lowering (also consumed by stack_layout/analysis).
    // The old positional rule ("first params.len() entry allocas") went
    // stale the moment mem2reg promoted scalar param allocas away: in
    // `fill(ParticleGroup*, int)` both scalar param homes are deleted, so
    // the position count excluded the first two makeparticle TEMPORARIES
    // and half the struct-return copies stayed (struct_copy 3.1×). When
    // the recorded list is empty (tail_call_elim clears it; inliner-built
    // functions never had one) fall back to the positional rule, fail
    // closed.
    let param_slot_roots: FxHashSet<u32> = if !func.param_alloca_values.is_empty() {
        func.param_alloca_values.iter().map(|v| v.0).collect()
    } else {
        func.blocks
            .first()
            .map(|b| {
                b.instructions
                    .iter()
                    .filter_map(|i| match i {
                        Instruction::Alloca { dest, .. } => Some(dest.0),
                        _ => None,
                    })
                    .take(func.params.len())
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut copies: FxHashMap<u32, (usize, usize, Value)> = FxHashMap::default();
    let mut copy_sizes: FxHashMap<u32, i64> = FxHashMap::default();
    let mut duplicate = FxHashSet::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            if let Instruction::Memcpy { dest, src, size } = inst {
                let Some((root, path)) = paths.get(&src.0) else {
                    continue;
                };
                if !path.is_empty() || !roots.contains(root) {
                    continue;
                }
                if param_slot_roots.contains(root) {
                    continue;
                }
                if paths.get(&dest.0).is_some_and(|p| p.0 == *root) {
                    continue;
                }
                if copies.insert(*root, (bi, ii, *dest)).is_some() {
                    duplicate.insert(*root);
                }
                copy_sizes.insert(*root, *size as i64);
            }
        }
    }
    for root in duplicate {
        copies.remove(&root);
    }

    // A candidate must have at least one write that this analysis can
    // attribute to its root. Loop pointer promotion can replace a precise GEP
    // with a marching phi that `pointer_paths` intentionally cannot prove;
    // treating the then apparently write-free temporary as dead deletes the
    // memcpy while leaving the real stores in the old object (GCC torture
    // 20001026-1). Unknown writes therefore fail closed instead of proving an
    // empty construction.
    copies.retain(|root, _| {
        func.blocks.iter().any(|block| {
            block
                .instructions
                .iter()
                .any(|instruction| match instruction {
                    Instruction::Store { ptr, .. } => {
                        paths.get(&ptr.0).is_some_and(|path| path.0 == *root)
                    }
                    Instruction::Memcpy { dest, .. } => {
                        paths.get(&dest.0).is_some_and(|path| path.0 == *root)
                    }
                    _ => false,
                })
        })
    });

    // Byte offset of a tracked pointer when its whole GEP path is constant.
    let const_offset = |value: u32| -> Option<i64> {
        let (_, path) = paths.get(&value)?;
        let mut total = 0i64;
        for (op, _) in path {
            match op {
                Operand::Const(c) => total += c.to_i64()?,
                _ => return None,
            }
        }
        Some(total)
    };

    // The rewrite below redirects uses of `root` (at indices BEFORE the
    // memcpy) onto `dest`. That is only sound if `dest` is already DEFINED
    // at every redirected point. `dest` is frequently the sret pointer
    // loaded immediately before the memcpy — rewriting an earlier Store to
    // use it produced a use-before-def (uninitialized %r11 store, SIGSEGV
    // in struct_by_value). Fail closed: `dest` must be defined in the same
    // block strictly before the FIRST use of `root`, or be an entry-block
    // definition when the copy lives in a later block.
    let mut def_site: FxHashMap<u32, (usize, usize)> = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            if let Some(d) = inst.dest() {
                def_site.entry(d.0).or_insert((bi, ii));
            }
        }
    }
    let mut first_root_use: FxHashMap<u32, usize> = FxHashMap::default();
    for (root, (copy_b, _, _)) in &copies {
        for (ii, inst) in func.blocks[*copy_b].instructions.iter().enumerate() {
            let mut hit = false;
            inst.for_each_used_value(|v| {
                if v == *root {
                    hit = true;
                }
            });
            if hit {
                first_root_use.insert(*root, ii);
                break;
            }
        }
    }
    // HOIST: the copy destination is very often a pure-address GEP emitted
    // immediately before the memcpy (`g->particles[i]` in
    // `build(tmp); memcpy(&g->particles[i], tmp)`), i.e. AFTER the first
    // store into the temporary. That alone made the ordering check below
    // reject 3 of 4 struct-return copies per make_group (struct_copy 3.1×).
    // A GetElementPtr with a constant offset has no side effects and reads
    // only its base, so moving it earlier in the same block is sound
    // whenever the base is already defined there. Hoist it to just before
    // the first root use and let the ordering check pass. One hoist per
    // run() call keeps every recorded index valid (the driver runs this
    // pass to a fixed point, so all candidates get their turn).
    {
        let mut hoist: Option<(usize, usize, usize)> = None; // (block, gep_idx, insert_at)
        for (root, (copy_b, _copy_i, dest)) in &copies {
            let Some(&(db, di)) = def_site.get(&dest.0) else {
                continue;
            };
            if db != *copy_b {
                continue;
            }
            let first_use = first_root_use.get(root).copied().unwrap_or(usize::MAX);
            if di < first_use {
                continue;
            } // already ordered
            let inst = &func.blocks[db].instructions[di];
            let Instruction::GetElementPtr {
                base,
                offset: Operand::Const(_),
                ..
            } = inst
            else {
                continue;
            };
            // Base must be defined before the insertion point: an earlier
            // instruction of this block, an entry-block value when this is
            // a later block, or a function parameter (no def site).
            let base_ok = match def_site.get(&base.0) {
                Some(&(bb, bi)) => (bb == db && bi < first_use) || (bb == 0 && db != 0),
                None => true,
            };
            if !base_ok {
                continue;
            }
            hoist = Some((db, di, first_use));
            break;
        }
        if let Some((bi, gep_idx, insert_at)) = hoist {
            if std::env::var("CCC_DEBUG_AGGFWD").is_ok() {
                eprintln!(
                    "[STOREFWD-HOIST] {} b{} gep@{} -> {}",
                    func.name, bi, gep_idx, insert_at
                );
            }
            let block = &mut func.blocks[bi];
            let inst = block.instructions.remove(gep_idx);
            block.instructions.insert(insert_at, inst);
            if !block.source_spans.is_empty() {
                let span = block.source_spans.remove(gep_idx);
                block.source_spans.insert(insert_at, span);
            }
            // Indices recorded in `copies`/`def_site` for this block are
            // now stale; count the hoist as a change and let the caller's
            // fixed-point loop re-run the analysis on fresh indices.
            return 1;
        }
    }

    copies.retain(|root, (copy_b, _, dest)| {
        match def_site.get(&dest.0) {
            Some((db, di)) => {
                if db == copy_b {
                    // Defined in the copy's block: must precede every
                    // redirected use, i.e. the first use of root.
                    *di < first_root_use.get(root).copied().unwrap_or(usize::MAX)
                } else {
                    // Only the entry block is guaranteed to dominate.
                    *db == 0 && *copy_b != 0
                }
            }
            // No def site: function parameter — defined on entry.
            None => true,
        }
    });

    let mut invalid = FxHashSet::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            let mut used = Vec::with_capacity(16);
            inst.for_each_used_value(|v| used.push(v));
            for value in used {
                let Some((root, _)) = paths.get(&value) else {
                    continue;
                };
                let Some((copy_b, copy_i, _)) = copies.get(root) else {
                    continue;
                };
                let copy_size = copy_sizes.get(root).copied().unwrap_or(0);
                // Every redirected memory access must land STRICTLY inside the
                // copied byte range [0, copy_size). A bitfield read-modify-write
                // can legally overhang the source aggregate into its own
                // padding — but after forwarding, the same overhang lands in
                // the DESTINATION object and corrupts whatever neighbors it
                // there (structs_bitfields: 12-byte memcpy, 8..16 store).
                let within_copy = |v: u32, ty: crate::common::types::IrType| -> bool {
                    const_offset(v).is_some_and(|off| off >= 0 && off + type_size(ty) <= copy_size)
                };
                let allowed = match inst {
                    Instruction::GetElementPtr { base, .. } => base.0 == value,
                    Instruction::Store { ptr, ty, .. } => ptr.0 == value && within_copy(value, *ty),
                    Instruction::Copy {
                        src: Operand::Value(v),
                        ..
                    } => v.0 == value,
                    Instruction::Phi { incoming, .. } => incoming
                        .iter()
                        .any(|(op, _)| matches!(op, Operand::Value(v) if v.0 == value)),
                    Instruction::Memcpy { dest, src, .. } => {
                        (src.0 == value && bi == *copy_b && ii == *copy_i)
                            || (dest.0 == value && bi == *copy_b && ii < *copy_i)
                    }
                    _ => false,
                };
                let path_merge = matches!(inst, Instruction::Copy { .. } | Instruction::Phi { .. });
                let ordered = path_merge || ii <= *copy_i;
                let located = path_merge || bi == *copy_b;
                if !allowed || !located || !ordered {
                    invalid.insert(*root);
                }
            }
        }
    }
    if std::env::var("CCC_DEBUG_AGGFWD").is_ok() {
        for r in &invalid {
            eprintln!("[STOREFWD-REJ] {} root=v{} invalid-use", func.name, r);
        }
    }
    for root in invalid {
        copies.remove(&root);
    }
    if copies.is_empty() {
        return 0;
    }

    // DEST-LIVENESS WINDOW: forwarding moves every write of `root` up to the
    // memcpy so that it lands in `dest` EARLY. That is only sound if dest's
    // OLD contents are provably dead throughout [first redirected use, copy):
    // any read of dest's memory in that window would now observe the new
    // value; any write would be clobbered by the redirected stores. The
    // previous validation only inspected uses of ROOT — never of DEST.
    // zlib-ng fold_1: `x_tmp3=*c3; *c3=*c0; *c1=*c2; ...; *c2=x_tmp3` — the
    // snapshot write was redirected into c2 while `*c1=*c2` still read c2's
    // old value (CRC mismatch on every odd-length fold_copy+fold sequence).
    {
        let mut reject = FxHashSet::default();
        for (&root, &(copy_b, copy_i, dest)) in &copies {
            // dest's aliasing root (dest may be a GEP into a larger object;
            // any access to that object is conservatively a conflict).
            let dest_root = paths.get(&dest.0).map(|p| p.0).unwrap_or(dest.0);
            // First instruction that references `root` before the copy.
            let mut first_use = copy_i;
            for (ii, inst) in func.blocks[copy_b].instructions.iter().enumerate() {
                if ii >= copy_i {
                    break;
                }
                let mut hit = false;
                inst.for_each_used_value(|v| {
                    if paths.get(&v).is_some_and(|p| p.0 == root) {
                        hit = true;
                    }
                });
                if let Some(d) = inst.dest() {
                    if paths.get(&d.0).is_some_and(|p| p.0 == root) {
                        hit = true;
                    }
                }
                if hit {
                    first_use = ii;
                    break;
                }
            }
            // dest's MEMORY must be untouched inside the window (the memcpy
            // itself at copy_i is the legitimate final write). Pure address
            // computations — GetElementPtr / Copy / Phi over pointers rooted
            // at dest — do not read or write that memory and must not
            // reject (the hoisted destination GEP itself sits inside the
            // window by construction). Everything else that references a
            // dest-rooted value is a read, a write, or an escape
            // (Load/Store/Memcpy/Call/Intrinsic/InlineAsm/Cast/...) and
            // fails closed.
            for ii in first_use..copy_i {
                let inst = &func.blocks[copy_b].instructions[ii];
                if matches!(
                    inst,
                    Instruction::GetElementPtr { .. }
                        | Instruction::Copy { .. }
                        | Instruction::Phi { .. }
                ) {
                    continue;
                }
                let mut touches_dest = false;
                inst.for_each_used_value(|v| {
                    if v == dest.0 || paths.get(&v).is_some_and(|p| p.0 == dest_root) {
                        touches_dest = true;
                    }
                });
                if let Some(d) = inst.dest() {
                    if d.0 == dest.0 || paths.get(&d.0).is_some_and(|p| p.0 == dest_root) {
                        touches_dest = true;
                    }
                }
                if touches_dest {
                    reject.insert(root);
                    break;
                }
            }
        }
        if std::env::var("CCC_DEBUG_AGGFWD").is_ok() {
            for r in &reject {
                eprintln!("[STOREFWD-REJ] {} root=v{} dest-window", func.name, r);
            }
        }
        for root in reject {
            copies.remove(&root);
        }
    }
    if copies.is_empty() {
        return 0;
    }

    if std::env::var("CCC_DEBUG_AGGFWD").is_ok() {
        for (&root, &(bi, copy_i, dest)) in &copies {
            eprintln!(
                "[STOREFWD] {} root=v{} dest=v{} at b{}:{}",
                func.name, root, dest.0, bi, copy_i
            );
        }
    }
    let mut changes = 0;
    for (&root, &(bi, copy_i, dest)) in &copies {
        for (ii, inst) in func.blocks[bi].instructions.iter_mut().enumerate() {
            match inst {
                Instruction::GetElementPtr { base, .. } if ii < copy_i && base.0 == root => {
                    *base = dest;
                    changes += 1;
                }
                Instruction::Store { ptr, .. } if ii < copy_i && ptr.0 == root => {
                    *ptr = dest;
                    changes += 1;
                }
                Instruction::Copy {
                    src: Operand::Value(v),
                    ..
                } if v.0 == root => {
                    *v = dest;
                    changes += 1;
                }
                Instruction::Phi { incoming, .. } => {
                    for (op, _) in incoming {
                        if matches!(op, Operand::Value(v) if v.0 == root) {
                            *op = Operand::Value(dest);
                            changes += 1;
                        }
                    }
                }
                Instruction::Memcpy {
                    dest: copy_dest, ..
                } if ii < copy_i && copy_dest.0 == root => {
                    *copy_dest = dest;
                    changes += 1;
                }
                _ => {}
            }
        }
    }
    let mut removals: Vec<(usize, usize)> = copies.values().map(|(b, i, _)| (*b, *i)).collect();
    removals.sort_unstable_by(|a, b| b.cmp(a));
    for (bi, ii) in removals {
        func.blocks[bi].instructions.remove(ii);
        if !func.blocks[bi].source_spans.is_empty() {
            func.blocks[bi].source_spans.remove(ii);
        }
        changes += 1;
    }
    changes
}

pub(crate) fn run(func: &mut IrFunction) -> usize {
    // Sub-pass gates for miscompile bisection (this pass family has produced
    // eight distinct unsoundness bugs; being able to isolate the store-only
    // forwarding, the main forwarding, and the dead-store elimination
    // independently cuts a bisection from hours to minutes).
    let reverse_changes = if std::env::var("CCC_NO_AGG_STORE_FWD").is_ok() {
        0
    } else {
        forward_store_only_temporaries(func)
    };
    if std::env::var("CCC_NO_AGG_MAIN").is_ok() {
        return reverse_changes
            + if std::env::var("CCC_NO_AGG_DEAD_STORES").is_ok() {
                0
            } else {
                eliminate_dead_aggregate_field_stores(func)
            };
    }
    let cfg = crate::ir::analysis::CfgAnalysis::build(func);
    // `CfgAnalysis::idom` uses usize::MAX as the sentinel for blocks that are
    // unreachable from entry, so the walk must bounds-check before indexing.
    // Unreachable code does occur in real input (the kernel's BUG()/unreachable()
    // paths leave blocks with no predecessors), and indexing with the sentinel
    // panicked with "index out of bounds: the len is 154 but the index is
    // 18446744073709551615" while compiling kernel/signal.c.
    //
    // An unreachable block is dominated by nothing, so returning false is both
    // safe and correct: the caller then declines the transform.
    let dominates = |a: usize, mut b: usize| {
        if a == b {
            return true;
        }
        for _ in 0..cfg.idom.len() {
            if b >= cfg.idom.len() {
                return false;
            }
            let parent = cfg.idom[b];
            if parent == usize::MAX {
                return false;
            }
            if parent == b {
                break;
            }
            if parent == a {
                return true;
            }
            b = parent;
        }
        false
    };
    let paths = pointer_paths(func);
    let alloca_roots: FxHashSet<u32> = paths
        .iter()
        .filter_map(|(&v, (root, path))| (v == *root && path.is_empty()).then_some(v))
        .collect();

    let mut candidates: FxHashMap<u32, CopyCandidate> = FxHashMap::default();
    let mut duplicate = FxHashSet::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            if let Instruction::Memcpy { dest, src, .. } = inst {
                // An arbitrary pointer (global, parameter, or loaded pointer) may be
                // overwritten between this copy and a later read.  Restrict forwarding
                // to compiler-known stack objects so those writes can be checked below.
                if alloca_roots.contains(&dest.0) {
                    let Some((source_root, _)) = paths.get(&src.0) else {
                        continue;
                    };
                    if *source_root == dest.0 {
                        continue;
                    }
                    if candidates
                        .insert(
                            dest.0,
                            CopyCandidate {
                                block: bi,
                                inst: ii,
                                source: *src,
                                source_root: *source_root,
                            },
                        )
                        .is_some()
                    {
                        duplicate.insert(dest.0);
                    }
                }
            }
        }
    }
    for root in duplicate {
        candidates.remove(&root);
    }
    if candidates.is_empty() {
        return reverse_changes + eliminate_dead_aggregate_field_stores(func);
    }

    // Reject escaping, written, cross-block, or pre-copy uses of each temporary.
    let mut invalid = FxHashSet::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            let mut used = Vec::with_capacity(16);
            inst.for_each_used_value(|v| used.push(v));
            for value in used {
                let Some((root, _)) = paths.get(&value) else {
                    continue;
                };
                let Some(candidate) = candidates.get(root) else {
                    continue;
                };
                let allowed_shape = match inst {
                    Instruction::GetElementPtr { base, .. } => base.0 == value,
                    Instruction::Load { ptr, .. } => ptr.0 == value,
                    Instruction::Memcpy { dest, src, .. } => dest.0 == *root || src.0 == value,
                    Instruction::Copy {
                        src: Operand::Value(v),
                        ..
                    } => v.0 == value,
                    Instruction::Phi { incoming, .. } => incoming
                        .iter()
                        .any(|(op, _)| matches!(op, Operand::Value(v) if v.0 == value)),
                    _ => false,
                };
                let is_defining_copy = matches!(inst, Instruction::Memcpy { dest, .. } if dest.0 == *root)
                    && bi == candidate.block
                    && ii == candidate.inst;
                let is_path_definition = matches!(inst, Instruction::GetElementPtr { base, .. } if base.0 == value)
                    || matches!(inst, Instruction::Copy { src: Operand::Value(v), .. } if v.0 == value)
                    || matches!(inst, Instruction::Phi { incoming, .. } if incoming.iter().any(|(op, _)| matches!(op, Operand::Value(v) if v.0 == value)));
                let ordered_read = if bi == candidate.block {
                    ii > candidate.inst
                } else {
                    dominates(candidate.block, bi)
                };
                // Keep the snapshot lifetime local to one block.  This makes the
                // source-mutation proof below exact even when the block is in a loop:
                // a write in the next iteration cannot precede a read in this one.
                let cross_block_read =
                    !is_defining_copy && !is_path_definition && bi != candidate.block;
                if !allowed_shape
                    || cross_block_read
                    || (!is_defining_copy && !is_path_definition && !ordered_read)
                {
                    invalid.insert(*root);
                }
            }
        }
    }
    for root in invalid {
        candidates.remove(&root);
    }
    if candidates.is_empty() {
        return reverse_changes + eliminate_dead_aggregate_field_stores(func);
    }

    // The source must also remain unchanged after the copy.  Otherwise replacing
    // a temporary read with a source read changes snapshot semantics (TinyCC's
    // `tmp = *vtop; *vtop = ...; use(tmp)` exposed this).
    //
    // Writes come in two shapes:
    //  * attributable — Store/Memcpy through a tracked pointer path; and
    //  * opaque — calls, inline asm, atomics, impure intrinsics.  libc
    //    `memset`/`memcpy` lower to plain `Call`s, so they land here.
    //
    // An opaque write can touch the source whenever the source's address has
    // escaped (passed to a call, stored as data, or used outside the tracked
    // load/store/GEP shapes).  SQLite memjrnlCreateFile miscompiled exactly
    // so: `copy = *p; memset(p, 0, sizeof *p); use copy.pVfs` — the memset
    // Call was invisible to the old Store/Memcpy-only scan, the copy was
    // elided, and the forwarded reads observed the zeroed struct (speedtest1
    // --testset json SIGSEGV via a NULL xOpen function pointer).
    let mut escaped_roots: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            let mut used = Vec::with_capacity(16);
            inst.for_each_used_value(|v| used.push(v));
            for value in used {
                let Some((root, _)) = paths.get(&value) else {
                    continue;
                };
                let non_escaping_shape = match inst {
                    Instruction::GetElementPtr { base, .. } => base.0 == value,
                    Instruction::Load { ptr, .. } => ptr.0 == value,
                    Instruction::Store { ptr, val, .. } => {
                        // Address position is fine; storing the POINTER as a
                        // value escapes it.
                        ptr.0 == value && !matches!(val, Operand::Value(v) if v.0 == value)
                    }
                    Instruction::Memcpy { dest, src, .. } => dest.0 == value || src.0 == value,
                    Instruction::Copy {
                        src: Operand::Value(v),
                        ..
                    } => v.0 == value,
                    Instruction::Phi { incoming, .. } => incoming
                        .iter()
                        .any(|(op, _)| matches!(op, Operand::Value(v) if v.0 == value)),
                    _ => false,
                };
                if !non_escaping_shape {
                    escaped_roots.insert(*root);
                }
            }
        }
    }
    let mut invalid = FxHashSet::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            if is_opaque_memory_write(inst) {
                for (&dest_root, candidate) in &candidates {
                    let after_copy = bi == candidate.block && ii > candidate.inst;
                    // A non-escaping alloca cannot be written by an opaque
                    // instruction; anything else must be assumed clobbered.
                    let source_provably_private = alloca_roots.contains(&candidate.source_root)
                        && !escaped_roots.contains(&candidate.source_root);
                    if after_copy && !source_provably_private {
                        invalid.insert(dest_root);
                    }
                }
                continue;
            }
            let written_root = match inst {
                Instruction::Store { ptr, .. } => paths.get(&ptr.0).map(|p| p.0),
                Instruction::Memcpy { dest, .. } => paths.get(&dest.0).map(|p| p.0),
                _ => None,
            };
            let Some(written_root) = written_root else {
                continue;
            };
            for (&dest_root, candidate) in &candidates {
                let after_copy = bi == candidate.block && ii > candidate.inst;
                if after_copy && written_root == candidate.source_root {
                    invalid.insert(dest_root);
                }
            }
        }
    }
    for root in invalid {
        candidates.remove(&root);
    }
    if candidates.is_empty() {
        return reverse_changes + eliminate_dead_aggregate_field_stores(func);
    }

    fn resolve_source(mut source: Value, candidates: &FxHashMap<u32, CopyCandidate>) -> Value {
        let mut seen = FxHashSet::default();
        while seen.insert(source.0) {
            if let Some(next) = candidates.get(&source.0) {
                source = next.source;
            } else {
                break;
            }
        }
        source
    }

    if std::env::var("CCC_DEBUG_AGGFWD").is_ok() {
        for (root, c) in &candidates {
            eprintln!(
                "[AGGFWD] {} root=v{} src=v{} src_root=v{} at b{}:{}",
                func.name, root, c.source.0, c.source_root, c.block, c.inst
            );
        }
    }
    let mut changes = 0;
    for bi in 0..func.blocks.len() {
        let old = std::mem::take(&mut func.blocks[bi].instructions);
        let old_spans = std::mem::take(&mut func.blocks[bi].source_spans);
        // Same 1:1 length requirement as above (mismatch => drop spans, never index OOB).
        let has_spans = old_spans.len() == old.len();
        let mut out = Vec::with_capacity(old.len());
        let mut spans = Vec::with_capacity(16);
        for (ii, mut inst) in old.into_iter().enumerate() {
            if let Instruction::Memcpy { dest, .. } = &inst {
                if candidates
                    .get(&dest.0)
                    .is_some_and(|c| c.block == bi && c.inst == ii)
                {
                    changes += 1;
                    continue;
                }
            }
            if let Instruction::Memcpy { src, .. } = &mut inst {
                if let Some(candidate) = candidates.get(&src.0) {
                    *src = resolve_source(candidate.source, &candidates);
                    changes += 1;
                }
            }
            if let Instruction::Load { ptr, .. } = &mut inst {
                if let Some((root, path)) = paths.get(&ptr.0) {
                    if let Some(candidate) = candidates.get(root) {
                        let mut base = resolve_source(candidate.source, &candidates);
                        for (offset, ty) in path {
                            let dest = Value(func.next_value_id);
                            func.next_value_id += 1;
                            out.push(Instruction::GetElementPtr {
                                dest,
                                base,
                                offset: offset.clone(),
                                ty: *ty,
                            });
                            if has_spans {
                                spans.push(old_spans[ii]);
                            }
                            base = dest;
                        }
                        *ptr = base;
                        changes += 1;
                    }
                }
            }
            out.push(inst);
            if has_spans {
                spans.push(old_spans[ii]);
            }
        }
        func.blocks[bi].instructions = out;
        if has_spans {
            func.blocks[bi].source_spans = spans;
        }
    }
    changes + reverse_changes + eliminate_dead_aggregate_field_stores(func)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::{AddressSpace, IrType};
    use crate::ir::reexports::{BasicBlock, BlockId, IrConst, Terminator};

    #[test]
    fn preserves_copy_when_source_write_uses_untracked_pointer_arithmetic() {
        let mut func = IrFunction::new("untracked_store".into(), IrType::I32, vec![], false);
        func.next_value_id = 4;
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Alloca {
                    dest: Value(0),
                    ty: IrType::I32,
                    size: 4,
                    align: 4,
                    volatile: false,
                    semantic_volatile: false,
                },
                Instruction::Alloca {
                    dest: Value(1),
                    ty: IrType::I32,
                    size: 4,
                    align: 4,
                    volatile: false,
                    semantic_volatile: false,
                },
                Instruction::BinOp {
                    dest: Value(2),
                    op: crate::ir::reexports::IrBinOp::Add,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I64(0)),
                    ty: IrType::Ptr,
                },
                Instruction::Store {
                    volatile: false,
                    val: Operand::Const(IrConst::I32(7)),
                    ptr: Value(2),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                },
                Instruction::Memcpy {
                    dest: Value(1),
                    src: Value(0),
                    size: 4,
                },
                Instruction::Load {
                    volatile: false,
                    dest: Value(3),
                    ptr: Value(1),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(3)))),
            source_spans: vec![],
        });

        assert_eq!(forward_store_only_temporaries(&mut func), 0);
        assert!(
            func.blocks[0]
                .instructions
                .iter()
                .any(|instruction| matches!(instruction, Instruction::Memcpy { .. }))
        );
    }

    #[test]
    fn preserves_snapshot_when_copy_source_is_overwritten() {
        let mut func = IrFunction::new("snapshot".into(), IrType::I32, vec![], false);
        func.next_value_id = 3;
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Alloca {
                    dest: Value(0),
                    ty: IrType::I32,
                    size: 4,
                    align: 4,
                    volatile: false,
                    semantic_volatile: false,
                },
                Instruction::Alloca {
                    dest: Value(1),
                    ty: IrType::I32,
                    size: 4,
                    align: 4,
                    volatile: false,
                    semantic_volatile: false,
                },
                Instruction::Store {
                    volatile: false,
                    val: Operand::Const(IrConst::I32(1)),
                    ptr: Value(0),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                },
                Instruction::Memcpy {
                    dest: Value(1),
                    src: Value(0),
                    size: 4,
                },
                Instruction::Store {
                    volatile: false,
                    val: Operand::Const(IrConst::I32(2)),
                    ptr: Value(0),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                },
                Instruction::Load {
                    volatile: false,
                    dest: Value(2),
                    ptr: Value(1),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(2)))),
            source_spans: vec![],
        });

        assert_eq!(run(&mut func), 0);
        assert!(func.blocks[0].instructions.iter().any(|inst| matches!(
            inst,
            Instruction::Memcpy {
                dest: Value(1),
                src: Value(0),
                ..
            }
        )));
    }
}
