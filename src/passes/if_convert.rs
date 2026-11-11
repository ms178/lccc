//! If-conversion pass: converts simple branch+phi diamonds to Select instructions.
//!
//! This pass identifies diamond-shaped CFG patterns:
//!
//! ```text
//!     pred_block:
//!         ...
//!         condbranch %cond, true_block, false_block
//!
//!     true_block:
//!         (0-1 simple instructions)
//!         branch merge_block
//!
//!     false_block:
//!         (0-1 simple instructions)
//!         branch merge_block
//!
//!     merge_block:
//!         %result = phi [true_val, true_block], [false_val, false_block]
//!         ...
//! ```
//!
//! And converts them to:
//!
//! ```text
//!     pred_block:
//!         ...
//!         %result = select %cond, true_val, false_val
//!         branch merge_block
//! ```
//!
//! This eliminates branches in favor of conditional moves (cmov/csel),
//! which is critical for performance in tight loops with simple conditionals
//! (e.g., `x >= wsize ? x - wsize : 0` in zlib's slide_hash).
//!
//! Safety: Only converts when both arms are side-effect-free (no stores, calls,
//! or memory operations). This ensures the Select semantics (evaluate both
//! operands) match the original branch semantics.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::{AddressSpace, IrType};
use crate::ir::analysis;
use crate::ir::reexports::{
    BasicBlock, BlockId, Instruction, IrBinOp, IrCmpOp, IrConst, IrFunction, Operand, Terminator,
    Value,
};

/// Run if-conversion on a single function.
pub(crate) fn if_convert_function(func: &mut IrFunction) -> usize {
    if super::fp_environment_blocks_reordering(func) {
        return 0;
    }
    let num_blocks = func.blocks.len();
    if num_blocks < 3 {
        return 0; // Need at least 3 blocks (pred + one arm + merge)
    }

    let mut total = 0;

    // Iterate to a fixpoint since converting one diamond may expose another.
    // Store sinking runs in the same fixpoint: each round it turns
    // store-in-arm decision trees into phi-driven single stores, which the
    // diamond/triangle converters then fold into Selects.
    //
    // Conditional-load sinking runs BETWEEN those two: the store sink exposes
    // the `Phi [load_t, T], [load_f, F]` that this transform keys on, and the
    // resulting pointer Select is then a legal input to the diamond converter.
    // It turns `cond ? *p : *q` diamonds (which the ordinary converter must
    // refuse, because speculating a LOAD can trap) into a pointer Select plus
    // ONE load, without adding any memory access.  The pred's CondBranch is
    // gone afterwards, so this pass cannot re-fire on its own output.
    loop {
        let sunk = sink_conditional_stores(func);
        let loads = sink_conditional_loads(func);
        let converted = if_convert_once(func);
        if sunk == 0 && loads == 0 && converted == 0 {
            break;
        }
        total += converted + loads;
    }

    total
}

/// Canonical address key of a pointer VALUE: `GEP(base, offset)` reduces
/// to (resolved-base, canonical-offset) — the offset chain (Cast(IV),
/// Shl(IV, k), Copy) is traced to its underlying index so two GEPs built
/// from the same IV and base (but with distinct SSA ids — exactly what the
/// conditional-reduction diamond produces per arm) compare equal.
fn canonical_addr_key_impl(
    defs: &FxHashMap<u32, Instruction>,
    copy_of: &FxHashMap<Value, Value>,
    ptr: &Value,
) -> String {
    let resolve = |v: &Value| -> Value {
        let mut cur = *v;
        for _ in 0..64 {
            match copy_of.get(&cur) {
                Some(&next) if next != cur => cur = next,
                _ => break,
            }
        }
        cur
    };
    let root = resolve(ptr);
    match defs.get(&root.0) {
        Some(Instruction::GetElementPtr { base, offset, .. }) => {
            // Base identity: GlobalAddr SSA ids are per-use-site (the
            // frontend emits a fresh one per source reference), so two GEPs
            // over the same global have DIFFERENT base value ids.
            // Canonicalize the base to its symbol when it resolves to a
            // GlobalAddr; everything else keeps the SSA id.
            let base_root = resolve(base);
            let base_key: String = match defs.get(&base_root.0) {
                Some(Instruction::GlobalAddr { name, .. }) => format!("sym({})", name),
                _ => format!("val({})", base_root.0),
            };
            // Offset canonicalization must be INJECTIVE over addresses: two
            // GEPs get the same key iff they denote the same address. The
            // walk descends through value-preserving links (Copy, and Cast —
            // the frontend materializes a fresh cast per source mention, so
            // the trace is what lets two arms' `d[i]` GEPs match at all).
            // A `Shl(v, k)` scales the offset by 2^k: the shift constant is
            // part of the ADDRESS, so it is recorded in the key. Collapsing
            // through Shl without recording k (the original code) mapped
            // `d[i]` (off = i<<2) and `d[2*i]` (off = i<<3) to the SAME key —
            // rewrite_covered_arm_loads then replaced the arm's `d[2*i]`
            // load with the pred's `d[i]` value, and sink_conditional_stores
            // merged stores to the two DIFFERENT addresses into one store.
            let mut off_root = offset.clone();
            let mut scale_suffix = String::new();
            for _ in 0..16 {
                let vid = match &off_root {
                    Operand::Value(v) => v.0,
                    Operand::Const(c) => {
                        return format!("gep({}@{:?}{})", base_key, c, scale_suffix);
                    }
                };
                match defs.get(&vid) {
                    // A cast may be descended through only when the composite
                    // root->offset function it contributes is RECORDED in the
                    // key, so that equal keys still mean equal ADDRESSES.
                    //
                    // * int->int, same size: a bit-identical reinterpretation
                    //   — transparent. The signedness change matters only for
                    //   a LATER widening, and that widening records its own
                    //   `from_ty` below.
                    // * int->int, widening: value-preserving, but `sext` and
                    //   `zext` of the same source DIFFER for negative inputs.
                    //   `d[i]` and `d[(unsigned char) i]` with `signed char
                    //   i = -1` are `d - 1` and `d + 255`; collapsing them
                    //   into one key made the arm rewrite forward a load
                    //   across 256 bytes and made `sink_conditional_stores`
                    //   merge two stores to different addresses (a
                    //   wrong-address WRITE). The extension kind and both
                    //   widths therefore go into the key. The walk must still
                    //   descend, because the frontend materializes a fresh
                    //   cast per source mention and that is what lets two
                    //   arms' `d[i]` GEPs match at all.
                    // * int->int, truncating: `d[(int) big]` and `d[big]` are
                    //   different addresses. Stop and key on the cast itself.
                    // * anything touching a floating-point type: not
                    //   injective over the integer offset — `(long)(float) i`
                    //   rounds for |i| > 2^24 with no UB. Stop.
                    Some(Instruction::Cast {
                        src: Operand::Value(v),
                        from_ty,
                        to_ty,
                        ..
                    }) => {
                        if from_ty.is_float() || to_ty.is_float() {
                            break;
                        }
                        let (fs, ts) = (from_ty.size(), to_ty.size());
                        if ts < fs {
                            break;
                        }
                        if ts > fs {
                            let kind = if from_ty.is_unsigned() { "zx" } else { "sx" };
                            scale_suffix = format!("{}{}>{}{}", kind, fs, ts, scale_suffix);
                        }
                        off_root = Operand::Value(*v);
                    }
                    Some(Instruction::Copy {
                        src: Operand::Value(v),
                        ..
                    }) => {
                        off_root = Operand::Value(*v);
                    }
                    Some(Instruction::BinOp {
                        op: IrBinOp::Shl,
                        lhs: Operand::Value(v),
                        rhs: rhs @ Operand::Const(_),
                        ty,
                        ..
                    }) => {
                        // Value-preserving only in the key sense if the
                        // constant is recorded; keep walking below it.
                        // The shift WIDTH is part of the function too: a shift
                        // wraps at its own type width, so `(long)(u << 1)`
                        // (32-bit shift, then widen) and `((long) u) << 1`
                        // (widen, then 64-bit shift) differ for `u >= 2^31`
                        // with no UB on the unsigned form.
                        scale_suffix = format!("<<{:?}:{}{}", rhs, ty.size(), scale_suffix);
                        off_root = Operand::Value(*v);
                    }
                    _ => break,
                }
            }
            format!("gep({}@{:?}{})", base_key, off_root, scale_suffix)
        }
        _ => format!("val({})", root.0),
    }
}

/// Rewrite loads in single-predecessor arm blocks that re-load an address the
/// branch predecessor already loads. The arm always executes immediately after
/// the pred on any path that reaches it, so when both blocks are free of
/// memory writes the pred's identical load produces exactly the value the arm
/// load would see; replacing it with a Copy eliminates a redundant memory
/// access and often makes the arm side-effect-free enough for if-conversion
/// (e.g. `if (a[i] > 0) s += a[i]` reloading `a[i]` in the then-arm).
fn rewrite_covered_arm_loads(
    func: &mut IrFunction,
    label_to_idx: &FxHashMap<BlockId, usize>,
    preds: &analysis::FlatAdj,
) -> usize {
    /// Free of memory writes and other externally visible effects.
    /// Loads are fine (they don't change memory). Conservative: intrinsics,
    /// calls, stores, atomics, allocas are all treated as barriers.
    fn is_pure(inst: &Instruction) -> bool {
        matches!(
            inst,
            Instruction::Load { .. }
                | Instruction::BinOp { .. }
                | Instruction::UnaryOp { .. }
                | Instruction::Cmp { .. }
                | Instruction::Cast { .. }
                | Instruction::Copy { .. }
                | Instruction::GetElementPtr { .. }
                | Instruction::GlobalAddr { .. }
                | Instruction::Select { .. }
                | Instruction::Phi { .. }
        )
    }

    let mut rewrites = 0;
    let debug = std::env::var("CCC_DEBUG_IFCONV").is_ok();

    // Resolve pointer copies to their root value: `p = q; load [p]` loads the
    // same address as `load [q]`. SSA guarantees the root dominates both uses.
    let mut copy_of: FxHashMap<Value, Value> = FxHashMap::default();
    for b in &func.blocks {
        for i in &b.instructions {
            if let Instruction::Copy {
                dest,
                src: Operand::Value(src),
            } = i
            {
                copy_of.insert(*dest, *src);
            }
        }
    }

    // Cloned defs: the rewrite loop below mutates func.blocks, so the map
    // must not hold borrows into it.
    let defs: FxHashMap<u32, Instruction> = func
        .blocks
        .iter()
        .flat_map(|b| b.instructions.iter())
        .filter_map(|i| i.dest().map(|d| (d.0, i.clone())))
        .collect();
    let canonical_addr_key =
        |ptr: &Value| -> String { canonical_addr_key_impl(&defs, &copy_of, ptr) };

    for pred_idx in 0..func.blocks.len() {
        let (true_label, false_label) = match &func.blocks[pred_idx].terminator {
            Terminator::CondBranch {
                true_label,
                false_label,
                ..
            } => (*true_label, *false_label),
            _ => continue,
        };
        // The pred must not write memory between its load and the arm.
        if !func.blocks[pred_idx].instructions.iter().all(is_pure) {
            continue;
        }
        // Collect the pred's loads: (ptr, ty, seg) -> dest. Linear-scan Vec;
        // preds hold only a handful of loads.
        let mut pred_loads: Vec<(String, IrType, AddressSpace, Value)> = Vec::new();
        for inst in &func.blocks[pred_idx].instructions {
            if let Instruction::Load {
                dest,
                ptr,
                ty,
                seg_override,
                volatile,
            } = inst
            {
                // A volatile access is an observable event in its own right
                // (C11 5.1.2.3): it neither substitutes for another access
                // nor may be substituted by one. Keep it out of the covering
                // set entirely.
                if *volatile {
                    continue;
                }
                let key = canonical_addr_key(ptr);
                // On a duplicate, keep the last load (same value either way).
                if let Some(entry) = pred_loads
                    .iter_mut()
                    .find(|(p, t, s, _)| *p == key && *t == *ty && *s == *seg_override)
                {
                    entry.3 = *dest;
                } else {
                    pred_loads.push((key, *ty, *seg_override, *dest));
                }
            }
        }
        if pred_loads.is_empty() {
            continue;
        }
        for label in [true_label, false_label] {
            let Some(&arm_idx) = label_to_idx.get(&label) else {
                continue;
            };
            if arm_idx == pred_idx {
                continue;
            }
            // The arm must be reached only from this pred, and must not write
            // memory before (or after) its loads.
            if preds.len(arm_idx) != 1 || preds.row(arm_idx)[0] as usize != pred_idx {
                continue;
            }
            if !func.blocks[arm_idx].instructions.iter().all(is_pure) {
                continue;
            }
            // Collect rewrites first (immutable), then apply.
            let mut pending: Vec<(usize, Value, Value)> = Vec::new();
            for (inst_pos, inst) in func.blocks[arm_idx].instructions.iter().enumerate() {
                if let Instruction::Load {
                    dest,
                    ptr,
                    ty,
                    seg_override,
                    volatile,
                } = inst
                {
                    // `volatile int g; if (g > 0) return g;` must read `g`
                    // TWICE. Replacing the arm's volatile load with the
                    // pred's value deletes an observable access. Measured
                    // before this guard: lccc emitted ONE `g(%rip)` load at
                    // -O2 where GCC and Clang emit two. The speculation and
                    // store-sink paths were already volatile-aware; this
                    // pattern matched `Load { .. }` and was blind to the flag.
                    if *volatile {
                        continue;
                    }
                    let key = canonical_addr_key(ptr);
                    if let Some((_, _, _, covering)) = pred_loads
                        .iter()
                        .find(|(p, t, s, _)| *p == key && *t == *ty && *s == *seg_override)
                    {
                        pending.push((inst_pos, *dest, *covering));
                    } else if debug {
                        lccc::cli::stderr_line(format_args!(
                            "[IFCONV] arm block {} load ptr={} ty={:?} not covered by pred loads {:?}",
                            arm_idx,
                            ptr.0,
                            ty,
                            pred_loads
                                .iter()
                                .map(|(p, t, _, d)| (p.as_str(), *t, d.0))
                                .collect::<Vec<_>>()
                        ));
                        for b in &func.blocks {
                            for i in &b.instructions {
                                if i.dest() == Some(*ptr)
                                    || pred_loads.iter().any(|(_, _, _, d)| i.dest() == Some(*d))
                                {
                                    lccc::cli::stderr_line(format_args!("[IFCONV]   def: {:?}", i));
                                }
                            }
                        }
                    }
                }
            }
            for (inst_pos, dest, covering) in pending {
                func.blocks[arm_idx].instructions[inst_pos] = Instruction::Copy {
                    dest,
                    src: Operand::Value(covering),
                };
                rewrites += 1;
            }
        }
    }
    rewrites
}

/// Sink per-predecessor conditional stores to one phi-driven store in the
/// common successor.  When EVERY predecessor of a block M ends with a plain
/// store to the same canonical address (allowing distinct SSA ids for the
/// per-path GEPs), exactly one of those stores executes per visit of M —
/// replace them with a phi of the stored values plus a single store at the
/// top of M.
///
/// This unblocks diamond if-conversion for the store-arm form
/// (`if (c) d[i]=x; else d[i]=y;` — the classic clamp/branchy-store loop
/// shape): the phi turns the store into dataflow the diamond converter
/// turns into Selects.  The rewrite is value-exact and order-preserving in
/// every context (no loop precondition needed): the sunk store executes at
/// the same visit with the same value and address as before, after all of
/// M's phis and before any of M's other instructions, and every other
/// write in a pred still precedes it.
fn sink_conditional_stores(func: &mut IrFunction) -> usize {
    let num_blocks = func.blocks.len();
    if num_blocks < 3 {
        return 0;
    }
    let label_to_idx = analysis::build_label_map(func);
    let (preds, _succs) = analysis::build_cfg(func, &label_to_idx);
    let cfg = analysis::CfgAnalysis::build(func);
    let defs: FxHashMap<u32, Instruction> = func
        .blocks
        .iter()
        .flat_map(|b| b.instructions.iter())
        .filter_map(|i| i.dest().map(|d| (d.0, i.clone())))
        .collect();
    // Where each value is defined. The address-chain cloner needs this: a leaf
    // it may not simply keep is one defined in an arm that merely *reaches* the
    // merge (see the GlobalAddr note in `clone_addr_chain_into_merge`).
    let def_block: FxHashMap<u32, usize> = func
        .blocks
        .iter()
        .enumerate()
        .flat_map(|(bi, b)| {
            b.instructions
                .iter()
                .filter_map(move |i| i.dest().map(|d| (d.0, bi)))
        })
        .collect();
    let mut copy_of = FxHashMap::default();
    for b in &func.blocks {
        for i in &b.instructions {
            if let Instruction::Copy {
                dest,
                src: Operand::Value(src),
            } = i
            {
                copy_of.insert(*dest, *src);
            }
        }
    }

    struct SinkCand {
        pred_idx: usize,
        ty: IrType,
        key: String,
        val: Operand,
        ptr: Value,
    }

    let mut sunk = 0;
    for merge_idx in 0..num_blocks {
        // Distinct predecessors in ascending block order (a filter over
        // every block per merge was O(blocks^2)).
        let mut pred_list: Vec<usize> = preds.row(merge_idx).iter().map(|&q| q as usize).collect();
        pred_list.sort_unstable();
        pred_list.dedup();
        if pred_list.len() < 2 || pred_list.contains(&merge_idx) {
            continue;
        }
        let merge_label = func.blocks[merge_idx].label;
        let mut cands: Vec<SinkCand> = Vec::new();
        let mut ok = true;
        for &p in &pred_list {
            let block = &func.blocks[p];
            if !matches!(block.terminator, Terminator::Branch(l) if l == merge_label) {
                ok = false;
                break;
            }
            let Some(Instruction::Store {
                val,
                ptr,
                ty,
                volatile,
                seg_override,
            }) = block.instructions.last()
            else {
                ok = false;
                break;
            };
            if *volatile || *seg_override != AddressSpace::Default {
                ok = false;
                break;
            }
            cands.push(SinkCand {
                pred_idx: p,
                ty: *ty,
                key: canonical_addr_key_impl(&defs, &copy_of, ptr),
                val: *val,
                ptr: *ptr,
            });
        }
        if !ok {
            continue;
        }
        let ty = cands[0].ty;
        if !cands.iter().all(|c| c.ty == ty && c.key == cands[0].key) {
            continue;
        }
        // Rebuild ONE address chain inside M (fresh dests): the store GEP
        // from the first candidate, cloned through Copy/Cast/Shl/Add/Mul
        // nodes so every SSA def dominates M.  Chain leaves (parameters,
        // globals, IV phis, constants) dominate M by construction; a leaf
        // that is any other instruction makes the sink bail (fail closed).
        let Some((gep_val, chain_insts)) = clone_addr_chain_into_merge(
            &defs,
            &def_block,
            &cfg,
            merge_idx,
            &copy_of,
            cands[0].ptr,
            func,
        ) else {
            continue;
        };

        // Phi + GEP chain + store at the top of M (after M's existing
        // phis), in that order: the store uses the chain's fresh GEP.
        let phi_dest = Value(func.next_value_id);
        func.next_value_id += 1;
        let incoming: Vec<(Operand, BlockId)> = cands
            .iter()
            .map(|c| (c.val, func.blocks[c.pred_idx].label))
            .collect();
        let pos = func.blocks[merge_idx]
            .instructions
            .iter()
            .position(|i| !matches!(i, Instruction::Phi { .. }))
            .unwrap_or(func.blocks[merge_idx].instructions.len());
        let mut sunk_insts = Vec::with_capacity(2 + chain_insts.len());
        sunk_insts.push(Instruction::Phi {
            dest: phi_dest,
            ty,
            incoming,
        });
        sunk_insts.extend(chain_insts);
        sunk_insts.push(Instruction::Store {
            val: Operand::Value(phi_dest),
            ptr: gep_val,
            ty,
            volatile: false,
            seg_override: AddressSpace::Default,
        });
        // The primitive keeps spans AND inline provenance parallel; the sunk
        // store implements the merge block's entry context.
        func.blocks[merge_idx].insert_insts(pos, sunk_insts, crate::common::source::Span::dummy());
        // Drop the per-pred stores (and their spans/provenance).
        for c in &cands {
            let last = func.blocks[c.pred_idx].instructions.len().saturating_sub(1);
            drop(func.blocks[c.pred_idx].remove_inst(last));
        }
        sunk += cands.len();
    }
    sunk
}

/// Clone an address computation into the merge block with fresh SSA dests.
/// Walks Copy/Cast/Shl/Add/Mul chains from `ptr` down to dominating leaves;
/// bails when any link is outside that grammar.
fn clone_addr_chain_into_merge(
    defs: &FxHashMap<u32, Instruction>,
    def_block: &FxHashMap<u32, usize>,
    cfg: &analysis::CfgAnalysis,
    merge_idx: usize,
    copy_of: &FxHashMap<Value, Value>,
    ptr: Value,
    func: &mut IrFunction,
) -> Option<(Value, Vec<Instruction>)> {
    fn resolve(copy_of: &FxHashMap<Value, Value>, v: &Value) -> Value {
        let mut cur = *v;
        for _ in 0..64 {
            match copy_of.get(&cur) {
                Some(&next) if next != cur => cur = next,
                _ => break,
            }
        }
        cur
    }

    let mut emitted: Vec<Instruction> = Vec::new();
    let mut memo: FxHashMap<Value, Value> = FxHashMap::default();
    let mut next_val = func.next_value_id;

    fn clone_operand(
        operand: &Operand,
        defs: &FxHashMap<u32, Instruction>,
        def_block: &FxHashMap<u32, usize>,
        cfg: &analysis::CfgAnalysis,
        merge_idx: usize,
        copy_of: &FxHashMap<Value, Value>,
        memo: &mut FxHashMap<Value, Value>,
        emitted: &mut Vec<Instruction>,
        next_val: &mut u32,
    ) -> Option<Operand> {
        match operand {
            Operand::Const(c) => Some(Operand::Const(*c)),
            Operand::Value(v) => Some(Operand::Value(clone_value(
                *v, defs, def_block, cfg, merge_idx, copy_of, memo, emitted, next_val,
            )?)),
        }
    }

    #[allow(clippy::too_many_arguments)] // one recursive cloner, threaded context
    fn clone_value(
        v: Value,
        defs: &FxHashMap<u32, Instruction>,
        def_block: &FxHashMap<u32, usize>,
        cfg: &analysis::CfgAnalysis,
        merge_idx: usize,
        copy_of: &FxHashMap<Value, Value>,
        memo: &mut FxHashMap<Value, Value>,
        emitted: &mut Vec<Instruction>,
        next_val: &mut u32,
    ) -> Option<Value> {
        if let Some(&nv) = memo.get(&v) {
            return Some(nv);
        }
        let resolved = resolve(copy_of, &v);
        let Some(def) = defs.get(&resolved.0) else {
            // No def: a parameter or other block-boundary value — dominates
            // the merge block.
            memo.insert(v, resolved);
            return Some(resolved);
        };
        // Reserve this node's destination BEFORE recursively cloning its
        // operands.  Reserving afterwards gives every node in a nested chain
        // the same ID: Cast(iv)->Shl(2)->GEP became `vN=cast; vN=shl vN`, and
        // each optimizer iteration then composed the self-shift (2→4→8→16).
        // The Linux workqueue shard store consequently indexed int[c] as
        // c<<16 and corrupted an unrelated SCSI pointer 64 KiB away.
        //
        // A failed clone may leave an unused ID, which is harmless; reusing an
        // ID is not.  Children are still emitted before parents, preserving
        // def-before-use order independently of numeric ID order.
        let nv = Value(*next_val);
        *next_val += 1;
        match def {
            Instruction::Copy { src, .. } => {
                let ns = clone_operand(
                    src, defs, def_block, cfg, merge_idx, copy_of, memo, emitted, next_val,
                )?;
                emitted.push(Instruction::Copy { dest: nv, src: ns });
            }
            Instruction::Cast {
                src,
                to_ty,
                from_ty,
                ..
            } => {
                let ns = clone_operand(
                    src, defs, def_block, cfg, merge_idx, copy_of, memo, emitted, next_val,
                )?;
                emitted.push(Instruction::Cast {
                    dest: nv,
                    src: ns,
                    to_ty: *to_ty,
                    from_ty: *from_ty,
                });
            }
            Instruction::BinOp {
                op: op @ (IrBinOp::Shl | IrBinOp::Add | IrBinOp::Mul),
                lhs,
                rhs,
                ty,
                ..
            } => {
                let nl = clone_operand(
                    lhs, defs, def_block, cfg, merge_idx, copy_of, memo, emitted, next_val,
                )?;
                let nr = clone_operand(
                    rhs, defs, def_block, cfg, merge_idx, copy_of, memo, emitted, next_val,
                )?;
                emitted.push(Instruction::BinOp {
                    dest: nv,
                    op: *op,
                    lhs: nl,
                    rhs: nr,
                    ty: *ty,
                });
            }
            Instruction::Phi { .. } => {
                // A phi is a *dominating* leaf only when its block dominates the
                // merge.  The old blanket "keep the original" was wrong for a
                // switch-arm phi that merely reaches the merge: the sunk store
                // then read a value the other arms never defined, which is
                // exactly the def-dominates-use violation CCC_VERIFY_IR reports
                // for `minmax_refused.c`.  No dominator proof here -> refuse.
                let Some(&pb) = def_block.get(&resolved.0) else {
                    return None;
                };
                if !analysis::dominates_idom(&cfg.idom, pb, merge_idx) {
                    return None;
                }
                memo.insert(v, resolved);
                return Some(resolved);
            }
            Instruction::GlobalAddr { name, .. } => {
                // Reuse the original when it dominates the merge (the usual
                // case: one `GlobalAddr` in the entry block serves every arm),
                // and rematerialise when it does not — the address of a global
                // is the same at every point, so a fresh definition in the merge
                // is always available and always correct.  Reusing a
                // non-dominating one is the `minmax_refused.c` violation: the
                // switch arm's `GlobalAddr "a"` fed a store sunk into a
                // 4-incoming join, so the other arms' paths read a value they
                // never defined.
                if def_block
                    .get(&resolved.0)
                    .is_some_and(|&gb| analysis::dominates_idom(&cfg.idom, gb, merge_idx))
                {
                    memo.insert(v, resolved);
                    return Some(resolved);
                }
                emitted.push(Instruction::GlobalAddr {
                    dest: nv,
                    name: name.clone(),
                });
            }
            Instruction::ParamRef { .. } => {
                // Reuse only with a proof of dominance: canonical lowering puts
                // `ParamRef` in the entry block, which dominates every merge, so
                // the common case is unchanged.  It must NOT be rematerialised —
                // the backend materialises a parameter from its ABI read, which
                // only happens in the entry block.  Measured: cloning one into a
                // merge dropped the second parameter's read and
                // `bb_slp_nested_ifconv` stored through a garbage pointer (rc=139).
                // Without the proof, refuse the sink.
                let Some(&pb) = def_block.get(&resolved.0) else {
                    return None;
                };
                if !analysis::dominates_idom(&cfg.idom, pb, merge_idx) {
                    return None;
                }
                memo.insert(v, resolved);
                return Some(resolved);
            }
            _ => return None,
        }
        memo.insert(v, nv);
        Some(nv)
    }

    let resolved = resolve(copy_of, &ptr);
    let Some(Instruction::GetElementPtr {
        base,
        offset,
        ty,
        dest: _,
    }) = defs.get(&resolved.0)
    else {
        return None;
    };
    let nb = clone_value(
        *base,
        defs,
        def_block,
        cfg,
        merge_idx,
        copy_of,
        &mut memo,
        &mut emitted,
        &mut next_val,
    )?;
    let noff = clone_operand(
        offset,
        defs,
        def_block,
        cfg,
        merge_idx,
        copy_of,
        &mut memo,
        &mut emitted,
        &mut next_val,
    )?;
    let gep_val = Value(next_val);
    next_val += 1;
    emitted.push(Instruction::GetElementPtr {
        dest: gep_val,
        base: nb,
        offset: noff,
        ty: *ty,
    });
    func.next_value_id = next_val;
    Some((gep_val, emitted))
}

/// Single pass of if-conversion. Returns number of diamonds converted.
/// A short-circuit branch chain that can be folded into ONE branch.
struct IfCombine {
    /// The block holding the first `CondBranch`.
    pred_idx: usize,
    /// The single-predecessor block holding the second `CondBranch`.
    inner_idx: usize,
    /// `true` for `&&` (the two branches share their FALSE target),
    /// `false` for `||` (they share their TRUE target).
    is_and: bool,
    /// First condition (already in `pred`).
    outer_cond: Operand,
    /// Second condition (computed by `inner`).
    inner_cond: Operand,
    /// Instructions to hoist out of `inner` into `pred`.
    hoist: Vec<Instruction>,
    /// Targets of the folded branch.
    new_true: BlockId,
    new_false: BlockId,
    /// The label that loses one of its two incoming edges.
    shared: BlockId,
    /// The label whose incoming edge moves from `inner` to `pred`.
    retargeted: BlockId,
}

/// Fold a two-level short-circuit condition chain into a single branch on a
/// bitwise `and`/`or` (GCC calls this `tree-ssa-ifcombine`).
///
/// C's `&&` and `||` lower to a branch chain, and if-conversion only sees
/// diamonds and triangles, so a chain is converted at most one level deep.
/// `((c>='a' && c<='z') || (c>='A' && c<='Z')) ? 1 : 0` therefore came out
/// half converted: the second conjunction became two `Select`s, while the
/// first stayed a branch chain and blocked vectorization of the whole loop
/// (`BAIL: internal condbranch`).
///
/// The shapes, with `preds(inner) == {pred}`:
///
/// ```text
///   AND                              OR
///   pred: br %a, inner, F            pred: br %a, T, inner
///   inner: <insts> br %b, X, F       inner: <insts> br %b, T, Y
///   =>                               =>
///   pred: <insts> %c = and %a, %b    pred: <insts> %c = or %a, %b
///         br %c, X, F                      br %c, T, Y
/// ```
///
/// Soundness, every clause fail-closed:
///
/// 1. **`inner` has exactly one predecessor, `pred`.**  Hoisting its
///    instructions into `pred` then changes no other path, and deleting the
///    `pred -> inner` edge makes `inner` unreachable rather than merely
///    less used.
/// 2. **`inner` is side-effect free and non-trapping** (`is_side_effect_free`
///    excludes loads, stores, calls and trapping arithmetic), so executing
///    it when `%a` is false is unobservable.  Note the chain evaluates `%b`
///    eagerly, which is exactly what C's short-circuit semantics forbid
///    OBSERVING -- and an expression with no side effects and no traps has
///    nothing to observe.
/// 3. **Both conditions are `Cmp` results**, hence 0 or 1.  This is what
///    makes the BITWISE `and`/`or` equal to the logical connective: for
///    arbitrary integers `3 && 4` is true while `3 & 4` is 0.  Constants 0/1
///    are accepted for the same reason.
/// 4. **`inner` has no phis** (it has one predecessor, so any phi would be
///    degenerate, but a degenerate phi still needs rewriting -- reject
///    instead of guessing).
/// 5. **The shared target's phis agree on both edges.**  `shared` currently
///    has an edge from `pred` (condition false) and one from `inner`
///    (condition true, second false); after folding only one edge remains,
///    so a phi that distinguishes them cannot be preserved.  Equal incoming
///    values are fine and are the common case (the frontend threads values
///    through `Copy`, not phis).
/// 6. **The retargeted block's phis are relabelled** from `inner` to `pred`.
///    Values they name that were defined in `inner` are still in scope: the
///    hoist moved those definitions into `pred`, which dominates everything
///    `inner` dominated.
///
/// Termination: each application removes one `CondBranch` from the function,
/// and the caller's fixpoint loop re-derives the CFG each round.
fn detect_if_combine(ctx: &IfConvCtx<'_>, pred_idx: usize) -> Option<IfCombine> {
    let Terminator::CondBranch {
        cond: outer_cond,
        true_label,
        false_label,
    } = &ctx.func.blocks[pred_idx].terminator
    else {
        return None;
    };
    let (true_idx, false_idx) = (
        *ctx.label_to_idx.get(true_label)?,
        *ctx.label_to_idx.get(false_label)?,
    );

    // `&&` puts the chain on the TRUE edge, `||` on the FALSE edge.
    for (is_and, inner_idx) in [(true, true_idx), (false, false_idx)] {
        if inner_idx == pred_idx {
            continue;
        }
        let inner = &ctx.func.blocks[inner_idx];
        // (1) single predecessor
        if ctx.preds.row(inner_idx) != [pred_idx as u32] {
            continue;
        }
        // (4) no phis
        if inner
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::Phi { .. }))
        {
            continue;
        }
        // (2) speculatable
        if !is_side_effect_free(inner) {
            continue;
        }
        let Terminator::CondBranch {
            cond: inner_cond,
            true_label: it,
            false_label: if_,
        } = &inner.terminator
        else {
            continue;
        };
        // The chain must rejoin the outer branch's OTHER target.
        let (shared, retargeted) = if is_and {
            // pred: br %a, inner, F ; inner: br %b, X, F
            if if_ != false_label {
                continue;
            }
            (*false_label, *it)
        } else {
            // pred: br %a, T, inner ; inner: br %b, T, Y
            if it != true_label {
                continue;
            }
            (*true_label, *if_)
        };
        // A self-loop or a fold that collapses both targets is cfg_simplify's
        // job, not ours.
        if shared == retargeted || retargeted == ctx.func.blocks[inner_idx].label {
            continue;
        }
        // (3) both conditions must be 0/1
        if !is_boolean_condition(ctx, outer_cond) || !is_boolean_condition(ctx, inner_cond) {
            continue;
        }
        // (5) the shared block's phis must not distinguish the two edges
        let pred_label = ctx.func.blocks[pred_idx].label;
        let inner_label = ctx.func.blocks[inner_idx].label;
        let shared_idx = *ctx.label_to_idx.get(&shared)?;
        let mut phis_agree = true;
        for inst in &ctx.func.blocks[shared_idx].instructions {
            let Instruction::Phi { incoming, .. } = inst else {
                break; // phis are a prefix
            };
            let from_pred = incoming
                .iter()
                .find(|(_, l)| *l == pred_label)
                .map(|(v, _)| v);
            let from_inner = incoming
                .iter()
                .find(|(_, l)| *l == inner_label)
                .map(|(v, _)| v);
            if let (Some(a), Some(b)) = (from_pred, from_inner) {
                if a != b {
                    phis_agree = false;
                    break;
                }
            }
        }
        if !phis_agree {
            continue;
        }
        let (new_true, new_false) = if is_and {
            (retargeted, shared)
        } else {
            (shared, retargeted)
        };
        return Some(IfCombine {
            pred_idx,
            inner_idx,
            is_and,
            outer_cond: outer_cond.clone(),
            inner_cond: inner_cond.clone(),
            hoist: ctx.func.blocks[inner_idx].instructions.clone(),
            new_true,
            new_false,
            shared,
            retargeted,
        });
    }
    None
}

/// Apply this loop's short-circuit combines, but only if they pay off.
///
/// # The trade
///
/// If-combine converts a control dependence into a data dependence: `a && b`
/// stops branching around `b` and evaluates it unconditionally.  That is a
/// WIN only when the loop body ends up straight-line, because then the map /
/// reduction vectorizer can take it and the body runs 8-32 elements at a
/// time.  It is a LOSS otherwise: the branch stays, and every iteration now
/// pays for operands the short circuit used to skip.
///
/// The loss is not theoretical.  `expat_utf8_name_length`
/// (`tests/benchmark/programs/expat_xml_scan.c`) is a UTF-8 scanner whose
/// loop body contains FIVE `break`s, so it can never be vectorized.
/// Combining its
/// `(ptr[1] & 0xc0) != 0x80 || (width > 2 && ...) || (width > 3 && ...)`
/// chain grew the function 58 -> 90 instructions and made the common path —
/// which exits on the first test — do all the work of the last.  The
/// codegen-quality gate caught it as +24% on the whole workload.
///
/// # The test
///
/// Exactly the property that separates the two cases: after applying this
/// loop's combines, does every remaining internal conditional branch become
/// diamond/triangle-convertible — i.e. does the body become branch-free?
///
/// That is answered by SIMULATION, not prediction: the combines are applied
/// for real and rolled back if the answer is no.  The interaction between a
/// combine and the diamond converter is precisely what a hand-written
/// heuristic gets wrong — in the classifier case the combine leaves one
/// conditional branch which the diamond converter then removes, so a rule
/// like "the combine must remove them all" would reject the very case the
/// transform exists for.
///
/// Rollback is exact: `apply_if_combine` writes only the four blocks named
/// by each candidate (pred, inner, the shared target's phis and the
/// retargeted target's phis), so snapshotting those blocks and the value
/// counter restores the function bit-for-bit.  Cheap pre-filters run first
/// so the snapshot is only paid for on loops that could plausibly qualify.
///
/// Returns the number of combines actually applied (0 when rolled back).
fn try_if_combine_loop(
    func: &mut IrFunction,
    succs: &analysis::FlatAdj,
    nl: &crate::passes::loop_analysis::NaturalLoop,
    cands: &[IfCombine],
) -> usize {
    // Pre-filter 1: a call or inline asm in the body is a barrier the
    // vectorizer cannot cross, so straight-line-ness cannot pay for itself.
    // (Loads and stores are fine — they are the point.)
    let dbg = std::env::var("CCC_DEBUG_IFCOMBINE").is_ok();
    for &b in &nl.body {
        for inst in &func.blocks[b].instructions {
            if matches!(
                inst,
                Instruction::Call { .. } | Instruction::InlineAsm { .. }
            ) {
                if dbg {
                    lccc::cli::stderr_line(format_args!(
                        "[IFCOMB] {} loop@{}: reject (call/asm)",
                        func.name, nl.header
                    ));
                }
                return 0;
            }
        }
    }
    // Pre-filter 2: the loop must have exactly ONE exiting block.
    //
    // That block carries the loop control and its branch must survive; every
    // OTHER conditional branch in the body is a value computation that
    // predication can remove.  A body with `break`/`return` has several
    // exiting blocks, i.e. data-dependent exits that predication cannot
    // remove, so the branches survive whatever we do and the combine is pure
    // added work.
    //
    // Note the exiting block is NOT required to be the header: loop rotation
    // leaves the canonical bottom-tested shape, where the guard is in the
    // header's predecessor and the live exit is the LATCH.  Requiring the
    // header here rejected every rotated loop — including the ASCII
    // classifier this transform exists for.
    let mut exiting: Option<usize> = None;
    for &b in &nl.body {
        let leaves = succs
            .row(b)
            .iter()
            .any(|&t| !nl.body.contains(&(t as usize)));
        if leaves {
            if exiting.is_some() {
                if dbg {
                    lccc::cli::stderr_line(format_args!(
                        "[IFCOMB] {} loop@{}: reject (multiple exiting blocks)",
                        func.name, nl.header
                    ));
                }
                return 0;
            }
            exiting = Some(b);
        }
    }
    let Some(exiting) = exiting else {
        return 0;
    };
    // The exiting block's branch IS the loop control; folding it into a
    // chain would restructure the exit, which is not what this transform is
    // for.  Fail closed on the whole loop rather than partially applying.
    if cands
        .iter()
        .any(|c| c.pred_idx == exiting || c.inner_idx == exiting)
    {
        if dbg {
            lccc::cli::stderr_line(format_args!(
                "[IFCOMB] {} loop@{}: reject (candidate touches the exiting block {})",
                func.name, nl.header, exiting
            ));
        }
        return 0;
    }

    // Pre-filter 4: profitability — the guard the module docs promise, and
    // which the module docs say is answered by SIMULATION.
    //
    // A combine converts a control dependence into a data dependence. The
    // strongest win is a branch-free loop body (straight-line goes to the
    // vectorizer); independently, branches in the candidate chain may remain
    // because these are the edges this transform simplifies. Unrelated
    // internal branches must still become convertible, or the combine risks
    // adding per-iteration predicate work without removing control flow.
    //
    // The question is therefore: after applying THIS round's combines, can
    // every remaining non-chain conditional become diamond/triangle
    // convertible? A branch in a candidate chain block may remain: that is
    // precisely the short-circuit edge this combine evaluates. Probing the
    // UNMODIFIED IR — the historical behaviour — cannot answer the question
    // whenever two chain links share a join block:
    //
    //     ptr:  br a, B2, JOIN       (|| link 1)
    //     B3:   br b, JOIN, B4       (|| link 2)
    //
    // On the live IR neither link converts (`plan_shared_arm_dup` only
    // duplicates a one-level diamond, and it refuses here because B3's other
    // arm continues the chain), so `a || b || c || ...` — the ASCII
    // classifier this transform exists for — was rejected outright with
    // "branch in block N survives". Three of four links DID convert; testing
    // them one at a time hides exactly that.
    //
    // So: simulate. The combines are applied for real to a copy of the
    // function, the diamond/triangle detectors are run to fixpoint on the
    // result with the same CPU-model budgets, and the whole thing is thrown
    // away — `func` is never written here. `apply_if_combine` is
    // deterministic in the values the detectors read (labels, terminators,
    // phi edges, conditions), so re-applying the same candidates to the real
    // function below is exactly what was simulated.
    //
    // Fail-closed (reject = keep the short circuit) is always safe: the
    // uncombined IR is the pipeline's common case.
    {
        // A candidate may leave a conditional in one of its own chain blocks:
        // that is the branch whose combine this round is evaluating. The old
        // profitability check intentionally exempted these blocks and checked
        // all other loop branches. Keep that contract, but now test the
        // post-combine CFG and simulate every convertible non-chain branch.
        let chain_blocks: FxHashSet<usize> = cands
            .iter()
            .flat_map(|c| [c.pred_idx, c.inner_idx])
            .collect();
        let mut probe = func.clone();
        let mut next_real = probe.next_value_id;
        let applied = cands
            .iter()
            .filter(|c| apply_if_combine(&mut probe, c, &mut next_real))
            .count();
        probe.next_value_id = next_real;
        if applied == 0 {
            // Every candidate was stale; nothing to simulate (and the
            // diagnostic below is about the unmodified loop).
            if dbg {
                lccc::cli::stderr_line(format_args!(
                    "[IFCOMB] {} loop@{}: reject (no candidate applicable)",
                    func.name, nl.header
                ));
            }
            return 0;
        }
        if !body_becomes_branch_free(&mut probe, nl, exiting, &chain_blocks, dbg) {
            if dbg {
                lccc::cli::stderr_line(format_args!(
                    "[IFCOMB] {} loop@{}: reject (a branch in the body survives the combine)",
                    func.name, nl.header
                ));
            }
            return 0;
        }
    }
    if dbg {
        lccc::cli::stderr_line(format_args!(
            "[IFCOMB] {} loop@{}: accept, {} candidate(s), exiting={}",
            func.name,
            nl.header,
            cands.len(),
            exiting
        ));
    }
    // Accepted: apply this round's candidates.  The caller's fixpoint comes
    // back for the next level of the chain and re-derives the same decision.
    let mut next_real = func.next_value_id;
    let mut applied = 0usize;
    for c in cands {
        if apply_if_combine(func, c, &mut next_real) {
            applied += 1;
        }
    }
    func.next_value_id = next_real;
    applied
}

/// Does every non-exiting, non-chain conditional branch become
/// diamond/triangle convertible once the combines have been applied?
///
/// Runs the real detectors to a fixpoint, converting as it goes, so a branch
/// that only becomes convertible AFTER an inner one folded still counts --
/// the chain `((a || b) || c) || d` needs its links converted inside-out. The
/// detectors are the same ones `if_convert_diamonds` uses, with the same
/// CPU-model budgets, so this cannot drift from what the pass will actually
/// do to the loop afterwards.
///
/// `nl.body` is the PRE-combine body set: combines only merge blocks inside
/// it (they retarget edges between body blocks and reuse the same labels), so
/// it remains a superset of the post-combine body and testing it can only be
/// conservative. The candidate chain blocks are exempt from the final
/// survivor test; their conditional edges are the transform's subject.
fn body_becomes_branch_free(
    probe: &mut IrFunction,
    nl: &crate::passes::loop_analysis::NaturalLoop,
    exiting: usize,
    chain_blocks: &FxHashSet<usize>,
    dbg: bool,
) -> bool {
    let body: Vec<usize> = {
        let mut v: Vec<usize> = nl.body.iter().copied().collect();
        v.sort_unstable();
        v
    };
    // Bounded: each conversion removes at least one conditional branch and a
    // body of B blocks holds at most B of them. The bound is belt-and-braces
    // against a detector/apply pair that disagrees about progress.
    for _ in 0..=body.len() {
        let ctx = IfConvCtx::build(probe);
        let mut plan: Option<DiamondInfo> = None;
        for &b in &body {
            if b == exiting || b >= probe.blocks.len() {
                continue;
            }
            if !matches!(probe.blocks[b].terminator, Terminator::CondBranch { .. }) {
                continue;
            }
            // Same precedence as `if_convert_diamonds` (diamond, else
            // triangle), same detectors, same CPU-model budgets.
            if let Some(d) = detect_diamond_inner(&ctx, b, true) {
                plan = Some(d);
                break;
            }
            if let Some(t) = detect_triangle_inner(&ctx, b, true) {
                plan = Some(t);
                break;
            }
        }
        drop(ctx);
        let Some(plan) = plan else { break };
        if !apply_diamond(probe, &plan) {
            break;
        }
        if dbg {
            lccc::cli::stderr_line(format_args!(
                "[IFCOMB]     sim: converted pred={} merge={}",
                plan.pred_idx, plan.merge_idx
            ));
        }
    }
    if dbg {
        for &b in &body {
            if b != exiting && b < probe.blocks.len() {
                if let Terminator::CondBranch { cond, .. } = &probe.blocks[b].terminator {
                    lccc::cli::stderr_line(format_args!(
                        "[IFCOMB]     sim: survivor block {} cond {:?} insts {}",
                        b,
                        cond,
                        probe.blocks[b].instructions.len()
                    ));
                }
            }
        }
    }
    // The current chain may retain the branch whose combine was just
    // evaluated; every other surviving conditional must be the loop exit or
    // the profitability proof failed.
    body.iter().all(|&b| {
        b == exiting
            || chain_blocks.contains(&b)
            || b >= probe.blocks.len()
            || !matches!(probe.blocks[b].terminator, Terminator::CondBranch { .. })
    })
}

/// Is `op` provably 0 or 1?  Only then is a bitwise `and`/`or` the logical
/// connective (see `detect_if_combine` clause 3).
fn is_boolean_condition(ctx: &IfConvCtx<'_>, op: &Operand) -> bool {
    match op {
        Operand::Const(c) => matches!(c.to_i64(), Some(0) | Some(1)),
        Operand::Value(v) => {
            let root = ctx.resolve(v);
            matches!(ctx.defs.get(&root.0), Some(Instruction::Cmp { .. }))
        }
    }
}

fn apply_if_combine(func: &mut IrFunction, c: &IfCombine, next_val: &mut u32) -> bool {
    // Re-verify the shape against the live function: the caller may have
    // applied an earlier combine that touched these blocks.
    let pred_label = func.blocks[c.pred_idx].label;
    let inner_label = func.blocks[c.inner_idx].label;
    if !matches!(&func.blocks[c.pred_idx].terminator,
        Terminator::CondBranch { true_label, false_label, .. }
            if (c.is_and && *true_label == inner_label && *false_label == c.shared)
                || (!c.is_and && *false_label == inner_label && *true_label == c.shared))
    {
        return false;
    }

    let combined = Value(*next_val);
    *next_val += 1;
    let ty = IrType::I32;
    let mut hoisted = c.hoist.clone();
    hoisted.push(Instruction::BinOp {
        dest: combined,
        op: if c.is_and { IrBinOp::And } else { IrBinOp::Or },
        lhs: c.outer_cond.clone(),
        rhs: c.inner_cond.clone(),
        ty,
    });
    func.blocks[c.pred_idx].extend_insts(hoisted, crate::common::source::Span::dummy());
    func.blocks[c.pred_idx].terminator = Terminator::CondBranch {
        cond: Operand::Value(combined),
        true_label: c.new_true,
        false_label: c.new_false,
    };

    // The retargeted block's phi edges now come from `pred`.
    if let Some(&ri) = build_label_map_once(func).get(&c.retargeted) {
        for inst in &mut func.blocks[ri].instructions {
            let Instruction::Phi { incoming, .. } = inst else {
                break;
            };
            for (_, label) in incoming.iter_mut() {
                if *label == inner_label {
                    *label = pred_label;
                }
            }
        }
    }
    // The shared block loses its edge from `inner` (clause 5 proved the
    // remaining edge carries the same values).
    if let Some(&si) = build_label_map_once(func).get(&c.shared) {
        for inst in &mut func.blocks[si].instructions {
            let Instruction::Phi { incoming, .. } = inst else {
                break;
            };
            incoming.retain(|(_, label)| *label != inner_label);
        }
    }
    // `inner` is now unreachable; leave the empty shell for cfg_simplify
    // rather than renumbering blocks mid-pass.
    func.blocks[c.inner_idx].clear_insts();
    func.blocks[c.inner_idx].terminator = Terminator::Branch(c.shared);
    true
}

fn build_label_map_once(func: &IrFunction) -> FxHashMap<BlockId, usize> {
    analysis::build_label_map(func)
}

// ── Conditional-load sinking: speculate the ADDRESS, never the LOAD ────────
//
// §9b/SV-03's sharpest datum is `select_f32`: a 35x slowdown because the loop
// stays a scalar branch diamond (`ucomiss`/`jbe`) on unpredictable data.  The
// ordinary diamond converter declines it: hoisting both arms into the pred
// would speculate BOTH loads, and a load can trap.  `arm_load_speculation_ok`
// only accepts an arm load when a covering access dominates every path, which
// two DISTINCT arrays (`fb[i]` / `fd[i]`) can never provide.
//
// The sound alternative is to speculate the ADDRESS and keep exactly ONE
// memory read:
//
//   pred:  <pure address chains from both arms, hoisted>   (no memory ops)
//          %sel = select %cond, %p_true, %p_false          (Ptr, cmov)
//          branch merge                                   (unconditional)
//   merge: %r = load %sel                                 (replaces the Phi)
//
// `%sel` is EXACTLY the address the original program dereferenced, so there
// is no new fault, no new observable memory access and no aliasing change.
// Computing an address (GEP / integer arithmetic / Cast / Copy / Cmp /
// GlobalAddr / Select) cannot trap; Div/Rem (`can_trap`), volatile loads,
// mismatched load types and every other arm instruction are rejected.  Both
// arm chains must be available at the pred (no cross-arm or post-merge SSA
// use), and each load result must have exactly one use: the merge Phi.
//
// `CCC_NO_COND_LOAD_SINK` is the A/B kill switch for the differential gates.

/// Pure, trap-free address/value computation that can be hoisted above the
/// diamond's guard.  Mirrors [`is_side_effect_free`] minus the memory ops.
fn cond_load_hoistable(inst: &Instruction) -> bool {
    match inst {
        Instruction::BinOp { op, .. } if op.can_trap() => false,
        Instruction::BinOp { .. }
        | Instruction::UnaryOp { .. }
        | Instruction::Cmp { .. }
        | Instruction::Cast { .. }
        | Instruction::Copy { .. }
        | Instruction::GetElementPtr { .. }
        | Instruction::GlobalAddr { .. }
        | Instruction::Select { .. } => true,
        _ => false,
    }
}

/// Faithful, injective integer-extension cast: no bit is lost and no
/// float/pointer reinterpretation occurs.  Swift is deliberately absent.
fn cond_load_widening_cast(from_ty: IrType, to_ty: IrType) -> bool {
    use IrType::*;
    matches!(
        (from_ty, to_ty),
        (I8, I16 | I32 | I64)
            | (U8, U16 | U32 | U64 | I16 | I32 | I64)
            | (I16, I32 | I64)
            | (U16, U32 | U64 | I32 | I64)
            | (I32, I64)
            | (U32, U64 | I64)
    )
}

/// One already-validated promotion Cast in an arm's linear tail.
///
/// Keeping the typed fields instead of a cloned `Instruction` makes the apply
/// phase total: it can emit a Cast without re-matching a variant, unwrapping a
/// destination, or reaching an `unreachable!` branch.  The parser is the only
/// place that inspects an untrusted shape.
#[derive(Clone, Copy, Debug)]
struct CondLoadCastTail {
    dest: Value,
    from_ty: IrType,
    to_ty: IrType,
}

struct CondLoadArm {
    /// Pure instructions BEFORE the load only, in original order.  Anything
    /// AFTER the load is a promotion tail and is re-emitted in the merge, not
    /// hoisted into the pred (otherwise one SSA id gets two definitions).
    pure_insts: Vec<Instruction>,
    load_dest: Value,
    load_ptr: Value,
    ty: IrType,
    seg: AddressSpace,
    /// All destination ids defined anywhere in this arm.
    defined: FxHashSet<u32>,
    /// Pure, bijective integer-promotion/sign-extension casts between the
    /// load and the merge Phi.  Empty for the canonical direct-load shape.
    /// Both arms must have the IDENTICAL type chain.
    tail: Vec<CondLoadCastTail>,
    /// Phi incoming: the last `tail` dest, or the load dest when empty.
    tail_result: Value,
    /// Type of `tail_result`: the final Cast's target, or the load type.
    result_ty: IrType,
}

fn parse_cond_load_arm(block: &BasicBlock) -> Option<CondLoadArm> {
    let mut pure_insts = Vec::with_capacity(block.instructions.len());
    let mut defined = FxHashSet::default();
    let mut load: Option<(usize, Value, Value, IrType, AddressSpace)> = None;
    for (idx, inst) in block.instructions.iter().enumerate() {
        if let Some(d) = inst.dest() {
            defined.insert(d.0);
        }
        match inst {
            Instruction::Load {
                dest,
                ptr,
                ty,
                seg_override,
                volatile,
            } => {
                // Volatile reads are observable; atomics are a different
                // instruction kind and never match here.  Exactly ONE load is
                // the whole point: two loads would need two selects.
                if *volatile || load.is_some() {
                    return None;
                }
                load = Some((idx, *dest, *ptr, *ty, *seg_override));
            }
            other if cond_load_hoistable(other) => {
                // Only the address chain BEFORE the load may be hoisted.
                // Pure instructions after it are validated as a promotion
                // tail below and re-emitted after the sunk load.
                if load.is_none() {
                    pure_insts.push(other.clone());
                }
            }
            _ => return None,
        }
    }
    let (load_index, load_dest, load_ptr, ty, seg) = load?;
    // C integer promotion puts Cast(I8/U8/I16/U16 -> I32) between a narrow
    // load and a merge Phi.  Refusing those shapes leaves LCCC branchy while
    // GCC 16.2 and Clang 23.1.5 emit `cmov` + one `movzbl`/`movswl`.
    // Accept ONLY a pure linear chain of *bijective* integer casts whose
    // source type matches the previous result: no truncation, no
    // float/pointer reinterpretation, no address/segment changes.
    let mut tail: Vec<CondLoadCastTail> = Vec::new();
    let mut tail_result = load_dest;
    let mut expected_from = ty;
    for inst in &block.instructions[load_index + 1..] {
        let Instruction::Cast {
            dest,
            src,
            from_ty,
            to_ty,
        } = inst
        else {
            return None;
        };
        if !matches!(*src, Operand::Value(v) if v == tail_result)
            || *from_ty != expected_from
            || !cond_load_widening_cast(*from_ty, *to_ty)
        {
            return None;
        }
        tail.push(CondLoadCastTail {
            dest: *dest,
            from_ty: *from_ty,
            to_ty: *to_ty,
        });
        tail_result = *dest;
        expected_from = *to_ty;
    }
    let result_ty = tail.last().map_or(ty, |cast| cast.to_ty);
    Some(CondLoadArm {
        pure_insts,
        load_dest,
        load_ptr,
        ty,
        seg,
        defined,
        tail,
        tail_result,
        result_ty,
    })
}

/// Every value used by the arm must be available AFTER hoisting the arm's
/// instruction chain into the branch predecessor.  Three cases:
///   * defined EARLIER in this same arm → the hoisted order still defines it;
///   * defined in the OTHER arm → the other chain is hoisted too, but at a
///     LATER position than the arm currently executing, so it is NOT a valid
///     SSA use; refuse (cross-arm dataflow is not what this transform models);
///   * defined anywhere else → that block must DOMINATE the pred.  "Not in an
///     arm" alone is NOT enough: a definition in the merge block sits AFTER
///     the arms and cannot dominate the pred, so hoisting would create a
///     use-before-def.  This is a real gap the first revision had.
fn cond_load_arm_available(
    insts: &[Instruction],
    own_defined: &FxHashSet<u32>,
    other_defined: &FxHashSet<u32>,
    def_block: &FxHashMap<u32, usize>,
    dom: &analysis::DominanceChecker,
    pred: usize,
) -> bool {
    let mut seen: FxHashSet<u32> = FxHashSet::default();
    for inst in insts {
        let mut ok = true;
        inst.for_each_used_value(|v| {
            if other_defined.contains(&v) {
                ok = false;
            } else if own_defined.contains(&v) {
                if !seen.contains(&v) {
                    ok = false;
                }
            } else {
                // Fail closed on an unknown defining block (an unmaterialized
                // value, or IR the def census did not see).
                ok &= def_block.get(&v).is_some_and(|&b| dom.dominates(b, pred));
            }
        });
        if !ok {
            return false;
        }
        if let Some(d) = inst.dest() {
            seen.insert(d.0);
        }
    }
    true
}

/// Load types this transform may carry.  Everything here is a single GPR or
/// SSE-scalar load; 128-bit integers, `long double` and `Void` need aggregate
/// or multi-register paths the pointer Select does not model.
fn cond_load_type_supported(ty: IrType) -> bool {
    matches!(
        ty,
        IrType::I8
            | IrType::U8
            | IrType::I16
            | IrType::U16
            | IrType::I32
            | IrType::U32
            | IrType::I64
            | IrType::U64
            | IrType::F32
            | IrType::F64
            | IrType::D32
            | IrType::D64
            | IrType::Ptr
    )
}

/// Types the backend's `Select` lowering carries without an aggregate path.
/// Narrow integers have no `cmov` form and D32/D64 are decimal carriers that
/// take the FP-blend path; refuse those for auxiliary phis (fail closed).
fn cond_load_phi_selectable(ty: IrType) -> bool {
    matches!(
        ty,
        IrType::I32
            | IrType::U32
            | IrType::I64
            | IrType::U64
            | IrType::F32
            | IrType::F64
            | IrType::Ptr
    )
}

/// A merge Phi OTHER than the load-result phi.  Once the arms disappear, M has
/// a single predecessor, so these phis must become Selects in the pred with
/// their original destinations (then be removed from M).
struct CondLoadPhi {
    idx: usize,
    dest: Value,
    ty: IrType,
    tv: Operand,
    fv: Operand,
}

struct CondLoadPlan {
    pred: usize,
    t: usize,
    f: usize,
    merge: usize,
    merge_label: BlockId,
    cond: Operand,
    t_arm: CondLoadArm,
    f_arm: CondLoadArm,
    /// Shared promotion chain, re-emitted after the sunk load.
    tail: Vec<CondLoadCastTail>,
    phi_idx: usize,
    phi_dest: Value,
    extra_phis: Vec<CondLoadPhi>,
}

/// Structural identity for an address expression, independent of SSA id.
///
/// A guard that reads `a[i]` and the arm that reloads `a[i]` frequently use
/// DIFFERENT IR spellings with DIFFERENT SSA ids: the frontend emits a
/// `GetElementPtr` for one and `BinOp::Add` (or `Sub`) for the other.  This
/// was measured on generated shapes (`GEP(a, 24)` vs `Add(a, 24)`), and an
/// SSA-id-only comparison missed exactly those cases.  Both forms are folded
/// into ONE canonical sum of atomic operands plus a byte displacement, with
/// associativity/commutativity and zero/nested constant offsets normalised.
///
/// Ambiguous definitions, side-effecting instructions, multiply-defined SSA
/// homes, arithmetic overflow and recursion beyond a small bound produce an
/// OPAQUE, UNIQUE key.  The comparison must never claim two unknown addresses
/// are equal.  These keys are used ONLY to DECLINE an optimization, so even a
/// hypothetical false positive is conservative; every rule below is still
/// designed to be sound.
type CondLoadDefs<'a> = FxHashMap<u32, Vec<&'a Instruction>>;

#[derive(Clone, Debug, PartialEq, Eq)]
struct CondAddrExpr {
    /// Sorted multiset of non-constant, non-Add terms.
    atoms: Vec<String>,
    /// Sum of every constant byte displacement in the Add chain.
    offset: i128,
}

impl CondAddrExpr {
    fn atom(key: String) -> Self {
        Self {
            atoms: vec![key],
            offset: 0,
        }
    }

    fn constant(n: i128) -> Self {
        Self {
            atoms: Vec::new(),
            offset: n,
        }
    }

    fn opaque(v: Value) -> Self {
        Self::atom(format!("opaque:{}", v.0))
    }

    fn add(self, rhs: Self, v: Value) -> Self {
        let Some(offset) = self.offset.checked_add(rhs.offset) else {
            // i128 exhaustion is only reachable from malformed/very deep IR.
            // Fail closed with a deterministic, operand-dependent token.
            return Self::atom(format!(
                "opaque:overflow:{}:{}:{}",
                v.0,
                self.key(),
                rhs.key()
            ));
        };
        let mut atoms = self.atoms;
        atoms.extend(rhs.atoms);
        Self { atoms, offset }
    }

    fn key(&self) -> String {
        let mut atoms = self.atoms.clone();
        atoms.sort_unstable();
        if self.offset == 0 {
            // The three cases are exhaustive; no indexing, unwrap or
            // unreachable branch can abort on malformed IR.
            return match atoms.as_slice() {
                [] => "const(0)".to_string(),
                [only] => only.clone(),
                _ => format!("add({})", atoms.join(",")),
            };
        }
        let mut parts = Vec::with_capacity(atoms.len() + 1);
        parts.push(format!("const({})", self.offset));
        parts.extend(atoms);
        format!("add({})", parts.join(","))
    }
}

fn cond_load_const_offset(op: &Operand) -> Option<i128> {
    match op {
        Operand::Const(IrConst::I8(v)) => Some(*v as i128),
        Operand::Const(IrConst::I16(v)) => Some(*v as i128),
        Operand::Const(IrConst::I32(v)) => Some(*v as i128),
        Operand::Const(IrConst::I64(v)) => Some(*v as i128),
        Operand::Const(IrConst::I128(v)) => Some(*v),
        Operand::Const(IrConst::Zero) => Some(0),
        _ => None,
    }
}

fn cond_load_operand_expr(op: &Operand, defs: &CondLoadDefs<'_>, depth: usize) -> CondAddrExpr {
    match op {
        Operand::Value(v) => cond_load_expr(*v, defs, depth + 1),
        Operand::Const(c) => match cond_load_const_offset(op) {
            Some(n) => CondAddrExpr::constant(n),
            None => CondAddrExpr::atom(format!("const({c:?})")),
        },
    }
}

/// Casts that preserve every address bit.  Same-width integer/integer and
/// integer/pointer reinterpretations are no-ops in every backend; narrowing,
/// widening, float conversion and signedness-changing casts of DIFFERENT
/// widths are not, so those become opaque atoms.
fn cond_load_cast_preserves_address(from_ty: IrType, to_ty: IrType) -> bool {
    let integerish = |t: IrType| t.is_integer() || t == IrType::Ptr;
    integerish(from_ty) && integerish(to_ty) && from_ty.size() == to_ty.size()
}

fn cond_load_expr(v: Value, defs: &CondLoadDefs<'_>, depth: usize) -> CondAddrExpr {
    if depth > 8 {
        return CondAddrExpr::opaque(v);
    }
    let Some(one) = defs.get(&v.0) else {
        return CondAddrExpr::opaque(v);
    };
    if one.len() != 1 {
        return CondAddrExpr::opaque(v);
    }
    match one[0] {
        Instruction::GetElementPtr { base, offset, .. } => cond_load_expr(*base, defs, depth + 1)
            .add(cond_load_operand_expr(offset, defs, depth + 1), v),
        Instruction::BinOp {
            op: IrBinOp::Add,
            lhs,
            rhs,
            ..
        } => cond_load_operand_expr(lhs, defs, depth + 1)
            .add(cond_load_operand_expr(rhs, defs, depth + 1), v),
        Instruction::BinOp {
            op: IrBinOp::Sub,
            lhs,
            rhs,
            ty,
            ..
        } => {
            let l = cond_load_operand_expr(lhs, defs, depth + 1);
            let r = cond_load_operand_expr(rhs, defs, depth + 1);
            if r.atoms.is_empty() {
                // `base - const` normalises to `base + (-const)`.
                // `wrapping_neg` handles i128::MIN without a debug panic.
                l.add(CondAddrExpr::constant(r.offset.wrapping_neg()), v)
            } else {
                CondAddrExpr::atom(format!("sub:{ty:?}({},{})", l.key(), r.key()))
            }
        }
        Instruction::BinOp {
            op, lhs, rhs, ty, ..
        } => {
            let mut keys = [
                cond_load_operand_expr(lhs, defs, depth + 1).key(),
                cond_load_operand_expr(rhs, defs, depth + 1).key(),
            ];
            if op.is_commutative() {
                keys.sort_unstable();
            }
            CondAddrExpr::atom(format!("{op:?}:{ty:?}({},{})", keys[0], keys[1]))
        }
        Instruction::UnaryOp { op, src, ty, .. } => CondAddrExpr::atom(format!(
            "{op:?}:{ty:?}({})",
            cond_load_operand_expr(src, defs, depth + 1).key()
        )),
        Instruction::Cmp {
            op, lhs, rhs, ty, ..
        } => CondAddrExpr::atom(format!(
            "{op:?}:{ty:?}({},{})",
            cond_load_operand_expr(lhs, defs, depth + 1).key(),
            cond_load_operand_expr(rhs, defs, depth + 1).key()
        )),
        Instruction::Select {
            cond,
            true_val,
            false_val,
            ty,
            ..
        } => CondAddrExpr::atom(format!(
            "select:{ty:?}({},{},{})",
            cond_load_operand_expr(cond, defs, depth + 1).key(),
            cond_load_operand_expr(true_val, defs, depth + 1).key(),
            cond_load_operand_expr(false_val, defs, depth + 1).key()
        )),
        Instruction::Cast {
            src,
            from_ty,
            to_ty,
            ..
        } if cond_load_cast_preserves_address(*from_ty, *to_ty) => {
            cond_load_operand_expr(src, defs, depth + 1)
        }
        Instruction::Cast {
            src,
            from_ty,
            to_ty,
            ..
        } => CondAddrExpr::atom(format!(
            "cast:{from_ty:?}->{to_ty:?}({})",
            cond_load_operand_expr(src, defs, depth + 1).key()
        )),
        Instruction::Copy { src, .. } => cond_load_operand_expr(src, defs, depth + 1),
        Instruction::GlobalAddr { name, .. } => CondAddrExpr::atom(format!("global({name})")),
        Instruction::ParamRef { param_idx, ty, .. } => {
            CondAddrExpr::atom(format!("param({param_idx}:{ty:?})"))
        }
        _ => CondAddrExpr::opaque(v),
    }
}

fn cond_load_ptr_key(v: Value, defs: &CondLoadDefs<'_>, depth: usize) -> String {
    cond_load_expr(v, defs, depth).key()
}

/// Profitability proof for the `a[i] < b[i] ? a[i] : b[i]` family.
///
/// When the branch predicate is data-dependent on a load from either arm's
/// address, BOTH addresses have already been dereferenced unconditionally by
/// the guard.  There is therefore no untaken-path fault to avoid and no second
/// load to eliminate: the existing `Cmp` + value-`Select` form costs two loads
/// and vectorizes to `pminud`/`pmaxud`/`pminsd`, whereas sinking would emit a
/// pointer `Select` plus a THIRD load through it and break the SLP min/max
/// idiom.  Refusing is always sound (it only declines a rewrite); the walk is
/// over the guard's transitive def chain, so a guard loaded from an unrelated
/// address (`if (*flag) x = *p; else x = *q;`) still sinks.
fn cond_load_guard_reads_arm_address(
    cond: Operand,
    t_key: &str,
    f_key: &str,
    defs: &CondLoadDefs<'_>,
) -> bool {
    let Operand::Value(root) = cond else {
        return false;
    };
    let mut seen: FxHashSet<u32> = FxHashSet::default();
    let mut work = vec![root.0];
    while let Some(id) = work.pop() {
        if !seen.insert(id) {
            continue;
        }
        // Conventional SSA permits several `Copy` definitions of one phi
        // home; keeping ALL of them prevents an earlier alias from hiding the
        // guard's load.
        let Some(defs_for_id) = defs.get(&id) else {
            continue;
        };
        for &inst in defs_for_id.iter() {
            if let Instruction::Load { ptr, .. } = inst {
                let key = cond_load_ptr_key(*ptr, defs, 0);
                if key == t_key || key == f_key {
                    return true;
                }
            }
            inst.for_each_used_value(|u| work.push(u));
        }
    }
    false
}

/// Collect every sound `cond ? *p : *q` diamond in the function.  Nothing is
/// mutated here; the caller applies non-overlapping plans.
fn plan_cond_load_sinks(func: &IrFunction) -> Vec<CondLoadPlan> {
    let num_blocks = func.blocks.len();
    if num_blocks < 4 {
        return Vec::new();
    }
    let label_to_idx = analysis::build_label_map(func);
    let (preds, succs) = analysis::build_cfg(func, &label_to_idx);
    let idom = analysis::compute_dominators(num_blocks, &preds, &succs);
    let dom = analysis::DominanceChecker::new(num_blocks, &idom);
    // Where every SSA value is defined.  A value with no entry is treated as
    // unavailable (fail closed) by the availability proof below.
    let mut def_block: FxHashMap<u32, usize> = FxHashMap::default();
    // Every definition instruction for each SSA id, used by the
    // guard-dependence profitability proof below.  Conventional SSA permits
    // several `Copy` definitions of one phi home, so keep ALL of them: taking
    // only the first could hide the guard's load behind an earlier alias.
    let mut def_inst: CondLoadDefs<'_> = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            if let Some(d) = inst.dest() {
                def_block.insert(d.0, bi);
                def_inst.entry(d.0).or_default().push(inst);
            }
        }
    }

    // One global pass instead of an O(n^2) rescan per candidate.
    let mut use_counts: FxHashMap<u32, usize> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            inst.for_each_used_value(|v| *use_counts.entry(v).or_insert(0) += 1);
        }
        block
            .terminator
            .for_each_used_value(|v| *use_counts.entry(v).or_insert(0) += 1);
    }

    let mut plans = Vec::new();
    for p in 0..num_blocks {
        let (cond, tl, fl) = match &func.blocks[p].terminator {
            Terminator::CondBranch {
                cond,
                true_label,
                false_label,
            } if true_label != false_label => (*cond, *true_label, *false_label),
            _ => continue,
        };
        let (Some(&t), Some(&f)) = (label_to_idx.get(&tl), label_to_idx.get(&fl)) else {
            continue;
        };
        if t == p || f == p || t == f {
            continue;
        }
        // Single-predecessor arms: the branch pred owns them exclusively.
        if preds.len(t) != 1 || preds.row(t)[0] as usize != p {
            continue;
        }
        if preds.len(f) != 1 || preds.row(f)[0] as usize != p {
            continue;
        }
        let (Terminator::Branch(t_merge), Terminator::Branch(f_merge)) =
            (&func.blocks[t].terminator, &func.blocks[f].terminator)
        else {
            continue;
        };
        if t_merge != f_merge {
            continue;
        }
        let Some(&m) = label_to_idx.get(t_merge) else {
            continue;
        };
        if m == p || m == t || m == f {
            continue;
        }
        // FULL merge only: after the rewrite the pred is its sole predecessor.
        if preds.len(m) != 2 {
            continue;
        }
        let mp: FxHashSet<u32> = preds.row(m).iter().copied().collect();
        if !mp.contains(&(t as u32)) || !mp.contains(&(f as u32)) {
            continue;
        }

        let Some(t_arm) = parse_cond_load_arm(&func.blocks[t]) else {
            continue;
        };
        let Some(f_arm) = parse_cond_load_arm(&func.blocks[f]) else {
            continue;
        };
        // The single merged load must have one unambiguous type/segment.
        if t_arm.ty != f_arm.ty || t_arm.seg != f_arm.seg {
            continue;
        }
        if !cond_load_type_supported(t_arm.ty) {
            continue;
        }
        // Do not steal a min/max/select idiom whose guard already loads the
        // two arm addresses: sinking would add a third load and destroy the
        // packed pminud/pmaxud/pminsd form (hosted CI shards, 2026-11-08).
        let t_key = cond_load_ptr_key(t_arm.load_ptr, &def_inst, 0);
        let f_key = cond_load_ptr_key(f_arm.load_ptr, &def_inst, 0);
        if cond_load_guard_reads_arm_address(cond, &t_key, &f_key, &def_inst) {
            continue;
        }
        if !cond_load_arm_available(
            &func.blocks[t].instructions,
            &t_arm.defined,
            &f_arm.defined,
            &def_block,
            &dom,
            p,
        ) || !cond_load_arm_available(
            &func.blocks[f].instructions,
            &f_arm.defined,
            &t_arm.defined,
            &def_block,
            &dom,
            p,
        ) {
            continue;
        }
        // Each load result and each promotion-tail id must be used ONLY by
        // the next chain instruction (or, for the final tail result, by the
        // merge Phi).  Otherwise emptying the arms leaves a dangling SSA use.
        let tail_single_use = |arm: &CondLoadArm| {
            arm.tail
                .iter()
                .all(|cast| use_counts.get(&cast.dest.0).copied().unwrap_or(0) == 1)
        };
        if use_counts.get(&t_arm.load_dest.0).copied().unwrap_or(0) != 1
            || use_counts.get(&f_arm.load_dest.0).copied().unwrap_or(0) != 1
            || !tail_single_use(&t_arm)
            || !tail_single_use(&f_arm)
        {
            continue;
        }
        let t_label = func.blocks[t].label;
        let f_label = func.blocks[f].label;
        // EVERY Phi in the merge must be accounted for.  Once the arms are
        // gone M has one predecessor, so a surviving Phi would name edges
        // that no longer exist (invalid SSA, and a backend ICE in the worst
        // case).  The load-result Phi becomes a Load; all others become
        // Selects in the pred with their original destinations.
        let mut phi = None;
        let mut extra_phis: Vec<CondLoadPhi> = Vec::new();
        let mut bad_phi = false;
        for (idx, inst) in func.blocks[m].instructions.iter().enumerate() {
            let Instruction::Phi { dest, ty, incoming } = inst else {
                continue;
            };
            if incoming.len() != 2 || !cond_load_type_supported(*ty) {
                bad_phi = true;
                break;
            }
            let (mut tv, mut fv, mut extra) = (None, None, false);
            for (op, lbl) in incoming {
                if *lbl == t_label {
                    tv = Some(*op);
                } else if *lbl == f_label {
                    fv = Some(*op);
                } else {
                    extra = true;
                }
            }
            if extra {
                bad_phi = true;
                break;
            }
            let (Some(tv), Some(fv)) = (tv, fv) else {
                bad_phi = true;
                break;
            };
            let is_target = matches!(
                (tv, fv),
                (Operand::Value(a), Operand::Value(b))
                    if a.0 == t_arm.tail_result.0 && b.0 == f_arm.tail_result.0
            );
            if is_target {
                if phi.is_some() || *ty != t_arm.result_ty {
                    bad_phi = true;
                    break;
                }
                phi = Some((idx, *dest));
                continue;
            }
            // A different Phi becomes a Select: it needs a Select-able type,
            // must not consume either doomed load result, and both of its
            // incomings must be available at the pred.
            // Arm-local pure chains ARE hoisted into the pred, but the
            // promotion tail is NOT: it is re-emitted in the merge.  A Select
            // consuming any of those ids would therefore be invalid.
            let doomed: FxHashSet<u32> = [t_arm.load_dest.0, f_arm.load_dest.0]
                .into_iter()
                .chain(t_arm.tail.iter().map(|cast| cast.dest.0))
                .chain(f_arm.tail.iter().map(|cast| cast.dest.0))
                .collect();
            let mut incoming_ok = cond_load_phi_selectable(*ty);
            for op in [tv, fv] {
                if let Operand::Value(v) = op {
                    if doomed.contains(&v.0) {
                        incoming_ok = false;
                    } else if !t_arm.defined.contains(&v.0) && !f_arm.defined.contains(&v.0) {
                        incoming_ok &= def_block.get(&v.0).is_some_and(|&b| dom.dominates(b, p));
                    }
                }
            }
            if !incoming_ok {
                bad_phi = true;
                break;
            }
            extra_phis.push(CondLoadPhi {
                idx,
                dest: *dest,
                ty: *ty,
                tv,
                fv,
            });
        }
        if bad_phi {
            continue;
        }
        let Some((phi_idx, phi_dest)) = phi else {
            continue;
        };
        // Both arms must present the SAME promotion chain.  Different chains
        // cannot share one sunk load; declining is always sound.
        if t_arm.tail.len() != f_arm.tail.len()
            || !t_arm
                .tail
                .iter()
                .zip(&f_arm.tail)
                .all(|(a, b)| a.from_ty == b.from_ty && a.to_ty == b.to_ty)
        {
            continue;
        }
        let tail = t_arm.tail.clone();
        plans.push(CondLoadPlan {
            pred: p,
            t,
            f,
            merge: m,
            merge_label: func.blocks[m].label,
            cond,
            t_arm,
            f_arm,
            tail,
            phi_idx,
            phi_dest,
            extra_phis,
        });
    }
    plans
}

fn apply_cond_load_sink(func: &mut IrFunction, p: &CondLoadPlan) -> bool {
    if p.pred >= func.blocks.len()
        || p.t >= func.blocks.len()
        || p.f >= func.blocks.len()
        || p.merge >= func.blocks.len()
        || p.phi_idx >= func.blocks[p.merge].instructions.len()
    {
        return false;
    }
    let fresh = Value(func.sound_next_value_id());
    let fresh_load = Value(fresh.0.saturating_add(1));
    func.next_value_id = fresh_load.0.saturating_add(1);
    // With a promotion tail, the sunk Load must define a NEW id (the pointer
    // Select already owns `fresh`) and the tail must end at the ORIGINAL phi
    // destination, preserving every downstream use without rewriting them.
    let (load_dest, tail_dest) = if p.tail.is_empty() {
        (p.phi_dest, p.phi_dest)
    } else {
        (fresh_load, p.phi_dest)
    };
    let dummy = crate::common::source::Span::dummy();
    {
        let pred = &mut func.blocks[p.pred];
        // Hoist the pure address chains with their ORIGINAL SSA ids, so every
        // surviving use (including the merge store) stays valid.
        for inst in &p.t_arm.pure_insts {
            pred.push_inst(inst.clone(), dummy);
        }
        for inst in &p.f_arm.pure_insts {
            pred.push_inst(inst.clone(), dummy);
        }
        // Any OTHER merge Phi becomes a Select with the SAME destination;
        // the Phi itself is removed below so there is exactly one definition.
        for x in &p.extra_phis {
            pred.push_inst(
                Instruction::Select {
                    dest: x.dest,
                    cond: p.cond,
                    true_val: x.tv,
                    false_val: x.fv,
                    ty: x.ty,
                },
                dummy,
            );
        }
        pred.push_inst(
            Instruction::Select {
                dest: fresh,
                cond: p.cond,
                true_val: Operand::Value(p.t_arm.load_ptr),
                false_val: Operand::Value(p.f_arm.load_ptr),
                ty: IrType::Ptr,
            },
            dummy,
        );
        pred.terminator = Terminator::Branch(p.merge_label);
    }
    // Replace the load-result Phi IN PLACE: the destination (and therefore
    // every use of it) is unchanged, and the parallel span/provenance arrays
    // stay aligned because the instruction count does not change.
    func.blocks[p.merge].instructions[p.phi_idx] = Instruction::Load {
        dest: load_dest,
        ptr: fresh,
        ty: p.t_arm.ty,
        seg_override: p.t_arm.seg,
        volatile: false,
    };
    if !p.tail.is_empty() {
        let mut prev = load_dest;
        let mut rewritten_tail = Vec::with_capacity(p.tail.len());
        for (k, cast) in p.tail.iter().enumerate() {
            let dest = if k + 1 == p.tail.len() {
                tail_dest
            } else {
                cast.dest
            };
            rewritten_tail.push(Instruction::Cast {
                dest,
                src: Operand::Value(prev),
                from_ty: cast.from_ty,
                to_ty: cast.to_ty,
            });
            prev = dest;
        }
        // Insert DIRECTLY after the replaced Phi, before the original Store.
        // `insert_insts` keeps spans/inline ids in lockstep; appending would
        // place the casts after a use of the phi destination (invalid SSA).
        func.blocks[p.merge].insert_insts(p.phi_idx + 1, rewritten_tail, dummy);
    }
    // Remove the other phis (their values now come from the pred's Selects).
    // retain_insts keeps spans/inline ids in lockstep.
    if !p.extra_phis.is_empty() {
        let gone: FxHashSet<u32> = p.extra_phis.iter().map(|x| x.dest.0).collect();
        func.blocks[p.merge].retain_insts(
            |inst| !matches!(inst, Instruction::Phi { dest, .. } if gone.contains(&dest.0)),
        );
    }
    // The arms are unreachable now.  Clear them so no second definition of a
    // hoisted destination (or a dangling load result) survives for the SSA
    // verifier; CFG simplification deletes the empty blocks afterwards.
    func.blocks[p.t].clear_insts();
    func.blocks[p.f].clear_insts();
    true
}

fn sink_conditional_loads(func: &mut IrFunction) -> usize {
    if !super::cond_load_sink_enabled() {
        return 0;
    }
    let plans = plan_cond_load_sinks(func);
    if plans.is_empty() {
        return 0;
    }
    let mut modified: FxHashSet<usize> = FxHashSet::default();
    let mut converted = 0usize;
    for plan in &plans {
        if [plan.pred, plan.t, plan.f, plan.merge]
            .iter()
            .any(|b| modified.contains(b))
        {
            continue;
        }
        if apply_cond_load_sink(func, plan) {
            modified.extend([plan.pred, plan.t, plan.f, plan.merge]);
            converted += 1;
        }
    }
    converted
}

fn if_convert_once(func: &mut IrFunction) -> usize {
    let num_blocks = func.blocks.len();
    if num_blocks < 3 {
        return 0;
    }

    // Build CFG
    let label_to_idx = analysis::build_label_map(func);
    let (preds, succs) = analysis::build_cfg(func, &label_to_idx);

    // Loop nesting for the shared-arm budget: the same dominator/loop run the
    // detector context performs, but on the PRE-duplication CFG the plan is
    // derived from.  `block_depth` is 0 for every block outside all loops.
    let dup_loop_depth = {
        let idom = analysis::compute_dominators(num_blocks, &preds, &succs);
        let nested =
            crate::passes::loop_analysis::find_natural_loops(num_blocks, &preds, &succs, &idom);
        crate::passes::loop_analysis::LoopNest::analyze(num_blocks, &nested).block_depth
    };

    // Eliminate arm-block reloads of addresses the branch pred already loads;
    // this both removes redundant memory traffic and unblocks diamond/triangle
    // detection below (loads make arms fail the side-effect-free check).
    let rewrites = rewrite_covered_arm_loads(func, &label_to_idx, &preds);

    // Shared-arm tail duplication (see `plan_shared_arm_dup`): restructure
    // one nested conditional at a time so the diamond detectors below can
    // convert it inside-out on this and the following fixpoint iterations.
    // Planned on the post-rewrite CFG (label_to_idx/preds are still current
    // — the covered-load rewrites are instruction-level); the detector
    // context is built AFTER the duplication so it sees the fresh CFG.
    let dup = match plan_shared_arm_dup(func, &label_to_idx, &preds, &dup_loop_depth) {
        Some(plan) => usize::from(apply_shared_arm_dup(func, &plan)),
        None => 0,
    };
    // Detector context: natural loops (for the speculative arm-load gate) and
    // the def/copy maps.  Built AFTER the covered-load rewrites so the SSA
    // defs are the ones the detectors will actually see.
    let ctx = IfConvCtx::build(func);

    // Fold short-circuit branch chains first: each fold turns a two-level
    // chain into ONE branch, which is what exposes the diamond/triangle the
    // detectors below can convert.  Without this the second level survives
    // as an internal conditional branch and blocks vectorization of the
    // whole loop.
    // If-combine is restricted to NATURAL-LOOP BODIES, deliberately.
    //
    // Folding `a && b` into `and(a, b)` converts a control dependence into a
    // data dependence.  That trade pays where the resulting straight-line
    // block can be vectorized or where the branch is unpredictable -- i.e.
    // inside loops.  Outside them it is a net loss, because it destroys the
    // path-sensitivity that correlated-value propagation and range folding
    // rely on: on the false edge of `var <= 0` a signed `var` is known
    // positive, which is what proves `(unsigned)(var - 1) < UINT_MAX` and
    // deletes an unreachable `link_failure()` call
    // (`tests/regression/path_range_var_minus_one_uintmax.c`, reduced from
    // gcc.c-torture/execute/20041114-1.c).  Merging the two conditions into
    // one branch erases the edge that carried the fact, and the call
    // survives to link time.
    //
    // GCC resolves the same conflict by ordering (`ifcombine` runs after
    // VRP); scoping to loop bodies is the equivalent guarantee here and is
    // independent of how many times the pass pipeline re-enters this pass.
    let loop_cfg = analysis::CfgAnalysis::build(func);
    let natural_loops = if std::env::var("CCC_NO_IFCOMBINE").is_ok() {
        Vec::new()
    } else {
        crate::passes::loop_analysis::find_natural_loops(
            num_blocks,
            &loop_cfg.preds,
            &loop_cfg.succs,
            &loop_cfg.idom,
        )
    };
    // Per LOOP, not per candidate: whether a combine pays off is a property
    // of the whole loop (does its body become branch-free?), so candidates
    // are grouped by loop and accepted or rejected together.
    let mut per_loop: Vec<(usize, Vec<IfCombine>)> = Vec::new();
    for (li, nl) in natural_loops.iter().enumerate() {
        let mut seen: crate::common::fx_hash::FxHashSet<usize> =
            crate::common::fx_hash::FxHashSet::default();
        let mut loop_cands: Vec<IfCombine> = Vec::new();
        // Every body block is a candidate PRED, the header included: in the
        // canonical rotated shape the header is exactly where the value
        // computation (and therefore the head of the `&&`/`||` chain) lives.
        // Excluding it rejected the ASCII classifier outright.
        let mut ordered: Vec<usize> = nl.body.iter().copied().collect();
        ordered.sort_unstable();
        for b in ordered {
            if let Some(c) = detect_if_combine(&ctx, b) {
                // Overlapping candidates would invalidate each other; keep
                // the first for each pred/inner pair.
                if seen.insert(c.pred_idx) && seen.insert(c.inner_idx) {
                    loop_cands.push(c);
                }
            }
        }
        if std::env::var("CCC_DEBUG_IFCOMBINE").is_ok() {
            lccc::cli::stderr_line(format_args!(
                "[IFCOMB] {} loop@{} body={:?}: {} candidate(s)",
                func.name,
                nl.header,
                {
                    let mut v: Vec<usize> = nl.body.iter().copied().collect();
                    v.sort_unstable();
                    v
                },
                loop_cands.len()
            ));
        }
        if !loop_cands.is_empty() {
            per_loop.push((li, loop_cands));
        }
    }
    if !per_loop.is_empty() {
        // `ctx` borrows `func`; drop it before mutating.
        drop(ctx);
        let mut combines = 0usize;
        for (li, loop_cands) in &per_loop {
            combines += try_if_combine_loop(func, &loop_cfg.succs, &natural_loops[*li], loop_cands);
        }
        if combines > 0 {
            // The CFG changed; let the caller's fixpoint loop rebuild it
            // before looking for diamonds.
            return rewrites + dup + combines;
        }
        // Everything was rejected or stale: fall through to the diamonds.
        return rewrites + dup + if_convert_diamonds(func);
    }
    drop(ctx);
    rewrites + if_convert_diamonds(func)
}

/// Diamond/triangle conversion proper, split out so the if-combine phase can
/// rebuild the analysis context before calling it.
fn if_convert_diamonds(func: &mut IrFunction) -> usize {
    let num_blocks = func.blocks.len();
    let ctx = IfConvCtx::build(func);

    // Collect diamond candidates
    let mut diamonds: Vec<DiamondInfo> = Vec::new();

    for pred_idx in 0..num_blocks {
        if let Some(diamond) = detect_diamond(&ctx, pred_idx) {
            diamonds.push(diamond);
        } else if let Some(triangle) = detect_triangle(&ctx, pred_idx) {
            diamonds.push(triangle);
        }
    }

    if diamonds.is_empty() {
        return 0;
    }
    drop(ctx);

    // Apply conversions. Track modified blocks to avoid applying overlapping diamonds
    // (e.g., nested ternaries where converting one invalidates another).
    //
    // The touched set is the WHOLE footprint: the pred, the merge, and every
    // block of both arm regions — an arm chain reaches into blocks a
    // single-block view never named, and two planned diamonds can share one.
    let footprint = |d: &DiamondInfo| -> Vec<usize> {
        let mut v = Vec::with_capacity(4 + d.true_arm_blocks.len() + d.false_arm_blocks.len());
        v.push(d.pred_idx);
        v.push(d.merge_idx);
        v.extend(d.true_arm_blocks.iter().copied());
        v.extend(d.false_arm_blocks.iter().copied());
        v
    };
    let mut converted = 0;
    let mut modified_blocks: crate::common::fx_hash::FxHashSet<usize> =
        crate::common::fx_hash::FxHashSet::default();
    for diamond in &diamonds {
        // Skip if any of the diamond's blocks were already modified
        let touched = footprint(diamond);
        if touched.iter().any(|b| modified_blocks.contains(b)) {
            continue;
        }
        if apply_diamond(func, diamond) {
            modified_blocks.extend(touched);
            converted += 1;
        }
    }

    // Clean up: run a quick dead block pass to remove the now-empty arm blocks
    // (they'll have no instructions and just an unconditional branch)
    // This is handled by the CFG simplification pass that runs after us.

    converted
}

/// A planned shared-arm duplication: the outer head H's edge to the shared
/// arm S is redirected to a fresh clone of S, leaving S the sole arm of the
/// inner head H2.
struct SharedArmDup {
    /// Outer CondBranch head (its S-edge is redirected).
    h_idx: usize,
    /// True when H's TRUE edge targets S.
    h_edge_is_true: bool,
    /// The shared arm (preds {H, H2} before the transform).
    s_idx: usize,
}

/// Plan the tail duplication of a SHARED ARM of a nested conditional.
///
/// The `c1 && c2 ? T : S` / `c1 || c2 ? S : T` statement forms lower to a
/// CFG where the inner diamond's arm S has TWO predecessors (the outer head
/// H and the inner head H2):
///
/// ```text
///     H : CondBranch c1 -> H2 / S      (one edge to S, one to H2)
///     H2: CondBranch c2 -> T / S       (S shared)
///     S : ...pure...; Branch M         (preds {H, H2})
///     T : ...pure...; Branch M         (preds {H2})
///     M : phi [(vT, T), (vS, S), ...]
/// ```
///
/// `detect_diamond` requires single-predecessor arms, so neither level
/// converts and the whole shape stays branchy — this is exactly the shape
/// of the `if (c >= 'a' && c <= 'z') s[i] = c - 32;` byte-casefold family
/// (and its dword/word siblings), where GCC if-converts and then
/// auto-vectorizes.
///
/// The transform redirects H's S-edge to a fresh clone S' of S (identical
/// pure instructions, fresh SSA dests, operands unchanged) and extends M's
/// phis with an S'-incoming carrying the same (clone-mapped) value:
///
/// * SSA dominance guarantees every operand of S's instructions is defined
///   on the H->S path outside {H2, S} (H2 has not executed there), so the
///   clone's operands are valid on its H->S' edge without any rewriting.
/// * Values internal to S are remapped to the clone's dests; the phi
///   S-incoming is replicated for S' with that mapping.
/// * Per executed path the transform is neutral: S' runs exactly where S
///   would have run.  It costs |S| duplicated instructions (bounded by the
///   same arm budget the conversions use) and ENABLES the existing
///   machinery: the inner diamond converts partially (3-pred merge), then
///   the outer diamond converts fully, yielding the nested Select
///   `Select(c1, Select(c2, vT, vS), vS)` — which the map vectorizer's
///   MaskConj recovery fuses into one compare-conjunction blend.
///
/// One plan per pass invocation: the fixpoint in `if_convert_function`
/// re-enters for deeper nests (a && b && c) and for the conversions
/// themselves.  Fail-closed everywhere — any shape the diamond detectors
/// would refuse (impure arms, budget overruns, unconvertible phis,
/// boolean-diamond profitability guards, constant conditions) is not
/// duplicated.
fn plan_shared_arm_dup(
    func: &IrFunction,
    label_to_idx: &crate::common::fx_hash::FxHashMap<BlockId, usize>,
    preds: &analysis::FlatAdj,
    loop_depth: &[usize],
) -> Option<SharedArmDup> {
    for h_idx in 0..func.blocks.len() {
        let (c1, t1, f1) = match &func.blocks[h_idx].terminator {
            Terminator::CondBranch {
                cond,
                true_label,
                false_label,
            } => (cond, *true_label, *false_label),
            _ => continue,
        };
        if is_constant_condition(&func.blocks[h_idx], c1) {
            continue;
        }
        // H's two targets are the shared arm S and the inner head H2 (both
        // orientations: `&&` puts H2 on the true edge, `||` on the false).
        for (s_label, h2_label) in [(t1, f1), (f1, t1)] {
            let Some(&s_idx) = label_to_idx.get(&s_label) else {
                continue;
            };
            let Some(&h2_idx) = label_to_idx.get(&h2_label) else {
                continue;
            };
            if s_idx == h2_idx || s_idx == h_idx || h2_idx == h_idx {
                continue;
            }
            // H2 is the inner conditional head.  Its targets are T and the
            // shared arm S (either orientation).
            let (c2, t2, f2) = match &func.blocks[h2_idx].terminator {
                Terminator::CondBranch {
                    cond,
                    true_label,
                    false_label,
                } => (cond, *true_label, *false_label),
                _ => continue,
            };
            if is_constant_condition(&func.blocks[h2_idx], c2) || t2 == f2 {
                continue;
            }
            let t_label = if t2 == s_label {
                f2
            } else if f2 == s_label {
                t2
            } else {
                continue;
            };
            let Some(&t_idx) = label_to_idx.get(&t_label) else {
                continue;
            };
            if t_idx == s_idx || t_idx == h_idx || t_idx == h2_idx {
                continue;
            }
            // Structural shape: S has exactly the two heads as preds; T and
            // H2 have exactly one; both arms merge at M.
            if preds.len(s_idx) != 2 || preds.len(t_idx) != 1 || preds.len(h2_idx) != 1 {
                continue;
            }
            let s_pred_set = {
                let mut v = preds.row(s_idx).to_vec();
                v.sort_unstable();
                let mut expected = vec![h_idx as u32, h2_idx as u32];
                expected.sort_unstable();
                v == expected
            };
            if !s_pred_set
                || preds.row(t_idx)[0] as usize != h2_idx
                || preds.row(h2_idx)[0] as usize != h_idx
            {
                continue;
            }
            let s_block = &func.blocks[s_idx];
            let t_block = &func.blocks[t_idx];
            let m_idx = match (&s_block.terminator, &t_block.terminator) {
                (Terminator::Branch(a), Terminator::Branch(b)) if a == b => *label_to_idx.get(a)?,
                _ => continue,
            };
            if m_idx == h_idx || m_idx == h2_idx || m_idx == s_idx || m_idx == t_idx {
                continue;
            }
            // Purity/budget, mirroring the diamond detector's gates for
            // the INNER diamond (S and T are its arms).  The strict
            // load-free purity gate is deliberate: a load in a DUPLICATED
            // arm doubles the speculative-load analysis surface for a
            // value-computation shape that almost never has one — fail
            // closed and keep such nests branchy (the detector's own
            // load-speculation gate still applies to non-shared diamonds).
            if !is_side_effect_free(s_block) || !is_side_effect_free(t_block) {
                continue;
            }
            // The INNER diamond (arms S and T, head H2) is the one whose
            // arms are speculative, so its head owns the budget decision.
            let max_arm_insts = loop_arm_budget(loop_depth.get(h2_idx).copied().unwrap_or(0));
            if effective_arm_len(s_block) > max_arm_insts
                || effective_arm_len(t_block) > max_arm_insts
            {
                continue;
            }
            // The inner conversion must have a convertible phi (both arms'
            // incomings, no F128/I128 stragglers), and must not be one of
            // the short-circuit boolean shapes the profitability guard
            // keeps branchy.  Without these the duplication would be dead
            // weight.
            let merge_block = &func.blocks[m_idx];
            let mut phi_selects: Vec<(Value, IrType, Operand, Operand)> = Vec::new();
            for inst in &merge_block.instructions {
                if let Instruction::Phi {
                    dest, ty, incoming, ..
                } = inst
                {
                    let mut tv = None;
                    let mut fv = None;
                    for (op, label) in incoming {
                        if *label == t_block.label {
                            tv = Some(*op);
                        } else if *label == s_block.label {
                            fv = Some(*op);
                        }
                    }
                    match (tv, fv) {
                        (Some(tv), Some(fv)) => {
                            if ty.is_long_double() || ty.is_128bit() {
                                // Mirror detect_diamond: a straggler phi
                                // blocks the whole conversion.
                                phi_selects.clear();
                                break;
                            }
                            phi_selects.push((*dest, *ty, tv, fv));
                        }
                        _ => {}
                    }
                }
            }
            if phi_selects.is_empty() || skip_boolean_diamond(func, &phi_selects) {
                continue;
            }
            // Phi completeness: every phi in M must carry an S-incoming (S
            // is a predecessor); the clone's edge needs one per phi.
            let all_phis_have_s = merge_block.instructions.iter().all(|inst| match inst {
                Instruction::Phi { incoming, .. } => {
                    incoming.iter().any(|(_, label)| *label == s_block.label)
                }
                _ => true,
            });
            if !all_phis_have_s {
                continue;
            }
            return Some(SharedArmDup {
                h_idx,
                h_edge_is_true: t1 == s_label,
                s_idx,
            });
        }
    }
    None
}

/// Apply a planned shared-arm duplication (see `plan_shared_arm_dup`).
fn apply_shared_arm_dup(func: &mut IrFunction, plan: &SharedArmDup) -> bool {
    // Validate the complete transaction before allocating IDs or mutating IR.
    // Plans are private, but may be stale when this helper is reused.
    let (Some(s_block), Some(h_block)) = (func.blocks.get(plan.s_idx), func.blocks.get(plan.h_idx))
    else {
        return false;
    };
    let s_label = s_block.label;
    let Terminator::CondBranch {
        true_label,
        false_label,
        ..
    } = h_block.terminator
    else {
        return false;
    };
    let edge = if plan.h_edge_is_true {
        true_label
    } else {
        false_label
    };
    if edge != s_label || true_label == false_label || plan.h_idx == plan.s_idx {
        return false;
    }
    let Terminator::Branch(m_label) = s_block.terminator else {
        return false;
    };
    let Some(m_idx) = func.blocks.iter().position(|b| b.label == m_label) else {
        return false;
    };
    if m_idx == plan.h_idx
        || m_idx == plan.s_idx
        || func.blocks.iter().any(|b| b.label.0 == func.next_label)
    {
        return false;
    }
    let mut phi_updates = Vec::new();
    for (ii, inst) in func.blocks[m_idx].instructions.iter().enumerate() {
        if let Instruction::Phi { incoming, .. } = inst {
            let mut values = incoming.iter().filter(|(_, label)| *label == s_label);
            let Some(&(value, _)) = values.next() else {
                return false;
            };
            if values.next().is_some() {
                return false;
            }
            phi_updates.push((ii, value));
        }
    }
    let Some(next_label) = func.next_label.checked_add(1) else {
        return false;
    };
    let dest_count = s_block
        .instructions
        .iter()
        .filter(|inst| inst.dest().is_some())
        .count();
    let Some(next_value_id) = u32::try_from(dest_count)
        .ok()
        .and_then(|n| func.next_value_id.checked_add(n))
    else {
        return false;
    };
    let (s_insts, s_term, s_spans, s_ids) = (
        s_block.instructions.clone(),
        s_block.terminator.clone(),
        s_block.source_spans.clone(),
        s_block.source_inline_ids.clone(),
    );
    let mut clone_map: crate::common::fx_hash::FxHashMap<u32, u32> =
        crate::common::fx_hash::FxHashMap::default();
    let mut cloned_insts: Vec<Instruction> = Vec::with_capacity(s_insts.len());
    for inst in &s_insts {
        let mut cloned = inst.clone();
        if let Some(dest) = inst.dest() {
            let fresh = Value(func.next_value_id);
            func.next_value_id += 1;
            clone_map.insert(dest.0, fresh.0);
            set_instruction_dest(&mut cloned, fresh);
        }
        remap_shared_arm_operands(&mut cloned, &clone_map);
        cloned_insts.push(cloned);
    }

    let s_prime_label = BlockId(func.next_label);
    func.next_label = next_label;
    debug_assert_eq!(func.next_value_id, next_value_id);
    func.blocks.push(BasicBlock {
        label: s_prime_label,
        instructions: cloned_insts,
        terminator: s_term.clone(),
        source_spans: s_spans,
        source_inline_ids: s_ids,
    });

    // All phi inputs were validated before mutation, so applying the repair
    // has no failure path and cannot leave a partially transformed function.
    for (ii, value) in phi_updates {
        let mapped = match value {
            Operand::Value(val) => clone_map
                .get(&val.0)
                .map_or(value, |&nv| Operand::Value(Value(nv))),
            _ => value,
        };
        if let Instruction::Phi { incoming, .. } = &mut func.blocks[m_idx].instructions[ii] {
            incoming.push((mapped, s_prime_label));
        }
    }

    // Redirect H's S-edge to the clone.
    let term = &mut func.blocks[plan.h_idx].terminator;
    if let Terminator::CondBranch {
        true_label,
        false_label,
        ..
    } = term
    {
        if *true_label == s_label {
            *true_label = s_prime_label;
        } else if *false_label == s_label {
            *false_label = s_prime_label;
        }
    }
    true
}

/// Rewrite the mutable value references of one (cloned) instruction through
/// a dest map: `Operand::Value` positions, GEP bases and load pointers.
/// Covers exactly the instruction kinds `arm_is_speculatable` admits.
fn remap_shared_arm_operands(
    inst: &mut Instruction,
    map: &crate::common::fx_hash::FxHashMap<u32, u32>,
) {
    let map_val = |v: &mut Value| {
        if let Some(&nv) = map.get(&v.0) {
            *v = Value(nv);
        }
    };
    let map_op = |o: &mut Operand| {
        if let Operand::Value(v) = o {
            map_val(v);
        }
    };
    match inst {
        Instruction::BinOp { lhs, rhs, .. } => {
            map_op(lhs);
            map_op(rhs);
        }
        Instruction::UnaryOp { src, .. } => map_op(src),
        Instruction::Cmp { lhs, rhs, .. } => {
            map_op(lhs);
            map_op(rhs);
        }
        Instruction::Cast { src, .. } => map_op(src),
        Instruction::Copy { src, .. } => map_op(src),
        Instruction::GetElementPtr { base, offset, .. } => {
            map_val(base);
            map_op(offset);
        }
        Instruction::GlobalAddr { .. } => {}
        Instruction::Select {
            cond,
            true_val,
            false_val,
            ..
        } => {
            map_op(cond);
            map_op(true_val);
            map_op(false_val);
        }
        Instruction::Load { ptr, .. } => map_val(ptr),
        // The speculatable gate admits nothing else; leaving an exotic
        // instruction untouched would be a silent miscompile, so refuse.
        _ => {}
    }
}

/// Set the destination of a cloned instruction (dest positions only; the
/// map covers every kind `arm_is_speculatable` admits).
fn set_instruction_dest(inst: &mut Instruction, fresh: Value) {
    match inst {
        Instruction::BinOp { dest, .. }
        | Instruction::UnaryOp { dest, .. }
        | Instruction::Cmp { dest, .. }
        | Instruction::Cast { dest, .. }
        | Instruction::Copy { dest, .. }
        | Instruction::GetElementPtr { dest, .. }
        | Instruction::GlobalAddr { dest, .. }
        | Instruction::Select { dest, .. }
        | Instruction::Load { dest, .. } => *dest = fresh,
        _ => {}
    }
}

/// Information about a detected diamond pattern.
struct DiamondInfo {
    /// The block containing the CondBranch
    pred_idx: usize,
    /// The block the TRUE arm's edge into the merge leaves from — the last
    /// block of its region (the arm head when the arm is one block).  The
    /// merge's phi incomings name THIS block, so it is the one the phi
    /// matching and the merge-predecessor test must use.
    true_idx: usize,
    /// The FALSE arm's exit block (see `true_idx`).
    false_idx: usize,
    /// The merge block index (with the Phi)
    merge_idx: usize,
    /// The condition operand from the CondBranch
    cond: Operand,
    /// Instructions to hoist from the true arm (before the Select)
    true_arm_insts: Vec<Instruction>,
    /// Instructions to hoist from the false arm (before the Select)
    false_arm_insts: Vec<Instruction>,
    /// Every block an arm's hoisted instructions came from: the arm head
    /// first, then the single-predecessor chain behind it in execution
    /// order.  `apply_diamond` clears all of them — nothing branches into
    /// them any more, and the CFG simplification that runs after this pass
    /// deletes them.
    true_arm_blocks: Vec<usize>,
    false_arm_blocks: Vec<usize>,
    /// Phi nodes in the merge block that can be converted to Select.
    /// Each entry is (phi_dest, phi_ty, true_val, false_val).
    phi_selects: Vec<(Value, IrType, Operand, Operand)>,
    /// True when the merge block has exactly the two diamond edges as
    /// predecessors (the converted phis are removed entirely); false for a
    /// partial conversion (converted phis keep their other incoming edges,
    /// fed by a fresh Select destination).
    full_merge: bool,
}

/// Check if a block contains only simple, side-effect-free instructions
/// that are safe to speculatively execute.
///
/// IMPORTANT: Load is NOT included here because loads can trap (segfault)
/// on invalid pointers. Hoisting a load past a null-pointer guard would
/// cause a crash. For example, `if (!p || !p[0])` has a diamond where
/// one arm loads `*p` — if-converting this would execute the load
/// unconditionally, crashing when `p` is NULL.
fn is_side_effect_free(block: &BasicBlock) -> bool {
    for inst in &block.instructions {
        match inst {
            // Division and remainder can trap with SIGFPE on divide-by-zero.
            // They must not be speculatively executed past a guard condition.
            Instruction::BinOp { op, .. } if op.can_trap() => return false,
            Instruction::BinOp { .. }
            | Instruction::UnaryOp { .. }
            | Instruction::Cmp { .. }
            | Instruction::Cast { .. }
            | Instruction::Copy { .. }
            | Instruction::GetElementPtr { .. }
            | Instruction::GlobalAddr { .. }
            | Instruction::Select { .. } => {}
            // Load can trap on invalid pointers — not safe to speculate.
            // Store, Call, atomics, etc. have write side effects.
            _ => return false,
        }
    }
    true
}

/// Analysis context shared by the diamond/triangle detectors for one pass
/// over the function: the CFG bundle, and the SSA def/copy maps for address
/// canonicalization.
struct IfConvCtx<'a> {
    func: &'a IrFunction,
    label_to_idx: FxHashMap<BlockId, usize>,
    preds: analysis::FlatAdj,
    /// Loop nesting depth of every block (0 = outside every loop), from
    /// `loop_analysis::LoopNest`.  The if-conversion budget is a function of
    /// this: a diamond reached once per call and a diamond reached once per
    /// loop iteration pay the same speculation cost for wildly different
    /// frequencies (see [`arm_budget_for`]).
    loop_depth: Vec<usize>,
    /// Dest -> defining instruction (cloned; stable across the pass).
    defs: FxHashMap<u32, Instruction>,
    /// Copy chains dest -> src.
    copy_of: FxHashMap<Value, Value>,
}

impl<'a> IfConvCtx<'a> {
    fn build(func: &'a IrFunction) -> Self {
        let label_to_idx = analysis::build_label_map(func);
        let cfg = analysis::CfgAnalysis::build(func);
        let nested = crate::passes::loop_analysis::find_natural_loops(
            cfg.num_blocks,
            &cfg.preds,
            &cfg.succs,
            &cfg.idom,
        );
        let loop_depth =
            crate::passes::loop_analysis::LoopNest::analyze(cfg.num_blocks, &nested).block_depth;
        let defs = func
            .blocks
            .iter()
            .flat_map(|b| b.instructions.iter())
            .filter_map(|i| i.dest().map(|d| (d.0, i.clone())))
            .collect();
        let mut copy_of = FxHashMap::default();
        for b in &func.blocks {
            for i in &b.instructions {
                if let Instruction::Copy {
                    dest,
                    src: Operand::Value(src),
                } = i
                {
                    copy_of.insert(*dest, *src);
                }
            }
        }
        IfConvCtx {
            func,
            label_to_idx,
            preds: cfg.preds,
            loop_depth,
            defs,
            copy_of,
        }
    }

    /// Follow copy chains to their root value.
    fn resolve(&self, v: &Value) -> Value {
        let mut cur = *v;
        for _ in 0..64 {
            match self.copy_of.get(&cur) {
                Some(&next) if next != cur => cur = next,
                _ => break,
            }
        }
        cur
    }
}

/// Static profitability prior, not a fact about the program's input distribution.
/// Equality with one pattern of at least eight masked bits is usually predictable;
/// eagerly executing a large arm then costs more than keeping its branch. Never
/// use this estimate to eliminate an edge or relax a speculation-safety check.
fn rare_masked_outcome(ctx: &IfConvCtx<'_>, cond: &Operand, fuel: usize) -> Option<bool> {
    if fuel == 0 {
        return None;
    }
    let Operand::Value(v) = cond else { return None };
    let inst = ctx.defs.get(&ctx.resolve(v).0)?;
    let masked_bits = |value: &Operand| -> Option<u64> {
        let Operand::Value(v) = value else {
            return None;
        };
        let Instruction::BinOp {
            op: IrBinOp::And,
            lhs,
            rhs,
            ty,
            ..
        } = ctx.defs.get(&ctx.resolve(v).0)?
        else {
            return None;
        };
        if !ty.is_integer() || ty.size() > 8 {
            return None;
        }
        let c = match (lhs, rhs) {
            (Operand::Const(c), _) | (_, Operand::Const(c)) => c.to_i64()? as u64,
            _ => return None,
        };
        let bits = ty.size() * 8;
        let mask = c & (u64::MAX >> (64 - bits));
        (mask.count_ones() >= 8).then_some(mask)
    };
    match inst {
        Instruction::Cmp {
            op: op @ (IrCmpOp::Eq | IrCmpOp::Ne),
            lhs,
            rhs,
            ..
        } => {
            let (value, constant) = match (lhs, rhs) {
                (Operand::Const(c), value) | (value, Operand::Const(c)) => {
                    (value, c.to_i64()? as u64)
                }
                _ => return None,
            };
            if let Some(mask) = masked_bits(value) {
                // An impossible pattern is left to constant folding, not priced
                // as an ordinary rare branch.
                return (constant & !mask == 0).then_some(*op == IrCmpOp::Eq);
            }
            // Frontend truth normalization can wrap a boolean compare in
            // `!= 0` / `== 1`. Restrict recursion to 0/1-producing comparisons.
            let Operand::Value(v) = value else {
                return None;
            };
            if constant > 1
                || !matches!(
                    ctx.defs.get(&ctx.resolve(v).0),
                    Some(Instruction::Cmp { .. })
                )
            {
                return None;
            }
            let rare = rare_masked_outcome(ctx, value, fuel - 1)?;
            Some((rare == (constant != 0)) == (*op == IrCmpOp::Eq))
        }
        Instruction::BinOp {
            op: IrBinOp::And, ..
        } => masked_bits(cond).map(|_| false),
        _ => None,
    }
}

/// Effective-instruction budget for one arm of the diamond headed by
/// `pred_idx`.
///
/// Two regimes, because the frequency of the speculation differs by orders of
/// magnitude (see `cpu_model::if_convert_arm_budget{,_in_loop}` for the
/// derivation):
///
///   * outside any loop — the diamond runs once per call, the branch costs at
///     most one mispredict, so the coin-flip budget applies (8 on Generic);
///   * inside a loop — the speculation executes every iteration, so the
///     conservative loop prior applies (3), which still admits the scalar
///     select and the conditional-reduction diamond.
fn arm_budget_for(ctx: &IfConvCtx<'_>, pred_idx: usize) -> usize {
    loop_arm_budget(ctx.loop_depth.get(pred_idx).copied().unwrap_or(0))
}

/// [`arm_budget_for`] keyed on the head's loop nesting depth, for callers that
/// have the depth but no [`IfConvCtx`] (`plan_shared_arm_dup` runs before the
/// context is built).
fn loop_arm_budget(depth: usize) -> usize {
    let tune = crate::backend::x86::cpu_model::active();
    let once_per_call = tune.if_convert_arm_budget();
    if depth == 0 {
        return once_per_call;
    }
    match crate::passes::ifconv_arm_budget_override() {
        Some(n) if n == usize::MAX => once_per_call,
        Some(n) => n,
        None => tune.if_convert_arm_budget_in_loop(),
    }
}

/// Arm budget for one OUTCOME of the diamond headed by `pred_idx`.
///
/// The two priors compose and the stricter one wins: the loop-resident
/// regime is about how often the speculation executes, the masked-outcome
/// prior is about whether the branch was predictable in the first place, and
/// an arm that is both inside a loop and the rare outcome of a masked compare
/// must satisfy both.  Taking the minimum is the only combination that
/// cannot be gamed by one prior ignoring the other.
fn speculative_arm_budget(
    ctx: &IfConvCtx<'_>,
    pred_idx: usize,
    cond: &Operand,
    outcome: bool,
) -> usize {
    let usual = arm_budget_for(ctx, pred_idx);
    if rare_masked_outcome(ctx, cond, 8) == Some(outcome) {
        usual.min(2)
    } else {
        usual
    }
}

/// Whether a load in an arm block may be executed unconditionally (hoisted
/// into the branch pred).  Loads trap on invalid addresses, so speculation
/// changes observable behavior unless the address is touched on EVERY path
/// the scalar could take.  The exact, target-independent rule:
///
/// a load of canonical address key K in arm A of a decision (pred P, arms
/// T/F, merge M) may be speculated only if every path already reads at least
/// the same bytes:
///   - diamond:  K ∈ reads(P) ∪ reads(other arm)
///   - triangle: K ∈ reads(P)
/// A preceding write alone is not evidence that the bytes are readable:
/// some systems permit mappings or device locations that can be written but
/// not read. The converted form can perform an additional nonvolatile read,
/// but only of bytes already read on that scalar path.
///
/// This is also precisely the reference compilers' contract.  GCC 14 (-O3,
/// x86-64-v3), measured: `d[i] = s[i] < 0 ? -a[i] : a[i]` loads `a[i]`
/// unconditionally in the vector loop (K covered by both arms), while
/// `d[i] = a[i] > t ? 1.0f : c[i]` keeps `c[i]` behind a `vmaskmovps`
/// (K uncovered — GCC masks the load instead of speculating it).  The
/// previous shape-based gate here ("IV-addressed inside a loop") accepted
/// the second form and read addresses the scalar never would — a fault the
/// source program cannot produce (e.g. a condition that is always true with
/// a short/invalid `c`).  Uncovered shapes stay branchy, which is exactly
/// what the non-vectorized lowering does.
fn arm_load_speculation_ok(
    ctx: &IfConvCtx<'_>,
    pred_idx: usize,
    sibling_idx: Option<usize>,
    ptr: &Value,
    load_width: usize,
) -> bool {
    let key = canonical_addr_key_impl(&ctx.defs, &ctx.copy_of, ptr);
    // The key names an ADDRESS; dereferenceability is about an EXTENT. A
    // covering access that is narrower than the speculated load proves
    // nothing about the bytes past its own end:
    //
    //     if (c) x = *(long *)p;  else  y = *(char *)p;
    //
    // with `p` on the last byte of a mapping, the char arm is fine and the
    // speculated 8-byte load faults. Require the covering dereference to be
    // at least as wide, which is the extent half of LLVM's
    // `isDereferenceableAndAlignedPointer`.
    let need = load_width;
    // derefs(P): every memory operation in the pred executes before the
    // branch, on both paths.
    //
    // The pred is only the NEAREST such block.  A nested conditional's inner
    // diamond has a pred that is itself an arm of the outer one and holds
    // nothing but the compare — the covering access sits further up, in the
    // outer head that loaded the value both levels test:
    //
    //     v = *p;                       <- covers *p, on every path
    //     if (v > hi) ...               <- outer head
    //     else if (v < lo) y = *p;      <- inner arm: the speculated load
    //
    // Walking the UNIQUE-PREDEDESSOR chain above the pred collects exactly
    // the blocks that dominate it (a block with one predecessor is dominated
    // by it), i.e. the blocks that have already executed on every path
    // reaching the branch.  Their dereferences are therefore valid coverage
    // evidence for both arms — the same argument, one dominator step up.
    if dominating_deref_keys(ctx, pred_idx)
        .get(&key)
        .is_some_and(|&w| w >= need)
    {
        return true;
    }
    // derefs(sibling): the OTHER arm covers the other path (diamonds only;
    // triangles pass None and fail closed here).
    match sibling_idx {
        Some(s) => block_deref_keys(ctx, s)
            .get(&key)
            .is_some_and(|&w| w >= need),
        None => false,
    }
}

/// True of an instruction that may deallocate or unmap memory — a direct
/// call, an indirect call, or inline assembly. A dereference proven BEFORE
/// such an instruction is not evidence that the same bytes are still
/// dereferenceable AFTER it (`free`/`munmap`/`realloc` behind the call may
/// have released them), so it must not serve as load-speculation coverage
/// across the barrier. This is the `CanBeFreed` hazard LLVM's
/// `isDereferenceableAndAlignedPointer` guards. Pure arithmetic, GEPs,
/// casts, and even atomic accesses and `memcpy` are deliberately NOT
/// barriers: none of them releases a mapping, so excluding them keeps the
/// coverage evidence as strong as it safely can be.
fn may_free_or_unmap(inst: &Instruction) -> bool {
    matches!(
        inst,
        Instruction::Call { .. } | Instruction::CallIndirect { .. } | Instruction::InlineAsm { .. }
    )
}

fn block_has_free_barrier(ctx: &IfConvCtx<'_>, block_idx: usize) -> bool {
    ctx.func.blocks[block_idx]
        .instructions
        .iter()
        .any(may_free_or_unmap)
}

/// Canonical-address read map of one block: for every non-volatile,
/// default-address-space load, the WIDEST read made through that address key.
/// A store does not establish that a speculative read is permitted.
///
/// FREES BARRIER: the scan is in program order and a potential
/// free/unmap (see [`may_free_or_unmap`]) clears everything accumulated so
/// far — a dereference before such a call proves nothing about
/// dereferenceability at any point after it, and every consumer of this map
/// (the speculated arm load) executes after the whole block has run. Only
/// the dereferences that follow the LAST barrier in the block are returned.
/// The width is the coverage evidence: a key present with width `w` proves
/// that `w` bytes at that address are dereferenceable on this path, and
/// nothing about byte `w + 1`.
fn block_deref_keys(ctx: &IfConvCtx<'_>, block_idx: usize) -> FxHashMap<String, usize> {
    let mut keys: FxHashMap<String, usize> = FxHashMap::default();
    for inst in &ctx.func.blocks[block_idx].instructions {
        if may_free_or_unmap(inst) {
            keys.clear();
            continue;
        }
        let (ptr, volatile, seg, ty) = match inst {
            Instruction::Load {
                ptr,
                volatile,
                seg_override,
                ty,
                ..
            } => (ptr, volatile, seg_override, ty),
            _ => continue,
        };
        if !*volatile && *seg == AddressSpace::Default {
            let key = canonical_addr_key_impl(&ctx.defs, &ctx.copy_of, ptr);
            let w = ty.size();
            keys.entry(key)
                .and_modify(|cur| *cur = (*cur).max(w))
                .or_insert(w);
        }
    }
    keys
}

/// Deepest unique-predecessor chain walked for dominance evidence.
///
/// A bound, not a correctness requirement: the walk is monotone (each step
/// moves to a distinct strict predecessor) so it already terminates on any
/// finite CFG, and the chain length in practice is the nesting depth of the
/// conditional being converted.  The cap keeps a pathological hand-built CFG
/// from turning a per-arm query into a per-block CFG walk.
const MAX_DOM_DEREF_CHAIN: usize = 64;

/// Widest dereference of every address key proven reachable on EVERY path
/// that reaches `block_idx`: the block's own dereferences unioned with those
/// of its transitive UNIQUE-predecessor ancestors.
///
/// Soundness: if `p` is the only predecessor of `b`, every path to `b`
/// contains `p`, so `p` dominates `b`.  Chaining that argument up from
/// `block_idx` yields blocks that have all executed — in order, before
/// `block_idx`'s terminator — on any execution that reaches `block_idx`, and
/// hence on both arms of a diamond or triangle hanging off it.  A
/// non-volatile, default-address-space read of `w` bytes there proves those
/// `w` bytes are readable on both arms, which is precisely the
/// evidence [`arm_load_speculation_ok`] requires.  Blocks with more than one
/// predecessor end the walk (they are not dominated), so the result is a
/// conservative subset of the true dominator set — never an over-estimate.
fn dominating_deref_keys(ctx: &IfConvCtx<'_>, block_idx: usize) -> FxHashMap<String, usize> {
    let mut keys = block_deref_keys(ctx, block_idx);
    let mut cur = block_idx;
    for _ in 0..MAX_DOM_DEREF_CHAIN {
        // A barrier anywhere in `cur` executed after every block above it,
        // so the unique-predecessor chain past it cannot contribute live
        // coverage: those dereferences may have been freed by the barrier.
        // (`block_deref_keys` already pruned `cur`'s own pre-barrier
        // dereferences; this check stops the walk from re-introducing the
        // ancestors'.)
        if block_has_free_barrier(ctx, cur) {
            break;
        }
        if ctx.preds.len(cur) != 1 {
            break;
        }
        let parent = ctx.preds.row(cur)[0] as usize;
        if parent == cur {
            break; // self-loop: no new dominance evidence, and it would spin
        }
        for (k, w) in block_deref_keys(ctx, parent) {
            keys.entry(k)
                .and_modify(|cur_w| *cur_w = (*cur_w).max(w))
                .or_insert(w);
        }
        cur = parent;
    }
    keys
}

/// Arm-block convertibility with the speculation extension: the pure
/// whitelist above plus loads whose address passes the path-coverage rule
/// (`arm_load_speculation_ok`).
fn arm_is_speculatable(
    ctx: &IfConvCtx<'_>,
    block: &BasicBlock,
    pred_idx: usize,
    sibling_idx: Option<usize>,
) -> bool {
    for inst in &block.instructions {
        match inst {
            Instruction::Load {
                ptr,
                volatile,
                seg_override,
                ty,
                ..
            } => {
                if *volatile || *seg_override != AddressSpace::Default {
                    return false;
                }
                if !arm_load_speculation_ok(ctx, pred_idx, sibling_idx, ptr, ty.size()) {
                    return false;
                }
            }
            Instruction::BinOp { op, .. } if op.can_trap() => return false,
            Instruction::BinOp { .. }
            | Instruction::UnaryOp { .. }
            | Instruction::Cmp { .. }
            | Instruction::Cast { .. }
            | Instruction::Copy { .. }
            | Instruction::GetElementPtr { .. }
            | Instruction::GlobalAddr { .. }
            | Instruction::Select { .. } => {}
            _ => return false,
        }
    }
    true
}

/// Check if a condition operand is a known constant or can be trivially resolved
/// to a constant within the block. When the condition is constant, the branch
/// should be folded by cfg_simplify rather than converted to a Select by
/// if_convert. Converting a constant-condition branch to Select delays dead
/// code elimination: the Select needs additional optimization iterations
/// (simplify + constfold + cfg_simplify) to fold away, and if the diminishing-
/// returns heuristic terminates the optimization loop early, dead code paths
/// (e.g., kernel's conditional calls to restore_tpidr2_context guarded by
/// system_supports_sme() which returns false) survive to the final output.
fn is_constant_condition(block: &BasicBlock, cond: &Operand) -> bool {
    match cond {
        Operand::Const(_) => true,
        Operand::Value(v) => {
            // Check if the value is defined as a constant Copy, or as a Cmp/Select
            // where all operands are constants, within the same block.
            for inst in &block.instructions {
                match inst {
                    Instruction::Copy {
                        dest,
                        src: Operand::Const(_),
                    } if *dest == *v => {
                        return true;
                    }
                    Instruction::Cmp { dest, lhs, rhs, .. } if *dest == *v => {
                        let lhs_const =
                            matches!(lhs, Operand::Const(_)) || is_value_const_in_block(block, lhs);
                        let rhs_const =
                            matches!(rhs, Operand::Const(_)) || is_value_const_in_block(block, rhs);
                        if lhs_const && rhs_const {
                            return true;
                        }
                    }
                    Instruction::Select {
                        dest,
                        true_val,
                        false_val,
                        ..
                    } if *dest == *v => {
                        // Select(cond, x, x) where both arms are the same constant
                        if same_value_or_both_zero(true_val, false_val) {
                            return true;
                        }
                    }
                    _ => {}
                }
            }
            false
        }
    }
}

/// Check if an operand is a constant within the block (either directly or via Copy).
fn is_value_const_in_block(block: &BasicBlock, op: &Operand) -> bool {
    match op {
        Operand::Const(_) => true,
        Operand::Value(v) => {
            for inst in &block.instructions {
                if let Instruction::Copy {
                    dest,
                    src: Operand::Const(_),
                } = inst
                {
                    if *dest == *v {
                        return true;
                    }
                }
            }
            false
        }
    }
}

/// Check if two operands are the same value or both integer-zero constants.
/// Used to detect Select(cond, x, x) patterns where the result is the same
/// regardless of the condition, making the condition effectively constant.
fn same_value_or_both_zero(a: &Operand, b: &Operand) -> bool {
    match (a, b) {
        (Operand::Const(ca), Operand::Const(cb)) => {
            ca.to_i64() == Some(0) && cb.to_i64() == Some(0)
        }
        (Operand::Value(va), Operand::Value(vb)) => va.0 == vb.0,
        _ => false,
    }
}

/// Whether `v` is consumed only by control flow (never materialized as data),
/// so the branchy form is at least as good as a cmov Select. Recursively
/// follows boolean-preserving edges: `Cmp(Eq/Ne, 0/1)`, `Select` conditions,
/// `Copy`, `Cast`, and `Phi` incoming values.
fn value_only_controls_branch(func: &IrFunction, v: Value, visited: &mut FxHashSet<u32>) -> bool {
    if !visited.insert(v.0) {
        return true; // cycle: nothing data-like seen yet
    }
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Cmp {
                    dest, op, lhs, rhs, ..
                } => {
                    if matches!(lhs, Operand::Value(x) if x.0 == v.0)
                        || matches!(rhs, Operand::Value(x) if x.0 == v.0)
                    {
                        // Only a boolean negation (v == 0 / v != 0 / v == 1 / v != 1)
                        // preserves branch-likeness; any other comparison is data.
                        let other = if matches!(lhs, Operand::Value(x) if x.0 == v.0) {
                            rhs
                        } else {
                            lhs
                        };
                        let is_bool_cmp = matches!(op, IrCmpOp::Eq | IrCmpOp::Ne)
                            && matches!(other, Operand::Const(c) if c.to_i64() == Some(0) || c.to_i64() == Some(1));
                        if !is_bool_cmp || !value_only_controls_branch(func, *dest, visited) {
                            return false;
                        }
                    }
                }
                Instruction::Select {
                    dest,
                    cond,
                    true_val,
                    false_val,
                    ..
                } => {
                    if matches!(cond, Operand::Value(x) if x.0 == v.0) {
                        if !value_only_controls_branch(func, *dest, visited) {
                            return false;
                        }
                    } else if matches!(true_val, Operand::Value(x) if x.0 == v.0)
                        || matches!(false_val, Operand::Value(x) if x.0 == v.0)
                    {
                        return false; // selected as a data value
                    }
                }
                Instruction::Copy { dest, src, .. } => {
                    if matches!(src, Operand::Value(x) if x.0 == v.0)
                        && !value_only_controls_branch(func, *dest, visited)
                    {
                        return false;
                    }
                }
                Instruction::Cast { dest, src, .. } => {
                    if matches!(src, Operand::Value(x) if x.0 == v.0)
                        && !value_only_controls_branch(func, *dest, visited)
                    {
                        return false;
                    }
                }
                Instruction::Phi { dest, incoming, .. } => {
                    for (op, _) in incoming {
                        if matches!(op, Operand::Value(x) if x.0 == v.0)
                            && !value_only_controls_branch(func, *dest, visited)
                        {
                            return false;
                        }
                    }
                }
                other => {
                    let mut used = false;
                    other.for_each_used_value(|u| {
                        if u == v.0 {
                            used = true;
                        }
                    });
                    if used {
                        return false; // data use (binop operand, store, call arg, ...)
                    }
                }
            }
        }
        match &block.terminator {
            // A branch condition is control flow — fine.
            Terminator::CondBranch { cond, .. } if matches!(cond, Operand::Value(x) if x.0 == v.0) =>
                {}
            // Return/Switch/IndirectBranch use the value as data.
            Terminator::Return(Some(op)) if matches!(op, Operand::Value(x) if x.0 == v.0) => {
                return false;
            }
            Terminator::Switch { val, .. } if matches!(val, Operand::Value(x) if x.0 == v.0) => {
                return false;
            }
            Terminator::IndirectBranch { target, .. } if matches!(target, Operand::Value(x) if x.0 == v.0) =>
            {
                return false;
            }
            _ => {}
        }
    }
    true
}

/// Skip if-conversion for a short-circuit `||` diamond whose result is
/// consumed only by control flow. `cond ? 1 : rhs` (`||`) materializes as a
/// cmov chain, but its whole point is short-circuit evaluation — a branch is
/// cheaper and lets the backend fuse each compare straight into a jump (no
/// setcc+movzbl+test+cmov per link). Keeping the branchy form is the
/// difference between GCC's Expat codegen (`cmpb $X,%dl; ja`) and a wall of
/// setcc/cmov.
///
/// Only the `||` form (true arm == constant 1) is skipped. The `&&` form
/// (false arm == constant 0) is intentionally still converted: if-converting
/// it exposes the `Select(cond, Cmp, 0)` pattern the range-check fold
/// collapses into a single `sub`+`cmp`, which then fuses into the `||` branch.
fn skip_boolean_diamond(
    func: &IrFunction,
    phi_selects: &[(Value, IrType, Operand, Operand)],
) -> bool {
    skip_boolean_diamond_inner(func, phi_selects, false)
}

/// `skip_boolean_diamond` with the loop-body override.
///
/// `in_vectorizable_loop` is set only by `body_becomes_branch_free`'s
/// simulation, i.e. while proving that a loop can be made straight-line so
/// the integer map vectorizer will take it. There the trade documented on
/// `skip_boolean_diamond` is reversed: a 32-byte-at-a-time vector body beats
/// any branch, so the branchy form it preserves is exactly what blocks the
/// win. Everywhere else the historical rule stands unchanged.
fn skip_boolean_diamond_inner(
    func: &IrFunction,
    phi_selects: &[(Value, IrType, Operand, Operand)],
    in_vectorizable_loop: bool,
) -> bool {
    if phi_selects.is_empty() {
        return false;
    }
    if in_vectorizable_loop {
        return false;
    }
    // Every converted phi must be the `||` shape: `cond ? 1 : rhs`.
    let all_or = phi_selects
        .iter()
        .all(|(_, _, tv, _fv)| matches!(tv, Operand::Const(c) if c.to_i64() == Some(1)));
    if !all_or {
        return false;
    }
    let mut visited = FxHashSet::default();
    phi_selects
        .iter()
        .all(|(dest, _, _, _)| value_only_controls_branch(func, *dest, &mut visited))
}

/// EFFECTIVE instruction count of a diamond arm: address-materialization
/// chains (IV Cast, Shl scaling, base Copy, the GEP itself) are pure address
/// math the backend folds into one SIB operand — they add nothing
/// speculative.  Count only what actually executes speculatively.  The
/// conditional reduction diamond (`if (arr[i] > 0) s += arr[i]`) is exactly
/// this shape: each arm = {Cast,Shl,Copy,Shl,GEP,Load,Cast,Add} — 8 raw,
/// 3 effective (Load, Cast, Add).
fn effective_arm_len(block: &BasicBlock) -> usize {
    block
        .instructions
        .iter()
        .filter(|inst| {
            !matches!(
                *inst,
                Instruction::GetElementPtr { .. }
                    | Instruction::Copy { .. }
                    | Instruction::GlobalAddr { .. }
                    | Instruction::BinOp {
                        op: IrBinOp::Shl,
                        ..
                    }
            )
        })
        .count()
}

/// Detect a diamond pattern starting from a block with a CondBranch terminator.
/// Deepest single-predecessor chain followed through one diamond arm.
/// Same bound rationale as `MAX_DOM_DEREF_CHAIN`: the walk is monotone, the
/// cap only fences off a pathological CFG.
const MAX_ARM_CHAIN: usize = 64;

/// One diamond arm walked as a single-entry single-exit straight-line region.
struct ArmRegion {
    /// The arm's first block (the one the CondBranch targets).
    head: usize,
    /// The block the arm's edge into the merge leaves from.
    exit: usize,
    /// The merge candidate: the target of `exit`'s unconditional branch, or
    /// `None` when the chain does not end in one (no diamond here).
    target: Option<usize>,
    /// Every block in the region, in execution order (`head` first).
    blocks: Vec<usize>,
    /// Concatenated instructions of every block, in execution order.
    insts: Vec<Instruction>,
    /// Summed EFFECTIVE arm length (see `effective_arm_len`).
    effective_len: usize,
}

/// Walk one arm of a diamond as far as the CFG keeps it a straight line.
///
/// The classic detector stops at the arm's first block, which leaves a NESTED
/// conditional permanently branchy: after the inner level converts, the outer
/// false arm is `head -> merge_of_inner`, two blocks, and the outer detector
/// sees a false arm that does not branch to the outer merge:
///
/// ```text
///     H : CondBranch c1 -> T / H2
///     T : ...; Branch M
///     H2: Select ...; Branch N        <- inner diamond already converted
///     N : (empty);  Branch M          <- single-pred tail of the false arm
///     M : phi [(vT, T), (vN, N)]
/// ```
///
/// Following the chain recovers the region `H2 -> N`, whose exit does branch
/// to `M` — and whose phi incoming names `N`, which is why the region carries
/// its exit separately from its head.
///
/// Soundness of following a block `n` after `cur`: `n` must have EXACTLY ONE
/// predecessor, and it must be `cur`.  Then every path that reaches `n`
/// reaches it only through `cur`, so the region executes exactly when the arm
/// head executes — hoisting its instructions into the pred speculates
/// precisely the same operations the single-block rule already admits, and
/// the side-effect and budget gates are applied to every block in it.  The
/// walk stops at the pred, at the sibling arm, at a block with any other
/// predecessor count, and on any revisit, so it can neither swallow the
/// merge nor loop.
fn walk_arm(
    ctx: &IfConvCtx<'_>,
    arm_idx: usize,
    pred_idx: usize,
    sibling_idx: usize,
) -> Option<ArmRegion> {
    let func = ctx.func;
    let mut region = ArmRegion {
        head: arm_idx,
        exit: arm_idx,
        target: None,
        blocks: vec![arm_idx],
        insts: func.blocks[arm_idx].instructions.clone(),
        effective_len: effective_arm_len(&func.blocks[arm_idx]),
    };
    let mut cur = arm_idx;
    for _ in 0..MAX_ARM_CHAIN {
        let Terminator::Branch(label) = &func.blocks[cur].terminator else {
            break;
        };
        let next = *ctx.label_to_idx.get(label)?;
        // The pred, the sibling arm and the arm head are never part of this
        // region: the first two belong to the diamond's other edges, the
        // third would close a cycle.
        if next == pred_idx || next == sibling_idx || next == arm_idx || next == cur {
            break;
        }
        // Only a block whose SOLE predecessor is `cur` is reached exactly
        // when `cur` is — anything else is a join point (possibly the merge
        // another edge also reaches) and ends the region.
        if ctx.preds.len(next) != 1 || ctx.preds.row(next)[0] as usize != cur {
            break;
        }
        region.blocks.push(next);
        region
            .insts
            .extend(func.blocks[next].instructions.iter().cloned());
        region.effective_len += effective_arm_len(&func.blocks[next]);
        region.exit = next;
        cur = next;
    }
    // The region's exit must branch somewhere for a merge to exist.
    if let Terminator::Branch(label) = &func.blocks[region.exit].terminator {
        region.target = ctx.label_to_idx.get(label).copied();
    }
    Some(region)
}

fn detect_diamond(ctx: &IfConvCtx<'_>, pred_idx: usize) -> Option<DiamondInfo> {
    detect_diamond_inner(ctx, pred_idx, false)
}

fn detect_diamond_inner(
    ctx: &IfConvCtx<'_>,
    pred_idx: usize,
    in_vectorizable_loop: bool,
) -> Option<DiamondInfo> {
    let func = &ctx.func;
    let label_to_idx = &ctx.label_to_idx;
    let preds = &ctx.preds;
    let pred_block = &func.blocks[pred_idx];

    // Must end with a CondBranch
    let (cond, true_label, false_label) = match &pred_block.terminator {
        Terminator::CondBranch {
            cond,
            true_label,
            false_label,
        } => (cond, true_label, false_label),
        _ => return None,
    };

    // Don't convert branches with constant conditions to Select.
    // cfg_simplify will fold these more efficiently (single pass vs multi-iteration).
    if is_constant_condition(pred_block, cond) {
        return None;
    }

    let true_idx = *label_to_idx.get(true_label)?;
    let false_idx = *label_to_idx.get(false_label)?;

    // The two arms must be different blocks
    if true_idx == false_idx {
        return None;
    }

    // Each arm is walked as a single-entry single-exit straight-line region
    // (see `walk_arm`): one block in the classic case, a chain when a nested
    // conditional has already been converted inside-out.
    let true_arm = walk_arm(ctx, true_idx, pred_idx, false_idx)?;
    let false_arm = walk_arm(ctx, false_idx, pred_idx, true_idx)?;

    // Both arms must end with unconditional branches to the same merge block
    let (Some(true_target), Some(false_target)) = (true_arm.target, false_arm.target) else {
        return None;
    };
    if true_target != false_target {
        return None; // Different merge blocks
    }
    let merge_idx = true_target;

    // The merge block must not be inside either arm region, nor the pred.
    if merge_idx == pred_idx
        || true_arm.blocks.contains(&merge_idx)
        || false_arm.blocks.contains(&merge_idx)
    {
        return None;
    }

    // The arm HEAD blocks should have exactly one predecessor each (the pred
    // block).  If they have other predecessors, other code flows into them
    // and we can't eliminate the blocks.  (The rest of each region is
    // single-predecessor by `walk_arm`'s construction.)
    if preds.len(true_idx) != 1 || preds.len(false_idx) != 1 {
        return None;
    }
    if preds.row(true_idx)[0] as usize != pred_idx || preds.row(false_idx)[0] as usize != pred_idx {
        return None;
    }

    // Every block of both arms must be side-effect-free.  Loads qualify only
    // through the path-coverage speculation gate (see
    // `arm_load_speculation_ok`).
    if !true_arm
        .blocks
        .iter()
        .all(|&b| arm_is_speculatable(ctx, &func.blocks[b], pred_idx, Some(false_idx)))
        || !false_arm
            .blocks
            .iter()
            .all(|&b| arm_is_speculatable(ctx, &func.blocks[b], pred_idx, Some(true_idx)))
    {
        return None;
    }

    // Limit the number of instructions in each arm to prevent over-speculation.
    // cmov is only profitable when the arms are cheap (a few instructions).
    // Arm budget from the CPU tuning model: `mispredict_penalty / 2`
    // (a coin-flip prior on the branch outcome; 8 on the Generic row, which
    // reproduces the historical constant, 6 on Zen3/Zen4 whose 13-cycle
    // penalty makes speculation less valuable, 8 on Raptor Lake).  C's type
    // system generates Load + Cast chains (parameter loads + sign
    // extensions) that inflate the count; a typical arm: Load, Cast, Load,
    // Cast, BinOp, Cast = 6 insts.  See docs/CPU_MODEL_AUDIT.md §4.
    let true_budget = speculative_arm_budget(ctx, pred_idx, cond, true);
    let false_budget = speculative_arm_budget(ctx, pred_idx, cond, false);
    // EFFECTIVE instruction count: address-materialization chains (IV Cast,
    // Shl scaling, base Copy, the GEP itself) are pure address math the
    // backend folds into one SIB operand — they add nothing speculative.
    // Count only what actually executes speculatively. The conditional
    // reduction diamond (`if (arr[i] > 0) s += arr[i]`) is exactly this
    // shape: each arm = {Cast,Shl,Copy,Shl,GEP,Load,Cast,Add} — 8 raw,
    // 3 effective (Load, Cast, Add).  For a multi-block region the budget
    // covers the WHOLE region — the speculation cost is the sum.
    if true_arm.effective_len > true_budget || false_arm.effective_len > false_budget {
        return None;
    }

    // The phi incomings name each arm's EXIT block (the one whose edge
    // reaches the merge), not its head.
    let true_idx = true_arm.exit;
    let false_idx = false_arm.exit;

    // The merge block must have Phi nodes that reference both arms.
    // Collect phi nodes we can convert.
    let merge_block = &func.blocks[merge_idx];
    let mut phi_selects = Vec::with_capacity(16);

    for inst in &merge_block.instructions {
        if let Instruction::Phi { dest, ty, incoming } = inst {
            // Find the values from each arm
            let mut true_val = None;
            let mut false_val = None;

            for (op, label) in incoming {
                let src_idx = label_to_idx.get(label).copied();
                if src_idx == Some(true_idx) {
                    true_val = Some(*op);
                } else if src_idx == Some(false_idx) {
                    false_val = Some(*op);
                }
                // Other predecessors are fine - they just won't be converted
            }

            match (true_val, false_val) {
                (Some(tv), Some(fv)) => {
                    // Support integer, pointer, F32, and F64 types.
                    // F32/F64 work because all backends implement Select by
                    // moving bit patterns through integer registers (cmov/csel/branch).
                    // Skip long double (F128) and 128-bit integers — they need
                    // multi-register handling that Select doesn't support.
                    if !ty.is_long_double() && !ty.is_128bit() {
                        phi_selects.push((*dest, *ty, tv, fv));
                    } else {
                        // There's an unconvertible phi (F128/I128) referencing
                        // both arms. We must NOT partially convert this diamond
                        // or the remaining phi nodes will reference removed blocks.
                        return None;
                    }
                }
                _ => {
                    // Phi doesn't have entries from both arms - can't convert
                    // This could happen if the phi also has entries from other preds
                }
            }
        }
    }

    if phi_selects.is_empty() {
        return None; // No convertible phis
    }

    // Keep short-circuit boolean diamonds branchy (see skip_boolean_diamond).
    if skip_boolean_diamond_inner(func, &phi_selects, in_vectorizable_loop) {
        return None;
    }

    // The merge block should only be reached from the two arms (and not from pred directly).
    // If the merge block has other predecessors, we need to preserve the Phi nodes for those.
    let merge_preds_from_diamond = preds
        .row(merge_idx)
        .iter()
        .filter(|&&p| p as usize == true_idx || p as usize == false_idx)
        .count();

    // If the merge block has predecessors other than the two arms, the
    // conversion is PARTIAL: the converted phis are rewritten to keep their
    // other incoming edges (a fresh Select feeds a preserved Phi).  This is
    // what lets nested conditionals convert inside-out: the inner
    // diamond/triangle converts partially (the outer arm still feeds the
    // merge phi), then the fixpoint loop converts the outer one fully.
    let full_merge = preds.len(merge_idx) == 2;
    if !full_merge && merge_preds_from_diamond != 2 {
        return None;
    }

    Some(DiamondInfo {
        pred_idx,
        true_idx,
        false_idx,
        merge_idx,
        cond: *cond,
        true_arm_insts: true_arm.insts.clone(),
        false_arm_insts: false_arm.insts.clone(),
        true_arm_blocks: true_arm.blocks.clone(),
        false_arm_blocks: false_arm.blocks.clone(),
        phi_selects,
        full_merge,
    })
}

/// Detect a triangle pattern: pred branches to arm and merge directly.
///
/// ```text
///     pred: CondBranch(cond, arm, merge)   -- or (cond, merge, arm)
///     arm:  side-effect-free instructions + Branch(merge)
///     merge: phi [arm_val, arm], [pred_val, pred]
/// ```
///
/// This handles ternaries like `a >= t ? a - t : 0` where the false arm
/// is a constant and doesn't need its own block.
fn detect_triangle(ctx: &IfConvCtx<'_>, pred_idx: usize) -> Option<DiamondInfo> {
    detect_triangle_inner(ctx, pred_idx, false)
}

fn detect_triangle_inner(
    ctx: &IfConvCtx<'_>,
    pred_idx: usize,
    in_vectorizable_loop: bool,
) -> Option<DiamondInfo> {
    let func = &ctx.func;
    let label_to_idx = &ctx.label_to_idx;
    let preds = &ctx.preds;
    let pred_block = &func.blocks[pred_idx];

    let (cond, true_label, false_label) = match &pred_block.terminator {
        Terminator::CondBranch {
            cond,
            true_label,
            false_label,
        } => (cond, true_label, false_label),
        _ => return None,
    };

    // Don't convert branches with constant conditions to Select.
    // cfg_simplify will fold these more efficiently (single pass vs multi-iteration).
    if is_constant_condition(pred_block, cond) {
        return None;
    }

    let true_idx = *label_to_idx.get(true_label)?;
    let false_idx = *label_to_idx.get(false_label)?;

    if true_idx == false_idx {
        return None;
    }

    // Determine which arm goes to a separate block and which goes directly to merge.
    // Case 1: true arm is a block, false arm goes directly to merge
    // Case 2: false arm is a block, true arm goes directly to merge
    let (arm_idx, merge_idx, arm_is_true) = {
        let true_block = &func.blocks[true_idx];
        let false_block = &func.blocks[false_idx];

        let true_target = match &true_block.terminator {
            Terminator::Branch(label) => label_to_idx.get(label).copied(),
            _ => None,
        };
        let false_target = match &false_block.terminator {
            Terminator::Branch(label) => label_to_idx.get(label).copied(),
            _ => None,
        };

        if let Some(tt) = true_target {
            if tt == false_idx {
                // true arm branches to false_idx which is the merge block
                (true_idx, false_idx, true)
            } else if let Some(ft) = false_target {
                if ft == true_idx {
                    // false arm branches to true_idx which is the merge block
                    (false_idx, true_idx, false)
                } else {
                    return None;
                }
            } else {
                return None;
            }
        } else if let Some(ft) = false_target {
            if ft == true_idx {
                (false_idx, true_idx, false)
            } else {
                return None;
            }
        } else {
            return None;
        }
    };

    // arm block must have exactly one predecessor (pred)
    if preds.len(arm_idx) != 1 || preds.row(arm_idx)[0] as usize != pred_idx {
        return None;
    }

    // merge must have the pred and the arm as predecessors; other
    // predecessors are allowed (partial conversion, see detect_diamond).
    let has_pred = preds.row(merge_idx).iter().any(|&p| p as usize == pred_idx);
    let has_arm = preds.row(merge_idx).iter().any(|&p| p as usize == arm_idx);
    if !has_pred || !has_arm {
        return None;
    }

    let arm_block = &func.blocks[arm_idx];

    // arm must be side-effect-free (path-coverage gate for loads: a triangle
    // arm load may only be speculated when the pred already dereferences the
    // same address — the direct P→M edge touches nothing else)
    if !arm_is_speculatable(ctx, arm_block, pred_idx, None) {
        return None;
    }

    // Same tuning-model budget as the diamond form above, including the
    // loop-resident split.
    let max_arm_insts: usize = speculative_arm_budget(ctx, pred_idx, cond, arm_is_true);
    let arm_insts = if arm_block.instructions.iter().all(|i| {
        matches!(
            i,
            Instruction::Cmp { .. }
                | Instruction::Select { .. }
                | Instruction::Cast { .. }
                | Instruction::Copy { .. }
        )
    }) {
        arm_block
            .instructions
            .iter()
            .filter(|i| matches!(i, Instruction::Cmp { .. }))
            .count()
    } else {
        arm_block.instructions.len()
    };
    if arm_insts > max_arm_insts {
        return None;
    }

    // Collect phi nodes from merge block
    let merge_block = &func.blocks[merge_idx];
    let mut phi_selects = Vec::with_capacity(16);

    for inst in &merge_block.instructions {
        if let Instruction::Phi { dest, ty, incoming } = inst {
            let mut arm_val = None;
            let mut pred_val = None;

            for (op, label) in incoming {
                let src_idx = label_to_idx.get(label).copied();
                if src_idx == Some(arm_idx) {
                    arm_val = Some(*op);
                } else if src_idx == Some(pred_idx) {
                    pred_val = Some(*op);
                }
            }

            if let (Some(av), Some(pv)) = (arm_val, pred_val) {
                // Support integer, pointer, F32, and F64 types.
                // Skip long double (F128) and 128-bit integers.
                if !ty.is_long_double() && !ty.is_128bit() {
                    // Map to true/false values based on which arm the block is
                    if arm_is_true {
                        phi_selects.push((*dest, *ty, av, pv));
                    } else {
                        phi_selects.push((*dest, *ty, pv, av));
                    }
                } else {
                    // Unconvertible phi — bail out to avoid partial conversion.
                    return None;
                }
            }
        }
    }

    if phi_selects.is_empty() {
        return None;
    }

    // Keep short-circuit boolean diamonds branchy (see skip_boolean_diamond).
    if skip_boolean_diamond_inner(func, &phi_selects, in_vectorizable_loop) {
        return None;
    }

    // For a triangle, we set the missing arm to merge_idx with empty instructions.
    // apply_diamond will hoist the arm instructions and the empty side is a no-op.
    let (true_idx_out, false_idx_out, true_insts, false_insts) = if arm_is_true {
        (
            arm_idx,
            merge_idx,
            arm_block.instructions.clone(),
            Vec::new(),
        )
    } else {
        (
            merge_idx,
            arm_idx,
            Vec::new(),
            arm_block.instructions.clone(),
        )
    };

    // A triangle arm is a single block, so each region is just its head and
    // the exit coincides with it.  The pseudo-arm (the merge block itself,
    // carrying no instructions) contributes NO blocks to clear — emptying the
    // merge would delete the code the whole conversion exists to feed.
    let (true_blocks, false_blocks) = if arm_is_true {
        (vec![arm_idx], Vec::new())
    } else {
        (Vec::new(), vec![arm_idx])
    };

    Some(DiamondInfo {
        pred_idx,
        true_idx: true_idx_out,
        false_idx: false_idx_out,
        merge_idx,
        cond: *cond,
        true_arm_insts: true_insts,
        false_arm_insts: false_insts,
        true_arm_blocks: true_blocks,
        false_arm_blocks: false_blocks,
        phi_selects,
        full_merge: preds.len(merge_idx) == 2,
    })
}

/// Apply a diamond conversion: rewrite the CFG to use Select instructions.
fn apply_diamond(func: &mut IrFunction, diamond: &DiamondInfo) -> bool {
    // Safety check: make sure the blocks haven't been modified by a previous conversion
    // in this same pass iteration
    if diamond.pred_idx >= func.blocks.len()
        || diamond.true_idx >= func.blocks.len()
        || diamond.false_idx >= func.blocks.len()
        || diamond.merge_idx >= func.blocks.len()
    {
        return false;
    }

    // Read the merge label before mutating
    let merge_label = func.blocks[diamond.merge_idx].label;

    // 1. Move instructions from both arms into the pred block (before the branch).
    //    Since both arms are side-effect-free, we can execute all their instructions
    //    unconditionally (both paths are computed).
    //    (Labels are read before the mutable borrow of the pred block.)
    let pred_label = func.blocks[diamond.pred_idx].label;
    let true_label = func.blocks[diamond.true_idx].label;
    let false_label = func.blocks[diamond.false_idx].label;
    let pred_block = &mut func.blocks[diamond.pred_idx];

    // Hoist both arms with dummy spans; the primitive inherits the pred
    // block's inline context and keeps provenance parallel.
    for inst in &diamond.true_arm_insts {
        pred_block.push_inst(inst.clone(), crate::common::source::Span::dummy());
    }
    for inst in &diamond.false_arm_insts {
        pred_block.push_inst(inst.clone(), crate::common::source::Span::dummy());
    }

    // 2. Add Select instructions for each Phi.  A full merge converts each
    //    phi into its Select (same destination, phi removed below).  A
    //    partial merge (the merge block has other predecessors) emits the
    //    Select into a FRESH destination and rewrites the surviving phi's
    //    incoming list to carry that value on the pred edge.
    let partial_rewrites: Vec<(Value, Value)> = diamond
        .phi_selects
        .iter()
        .map(|(dest, ty, true_val, false_val)| {
            let select_dest = if diamond.full_merge {
                *dest
            } else {
                let fresh = Value(func.next_value_id);
                func.next_value_id += 1;
                fresh
            };
            pred_block.push_inst(
                Instruction::Select {
                    dest: select_dest,
                    cond: diamond.cond,
                    true_val: *true_val,
                    false_val: *false_val,
                    ty: *ty,
                },
                crate::common::source::Span::dummy(),
            );
            (*dest, select_dest)
        })
        .collect();

    // 3. Change pred block's terminator to unconditional branch to merge
    pred_block.terminator = Terminator::Branch(merge_label);

    // 4. Remove the converted Phi nodes from the merge block (full merges),
    //    or rewrite their incoming lists to the fresh Select values
    //    (partial merges — the other incoming edges survive).
    let converted_dests: crate::common::fx_hash::FxHashSet<u32> = diamond
        .phi_selects
        .iter()
        .map(|(dest, _, _, _)| dest.0)
        .collect();
    if diamond.full_merge {
        // One retain predicate keeps instructions, spans and inline ids in
        // lockstep; a drifted span table degrades to untracked instead of
        // indexing past the instruction list (the old hard panic).
        func.blocks[diamond.merge_idx].retain_insts(|inst| {
            if let Instruction::Phi { dest, .. } = inst {
                !converted_dests.contains(&dest.0)
            } else {
                true
            }
        });
    } else {
        let merge_block = &mut func.blocks[diamond.merge_idx];
        for inst in &mut merge_block.instructions {
            let Instruction::Phi {
                dest,
                incoming,
                ty: _,
            } = inst
            else {
                continue;
            };
            let Some(&(_, fresh)) = partial_rewrites
                .iter()
                .find(|(phi_dest, _)| phi_dest.0 == dest.0)
            else {
                continue;
            };
            // The pred edge now carries the Select value; the arm edges
            // disappear with the arms.
            incoming.retain(|(_, lbl)| *lbl != true_label && *lbl != false_label);
            let mut replaced = false;
            for (op, lbl) in incoming.iter_mut() {
                if *lbl == pred_label {
                    *op = Operand::Value(fresh);
                    replaced = true;
                    break;
                }
            }
            if !replaced {
                incoming.push((Operand::Value(fresh), pred_label));
            }
        }
    }

    // 5. Empty the arm blocks (they'll be cleaned up by CFG simplification).
    // Keep them as empty blocks with unconditional branches - CFG simplify
    // will remove them as dead blocks since they'll have no predecessors.
    // For triangle patterns, one arm IS the merge block - don't clear it.
    //
    // Every block of an arm REGION is emptied, not just its head: the whole
    // region's instructions were hoisted into the pred, and every block of it
    // is unreachable once the pred branches straight to the merge.  Leaving
    // the tail blocks populated would leave a SECOND definition of each
    // hoisted destination in the function — an SSA violation the verifier
    // rightly rejects.
    for &b in diamond
        .true_arm_blocks
        .iter()
        .chain(diamond.false_arm_blocks.iter())
    {
        if b != diamond.merge_idx && b < func.blocks.len() {
            func.blocks[b].clear_insts();
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::AddressSpace;
    use crate::ir::reexports::{IrBinOp, IrConst};

    #[test]
    fn loop_profitability_exempts_only_the_candidate_chain_branch() {
        let mut f = IrFunction::new("chain_profitability".into(), IrType::Void, vec![], false);
        f.blocks.extend([
            BasicBlock {
                label: BlockId(0),
                instructions: vec![],
                terminator: Terminator::CondBranch {
                    cond: Operand::Const(IrConst::I32(1)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
                source_spans: vec![],
                source_inline_ids: Vec::new(),
            },
            BasicBlock {
                label: BlockId(1),
                instructions: vec![],
                terminator: Terminator::Return(None),
                source_spans: vec![],
                source_inline_ids: Vec::new(),
            },
            BasicBlock {
                label: BlockId(2),
                instructions: vec![],
                terminator: Terminator::Return(None),
                source_spans: vec![],
                source_inline_ids: Vec::new(),
            },
        ]);
        let mut body = FxHashSet::default();
        body.insert(0);
        let natural_loop = crate::passes::loop_analysis::NaturalLoop { header: 0, body };
        let mut candidate_chain = FxHashSet::default();
        candidate_chain.insert(0);
        assert!(body_becomes_branch_free(
            &mut f.clone(),
            &natural_loop,
            usize::MAX,
            &candidate_chain,
            false,
        ));
        assert!(!body_becomes_branch_free(
            &mut f,
            &natural_loop,
            usize::MAX,
            &FxHashSet::default(),
            false,
        ));
    }

    #[test]
    fn cond_load_addr_expr_key_is_total_on_every_shape_class() {
        // Empty / one atom / many atoms, with and without an offset.  The old
        // implementation used `unreachable!` + `unwrap()` for the one-part
        // case; this locks the exhaustive, allocation-light replacement.
        assert_eq!(CondAddrExpr::constant(0).key(), "const(0)");
        assert_eq!(CondAddrExpr::atom("global(a)".into()).key(), "global(a)");
        assert_eq!(
            CondAddrExpr::atom("b".into())
                .add(CondAddrExpr::atom("a".into()), Value(9))
                .key(),
            "add(a,b)"
        );
        assert_eq!(CondAddrExpr::constant(24).key(), "add(const(24))");
        assert_eq!(
            CondAddrExpr::atom("global(a)".into())
                .add(CondAddrExpr::constant(24), Value(9))
                .key(),
            "add(const(24),global(a))"
        );
        let overflow = CondAddrExpr::constant(i128::MAX)
            .add(CondAddrExpr::constant(1), Value(7))
            .key();
        assert!(overflow.starts_with("opaque:overflow:7:"), "{overflow}");
    }

    fn masked_triangle(
        mask: i64,
        pattern: i64,
        op: IrCmpOp,
        arm_true: bool,
        cost: u32,
    ) -> IrFunction {
        let mut func = IrFunction::new("masked_update".into(), IrType::U64, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::ParamRef {
                    dest: Value(0),
                    param_idx: 0,
                    ty: IrType::U64,
                },
                Instruction::BinOp {
                    dest: Value(1),
                    op: IrBinOp::And,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I64(mask)),
                    ty: IrType::U64,
                },
                Instruction::Cmp {
                    dest: Value(2),
                    op,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I64(pattern)),
                    ty: IrType::U64,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(2)),
                true_label: BlockId(if arm_true { 1 } else { 2 }),
                false_label: BlockId(if arm_true { 2 } else { 1 }),
            },
            source_spans: vec![],
            source_inline_ids: Vec::new(),
        });
        let instructions = (0..cost)
            .map(|i| Instruction::BinOp {
                dest: Value(10 + i),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(if i == 0 { 0 } else { 9 + i })),
                rhs: Operand::Value(Value(0)),
                ty: IrType::U64,
            })
            .collect();
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions,
            terminator: Terminator::Branch(BlockId(2)),
            source_spans: vec![],
            source_inline_ids: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![Instruction::Phi {
                dest: Value(30),
                ty: IrType::U64,
                incoming: vec![
                    (Operand::Value(Value(0)), BlockId(0)),
                    (Operand::Value(Value(9 + cost)), BlockId(1)),
                ],
            }],
            terminator: Terminator::Return(Some(Operand::Value(Value(30)))),
            source_spans: vec![],
            source_inline_ids: Vec::new(),
        });
        func.next_value_id = 31;
        func
    }

    #[test]
    fn rare_masked_updates_keep_expensive_arms_but_not_cheap_selects() {
        for (op, rare) in [(IrCmpOp::Eq, true), (IrCmpOp::Ne, false)] {
            for mask in [255, 0x101010101010101, -1] {
                for cost in [2, 3, 6] {
                    let func = masked_triangle(mask, 0, op, rare, cost);
                    let ctx = IfConvCtx::build(&func);
                    assert_eq!(
                        rare_masked_outcome(&ctx, &Operand::Value(Value(2)), 8),
                        Some(rare)
                    );
                    assert_eq!(detect_triangle(&ctx, 0).is_some(), cost <= 2);
                    // The opposite edge has the usual budget, not the rare-edge budget.
                    let common = masked_triangle(mask, 0, op, !rare, cost);
                    assert!(detect_triangle(&IfConvCtx::build(&common), 0).is_some());
                }
            }
        }
        for mask in [1, 127] {
            let func = masked_triangle(mask, 0, IrCmpOp::Eq, true, 6);
            assert!(detect_triangle(&IfConvCtx::build(&func), 0).is_some());
        }
    }

    #[test]
    fn rare_pattern_prior_handles_truth_normalization_and_impossible_patterns() {
        let mut func = masked_triangle(255, 19, IrCmpOp::Eq, true, 6);
        func.blocks[0].instructions.push(Instruction::Cmp {
            dest: Value(3),
            op: IrCmpOp::Eq,
            lhs: Operand::Value(Value(2)),
            rhs: Operand::Const(IrConst::I32(0)),
            ty: IrType::I32,
        });
        let ctx = IfConvCtx::build(&func);
        assert_eq!(
            rare_masked_outcome(&ctx, &Operand::Value(Value(3)), 8),
            Some(false)
        );
        assert_eq!(
            rare_masked_outcome(&ctx, &Operand::Value(Value(3)), 0),
            None
        );
        let impossible = masked_triangle(255, 256, IrCmpOp::Eq, true, 6);
        assert_eq!(
            rare_masked_outcome(&IfConvCtx::build(&impossible), &Operand::Value(Value(2)), 8),
            None
        );
        let relational = masked_triangle(255, 0, IrCmpOp::Ult, true, 6);
        assert_eq!(
            rare_masked_outcome(&IfConvCtx::build(&relational), &Operand::Value(Value(2)), 8),
            None
        );
    }

    #[test]
    fn diamond_rare_arm_uses_the_same_speculation_budget() {
        let mut func = masked_triangle(255, 0, IrCmpOp::Eq, true, 6);
        if let Terminator::CondBranch { false_label, .. } = &mut func.blocks[0].terminator {
            *false_label = BlockId(3);
        }
        if let Instruction::Phi { incoming, .. } = &mut func.blocks[2].instructions[0] {
            incoming[0].1 = BlockId(3);
        }
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(2)),
            source_spans: vec![],
            source_inline_ids: Vec::new(),
        });
        assert!(detect_diamond(&IfConvCtx::build(&func), 0).is_none());
    }

    #[test]
    fn test_value_only_controls_branch() {
        // v used only as a CondBranch condition => branch-only.
        let mut func = IrFunction::new("t".to_string(), IrType::I32, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::ParamRef {
                dest: Value(0),
                param_idx: 0,
                ty: IrType::I32,
            }],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(0)),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        });
        let mut visited = FxHashSet::default();
        assert!(value_only_controls_branch(&func, Value(0), &mut visited));

        // v returned as data => not branch-only.
        func.blocks[0].terminator = Terminator::Return(Some(Operand::Value(Value(0))));
        let mut visited = FxHashSet::default();
        assert!(!value_only_controls_branch(&func, Value(0), &mut visited));

        // v used by Cmp(Eq, 0) whose result feeds a CondBranch => branch-only.
        let mut func2 = IrFunction::new("t2".to_string(), IrType::I32, vec![], false);
        func2.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::ParamRef {
                    dest: Value(0),
                    param_idx: 0,
                    ty: IrType::I32,
                },
                Instruction::Cmp {
                    dest: Value(1),
                    op: IrCmpOp::Eq,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I32(0)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(1)),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        });
        let mut visited = FxHashSet::default();
        assert!(value_only_controls_branch(&func2, Value(0), &mut visited));
    }

    #[test]
    fn test_skip_boolean_diamond_or_shape() {
        // `||` shape: true arm == const 1, result only feeds a branch => skip.
        let mut func = IrFunction::new("t".to_string(), IrType::I32, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::ParamRef {
                dest: Value(0),
                param_idx: 0,
                ty: IrType::I32,
            }],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(0)),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(3)),
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(3)),
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![Instruction::Phi {
                dest: Value(3),
                ty: IrType::I32,
                incoming: vec![
                    (Operand::Const(IrConst::I32(1)), BlockId(1)),
                    (Operand::Value(Value(4)), BlockId(2)),
                ],
            }],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(3)),
                true_label: BlockId(4),
                false_label: BlockId(5),
            },
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        });
        func.next_value_id = 6;

        // `||` shape, branch-only => skipped (not converted).
        let ctx = IfConvCtx::build(&func);
        let diamond = detect_diamond(&ctx, 0);
        assert!(
            diamond.is_none(),
            "branch-only `||` diamond must not be converted"
        );

        // `&&` shape (false arm == const 0) => still converted (range fold needs it).
        let mut func2 = IrFunction::new("t2".to_string(), IrType::I32, vec![], false);
        func2.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::ParamRef {
                dest: Value(0),
                param_idx: 0,
                ty: IrType::I32,
            }],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(0)),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        });
        func2.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(3)),
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        });
        func2.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(3)),
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        });
        func2.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![Instruction::Phi {
                dest: Value(3),
                ty: IrType::I32,
                incoming: vec![
                    (Operand::Value(Value(4)), BlockId(1)),
                    (Operand::Const(IrConst::I32(0)), BlockId(2)),
                ],
            }],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(3)),
                true_label: BlockId(4),
                false_label: BlockId(5),
            },
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        });
        func2.next_value_id = 6;
        let ctx2 = IfConvCtx::build(&func2);
        assert!(
            detect_diamond(&ctx2, 0).is_some(),
            "branch-only `&&` diamond must still be converted for range folding"
        );
    }

    #[test]
    fn test_simple_diamond_conversion() {
        // Build a simple diamond:
        //   block0: condbranch %0, block1, block2
        //   block1: branch block3
        //   block2: branch block3
        //   block3: %3 = phi [const(1), block1], [const(0), block2]; return %3
        let mut func = IrFunction::new("test".to_string(), IrType::I32, vec![], false);

        // Block 0: condbranch
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(0)),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        });

        // Block 1: true arm
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(3)),
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        });

        // Block 2: false arm
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(3)),
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        });

        // Block 3: merge with phi
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![Instruction::Phi {
                dest: Value(3),
                ty: IrType::I32,
                incoming: vec![
                    (Operand::Const(IrConst::I32(1)), BlockId(1)),
                    (Operand::Const(IrConst::I32(0)), BlockId(2)),
                ],
            }],
            terminator: Terminator::Return(Some(Operand::Value(Value(3)))),
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        });

        func.next_value_id = 4;

        let converted = if_convert_function(&mut func);
        assert_eq!(converted, 1);

        // Block 0 should now have a Select instruction and branch to block3
        assert_eq!(func.blocks[0].instructions.len(), 1);
        match &func.blocks[0].instructions[0] {
            Instruction::Select {
                dest,
                cond,
                true_val,
                false_val,
                ty,
            } => {
                assert_eq!(dest.0, 3);
                assert!(matches!(cond, Operand::Value(Value(0))));
                assert!(matches!(true_val, Operand::Const(IrConst::I32(1))));
                assert!(matches!(false_val, Operand::Const(IrConst::I32(0))));
                assert_eq!(*ty, IrType::I32);
            }
            other => panic!("Expected Select, got {:?}", other),
        }

        // Block 0 should now branch unconditionally to block3
        assert!(matches!(
            func.blocks[0].terminator,
            Terminator::Branch(BlockId(3))
        ));

        // Merge block should have no phi
        assert!(
            !func.blocks[3]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::Phi { .. }))
        );
    }

    #[test]
    fn test_diamond_with_arm_instructions() {
        // Diamond where the true arm computes a value:
        //   block0: condbranch %0, block1, block2
        //   block1: %1 = sub %0, const(5); branch block3
        //   block2: branch block3
        //   block3: %2 = phi [%1, block1], [const(0), block2]; return %2
        let mut func = IrFunction::new("test".to_string(), IrType::I32, vec![], false);

        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(0)),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        });

        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![Instruction::BinOp {
                dest: Value(1),
                op: IrBinOp::Sub,
                lhs: Operand::Value(Value(0)),
                rhs: Operand::Const(IrConst::I32(5)),
                ty: IrType::I32,
            }],
            terminator: Terminator::Branch(BlockId(3)),
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        });

        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(3)),
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        });

        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![Instruction::Phi {
                dest: Value(2),
                ty: IrType::I32,
                incoming: vec![
                    (Operand::Value(Value(1)), BlockId(1)),
                    (Operand::Const(IrConst::I32(0)), BlockId(2)),
                ],
            }],
            terminator: Terminator::Return(Some(Operand::Value(Value(2)))),
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        });

        func.next_value_id = 3;

        let converted = if_convert_function(&mut func);
        assert_eq!(converted, 1);

        // Block 0 should have the hoisted BinOp and the Select
        assert_eq!(func.blocks[0].instructions.len(), 2);
        assert!(matches!(
            func.blocks[0].instructions[0],
            Instruction::BinOp { .. }
        ));
        assert!(matches!(
            func.blocks[0].instructions[1],
            Instruction::Select { .. }
        ));
    }

    /// Two functions carrying the IDENTICAL 4-effective-instruction diamond:
    /// one straight-line, one loop-resident.
    ///
    ///   * straight-line: the diamond executes once per call, so the branch
    ///     costs at most one mispredict and the coin-flip arm budget (>= 4 on
    ///     every tuning row) admits the conversion;
    ///   * loop-resident: the same speculation would execute on every
    ///     iteration *and* keep the arm's operands live across the loop body,
    ///     so the loop budget (3 on every row) keeps it a branch.
    ///
    /// This is the reduced form of `hot_web_loop`, where if-converting a
    /// 6-instruction rare arm turned a 16-instruction loop into 45
    /// instructions with four `cmov`s and six stack reloads per iteration.
    /// The assertion on the straight-line twin is what proves the budget is
    /// conditioned on loop nesting rather than merely lowered.
    #[test]
    fn loop_resident_diamond_uses_the_in_loop_arm_budget() {
        // Four effective instructions: excluded classifications (GEP, Copy,
        // GlobalAddr, Shl) do not apply, so the arm length is exactly 4.
        fn four_inst_arm(base: Value) -> (Vec<Instruction>, Value) {
            let ops = [IrBinOp::Add, IrBinOp::Sub, IrBinOp::Xor, IrBinOp::And];
            let mut insts = Vec::new();
            let mut cur = base;
            for (i, op) in ops.into_iter().enumerate() {
                let dest = Value(base.0 + 1 + i as u32);
                insts.push(Instruction::BinOp {
                    dest,
                    op,
                    lhs: Operand::Value(cur),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                });
                cur = dest;
            }
            (insts, cur)
        }
        fn build(in_loop: bool) -> IrFunction {
            let mut func = IrFunction::new("test".to_string(), IrType::I32, vec![], false);
            let arm_len = 4u32;
            // b0: entry
            func.blocks.push(BasicBlock {
                label: BlockId(0),
                instructions: vec![],
                terminator: Terminator::Branch(BlockId(1)),
                source_spans: Vec::new(),
                source_inline_ids: Vec::new(),
            });
            // b1: diamond head (also the loop header when loop-resident)
            func.blocks.push(BasicBlock {
                label: BlockId(1),
                instructions: vec![],
                terminator: Terminator::CondBranch {
                    cond: Operand::Value(Value(0)),
                    true_label: BlockId(2),
                    false_label: BlockId(3),
                },
                source_spans: Vec::new(),
                source_inline_ids: Vec::new(),
            });
            // b2: true arm
            let (arm, arm_val) = four_inst_arm(Value(0));
            func.blocks.push(BasicBlock {
                label: BlockId(2),
                instructions: arm,
                terminator: Terminator::Branch(BlockId(4)),
                source_spans: Vec::new(),
                source_inline_ids: Vec::new(),
            });
            // b3: false arm
            func.blocks.push(BasicBlock {
                label: BlockId(3),
                instructions: vec![],
                terminator: Terminator::Branch(BlockId(4)),
                source_spans: Vec::new(),
                source_inline_ids: Vec::new(),
            });
            // b4: merge, then either back to the header (loop) or out.
            func.blocks.push(BasicBlock {
                label: BlockId(4),
                instructions: vec![Instruction::Phi {
                    dest: Value(arm_len + 1),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Value(arm_val), BlockId(2)),
                        (Operand::Const(IrConst::I32(0)), BlockId(3)),
                    ],
                }],
                terminator: if in_loop {
                    Terminator::Branch(BlockId(1))
                } else {
                    Terminator::Return(Some(Operand::Value(Value(arm_len + 1))))
                },
                source_spans: Vec::new(),
                source_inline_ids: Vec::new(),
            });
            func.next_value_id = arm_len + 2;
            func
        }

        // Straight-line: converts.
        let mut straight = build(false);
        assert_eq!(if_convert_function(&mut straight), 1);
        assert!(
            straight.blocks[1]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::Select { .. }))
        );

        // Loop-resident: refused, and the branch is still the terminator.
        let mut looped = build(true);
        assert_eq!(
            if_convert_function(&mut looped),
            0,
            "loop-resident 4-instruction arm must not be if-converted"
        );
        assert!(
            looped.blocks[1]
                .instructions
                .iter()
                .all(|i| !matches!(i, Instruction::Select { .. }))
        );
        assert!(matches!(
            looped.blocks[1].terminator,
            Terminator::CondBranch { .. }
        ));
    }

    #[test]
    fn test_no_conversion_with_side_effects() {
        // Diamond where the true arm has a store (side effect) - should NOT convert
        let mut func = IrFunction::new("test".to_string(), IrType::I32, vec![], false);

        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::Alloca {
                dest: Value(10),
                ty: IrType::I32,
                size: 4,
                align: 4,
                volatile: false,
                semantic_volatile: false,
            }],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(0)),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        });

        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                // Side-effecting store!
                Instruction::Store {
                    volatile: false,
                    val: Operand::Const(IrConst::I32(42)),
                    ptr: Value(10),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                },
            ],
            terminator: Terminator::Branch(BlockId(3)),
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        });

        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(3)),
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        });

        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![Instruction::Phi {
                dest: Value(2),
                ty: IrType::I32,
                incoming: vec![
                    (Operand::Const(IrConst::I32(1)), BlockId(1)),
                    (Operand::Const(IrConst::I32(0)), BlockId(2)),
                ],
            }],
            terminator: Terminator::Return(Some(Operand::Value(Value(2)))),
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        });

        func.next_value_id = 11;

        let converted = if_convert_function(&mut func);
        assert_eq!(converted, 0); // Should NOT convert due to side effects
    }

    /// Helper: one float array parameter reference in block `bi`.
    fn float_param(dest: u32, param_idx: usize) -> Instruction {
        Instruction::ParamRef {
            dest: Value(dest),
            param_idx,
            ty: IrType::F32,
        }
    }

    /// Helper: `dest = shl(src, 2)` (byte scale for a float array index).
    fn shl2(dest: u32, src: u32) -> Instruction {
        Instruction::BinOp {
            dest: Value(dest),
            op: IrBinOp::Shl,
            lhs: Operand::Value(Value(src)),
            rhs: Operand::Const(IrConst::I32(2)),
            ty: IrType::I64,
        }
    }

    /// Helper: `dest = gep(base, offset)`.
    fn gep(dest: u32, base: u32, offset: u32) -> Instruction {
        Instruction::GetElementPtr {
            dest: Value(dest),
            base: Value(base),
            offset: Operand::Value(Value(offset)),
            ty: IrType::F32,
        }
    }

    fn block(label: u32, insts: Vec<Instruction>, term: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions: insts,
            terminator: term,
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        }
    }

    /// A canonical loop: entry -> header(iv phi + exit test) -> body -> latch,
    /// with a ternary diamond inside the body whose arms load `c[i]`.
    /// `sibling_covered = true` loads c[i] in BOTH arms (the p31-sign-apply
    /// shape): every path P→M dereferences the c[i] address, so the path-
    /// coverage gate admits the load and the diamond converts.  `false`
    /// loads c[i] only in the true arm — the scalar never touches c[i] on
    /// the false path, so speculating it could fault where the source
    /// cannot, and the gate must refuse.
    fn arm_load_diamond_loop(sibling_covered: bool) -> IrFunction {
        let mut f = IrFunction::new("t".to_string(), IrType::I32, vec![], false);
        // b0: entry — params d,a,b,c,n + branch to header.
        f.blocks.push(block(
            0,
            vec![
                float_param(0, 0), // d
                float_param(1, 1), // a
                float_param(2, 2), // b
                float_param(3, 3), // c
                Instruction::ParamRef {
                    dest: Value(4),
                    param_idx: 4,
                    ty: IrType::U64,
                }, // n
            ],
            Terminator::Branch(BlockId(1)),
        ));
        // b1: header — iv phi, exit test, CondBranch(body, exit).
        f.blocks.push(block(
            1,
            vec![
                Instruction::Phi {
                    dest: Value(10),
                    ty: IrType::U64,
                    incoming: vec![
                        (Operand::Const(IrConst::I64(0)), BlockId(0)),
                        (Operand::Value(Value(11)), BlockId(5)),
                    ],
                },
                Instruction::Cmp {
                    dest: Value(12),
                    op: IrCmpOp::Ult,
                    lhs: Operand::Value(Value(10)),
                    rhs: Operand::Value(Value(4)),
                    ty: IrType::U64,
                },
            ],
            Terminator::CondBranch {
                cond: Operand::Value(Value(12)),
                true_label: BlockId(2),
                false_label: BlockId(6),
            },
        ));
        // b2: body pred — load a[i], load b[i], FP compare, CondBranch.
        f.blocks.push(block(
            2,
            vec![
                shl2(20, 10),
                gep(21, 1, 20),
                Instruction::Load {
                    dest: Value(22),
                    ptr: Value(21),
                    ty: IrType::F32,
                    volatile: false,
                    seg_override: AddressSpace::Default,
                },
                shl2(23, 10),
                gep(24, 2, 23),
                Instruction::Load {
                    dest: Value(25),
                    ptr: Value(24),
                    ty: IrType::F32,
                    volatile: false,
                    seg_override: AddressSpace::Default,
                },
                Instruction::Cmp {
                    dest: Value(26),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(22)),
                    rhs: Operand::Value(Value(25)),
                    ty: IrType::F32,
                },
            ],
            Terminator::CondBranch {
                cond: Operand::Value(Value(26)),
                true_label: BlockId(3),
                false_label: BlockId(4),
            },
        ));
        // b3: true arm — load c[i].
        f.blocks.push(block(
            3,
            vec![
                shl2(30, 10),
                gep(31, 3, 30),
                Instruction::Load {
                    dest: Value(32),
                    ptr: Value(31),
                    ty: IrType::F32,
                    volatile: false,
                    seg_override: AddressSpace::Default,
                },
            ],
            Terminator::Branch(BlockId(5)),
        ));
        // b4: false arm — covered: load c[i] again (fresh SSA); uncovered:
        // reuse the pred's b[i] value.
        let false_incoming = if sibling_covered {
            f.blocks.push(block(
                4,
                vec![
                    shl2(33, 10),
                    gep(34, 3, 33),
                    Instruction::Load {
                        dest: Value(35),
                        ptr: Value(34),
                        ty: IrType::F32,
                        volatile: false,
                        seg_override: AddressSpace::Default,
                    },
                ],
                Terminator::Branch(BlockId(5)),
            ));
            Operand::Value(Value(35))
        } else {
            f.blocks
                .push(block(4, vec![], Terminator::Branch(BlockId(5))));
            Operand::Value(Value(25))
        };
        // b5: merge — phi for d[i], store d[i], iv increment, backedge.
        f.blocks.push(block(
            5,
            vec![
                Instruction::Phi {
                    dest: Value(40),
                    ty: IrType::F32,
                    incoming: vec![
                        (Operand::Value(Value(32)), BlockId(3)),
                        (false_incoming, BlockId(4)),
                    ],
                },
                shl2(41, 10),
                gep(42, 0, 41),
                Instruction::Store {
                    val: Operand::Value(Value(40)),
                    ptr: Value(42),
                    ty: IrType::F32,
                    volatile: false,
                    seg_override: AddressSpace::Default,
                },
                Instruction::BinOp {
                    dest: Value(11),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(10)),
                    rhs: Operand::Const(IrConst::I64(1)),
                    ty: IrType::U64,
                },
            ],
            Terminator::Branch(BlockId(1)),
        ));
        // b6: exit.
        f.blocks.push(block(6, vec![], Terminator::Return(None)));
        f.next_value_id = 50;
        f
    }

    #[test]
    fn test_sibling_covered_arm_load_converts() {
        // Both arms dereference c[i]: every P→M path touches the address,
        // so the unconditional hoist is trap-equivalent and must convert.
        let mut f = arm_load_diamond_loop(true);
        let converted = if_convert_function(&mut f);
        assert!(converted > 0, "covered arm load diamond must convert");
        assert!(
            f.blocks[2]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::Select { .. })),
            "expected a Select in the pred block"
        );
        assert!(f.blocks[3].instructions.is_empty());
        assert!(f.blocks[4].instructions.is_empty());
        assert!(
            !f.blocks[5]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::Phi { dest: d, .. } if d.0 == 40))
        );
    }

    #[test]
    fn a_store_does_not_prove_an_address_readable() {
        let mut f = IrFunction::new(
            "write_then_maybe_read".to_string(),
            IrType::I32,
            vec![],
            false,
        );
        f.blocks.push(block(
            0,
            vec![
                Instruction::ParamRef {
                    dest: Value(0),
                    param_idx: 0,
                    ty: IrType::Ptr,
                },
                Instruction::ParamRef {
                    dest: Value(1),
                    param_idx: 1,
                    ty: IrType::I32,
                },
            ],
            Terminator::Branch(BlockId(1)),
        ));
        f.blocks.push(block(
            1,
            vec![Instruction::Store {
                ptr: Value(0),
                val: Operand::Const(IrConst::I32(9)),
                ty: IrType::I32,
                volatile: false,
                seg_override: AddressSpace::Default,
            }],
            Terminator::CondBranch {
                cond: Operand::Value(Value(1)),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
        ));
        f.blocks.push(block(
            2,
            vec![Instruction::Load {
                dest: Value(2),
                ptr: Value(0),
                ty: IrType::I32,
                volatile: false,
                seg_override: AddressSpace::Default,
            }],
            Terminator::Branch(BlockId(3)),
        ));
        f.blocks.push(block(
            3,
            vec![Instruction::Phi {
                dest: Value(3),
                ty: IrType::I32,
                incoming: vec![
                    (Operand::Value(Value(2)), BlockId(2)),
                    (Operand::Const(IrConst::I32(0)), BlockId(1)),
                ],
            }],
            Terminator::Return(Some(Operand::Value(Value(3)))),
        ));
        f.next_value_id = 5;
        let ctx = IfConvCtx::build(&f);
        assert!(!arm_load_speculation_ok(&ctx, 1, None, &Value(0), 4));
        assert!(detect_triangle(&ctx, 1).is_none());

        // A genuine prior read covers the same bytes and retains the useful
        // if-conversion, even when the block also contains a store.
        f.blocks[1].instructions.push(Instruction::Load {
            dest: Value(4),
            ptr: Value(0),
            ty: IrType::I32,
            volatile: false,
            seg_override: AddressSpace::Default,
        });
        let ctx = IfConvCtx::build(&f);
        assert!(arm_load_speculation_ok(&ctx, 1, None, &Value(0), 4));
        assert!(detect_triangle(&ctx, 1).is_some());
    }

    #[test]
    fn test_uncovered_arm_load_does_not_convert() {
        // c[i] is dereferenced ONLY on the true path. Speculating it would
        // read addresses the source program never touches on the false path
        // (a fault the scalar cannot produce) — the gate must refuse and the
        // loop must stay branchy. This is GCC's vmaskmovps shape.
        let mut f = arm_load_diamond_loop(false);
        let converted = if_convert_function(&mut f);
        assert_eq!(converted, 0, "uncovered arm load must not be speculated");
        assert!(
            !f.blocks
                .iter()
                .flat_map(|b| &b.instructions)
                .any(|i| matches!(i, Instruction::Select { .. }))
        );
    }

    /// R2 regression at unit level: the canonical address key must be
    /// INJECTIVE over addresses. The original offset walk collapsed through
    /// `Shl` without recording the shift constant, mapping `d[i]`
    /// (offset iv<<2) and `d[2*i]` (offset iv<<3) to the SAME key —
    /// rewrite_covered_arm_loads then replaced the arm's `d[2*i]` load with
    /// the pred's `d[i]` value, and sink_conditional_stores merged stores
    /// to the two different addresses into one.
    #[test]
    fn canonical_addr_key_distinguishes_shift_scales() {
        let mut f = IrFunction::new("t".to_string(), IrType::I32, vec![], false);
        f.blocks.push(block(
            0,
            vec![
                float_param(0, 0), // d
                float_param(1, 1), // d2 (same base via distinct param refs below)
                Instruction::ParamRef {
                    dest: Value(2),
                    param_idx: 2,
                    ty: IrType::U64,
                }, // iv
                // off2 = iv << 2
                shl2(10, 2),
                // off3 = iv << 3 (same root, different scale)
                Instruction::BinOp {
                    dest: Value(11),
                    op: IrBinOp::Shl,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Const(IrConst::I32(3)),
                    ty: IrType::I64,
                },
                // off2b = iv << 2 again (fresh SSA — the two-arms shape)
                shl2(12, 2),
                Instruction::GetElementPtr {
                    dest: Value(20),
                    base: Value(0),
                    offset: Operand::Value(Value(10)),
                    ty: IrType::F32,
                },
                Instruction::GetElementPtr {
                    dest: Value(21),
                    base: Value(0),
                    offset: Operand::Value(Value(11)),
                    ty: IrType::F32,
                },
                Instruction::GetElementPtr {
                    dest: Value(22),
                    base: Value(0),
                    offset: Operand::Value(Value(12)),
                    ty: IrType::F32,
                },
            ],
            Terminator::Return(None),
        ));
        f.next_value_id = 30;
        let defs: FxHashMap<u32, Instruction> = f
            .blocks
            .iter()
            .flat_map(|b| b.instructions.iter())
            .filter_map(|i| i.dest().map(|d| (d.0, i.clone())))
            .collect();
        let copy_of = FxHashMap::default();
        let k_i = canonical_addr_key_impl(&defs, &copy_of, &Value(20));
        let k_2i = canonical_addr_key_impl(&defs, &copy_of, &Value(21));
        let k_i_again = canonical_addr_key_impl(&defs, &copy_of, &Value(22));
        assert_ne!(k_i, k_2i, "d[i] and d[2*i] must NOT share a canonical key");
        assert_eq!(
            k_i, k_i_again,
            "two arms' d[i] GEPs (fresh Shl SSAs, same scale) must share the key"
        );
    }

    /// Nested clamp: outer `a < 0` diamond whose false arm holds the inner
    /// `a > 1` TRIANGLE; the merge block has three predecessors (outer true
    /// arm, inner arm, inner pred).  The fixpoint must convert the inner
    /// triangle PARTIALLY (keeping the outer arm's phi entry), then the
    /// outer diamond fully — producing two nested Selects.
    #[test]
    fn test_partial_merge_phi_nested_clamp() {
        let mut f = IrFunction::new("t".to_string(), IrType::I32, vec![], false);
        // b0: entry.
        f.blocks.push(block(
            0,
            vec![float_param(0, 0)],
            Terminator::Branch(BlockId(1)),
        ));
        // b1: outer pred — load a (as a param value, v0), cmp a<0.
        f.blocks.push(block(
            1,
            vec![Instruction::Cmp {
                dest: Value(1),
                op: IrCmpOp::Slt,
                lhs: Operand::Value(Value(0)),
                rhs: Operand::Const(IrConst::F32(0.0)),
                ty: IrType::F32,
            }],
            Terminator::CondBranch {
                cond: Operand::Value(Value(1)),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
        ));
        // b2: outer true arm — 0.0.
        f.blocks
            .push(block(2, vec![], Terminator::Branch(BlockId(6))));
        // b3: inner pred — cmp a>1; triangle: true arm b4, false edge direct.
        f.blocks.push(block(
            3,
            vec![Instruction::Cmp {
                dest: Value(2),
                op: IrCmpOp::Sgt,
                lhs: Operand::Value(Value(0)),
                rhs: Operand::Const(IrConst::F32(1.0)),
                ty: IrType::F32,
            }],
            Terminator::CondBranch {
                cond: Operand::Value(Value(2)),
                true_label: BlockId(4),
                false_label: BlockId(6),
            },
        ));
        // b4: inner true arm — 1.0.
        f.blocks
            .push(block(4, vec![], Terminator::Branch(BlockId(6))));
        // b5: unused intermediate (keeps label ids realistic).
        // b6: merge — phi with THREE incoming, return it.
        f.blocks.push(block(
            6,
            vec![Instruction::Phi {
                dest: Value(5),
                ty: IrType::F32,
                incoming: vec![
                    (Operand::Const(IrConst::F32(0.0)), BlockId(2)),
                    (Operand::Const(IrConst::F32(1.0)), BlockId(4)),
                    (Operand::Value(Value(0)), BlockId(3)),
                ],
            }],
            Terminator::Return(Some(Operand::Value(Value(5)))),
        ));
        f.next_value_id = 10;

        let converted = if_convert_function(&mut f);
        assert!(converted >= 2, "nested clamp needs two conversions");
        let selects = f
            .blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .filter(|i| matches!(i, Instruction::Select { .. }))
            .count();
        assert_eq!(selects, 2, "expected two nested Selects");
        assert!(
            f.blocks
                .iter()
                .all(|b| !matches!(b.terminator, Terminator::CondBranch { .. }))
        );
        // The outer pred (block 1) must hold the final nested Select.
        assert!(
            f.blocks[1]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::Select { .. }))
        );
    }

    /// Store sinking: two predecessors store to the same IV-addressed slot;
    /// the sink must turn them into a phi plus ONE store in the merge.
    #[test]
    fn test_store_sinking_merges_conditional_stores() {
        let mut f = IrFunction::new("t".to_string(), IrType::I32, vec![], false);
        // b0: entry — d param, iv param, cond param, branch to pred.
        f.blocks.push(block(
            0,
            vec![
                float_param(0, 0), // d
                Instruction::ParamRef {
                    dest: Value(1),
                    param_idx: 1,
                    ty: IrType::U64,
                }, // iv
                Instruction::ParamRef {
                    dest: Value(2),
                    param_idx: 2,
                    ty: IrType::I32,
                }, // cond
            ],
            Terminator::Branch(BlockId(1)),
        ));
        // b1: pred — CondBranch to the two store arms.
        f.blocks.push(block(
            1,
            vec![],
            Terminator::CondBranch {
                cond: Operand::Value(Value(2)),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
        ));
        // b2: arm 1 — store 0.0f to d[iv].
        f.blocks.push(block(
            2,
            vec![
                Instruction::Cast {
                    dest: Value(9),
                    src: Operand::Value(Value(1)),
                    from_ty: IrType::U64,
                    to_ty: IrType::I64,
                },
                shl2(10, 9),
                gep(11, 0, 10),
                Instruction::Store {
                    val: Operand::Const(IrConst::F32(0.0)),
                    ptr: Value(11),
                    ty: IrType::F32,
                    volatile: false,
                    seg_override: AddressSpace::Default,
                },
            ],
            Terminator::Branch(BlockId(4)),
        ));
        // b3: arm 2 — store 1.0f to d[iv] (separate GEP SSA ids).
        f.blocks.push(block(
            3,
            vec![
                Instruction::Cast {
                    dest: Value(12),
                    src: Operand::Value(Value(1)),
                    from_ty: IrType::U64,
                    to_ty: IrType::I64,
                },
                shl2(13, 12),
                gep(14, 0, 13),
                Instruction::Store {
                    val: Operand::Const(IrConst::F32(1.0)),
                    ptr: Value(14),
                    ty: IrType::F32,
                    volatile: false,
                    seg_override: AddressSpace::Default,
                },
            ],
            Terminator::Branch(BlockId(4)),
        ));
        // b4: merge.
        f.blocks.push(block(4, vec![], Terminator::Return(None)));
        f.next_value_id = 20;

        let sunk = sink_conditional_stores(&mut f);
        assert_eq!(sunk, 2, "both per-pred stores must sink");
        assert!(!matches!(
            f.blocks[2].instructions.last(),
            Some(Instruction::Store { .. })
        ));
        assert!(!matches!(
            f.blocks[3].instructions.last(),
            Some(Instruction::Store { .. })
        ));
        let b4 = &f.blocks[4];
        assert!(
            b4.instructions
                .iter()
                .any(|i| matches!(i, Instruction::Phi { .. })),
            "merge block must hold the value phi"
        );
        let stores = b4
            .instructions
            .iter()
            .filter(|i| matches!(i, Instruction::Store { .. }))
            .count();
        assert_eq!(stores, 1, "exactly one store must remain");
        // The store must sit AFTER the phi and its cloned address chain.
        let phi_pos = b4
            .instructions
            .iter()
            .position(|i| matches!(i, Instruction::Phi { .. }))
            .unwrap();
        let store_pos = b4
            .instructions
            .iter()
            .position(|i| matches!(i, Instruction::Store { .. }))
            .unwrap();
        assert!(phi_pos < store_pos);

        // The rebuilt Cast -> Shl -> GEP chain must be strict SSA: every
        // destination is unique and the source scale remains exactly 2.
        // Allocating a parent ID only after recursively cloning its child made
        // all three definitions share one ID; simplify then doubled the shift
        // on each pipeline iteration (2 -> 4 -> 8 -> 16).
        let mut dests = FxHashSet::default();
        for inst in &b4.instructions {
            if let Some(dest) = inst.dest() {
                assert!(
                    dests.insert(dest),
                    "duplicate cloned SSA destination v{}",
                    dest.0
                );
            }
        }
        assert!(b4.instructions.iter().any(|inst| matches!(
            inst,
            Instruction::BinOp {
                op: IrBinOp::Shl,
                rhs: Operand::Const(IrConst::I32(2)),
                ..
            }
        )));
    }

    /// The sunk store's address chain must be available AT THE MERGE.
    ///
    /// Canonical lowering emits a fresh `GlobalAddr` per source reference, so
    /// each switch arm owns its own; the chain cloner used to treat a
    /// `GlobalAddr` leaf as "dominating by construction" and kept the arm's
    /// original.  With four arms reaching the join, the arm that owned it does
    /// not dominate the merge, and the store this pass moved into the merge read
    /// a value that only one path defined — the def-dominates-use violation
    /// `CCC_VERIFY_IR` reports on `tests/regression/minmax_refused.c`.
    /// Rematerialising the address in the merge is the fix.  A `ParamRef`
    /// must instead reuse its entry-block definition when that dominates the
    /// merge; it cannot be rematerialised outside the ABI-read entry block.
    #[test]
    fn sunk_store_address_is_available_at_the_merge() {
        use crate::common::types::{AddressSpace, IrType};
        use crate::ir::reexports::{BasicBlock, BlockId, IrFunction};

        let mut func = IrFunction::new("sink".into(), IrType::Void, vec![], false);
        func.next_label = 4; // fixture bookkeeping: labels 0..=3 are in use
        func.next_value_id = 100;
        let arm =
            |label: u32, gid: u32, off_id: u32, sh_id: u32, gep_id: u32, val: i32| BasicBlock {
                label: BlockId(label),
                instructions: vec![
                    Instruction::GlobalAddr {
                        dest: Value(gid),
                        name: "a".into(),
                    },
                    Instruction::Cast {
                        dest: Value(off_id),
                        src: Operand::Value(Value(9)),
                        from_ty: IrType::I32,
                        to_ty: IrType::I64,
                    },
                    Instruction::BinOp {
                        dest: Value(sh_id),
                        op: IrBinOp::Shl,
                        lhs: Operand::Value(Value(off_id)),
                        rhs: Operand::Const(IrConst::I64(2)),
                        ty: IrType::I64,
                    },
                    Instruction::GetElementPtr {
                        dest: Value(gep_id),
                        base: Value(gid),
                        offset: Operand::Value(Value(sh_id)),
                        ty: IrType::I32,
                    },
                    Instruction::Store {
                        val: Operand::Const(IrConst::I32(val)),
                        ptr: Value(gep_id),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                        volatile: false,
                    },
                ],
                terminator: Terminator::Branch(BlockId(3)),
                source_spans: Vec::new(),
                source_inline_ids: Vec::new(),
            };

        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::ParamRef {
                    dest: Value(9),
                    param_idx: 0,
                    ty: IrType::I32,
                },
                Instruction::Cmp {
                    dest: Value(10),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(9)),
                    rhs: Operand::Const(IrConst::I32(4)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(10)),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        });
        func.blocks.push(arm(1, 20, 21, 22, 23, 1));
        func.blocks.push(arm(2, 30, 31, 32, 33, 2));
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        });

        let sunk = sink_conditional_stores(&mut func);
        assert!(
            sunk > 0,
            "the two identical arm stores must sink into the join"
        );

        let mut violations = Vec::new();
        crate::passes::verify::verify_function(&func, "if_convert_sink_test", &mut violations);
        assert!(
            violations.is_empty(),
            "sunk store left invalid IR: {violations:#?}"
        );
    }
}

#[cfg(test)]
mod shared_arm_transaction_tests {
    use super::*;
    use crate::ir::reexports::IrConst;

    fn fixture() -> (IrFunction, SharedArmDup) {
        let mut f = IrFunction::new("shared_arm_transaction".into(), IrType::I64, vec![], false);
        let block = |label, instructions, terminator| BasicBlock {
            label: BlockId(label),
            instructions,
            terminator,
            source_spans: vec![],
            source_inline_ids: Vec::new(),
        };
        f.blocks = vec![
            block(
                0,
                vec![],
                Terminator::CondBranch {
                    cond: Operand::Const(IrConst::I64(1)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            block(
                1,
                vec![Instruction::Copy {
                    dest: Value(0),
                    src: Operand::Const(IrConst::I64(42)),
                }],
                Terminator::Branch(BlockId(3)),
            ),
            block(
                2,
                vec![Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Const(IrConst::I64(7)),
                }],
                Terminator::Branch(BlockId(3)),
            ),
            block(
                3,
                (2..4)
                    .map(|id| Instruction::Phi {
                        dest: Value(id),
                        ty: IrType::I64,
                        incoming: vec![
                            (Operand::Value(Value(0)), BlockId(1)),
                            (Operand::Value(Value(1)), BlockId(2)),
                        ],
                    })
                    .collect(),
                Terminator::Return(Some(Operand::Value(Value(2)))),
            ),
        ];
        f.next_value_id = 4;
        f.next_label = 4;
        (
            f,
            SharedArmDup {
                h_idx: 0,
                h_edge_is_true: true,
                s_idx: 1,
            },
        )
    }

    #[test]
    fn invalid_plans_and_capacity_failures_are_transactional() {
        for case in 0..10 {
            let (mut f, mut plan) = fixture();
            match case {
                0 => f.blocks[1].terminator = Terminator::Branch(BlockId(99)),
                1 | 2 => {
                    if let Instruction::Phi { incoming, .. } =
                        &mut f.blocks[3].instructions[case - 1]
                    {
                        incoming.remove(0);
                    }
                }
                3 => f.next_value_id = u32::MAX,
                4 => f.next_label = u32::MAX,
                5 => plan.h_idx = usize::MAX,
                6 => plan.s_idx = usize::MAX,
                7 => f.next_label = 3,
                8 => {
                    if let Instruction::Phi { incoming, .. } = &mut f.blocks[3].instructions[0] {
                        incoming.push(incoming[0]);
                    }
                }
                _ => plan.h_edge_is_true = false,
            }
            let before = format!("{f:?}");
            assert!(!apply_shared_arm_dup(&mut f, &plan), "case={case}");
            assert_eq!(format!("{f:?}"), before, "partial mutation in case={case}");
        }
    }

    #[test]
    fn success_repairs_all_phis_and_only_the_selected_head_edge() {
        for true_edge in [false, true] {
            let (mut f, mut plan) = fixture();
            if !true_edge {
                f.blocks[0].terminator = Terminator::CondBranch {
                    cond: Operand::Const(IrConst::I64(1)),
                    true_label: BlockId(2),
                    false_label: BlockId(1),
                };
                plan.h_edge_is_true = false;
            }
            assert!(apply_shared_arm_dup(&mut f, &plan));
            assert_eq!((f.next_value_id, f.next_label, f.blocks.len()), (5, 5, 5));
            for inst in &f.blocks[3].instructions {
                let Instruction::Phi { incoming, .. } = inst else {
                    panic!("expected phi")
                };
                assert!(incoming.contains(&(Operand::Value(Value(4)), BlockId(4))));
            }
            let mut violations = Vec::new();
            crate::passes::verify::verify_function(&f, "shared_arm_transaction", &mut violations);
            assert!(violations.is_empty(), "{violations:?}");
        }
    }
}

#[cfg(test)]
mod cond_load_sink_tests {
    use super::*;
    use crate::common::types::AddressSpace;
    use crate::ir::reexports::IrConst;

    fn block(label: u32, insts: Vec<Instruction>, term: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions: insts,
            terminator: term,
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        }
    }

    // ── conditional-load sinking (SV-03) ────────────────────────────────
    //
    // Canonical `r = cond ? *p : *q` diamond with FOUR blocks.  Every test
    // mutates one invariant and asserts the transform refuses it, so a
    // relaxed legality check cannot land silently.

    fn cond_load_diamond() -> IrFunction {
        let mut f = IrFunction::new("cond_load".into(), IrType::Void, vec![], false);
        f.blocks.push(block(
            0,
            vec![
                Instruction::ParamRef {
                    dest: Value(0),
                    param_idx: 0,
                    ty: IrType::Ptr,
                },
                Instruction::ParamRef {
                    dest: Value(1),
                    param_idx: 1,
                    ty: IrType::Ptr,
                },
                Instruction::ParamRef {
                    dest: Value(2),
                    param_idx: 2,
                    ty: IrType::I32,
                },
                Instruction::ParamRef {
                    dest: Value(3),
                    param_idx: 3,
                    ty: IrType::Ptr,
                },
            ],
            Terminator::Branch(BlockId(1)),
        ));
        f.blocks.push(block(
            1,
            vec![Instruction::Cmp {
                dest: Value(4),
                op: IrCmpOp::Ne,
                lhs: Operand::Value(Value(2)),
                rhs: Operand::Const(IrConst::I32(0)),
                ty: IrType::I32,
            }],
            Terminator::CondBranch {
                cond: Operand::Value(Value(4)),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
        ));
        for (label, base, gep, loaded) in [(2u32, 0u32, 10u32, 11u32), (3, 1, 20, 21)] {
            f.blocks.push(block(
                label,
                vec![
                    Instruction::GetElementPtr {
                        dest: Value(gep),
                        base: Value(base),
                        offset: Operand::Const(IrConst::I64(0)),
                        ty: IrType::F32,
                    },
                    Instruction::Load {
                        dest: Value(loaded),
                        ptr: Value(gep),
                        ty: IrType::F32,
                        seg_override: AddressSpace::Default,
                        volatile: false,
                    },
                ],
                Terminator::Branch(BlockId(4)),
            ));
        }
        f.blocks.push(block(
            4,
            vec![
                Instruction::Phi {
                    dest: Value(30),
                    ty: IrType::F32,
                    incoming: vec![
                        (Operand::Value(Value(11)), BlockId(2)),
                        (Operand::Value(Value(21)), BlockId(3)),
                    ],
                },
                Instruction::Store {
                    val: Operand::Value(Value(30)),
                    ptr: Value(3),
                    ty: IrType::F32,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
            ],
            Terminator::Return(None),
        ));
        // Leave headroom: sound_next_value_id() must not collide with the
        // auxiliary Phi (V(31)) some tests insert.
        f.next_value_id = 41;
        f.next_label = 5;
        f
    }

    #[test]
    fn cond_load_sink_selects_the_address_and_keeps_one_load() {
        let mut f = cond_load_diamond();
        assert_eq!(sink_conditional_loads(&mut f), 1);
        // pred: hoisted GEP from each arm, then ONE pointer Select.
        let pred = &f.blocks[1].instructions;
        assert_eq!(pred.len(), 4, "{pred:?}");
        assert!(matches!(pred[0], Instruction::Cmp { .. }));
        assert!(matches!(pred[1], Instruction::GetElementPtr { .. }));
        assert!(matches!(pred[2], Instruction::GetElementPtr { .. }));
        let sel = match &pred[3] {
            Instruction::Select {
                dest,
                ty,
                true_val,
                false_val,
                ..
            } => {
                assert_eq!(*ty, IrType::Ptr);
                assert_eq!(*true_val, Operand::Value(Value(10)));
                assert_eq!(*false_val, Operand::Value(Value(20)));
                *dest
            }
            other => panic!("expected pointer Select, got {other:?}"),
        };
        assert!(matches!(
            f.blocks[1].terminator,
            Terminator::Branch(BlockId(4))
        ));
        // merge: the Phi became a Load from the selected pointer, same dest.
        match &f.blocks[4].instructions[0] {
            Instruction::Load {
                dest,
                ptr,
                ty,
                volatile,
                ..
            } => {
                assert_eq!(*dest, Value(30));
                assert_eq!(*ptr, sel);
                assert_eq!(*ty, IrType::F32);
                assert!(!*volatile);
            }
            other => panic!("expected merged Load, got {other:?}"),
        }
        // arms emptied: no second definition of a hoisted destination.
        assert!(f.blocks[2].instructions.is_empty());
        assert!(f.blocks[3].instructions.is_empty());
        // Exactly ONE load remains in the whole function.
        assert_eq!(
            f.blocks
                .iter()
                .flat_map(|b| &b.instructions)
                .filter(|i| matches!(i, Instruction::Load { .. }))
                .count(),
            1
        );
        let mut violations = Vec::new();
        crate::passes::verify::verify_function(&f, "cond_load_sink", &mut violations);
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn cond_load_sink_refuses_a_volatile_load() {
        let mut f = cond_load_diamond();
        if let Instruction::Load { volatile, .. } = &mut f.blocks[2].instructions[1] {
            *volatile = true;
        }
        assert_eq!(sink_conditional_loads(&mut f), 0);
    }

    #[test]
    fn cond_load_sink_refuses_two_loads_in_one_arm() {
        let mut f = cond_load_diamond();
        f.blocks[2].instructions.push(Instruction::Load {
            dest: Value(12),
            ptr: Value(10),
            ty: IrType::F32,
            seg_override: AddressSpace::Default,
            volatile: false,
        });
        assert_eq!(sink_conditional_loads(&mut f), 0);
    }

    #[test]
    fn cond_load_sink_refuses_a_second_use_of_the_load_result() {
        let mut f = cond_load_diamond();
        f.blocks[2].instructions.push(Instruction::Copy {
            dest: Value(13),
            src: Operand::Value(Value(11)),
        });
        assert_eq!(sink_conditional_loads(&mut f), 0);
    }

    #[test]
    fn cond_load_sink_refuses_a_cross_arm_operand() {
        let mut f = cond_load_diamond();
        // T's GEP offset is defined in F: hoisting would invert SSA order.
        if let Instruction::GetElementPtr { offset, .. } = &mut f.blocks[2].instructions[0] {
            *offset = Operand::Value(Value(20));
        }
        assert_eq!(sink_conditional_loads(&mut f), 0);
    }

    #[test]
    fn cond_load_sink_refuses_a_value_defined_in_the_merge() {
        let mut f = cond_load_diamond();
        // V(40) is defined AFTER the arms; it cannot dominate the pred.
        f.blocks[4].instructions.insert(
            0,
            Instruction::Copy {
                dest: Value(40),
                src: Operand::Const(IrConst::I64(0)),
            },
        );
        if let Instruction::GetElementPtr { offset, .. } = &mut f.blocks[2].instructions[0] {
            *offset = Operand::Value(Value(40));
        }
        assert_eq!(sink_conditional_loads(&mut f), 0);
    }

    #[test]
    fn cond_load_sink_converts_an_auxiliary_phi_to_a_select() {
        let mut f = cond_load_diamond();
        f.blocks[4].instructions.insert(
            1,
            Instruction::Phi {
                dest: Value(31),
                ty: IrType::I32,
                incoming: vec![
                    (Operand::Const(IrConst::I32(7)), BlockId(2)),
                    (Operand::Const(IrConst::I32(9)), BlockId(3)),
                ],
            },
        );
        assert_eq!(sink_conditional_loads(&mut f), 1);
        let selects: Vec<_> = f.blocks[1]
            .instructions
            .iter()
            .filter(|i| matches!(i, Instruction::Select { .. }))
            .collect();
        assert_eq!(selects.len(), 2, "{selects:?}");
        assert!(selects.iter().any(|i| matches!(
            i,
            Instruction::Select {
                dest: Value(31),
                ty: IrType::I32,
                ..
            }
        )));
        assert_eq!(
            f.blocks[4]
                .instructions
                .iter()
                .filter(|i| matches!(i, Instruction::Phi { .. }))
                .count(),
            0
        );
        let mut violations = Vec::new();
        crate::passes::verify::verify_function(&f, "cond_load_sink_aux_phi", &mut violations);
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn cond_load_sink_refuses_mismatched_load_types_and_wide_types() {
        let mut f = cond_load_diamond();
        if let Instruction::Load { ty, .. } = &mut f.blocks[3].instructions[1] {
            *ty = IrType::F64;
        }
        assert_eq!(sink_conditional_loads(&mut f), 0);

        let mut g = cond_load_diamond();
        for b in [2usize, 3] {
            if let Instruction::Load { ty, .. } = &mut g.blocks[b].instructions[1] {
                *ty = IrType::I128;
            }
        }
        assert_eq!(sink_conditional_loads(&mut g), 0);
    }

    #[test]
    fn cond_load_sink_refuses_a_trapping_division_in_an_arm() {
        let mut f = cond_load_diamond();
        f.blocks[2].instructions.insert(
            0,
            Instruction::BinOp {
                dest: Value(14),
                op: IrBinOp::SDiv,
                lhs: Operand::Value(Value(2)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::I32,
            },
        );
        assert_eq!(sink_conditional_loads(&mut f), 0);
    }

    #[test]
    fn cond_load_sink_refuses_a_partial_merge_and_a_non_diamond() {
        let mut f = cond_load_diamond();
        // A third predecessor edge into the merge (a computed-goto target).
        f.blocks[0].terminator = Terminator::IndirectBranch {
            target: Operand::Value(Value(3)),
            possible_targets: vec![BlockId(1), BlockId(4)],
        };
        assert_eq!(sink_conditional_loads(&mut f), 0);
    }

    #[test]
    fn cond_load_sink_refuses_when_the_guard_reads_an_arm_address() {
        // `a[0] < b[0] ? a[0] : b[0]`: the guard already dereferences BOTH
        // arm addresses unconditionally.  Its address expressions are
        // separately materialized GEPs (different SSA ids, same base+offset),
        // which is exactly what the structural key exists to catch.
        let mut f = cond_load_diamond();
        let pred = &mut f.blocks[1];
        pred.instructions.clear();
        pred.instructions.push(Instruction::GetElementPtr {
            dest: Value(50),
            base: Value(0),
            offset: Operand::Const(IrConst::I64(0)),
            ty: IrType::I32,
        });
        pred.instructions.push(Instruction::Load {
            dest: Value(51),
            ptr: Value(50),
            ty: IrType::I32,
            seg_override: AddressSpace::Default,
            volatile: false,
        });
        pred.instructions.push(Instruction::GetElementPtr {
            dest: Value(52),
            base: Value(1),
            offset: Operand::Const(IrConst::I64(0)),
            ty: IrType::I32,
        });
        pred.instructions.push(Instruction::Load {
            dest: Value(53),
            ptr: Value(52),
            ty: IrType::I32,
            seg_override: AddressSpace::Default,
            volatile: false,
        });
        pred.instructions.push(Instruction::Cmp {
            dest: Value(54),
            op: IrCmpOp::Slt,
            lhs: Operand::Value(Value(51)),
            rhs: Operand::Value(Value(53)),
            ty: IrType::I32,
        });
        pred.terminator = Terminator::CondBranch {
            cond: Operand::Value(Value(54)),
            true_label: BlockId(2),
            false_label: BlockId(3),
        };
        f.next_value_id = 60;
        assert_eq!(sink_conditional_loads(&mut f), 0);
    }

    #[test]
    fn cond_load_sink_refuses_when_the_guard_uses_the_identical_pointer() {
        // SSA-identical variant: the guard loads straight from the very same
        // two arm pointers, so even a plain Value comparison must refuse.
        let mut f = cond_load_diamond();
        let pred = &mut f.blocks[1];
        pred.instructions.clear();
        pred.instructions.push(Instruction::Load {
            dest: Value(50),
            ptr: Value(0),
            ty: IrType::I32,
            seg_override: AddressSpace::Default,
            volatile: false,
        });
        pred.instructions.push(Instruction::Load {
            dest: Value(51),
            ptr: Value(1),
            ty: IrType::I32,
            seg_override: AddressSpace::Default,
            volatile: false,
        });
        pred.instructions.push(Instruction::Cmp {
            dest: Value(52),
            op: IrCmpOp::Slt,
            lhs: Operand::Value(Value(50)),
            rhs: Operand::Value(Value(51)),
            ty: IrType::I32,
        });
        pred.terminator = Terminator::CondBranch {
            cond: Operand::Value(Value(52)),
            true_label: BlockId(2),
            false_label: BlockId(3),
        };
        f.next_value_id = 60;
        assert_eq!(sink_conditional_loads(&mut f), 0);
    }

    #[test]
    fn cond_load_sink_accepts_when_the_guard_loads_an_unrelated_address() {
        // `if (*flag) x = *p; else x = *q;` — the guard's load is from a
        // THIRD address.  Neither arm value is already in a register, so the
        // pointer Select + single Load remains the right transformation.
        let mut f = cond_load_diamond();
        let pred = &mut f.blocks[1];
        pred.instructions.clear();
        pred.instructions.push(Instruction::Load {
            dest: Value(50),
            ptr: Value(3),
            ty: IrType::I32,
            seg_override: AddressSpace::Default,
            volatile: false,
        });
        pred.instructions.push(Instruction::Cmp {
            dest: Value(51),
            op: IrCmpOp::Ne,
            lhs: Operand::Value(Value(50)),
            rhs: Operand::Const(IrConst::I32(0)),
            ty: IrType::I32,
        });
        pred.terminator = Terminator::CondBranch {
            cond: Operand::Value(Value(51)),
            true_label: BlockId(2),
            false_label: BlockId(3),
        };
        f.next_value_id = 60;
        assert_eq!(sink_conditional_loads(&mut f), 1);
    }

    /// Narrow C integer loads: `Cast(U8/I8/U16/I16 -> I32)` sits between the
    /// arm Load and the merge Phi.  GCC 16.2 and Clang 23.1.5 both emit a
    /// branchless `cmov` + `movzbl`/`movswl` for this family; SV-03 must not
    /// leave LCCC's two-branch form behind.
    fn cond_load_narrow_diamond(
        from_ty: IrType,
        t_to: IrType,
        f_to: IrType,
        f_dest: Value,
    ) -> IrFunction {
        let mut f = cond_load_diamond();
        for (bi, to_ty, dest) in [(2usize, t_to, Value(12)), (3, f_to, f_dest)] {
            if let Some(Instruction::Load { ty, .. }) = f.blocks[bi].instructions.get_mut(1) {
                *ty = from_ty;
            }
            f.blocks[bi].instructions.push(Instruction::Cast {
                dest,
                src: Operand::Value(Value(if bi == 2 { 11 } else { 21 })),
                from_ty,
                to_ty,
            });
        }
        if let Instruction::Phi { ty, incoming, .. } = &mut f.blocks[4].instructions[0] {
            *ty = t_to;
            incoming[0].0 = Operand::Value(Value(12));
            incoming[1].0 = Operand::Value(f_dest);
        }
        if let Instruction::Store { ty, .. } = &mut f.blocks[4].instructions[1] {
            *ty = t_to;
        }
        f.next_value_id = 41;
        f
    }

    #[test]
    fn cond_load_arm_parse_records_a_typed_promotion_tail() {
        let f = cond_load_narrow_diamond(IrType::I8, IrType::I32, IrType::I32, Value(22));
        let arm = parse_cond_load_arm(&f.blocks[2]).expect("canonical narrow-load arm");
        assert_eq!(arm.tail.len(), 1);
        assert_eq!(arm.tail[0].from_ty, IrType::I8);
        assert_eq!(arm.tail[0].to_ty, IrType::I32);
        assert_eq!(arm.tail_result, arm.tail[0].dest);
        assert_eq!(arm.result_ty, IrType::I32);
        assert_ne!(arm.load_dest, arm.tail_result);
    }

    fn count_defs(f: &IrFunction, id: u32) -> usize {
        f.blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .filter(|i| i.dest().is_some_and(|d| d.0 == id))
            .count()
    }

    #[test]
    fn cond_load_sink_sinks_narrow_loads_through_integer_promotion() {
        let mut f = cond_load_narrow_diamond(IrType::U8, IrType::I32, IrType::I32, Value(22));
        assert_eq!(sink_conditional_loads(&mut f), 1);
        let sel = match f.blocks[1].instructions.last().unwrap() {
            Instruction::Select { dest, ty, .. } => {
                assert_eq!(*ty, IrType::Ptr);
                *dest
            }
            other => panic!("expected pointer Select, got {other:?}"),
        };
        let load_dest = match &f.blocks[4].instructions[0] {
            Instruction::Load {
                dest,
                ptr,
                ty,
                volatile,
                ..
            } => {
                assert_ne!(*dest, Value(30));
                assert_ne!(*dest, sel);
                assert_eq!(*ptr, sel);
                assert_eq!(*ty, IrType::U8);
                assert!(!*volatile);
                *dest
            }
            other => panic!("expected sunk narrow Load, got {other:?}"),
        };
        // The promoted cast must be inserted BEFORE the Store and define the
        // original phi destination, so downstream uses stay valid.
        match &f.blocks[4].instructions[1] {
            Instruction::Cast {
                dest,
                src,
                from_ty,
                to_ty,
            } => {
                assert_eq!(*dest, Value(30));
                assert_eq!(*src, Operand::Value(load_dest));
                assert_eq!(*from_ty, IrType::U8);
                assert_eq!(*to_ty, IrType::I32);
            }
            other => panic!("expected promotion Cast, got {other:?}"),
        }
        assert!(matches!(
            &f.blocks[4].instructions[2],
            Instruction::Store { val: Operand::Value(v), .. } if *v == Value(30)
        ));
        // Regression for the self-audited duplicate-definition defect: a tail
        // Cast must be re-emitted ONLY in the merge, never also hoisted into
        // the pred.  Otherwise the same SSA id has two definitions.
        assert!(
            !f.blocks[1]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::Cast { .. }))
        );
        assert_eq!(count_defs(&f, 30), 1);
        assert!(f.blocks[2].instructions.is_empty());
        assert!(f.blocks[3].instructions.is_empty());
        assert_eq!(
            f.blocks
                .iter()
                .flat_map(|b| &b.instructions)
                .filter(|i| matches!(i, Instruction::Load { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn cond_load_sink_sinks_a_multi_cast_promotion_chain() {
        let mut f = cond_load_narrow_diamond(IrType::I8, IrType::I32, IrType::I32, Value(22));
        // Replace each one-step I8->I32 tail with I8->I16->I32.
        for (bi, first, final_dest) in [(2usize, Value(13), Value(12)), (3, Value(23), Value(22))] {
            let insts = &mut f.blocks[bi].instructions;
            insts.pop();
            insts.push(Instruction::Cast {
                dest: first,
                src: Operand::Value(Value(if bi == 2 { 11 } else { 21 })),
                from_ty: IrType::I8,
                to_ty: IrType::I16,
            });
            insts.push(Instruction::Cast {
                dest: final_dest,
                src: Operand::Value(first),
                from_ty: IrType::I16,
                to_ty: IrType::I32,
            });
        }
        assert_eq!(sink_conditional_loads(&mut f), 1);
        assert!(matches!(
            f.blocks[4].instructions[0],
            Instruction::Load { .. }
        ));
        for inst in f.blocks[4].instructions.iter().take(3) {
            assert!(matches!(
                inst,
                Instruction::Load { .. } | Instruction::Cast { .. }
            ));
        }
        assert_eq!(f.blocks[4].instructions[1].dest().unwrap(), Value(13));
        assert_eq!(f.blocks[4].instructions[2].dest().unwrap(), Value(30));
        assert_eq!(count_defs(&f, 13), 1);
        assert_eq!(count_defs(&f, 30), 1);
        assert!(
            !f.blocks[1]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::Cast { .. }))
        );
        assert_eq!(
            f.blocks
                .iter()
                .flat_map(|b| &b.instructions)
                .filter(|i| matches!(i, Instruction::Load { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn cond_load_sink_refuses_mismatched_or_truncating_promotion_tails() {
        // Different final cast types => the merge Phi cannot have one type.
        let mut mismatched =
            cond_load_narrow_diamond(IrType::U8, IrType::I32, IrType::I64, Value(22));
        if let Instruction::Phi { ty, .. } = &mut mismatched.blocks[4].instructions[0] {
            *ty = IrType::I64;
        }
        assert_eq!(sink_conditional_loads(&mut mismatched), 0);
        // A truncating cast is not bijective: the original tail must survive.
        let mut truncating =
            cond_load_narrow_diamond(IrType::I32, IrType::I8, IrType::I8, Value(22));
        if let Instruction::Phi { ty, .. } = &mut truncating.blocks[4].instructions[0] {
            *ty = IrType::I8;
        }
        assert_eq!(sink_conditional_loads(&mut truncating), 0);
        // A side-effecting instruction after the load is never a tail.
        let mut impure = cond_load_narrow_diamond(IrType::U8, IrType::I32, IrType::I32, Value(22));
        impure.blocks[2].instructions.push(Instruction::Load {
            dest: Value(14),
            ptr: Value(10),
            ty: IrType::U8,
            seg_override: AddressSpace::Default,
            volatile: false,
        });
        assert_eq!(sink_conditional_loads(&mut impure), 0);
        // A second use of a tail id would dangle once the arms disappear.
        let mut extra_use =
            cond_load_narrow_diamond(IrType::U8, IrType::I32, IrType::I32, Value(22));
        extra_use.blocks[4].instructions.push(Instruction::Copy {
            dest: Value(35),
            src: Operand::Value(Value(12)),
        });
        assert_eq!(sink_conditional_loads(&mut extra_use), 0);
    }

    /// Build a tiny definition map for the structural address-key tests.
    /// Each instruction below defines Value(i+1); the map borrows the local
    /// Vec, so NO unsafe lifetime extension is needed.
    fn key_defs(insts: &[Instruction]) -> CondLoadDefs<'_> {
        insts
            .iter()
            .enumerate()
            .map(|(i, inst)| (i as u32 + 1, vec![inst]))
            .collect()
    }

    #[test]
    fn cond_load_ptr_key_normalises_gep_add_sub_and_zero_offsets() {
        let insts = vec![
            Instruction::GlobalAddr {
                dest: Value(1),
                name: "a".into(),
            },
            Instruction::GetElementPtr {
                dest: Value(2),
                base: Value(1),
                offset: Operand::Const(IrConst::I64(24)),
                ty: IrType::I64,
            },
            Instruction::BinOp {
                dest: Value(3),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I64(24)),
                ty: IrType::I64,
            },
            Instruction::GetElementPtr {
                dest: Value(4),
                base: Value(1),
                offset: Operand::Const(IrConst::Zero),
                ty: IrType::Ptr,
            },
            Instruction::BinOp {
                dest: Value(5),
                op: IrBinOp::Sub,
                lhs: Operand::Value(Value(2)),
                rhs: Operand::Const(IrConst::I64(8)),
                ty: IrType::I64,
            },
            Instruction::GetElementPtr {
                dest: Value(6),
                base: Value(1),
                offset: Operand::Const(IrConst::I64(16)),
                ty: IrType::I64,
            },
            Instruction::BinOp {
                dest: Value(7),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I64(8)),
                ty: IrType::I64,
            },
            Instruction::BinOp {
                dest: Value(8),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(7)),
                rhs: Operand::Const(IrConst::I64(8)),
                ty: IrType::I64,
            },
            Instruction::ParamRef {
                dest: Value(9),
                param_idx: 0,
                ty: IrType::I64,
            },
            Instruction::GetElementPtr {
                dest: Value(10),
                base: Value(1),
                offset: Operand::Value(Value(9)),
                ty: IrType::I64,
            },
            Instruction::BinOp {
                dest: Value(11),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(9)),
                rhs: Operand::Value(Value(1)),
                ty: IrType::I64,
            },
            Instruction::Cast {
                dest: Value(12),
                src: Operand::Value(Value(1)),
                from_ty: IrType::I64,
                to_ty: IrType::I8,
            },
            Instruction::BinOp {
                dest: Value(13),
                op: IrBinOp::Add,
                lhs: Operand::Const(IrConst::I128(i128::MAX)),
                rhs: Operand::Const(IrConst::I128(1)),
                ty: IrType::I128,
            },
        ];
        let defs = key_defs(&insts);
        assert_eq!(
            cond_load_ptr_key(Value(2), &defs, 0),
            cond_load_ptr_key(Value(3), &defs, 0)
        );
        assert_eq!(cond_load_ptr_key(Value(4), &defs, 0), "global(a)");
        assert_eq!(
            cond_load_ptr_key(Value(5), &defs, 0),
            cond_load_ptr_key(Value(6), &defs, 0)
        );
        assert_eq!(
            cond_load_ptr_key(Value(8), &defs, 0),
            cond_load_ptr_key(Value(6), &defs, 0)
        );
        assert_eq!(
            cond_load_ptr_key(Value(10), &defs, 0),
            cond_load_ptr_key(Value(11), &defs, 0)
        );
        assert_ne!(cond_load_ptr_key(Value(12), &defs, 0), "global(a)");
        assert!(cond_load_ptr_key(Value(13), &defs, 0).starts_with("opaque:overflow"));
        assert_ne!(cond_load_ptr_key(Value(99), &defs, 0), "global(a)");
    }

    #[test]
    fn cond_load_ptr_key_is_bounded_for_deep_chains() {
        let mut insts = vec![Instruction::GlobalAddr {
            dest: Value(1),
            name: "a".into(),
        }];
        for i in 2..=12u32 {
            insts.push(Instruction::GetElementPtr {
                dest: Value(i),
                base: Value(i - 1),
                offset: Operand::Const(IrConst::I64(1)),
                ty: IrType::Ptr,
            });
        }
        let defs = key_defs(&insts);
        let key = cond_load_ptr_key(Value(12), &defs, 0);
        assert!(key.contains("opaque:"), "unbounded chain key: {key}");
    }

    #[test]
    fn cond_load_guard_detects_binop_add_arm_addresses() {
        let insts = vec![
            Instruction::GlobalAddr {
                dest: Value(1),
                name: "a".into(),
            },
            Instruction::GetElementPtr {
                dest: Value(2),
                base: Value(1),
                offset: Operand::Const(IrConst::I64(24)),
                ty: IrType::I64,
            },
            Instruction::Load {
                dest: Value(3),
                ptr: Value(2),
                ty: IrType::U64,
                seg_override: AddressSpace::Default,
                volatile: false,
            },
            Instruction::Cmp {
                dest: Value(4),
                op: IrCmpOp::Ne,
                lhs: Operand::Value(Value(3)),
                rhs: Operand::Const(IrConst::I64(0)),
                ty: IrType::U64,
            },
            Instruction::BinOp {
                dest: Value(5),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I64(24)),
                ty: IrType::I64,
            },
            Instruction::GlobalAddr {
                dest: Value(6),
                name: "flag".into(),
            },
        ];
        let defs = key_defs(&insts);
        let t_key = cond_load_ptr_key(Value(5), &defs, 0);
        let f_key = cond_load_ptr_key(Value(6), &defs, 0);
        assert!(cond_load_guard_reads_arm_address(
            Operand::Value(Value(4)),
            &t_key,
            &f_key,
            &defs,
        ));
        let unrelated = cond_load_ptr_key(Value(6), &defs, 0);
        assert!(!cond_load_guard_reads_arm_address(
            Operand::Value(Value(4)),
            &unrelated,
            &f_key,
            &defs,
        ));
        // A second conventional-SSA definition of the guard value must not
        // hide the load behind the first (Cmp) definition.
        let extra_load = Instruction::Load {
            dest: Value(3),
            ptr: Value(2),
            ty: IrType::U64,
            seg_override: AddressSpace::Default,
            volatile: false,
        };
        let mut multi = key_defs(&insts);
        multi.entry(3).or_default().push(&extra_load);
        assert!(cond_load_guard_reads_arm_address(
            Operand::Value(Value(4)),
            &t_key,
            &f_key,
            &multi,
        ));
    }

    #[test]
    fn cond_load_sink_kill_switch_is_transactional() {
        if !crate::test_support::env_isolate(
            "passes::if_convert::cond_load_sink_tests::cond_load_sink_kill_switch_is_transactional",
        ) {
            return;
        }
        use crate::test_support::EnvGuard;
        let mut f = cond_load_diamond();
        let before = format!("{f:?}");
        let _guard = EnvGuard::set("CCC_NO_COND_LOAD_SINK", "1");
        assert_eq!(sink_conditional_loads(&mut f), 0);
        assert_eq!(format!("{f:?}"), before);
    }
}
