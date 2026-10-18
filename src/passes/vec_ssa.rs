//! Whole-vector SSA promotion (`vecssa`).
//!
//! GNU/ext vector values have no IR type: the lowering keeps every vector
//! object in a stack alloca and, for the SIMD-shaped widths (16/32 bytes,
//! `src/ir/lowering/vector_simd.rs`), reads and writes it exclusively through
//! the full-width `VecLoad*` / `VecStore*` intrinsic pair of one register
//! family.  Such an alloca is a scalar variable in disguise — only the
//! "scalar" is an XMM/YMM register image — and this pass is mem2reg for it:
//!
//! ```text
//!   %t = alloca 32, align 32
//!   VecStoreF32x8 %t <- %sum          ==>   (phi/copy web of %sum)
//!   %r = VecLoadF32x8 %t                    uses of %r read %sum directly
//! ```
//!
//! Eligibility is fail-closed.  An alloca is promoted only when EVERY
//! mention of it in the function is a full-width, zero-offset load or store
//! of one family whose width equals the alloca size.  Any other use — a GEP
//! (lane access), a `Memcpy`, a scalar `Load`/`Store`, an address escape
//! into a call, a terminator operand, a different family — keeps the object
//! in memory.  Parameter allocas take part too: their initial contents are
//! written by the backend prologue capture (no IR producer), so a read not
//! dominated by a store is seeded with ONE entry-prefix `VecLoad` of the
//! alloca — the shape `hoist_param_loads` produces for read-only parameters,
//! which the x86 prologue turns into a register move
//! (`x86_vector_param_loads`) — and the alloca itself stays as the capture's
//! target.  `v32f acc` parameters updated in a loop therefore become
//! register-resident accumulators exactly like locals.
//!
//! Phis are placed with the usual iterated-dominance-frontier construction.
//! A read reachable without any store (C-level use of an indeterminate
//! value) receives the family's `VecZero*` so the phi web stays total.  The
//! phi carries the lane IrType purely as a placeholder: after
//! `eliminate_phis` the value travels through `Copy` instructions and the
//! register allocator classifies it through the vector copy web
//! (`collect_vector_values`), exactly like the loop-carried accumulators the
//! loop vectorizer already produces.
//!
//! Runs late (after inlining and the optimisation loop, before the vector
//! temp promotion cleanups) so every vector chain is seen whole and no
//! scalar-typed pass ever meets a placeholder-typed phi.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::analysis;
use crate::ir::instruction::{replace_values_in_inst_map, replace_values_in_terminator_map};
use crate::ir::intrinsics::IntrinsicOp;
use crate::ir::reexports::{BasicBlock, Instruction, IrFunction, IrModule, Operand, Value};
use crate::passes::slp_vectorizer::family_for;

/// One register family admitted by the pass: the load/store endpoints, the
/// zero producer and the byte width of the register image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Family {
    load: IntrinsicOp,
    store: IntrinsicOp,
    zero: IntrinsicOp,
    lane_ty: IrType,
    bytes: usize,
}

/// Family lookup keyed by the load AND the store opcode.
fn family_table() -> FxHashMap<IntrinsicOp, Family> {
    let mut table = FxHashMap::default();
    let lane_types = [
        IrType::I8,
        IrType::U8,
        IrType::I16,
        IrType::U16,
        IrType::I32,
        IrType::U32,
        IrType::I64,
        IrType::U64,
        IrType::F32,
        IrType::F64,
    ];
    for lane_ty in lane_types {
        let lane_size = lane_ty.size();
        for bytes in [16usize, 32] {
            let lanes = bytes / lane_size;
            let Some(fam) = family_for(lane_ty, lanes) else {
                continue;
            };
            // Half-wide pair endpoints are 8-byte memory accesses; the
            // family table excludes them by requiring the register width to
            // equal the alloca width, which is what `bytes` encodes here.
            if fam.size as usize * lanes != bytes {
                continue;
            }
            let entry = Family {
                load: fam.load,
                store: fam.store,
                zero: fam.zero,
                lane_ty,
                bytes,
            };
            table.entry(fam.load).or_insert(entry);
            table.entry(fam.store).or_insert(entry);
        }
    }
    table
}

fn const_is_zero(op: &Operand) -> bool {
    matches!(op, Operand::Const(c) if c.to_i64() == Some(0))
}

/// Shape of one instruction with respect to candidate alloca `a`.
enum Access {
    /// `dest = VecLoad(a, 0)`.
    Load { dest: Value, op: IntrinsicOp },
    /// `VecStore(a) <- val`.
    Store { val: Value, op: IntrinsicOp },
}

fn classify(inst: &Instruction, a: u32, table: &FxHashMap<IntrinsicOp, Family>) -> Option<Access> {
    let Instruction::Intrinsic {
        dest,
        op,
        dest_ptr,
        args,
    } = inst
    else {
        return None;
    };
    let fam = table.get(op)?;
    if *op == fam.load {
        if dest_ptr.is_some() || args.len() != 2 {
            return None;
        }
        if !matches!(args[0], Operand::Value(p) if p.0 == a) || !const_is_zero(&args[1]) {
            return None;
        }
        return Some(Access::Load {
            dest: (*dest)?,
            op: *op,
        });
    }
    if *op == fam.store {
        if dest.is_some() || args.len() != 3 || *dest_ptr != Some(Value(a)) {
            return None;
        }
        let Operand::Value(val) = args[0] else {
            return None;
        };
        if val.0 == a
            || !matches!(args[1], Operand::Value(p) if p.0 == a)
            || !const_is_zero(&args[2])
        {
            return None;
        }
        return Some(Access::Store { val, op: *op });
    }
    None
}

struct Candidate {
    alloca: u32,
    family: Family,
    def_blocks: FxHashSet<usize>,
    /// A parameter alloca: the prologue capture is its (IR-invisible)
    /// initial store, so the seed for reads without a dominating store is
    /// ONE entry-prefix `VecLoad` of the alloca instead of the family zero,
    /// and the alloca itself stays (the capture's target).
    is_param: bool,
}

fn uses_of(inst: &Instruction, a: u32) -> usize {
    let mut n = 0;
    inst.for_each_used_value(|v| {
        if v == a {
            n += 1;
        }
    });
    n
}

fn find_candidates(func: &IrFunction, table: &FxHashMap<IntrinsicOp, Family>) -> Vec<Candidate> {
    let params: FxHashSet<u32> = func.param_alloca_values.iter().map(|v| v.0).collect();
    let mut cands: FxHashMap<u32, Candidate> = FxHashMap::default();
    let mut order: Vec<u32> = Vec::new();
    for inst in &func.blocks[0].instructions {
        if let Instruction::Alloca {
            dest,
            size,
            volatile: false,
            semantic_volatile: false,
            ..
        } = inst
        {
            if matches!(*size, 16 | 32) {
                cands.insert(
                    dest.0,
                    Candidate {
                        alloca: dest.0,
                        family: Family {
                            load: IntrinsicOp::VecZeroF32x4,
                            store: IntrinsicOp::VecZeroF32x4,
                            zero: IntrinsicOp::VecZeroF32x4,
                            lane_ty: IrType::Void,
                            bytes: 0,
                        },
                        def_blocks: FxHashSet::default(),
                        is_param: params.contains(&dest.0),
                    },
                );
                order.push(dest.0);
            }
        }
    }
    if cands.is_empty() {
        return Vec::new();
    }
    let mut rejected: FxHashSet<u32> = FxHashSet::default();
    let mut sizes: FxHashMap<u32, usize> = FxHashMap::default();
    for inst in &func.blocks[0].instructions {
        if let Instruction::Alloca { dest, size, .. } = inst {
            sizes.insert(dest.0, *size);
        }
    }
    for (bi, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            let mut touched: Vec<u32> = Vec::new();
            inst.for_each_used_value(|v| {
                if cands.contains_key(&v) && !touched.contains(&v) {
                    touched.push(v);
                }
            });
            for a in touched {
                if rejected.contains(&a) {
                    continue;
                }
                let access = classify(inst, a, table);
                let ok = match access {
                    Some(Access::Load { op, .. }) | Some(Access::Store { op, .. }) => {
                        let fam = table[&op];
                        // `touched` was filtered through `cands`; an absent
                        // entry would be a bookkeeping fault, answered as a
                        // rejection rather than a panic.
                        let Some(cand) = cands.get_mut(&a) else {
                            rejected.insert(a);
                            continue;
                        };
                        // Mentions of `a` must be exactly the pointer slots
                        // of this access: 1 for a load, 2 for a store
                        // (`for_each_used_value` visits `dest_ptr` and the
                        // pointer argument copy).
                        let expected = if matches!(access, Some(Access::Store { .. })) {
                            2
                        } else {
                            1
                        };
                        let shape_ok = uses_of(inst, a) == expected
                            && fam.bytes == sizes.get(&a).copied().unwrap_or(0)
                            && (cand.family.bytes == 0 || cand.family == fam);
                        if shape_ok {
                            cand.family = fam;
                            if matches!(access, Some(Access::Store { .. })) {
                                cand.def_blocks.insert(bi);
                            }
                        }
                        shape_ok
                    }
                    None => false,
                };
                if !ok {
                    rejected.insert(a);
                }
            }
        }
        block.terminator.for_each_used_value(|v| {
            if cands.contains_key(&v) {
                rejected.insert(v);
            }
        });
    }
    order
        .into_iter()
        .filter(|a| !rejected.contains(a))
        .filter_map(|a| cands.remove(&a))
        .filter(|c| c.family.bytes != 0)
        .collect()
}

fn alloc_value(func: &mut IrFunction) -> Value {
    if func.next_value_id == 0 {
        func.next_value_id = func.max_value_id() + 1;
    }
    let v = Value(func.next_value_id);
    func.next_value_id += 1;
    v
}

/// Remove the instructions at `drop` from `block`, keeping `source_spans`
/// parallel when it is tracked.
fn retain_instructions(block: &mut BasicBlock, drop: &FxHashSet<usize>) {
    if drop.is_empty() {
        return;
    }
    let synced = block.source_spans.len() == block.instructions.len();
    let mut idx = 0usize;
    block.instructions.retain(|_| {
        let keep = !drop.contains(&idx);
        idx += 1;
        keep
    });
    if synced {
        let mut idx = 0usize;
        block.source_spans.retain(|_| {
            let keep = !drop.contains(&idx);
            idx += 1;
            keep
        });
    } else if !block.source_spans.is_empty() {
        block.source_spans.clear();
    }
}

/// Insert `inst` at `idx` of `block`, keeping `source_spans` parallel.
fn insert_instruction(block: &mut BasicBlock, idx: usize, inst: Instruction) {
    if block.source_spans.len() == block.instructions.len() {
        let span = block
            .source_spans
            .get(idx.saturating_sub(1))
            .or_else(|| block.source_spans.get(idx))
            .copied()
            .unwrap_or_else(crate::common::source::Span::dummy);
        block.source_spans.insert(idx, span);
    } else if !block.source_spans.is_empty() {
        block.source_spans.clear();
    }
    block.instructions.insert(idx, inst);
}

/// Index just past the leading `Alloca`/`ParamRef`/`Phi` prefix of the
/// entry block: zero seeds go there so the prologue-prefix invariants the
/// register allocator checks (`x86_param_caller_homes_safe`) stay intact.
fn entry_insert_point(block: &BasicBlock) -> usize {
    block
        .instructions
        .iter()
        .position(|i| {
            !matches!(
                i,
                Instruction::Alloca { .. } | Instruction::ParamRef { .. } | Instruction::Phi { .. }
            )
        })
        .unwrap_or(block.instructions.len())
}

/// Blocks reachable from the entry, and the candidates no unreachable block
/// touches.  Blocks unreachable from the entry have no dominator; a
/// candidate touched there is left in memory (the rename walk never visits
/// it, so its accesses would survive the deletion sweep half-rewritten).
fn drop_unreachable_candidates(
    func: &IrFunction,
    succs: &analysis::FlatAdj,
    cands: Vec<Candidate>,
) -> (FxHashSet<usize>, Vec<Candidate>) {
    let mut reachable: FxHashSet<usize> = FxHashSet::default();
    let mut stack = vec![0usize];
    while let Some(b) = stack.pop() {
        if reachable.insert(b) {
            for &s in succs.row(b) {
                stack.push(s as usize);
            }
        }
    }
    let cand_index: FxHashMap<u32, usize> = cands
        .iter()
        .enumerate()
        .map(|(i, c)| (c.alloca, i))
        .collect();
    let mut touched_unreachable: FxHashSet<usize> = FxHashSet::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        if reachable.contains(&bi) {
            continue;
        }
        for inst in &block.instructions {
            inst.for_each_used_value(|v| {
                if let Some(&ci) = cand_index.get(&v) {
                    touched_unreachable.insert(ci);
                }
            });
        }
    }
    let cands = cands
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !touched_unreachable.contains(i))
        .map(|(_, c)| c)
        .collect();
    (reachable, cands)
}

/// Phi placement on the iterated dominance frontier of each candidate's
/// definition blocks: `phi_sites[block] = Vec<(cand_idx, phi_value)>`.
fn place_phis(
    func: &mut IrFunction,
    cands: &[Candidate],
    df: &[FxHashSet<usize>],
    reachable: &FxHashSet<usize>,
) -> Vec<Vec<(usize, Value)>> {
    let mut phi_sites: Vec<Vec<(usize, Value)>> = vec![Vec::new(); func.blocks.len()];
    for (ci, cand) in cands.iter().enumerate() {
        let mut has_phi: FxHashSet<usize> = FxHashSet::default();
        let mut work: Vec<usize> = cand.def_blocks.iter().copied().collect();
        let mut queued: FxHashSet<usize> = cand.def_blocks.clone();
        while let Some(b) = work.pop() {
            for &d in &df[b] {
                if !reachable.contains(&d) || !has_phi.insert(d) {
                    continue;
                }
                let v = alloc_value(func);
                phi_sites[d].push((ci, v));
                if queued.insert(d) {
                    work.push(d);
                }
            }
        }
    }
    phi_sites
}

/// Promote every eligible vector alloca of `func`. Returns the number of
/// allocas promoted.
/// The value a read of `cand` sees when no store dominates it: the family
/// zero (an indeterminate C object) or, for a parameter alloca, ONE
/// entry-prefix full-width load of the prologue-captured argument.  Created
/// once per candidate; the instruction is queued for the entry prefix.
fn seed_value(
    func: &mut IrFunction,
    cand: &Candidate,
    seed: &mut Option<Value>,
    seed_insts: &mut Vec<Instruction>,
) -> Value {
    if let Some(z) = *seed {
        return z;
    }
    let z = alloc_value(func);
    *seed = Some(z);
    seed_insts.push(if cand.is_param {
        Instruction::Intrinsic {
            dest: Some(z),
            op: cand.family.load,
            dest_ptr: None,
            args: vec![
                Operand::Value(Value(cand.alloca)),
                Operand::Const(crate::ir::reexports::IrConst::I64(0)),
            ],
        }
    } else {
        Instruction::Intrinsic {
            dest: Some(z),
            op: cand.family.zero,
            dest_ptr: None,
            args: Vec::new(),
        }
    });
    z
}

fn promote_function(func: &mut IrFunction, table: &FxHashMap<IntrinsicOp, Family>) -> usize {
    if func.is_declaration || func.blocks.is_empty() {
        return 0;
    }
    if func.blocks.iter().any(|b| {
        b.instructions
            .iter()
            .any(|i| matches!(i, Instruction::NonlocalGotoSave { .. }))
    }) {
        return 0;
    }
    let cands = find_candidates(func, table);
    if cands.is_empty() {
        return 0;
    }
    let num_blocks = func.blocks.len();
    let label_to_idx = analysis::build_label_map(func);
    let (preds, succs) = analysis::build_cfg(func, &label_to_idx);
    let idom = analysis::compute_dominators(num_blocks, &preds, &succs);
    let df = analysis::compute_dominance_frontiers(num_blocks, &preds, &idom);
    let children = analysis::build_dom_tree_children(num_blocks, &idom);
    let (reachable, cands) = drop_unreachable_candidates(func, &succs, cands);
    if cands.is_empty() {
        return 0;
    }
    let cand_index: FxHashMap<u32, usize> = cands
        .iter()
        .enumerate()
        .map(|(i, c)| (c.alloca, i))
        .collect();
    let phi_sites = place_phis(func, &cands, &df, &reachable);

    // ── Rename ─────────────────────────────────────────────────────────
    // rename[v] = value that replaces load dest `v`.
    let mut rename: FxHashMap<u32, u32> = FxHashMap::default();
    // Lazily created zero seeds per candidate.
    let mut zero_of: Vec<Option<Value>> = vec![None; cands.len()];
    let mut zero_insts: Vec<Instruction> = Vec::new();
    // phi_incoming[(block, phi_value)] = Vec<(value, pred_label)>
    let mut phi_incoming: FxHashMap<u32, Vec<(Value, crate::ir::reexports::BlockId)>> =
        FxHashMap::default();
    let mut delete: Vec<FxHashSet<usize>> = vec![FxHashSet::default(); num_blocks];
    let mut stacks: Vec<Vec<Value>> = vec![Vec::new(); cands.len()];

    fn resolve(rename: &FxHashMap<u32, u32>, mut v: Value) -> Value {
        while let Some(&n) = rename.get(&v.0) {
            v = Value(n);
        }
        v
    }

    // Explicit DFS over the dominator tree (no recursion: deep CFGs).
    enum Step {
        Enter(usize),
        Leave(usize, Vec<usize>),
    }
    let mut stack: Vec<Step> = vec![Step::Enter(0)];
    while let Some(step) = stack.pop() {
        match step {
            Step::Enter(b) => {
                let mut pushed: Vec<usize> = Vec::new();
                for &(ci, v) in &phi_sites[b] {
                    stacks[ci].push(v);
                    pushed.push(ci);
                }
                let block_label = func.blocks[b].label;
                let n_inst = func.blocks[b].instructions.len();
                for ii in 0..n_inst {
                    let inst = &func.blocks[b].instructions[ii];
                    let mut hit: Option<(usize, Access)> = None;
                    inst.for_each_used_value(|v| {
                        if hit.is_none() {
                            if let Some(&ci) = cand_index.get(&v) {
                                if let Some(acc) = classify(inst, v, table) {
                                    hit = Some((ci, acc));
                                }
                            }
                        }
                    });
                    let Some((ci, acc)) = hit else {
                        continue;
                    };
                    match acc {
                        Access::Load { dest, .. } => {
                            let cur = match stacks[ci].last() {
                                Some(&v) => v,
                                None => {
                                    seed_value(func, &cands[ci], &mut zero_of[ci], &mut zero_insts)
                                }
                            };
                            rename.insert(dest.0, resolve(&rename, cur).0);
                            delete[b].insert(ii);
                        }
                        Access::Store { val, .. } => {
                            stacks[ci].push(resolve(&rename, val));
                            pushed.push(ci);
                            delete[b].insert(ii);
                        }
                    }
                }
                for &s in succs.row(b) {
                    let s = s as usize;
                    for &(ci, phi_v) in &phi_sites[s] {
                        let cur = match stacks[ci].last() {
                            Some(&v) => v,
                            None => seed_value(func, &cands[ci], &mut zero_of[ci], &mut zero_insts),
                        };
                        phi_incoming
                            .entry(phi_v.0)
                            .or_default()
                            .push((cur, block_label));
                    }
                }
                stack.push(Step::Leave(b, pushed));
                for &c in children[b].iter().rev() {
                    if reachable.contains(&c) {
                        stack.push(Step::Enter(c));
                    }
                }
            }
            Step::Leave(_, pushed) => {
                for ci in pushed.into_iter().rev() {
                    stacks[ci].pop();
                }
            }
        }
    }

    // ── Apply ──────────────────────────────────────────────────────────
    // 1. Delete promoted loads/stores and the allocas themselves (parameter
    //    allocas stay: the prologue capture writes them and the entry seed
    //    load reads them).
    let alloca_ids: FxHashSet<u32> = cands
        .iter()
        .filter(|c| !c.is_param)
        .map(|c| c.alloca)
        .collect();
    for inst_idx in 0..func.blocks[0].instructions.len() {
        if let Instruction::Alloca { dest, .. } = &func.blocks[0].instructions[inst_idx] {
            if alloca_ids.contains(&dest.0) {
                delete[0].insert(inst_idx);
            }
        }
    }
    for (bi, block) in func.blocks.iter_mut().enumerate() {
        retain_instructions(block, &delete[bi]);
    }
    func.param_alloca_values
        .retain(|v| !alloca_ids.contains(&v.0));
    // 2. Insert phis (block heads) with their incoming operands.
    for (bi, sites) in phi_sites.iter().enumerate() {
        for &(ci, phi_v) in sites.iter().rev() {
            let incoming: Vec<(Operand, crate::ir::reexports::BlockId)> = phi_incoming
                .remove(&phi_v.0)
                .unwrap_or_default()
                .into_iter()
                .map(|(v, l)| (Operand::Value(resolve(&rename, v)), l))
                .collect();
            insert_instruction(
                &mut func.blocks[bi],
                0,
                Instruction::Phi {
                    dest: phi_v,
                    ty: cands[ci].family.lane_ty,
                    incoming,
                },
            );
        }
    }
    // 3. Seeds after the entry prefix: parameter loads FIRST (the x86
    //    prologue contract `x86_vector_param_loads` reads the prefix up to
    //    the first non-load instruction), then the zero seeds.
    if !zero_insts.is_empty() {
        let at = entry_insert_point(&func.blocks[0]);
        let is_load =
            |i: &Instruction| !matches!(i, Instruction::Intrinsic { args, .. } if args.is_empty());
        let (loads, zeros): (Vec<Instruction>, Vec<Instruction>) =
            zero_insts.into_iter().partition(is_load);
        for (k, inst) in loads.into_iter().chain(zeros).enumerate() {
            insert_instruction(&mut func.blocks[0], at + k, inst);
        }
    }
    // 4. Rewrite every read of a promoted load through the rename map
    //    (fully resolved so chains collapse in one sweep).
    let resolved: FxHashMap<u32, u32> = rename
        .keys()
        .map(|&k| (k, resolve(&rename, Value(k)).0))
        .collect();
    if !resolved.is_empty() {
        for block in &mut func.blocks {
            for inst in &mut block.instructions {
                replace_values_in_inst_map(inst, &resolved);
            }
            replace_values_in_terminator_map(&mut block.terminator, &resolved);
        }
    }
    // 5. Dead-phi sweep: phis whose only readers are other dead phis (or
    //    nobody) would otherwise survive to eliminate_phis as register
    //    copies.
    let inserted: FxHashSet<u32> = phi_sites
        .iter()
        .flat_map(|s| s.iter().map(|&(_, v)| v.0))
        .collect();
    if !inserted.is_empty() {
        loop {
            let mut uses: FxHashMap<u32, usize> = FxHashMap::default();
            for block in &func.blocks {
                for inst in &block.instructions {
                    inst.for_each_used_value(|v| {
                        if inserted.contains(&v) {
                            *uses.entry(v).or_insert(0) += 1;
                        }
                    });
                }
                block.terminator.for_each_used_value(|v| {
                    if inserted.contains(&v) {
                        *uses.entry(v).or_insert(0) += 1;
                    }
                });
            }
            let mut removed = false;
            for block in &mut func.blocks {
                let mut drop: FxHashSet<usize> = FxHashSet::default();
                for (i, inst) in block.instructions.iter().enumerate() {
                    if let Instruction::Phi { dest, .. } = inst {
                        if inserted.contains(&dest.0)
                            && uses.get(&dest.0).copied().unwrap_or(0) == 0
                        {
                            drop.insert(i);
                        }
                    }
                }
                if !drop.is_empty() {
                    removed = true;
                    retain_instructions(block, &drop);
                }
            }
            if !removed {
                break;
            }
        }
    }
    cands.len()
}

/// Hoist the whole-vector reads of a parameter alloca into ONE entry-prefix
/// `VecLoad`.
///
/// A vector parameter arrives in `%xmmN`/`%ymmN`; the backend prologue
/// captures it into the parameter alloca because the IR has no producer for
/// its value.  When every use of that alloca is a full-width load of one
/// family, the loads are all the same value — the parameter — so they
/// collapse to a single load placed right after the entry prefix (allocas,
/// `ParamRef`s).  That position is the contract the x86 backend relies on
/// (`backend::call_abi::x86_vector_param_loads`): the incoming register is
/// still intact there, so the prologue can move it straight into the load's
/// register home and skip both the capture store and the reload.
fn hoist_param_loads(func: &mut IrFunction, table: &FxHashMap<IntrinsicOp, Family>) -> usize {
    if func.is_declaration || func.blocks.is_empty() || func.param_alloca_values.is_empty() {
        return 0;
    }
    let params: FxHashSet<u32> = func.param_alloca_values.iter().map(|v| v.0).collect();
    let mut sizes: FxHashMap<u32, usize> = FxHashMap::default();
    for inst in &func.blocks[0].instructions {
        if let Instruction::Alloca {
            dest,
            size,
            volatile: false,
            semantic_volatile: false,
            ..
        } = inst
        {
            if params.contains(&dest.0) && matches!(*size, 16 | 32) {
                sizes.insert(dest.0, *size);
            }
        }
    }
    if sizes.is_empty() {
        return 0;
    }
    // alloca -> (family, load dests)
    let mut loads: FxHashMap<u32, (Family, Vec<Value>)> = FxHashMap::default();
    let mut rejected: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            let mut touched: Vec<u32> = Vec::new();
            inst.for_each_used_value(|v| {
                if sizes.contains_key(&v) && !touched.contains(&v) {
                    touched.push(v);
                }
            });
            for a in touched {
                let ok = match classify(inst, a, table) {
                    Some(Access::Load { dest, op }) => {
                        let fam = table[&op];
                        let entry = loads.entry(a).or_insert((fam, Vec::new()));
                        let shape_ok =
                            uses_of(inst, a) == 1 && fam.bytes == sizes[&a] && entry.0 == fam;
                        if shape_ok {
                            entry.1.push(dest);
                        }
                        shape_ok
                    }
                    _ => false,
                };
                if !ok {
                    rejected.insert(a);
                }
            }
        }
        block.terminator.for_each_used_value(|v| {
            if sizes.contains_key(&v) {
                rejected.insert(v);
            }
        });
    }
    let mut order: Vec<u32> = func
        .param_alloca_values
        .iter()
        .map(|v| v.0)
        .filter(|a| loads.contains_key(a) && !rejected.contains(a))
        .collect();
    order.dedup();
    if order.is_empty() {
        return 0;
    }
    let mut rename: FxHashMap<u32, u32> = FxHashMap::default();
    let mut hoisted: Vec<Instruction> = Vec::new();
    let mut drop_dests: FxHashSet<u32> = FxHashSet::default();
    for a in &order {
        let (fam, dests) = &loads[a];
        let lead = alloc_value(func);
        hoisted.push(Instruction::Intrinsic {
            dest: Some(lead),
            op: fam.load,
            dest_ptr: None,
            args: vec![
                Operand::Value(Value(*a)),
                Operand::Const(crate::ir::reexports::IrConst::I64(0)),
            ],
        });
        for d in dests {
            rename.insert(d.0, lead.0);
            drop_dests.insert(d.0);
        }
    }
    for block in &mut func.blocks {
        let drop: FxHashSet<usize> = block
            .instructions
            .iter()
            .enumerate()
            .filter(|(_, inst)| {
                matches!(inst, Instruction::Intrinsic { dest: Some(d), .. } if drop_dests.contains(&d.0))
            })
            .map(|(i, _)| i)
            .collect();
        retain_instructions(block, &drop);
    }
    let at = entry_insert_point(&func.blocks[0]);
    for (k, inst) in hoisted.into_iter().enumerate() {
        insert_instruction(&mut func.blocks[0], at + k, inst);
    }
    for block in &mut func.blocks {
        for inst in &mut block.instructions {
            replace_values_in_inst_map(inst, &rename);
        }
        replace_values_in_terminator_map(&mut block.terminator, &rename);
    }
    order.len()
}

/// Module driver. Returns the number of allocas promoted.
pub(crate) fn promote_vector_allocas(module: &mut IrModule) -> usize {
    let table = family_table();
    if table.is_empty() {
        return 0;
    }
    let mut total = 0;
    for func in &mut module.functions {
        total += promote_function(func, &table);
        total += hoist_param_loads(func, &table);
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::reexports::{BlockId, IrFunction, Terminator};

    fn fam(table: &FxHashMap<IntrinsicOp, Family>, op: IntrinsicOp) -> Family {
        table[&op]
    }

    fn func_with_blocks(blocks: Vec<BasicBlock>) -> IrFunction {
        let mut f = IrFunction::new("t".to_string(), IrType::Void, vec![], false);
        f.blocks = blocks;
        f
    }

    fn block(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions,
            terminator,
            source_spans: Vec::new(),
        }
    }

    #[test]
    fn straight_line_store_load_is_forwarded() {
        crate::passes::vectorize::set_x86_simd_isa(true, false, true, false);
        let table = family_table();
        let f32x4 = fam(&table, IntrinsicOp::VecLoadF32x4);
        let a = Value(0);
        let x = Value(1);
        let l = Value(2);
        let blocks = vec![block(
            0,
            vec![
                Instruction::Alloca {
                    dest: a,
                    ty: IrType::Ptr,
                    size: 16,
                    align: 16,
                    volatile: false,
                    semantic_volatile: false,
                },
                Instruction::Intrinsic {
                    dest: Some(x),
                    op: f32x4.zero,
                    dest_ptr: None,
                    args: vec![],
                },
                Instruction::Intrinsic {
                    dest: None,
                    op: f32x4.store,
                    dest_ptr: Some(a),
                    args: vec![
                        Operand::Value(x),
                        Operand::Value(a),
                        Operand::Const(crate::ir::reexports::IrConst::I64(0)),
                    ],
                },
                Instruction::Intrinsic {
                    dest: Some(l),
                    op: f32x4.load,
                    dest_ptr: None,
                    args: vec![
                        Operand::Value(a),
                        Operand::Const(crate::ir::reexports::IrConst::I64(0)),
                    ],
                },
                Instruction::Intrinsic {
                    dest: None,
                    op: f32x4.store,
                    dest_ptr: Some(Value(9)),
                    args: vec![
                        Operand::Value(l),
                        Operand::Value(Value(9)),
                        Operand::Const(crate::ir::reexports::IrConst::I64(0)),
                    ],
                },
            ],
            Terminator::Return(None),
        )];
        let mut f = func_with_blocks(blocks);
        f.next_value_id = 10;
        assert_eq!(promote_function(&mut f, &table), 1);
        let insts = &f.blocks[0].instructions;
        assert_eq!(insts.len(), 2, "{insts:?}");
        assert!(matches!(
            &insts[1],
            Instruction::Intrinsic { args, .. } if args[0] == Operand::Value(x)
        ));
    }

    #[test]
    fn lane_access_keeps_the_object_in_memory() {
        crate::passes::vectorize::set_x86_simd_isa(true, false, true, false);
        let table = family_table();
        let f32x4 = fam(&table, IntrinsicOp::VecLoadF32x4);
        let a = Value(0);
        let blocks = vec![block(
            0,
            vec![
                Instruction::Alloca {
                    dest: a,
                    ty: IrType::Ptr,
                    size: 16,
                    align: 16,
                    volatile: false,
                    semantic_volatile: false,
                },
                Instruction::Intrinsic {
                    dest: Some(Value(1)),
                    op: f32x4.load,
                    dest_ptr: None,
                    args: vec![
                        Operand::Value(a),
                        Operand::Const(crate::ir::reexports::IrConst::I64(0)),
                    ],
                },
                Instruction::GetElementPtr {
                    dest: Value(2),
                    base: a,
                    offset: Operand::Const(crate::ir::reexports::IrConst::I64(4)),
                    ty: IrType::F32,
                },
            ],
            Terminator::Return(None),
        )];
        let mut f = func_with_blocks(blocks);
        f.next_value_id = 10;
        assert_eq!(promote_function(&mut f, &table), 0);
        assert_eq!(f.blocks[0].instructions.len(), 3);
    }
}
