//! Copy coalescing and immediately-consumed value analysis.
//!
//! Stack-slot coalescing is conservative move coalescing on a CFG-precise,
//! copy-aware interference graph (Chaitin/Briggs): for `d = copy s`, `d`
//! interferes with every value live after the copy except `s`. Non-interfering
//! scalar (or vector) webs share one home. Liveness uses dense bitsets and a
//! worklist so large functions still get a proof.
//!
//! CFG construction matches `liveness.rs` (terminator edges + InlineAsm goto)
//! so coalescing cannot under-approximate live sets.
//!
//! Immediately-consumed analysis finds GPR values produced and consumed in
//! adjacent instructions so they can stay in the accumulator cache and skip
//! a stack slot. Vector-defer skips the home-slot store of a vector result
//! that is consumed once from the last-store peephole.

use std::sync::OnceLock;

use crate::backend::liveness::{
    for_each_operand_in_instruction, for_each_operand_in_terminator,
    for_each_value_use_in_instruction,
};
use crate::backend::regalloc::{detect_phi_coalesce_groups, PhysReg};
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::reexports::{Instruction, IrConst, IrFunction, Operand, Terminator};

fn cfg_copy_coalesce_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("CCC_NO_CFG_COPY_COALESCE").is_none())
}

fn env_flag(name: &'static str) -> bool {
    std::env::var_os(name).is_some()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CoalesceClass {
    /// 8-byte integer/pointer stack home.
    Scalar,
    /// Auto-vectorizer Vec* SSA value (16/32-byte home, slot-aware emitters).
    Vector,
}

fn scalar_type(ty: IrType) -> bool {
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
            | IrType::Ptr
    )
}

fn scalar_const(c: IrConst) -> bool {
    matches!(
        c,
        IrConst::I8(_) | IrConst::I16(_) | IrConst::I32(_) | IrConst::I64(_) | IrConst::Zero
    )
}

/// True when codegen leaves the result in the GPR accumulator cache.
fn ty_lives_in_gpr_cache(ty: IrType) -> bool {
    scalar_type(ty) && !ty.is_float() && !ty.is_128bit() && !ty.is_long_double()
}

/// Classify values that share a uniform stack representation. Floats, i128/F128,
/// allocas and opaque results are excluded: a missed coalesce is harmless,
/// a slot-size mismatch is not.
fn collect_scalar_values(func: &IrFunction) -> FxHashMap<u32, CoalesceClass> {
    let mut classes: FxHashMap<u32, CoalesceClass> = FxHashMap::default();
    let mut allocas: FxHashSet<u32> = FxHashSet::default();
    let mut copy_dests_of: FxHashMap<u32, Vec<u32>> = FxHashMap::default();

    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca { dest, .. } = inst {
                allocas.insert(dest.0);
            }
            match inst {
                Instruction::BinOp { dest, ty, .. }
                | Instruction::UnaryOp { dest, ty, .. }
                | Instruction::Cast { dest, to_ty: ty, .. }
                | Instruction::Load { dest, ty, .. }
                | Instruction::Select { dest, ty, .. }
                | Instruction::AtomicLoad { dest, ty, .. }
                | Instruction::AtomicRmw { dest, ty, .. }
                | Instruction::AtomicCmpxchg { dest, ty, .. }
                | Instruction::ParamRef { dest, ty, .. }
                    if scalar_type(*ty) =>
                {
                    classes.insert(dest.0, CoalesceClass::Scalar);
                }
                Instruction::Cmp { dest, .. }
                | Instruction::GetElementPtr { dest, .. }
                | Instruction::GlobalAddr { dest, .. }
                | Instruction::LabelAddr { dest, .. } => {
                    classes.insert(dest.0, CoalesceClass::Scalar);
                }
                Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. }
                    if info.dest.is_some() && scalar_type(info.return_type) =>
                {
                    if let Some(d) = info.dest {
                        classes.insert(d.0, CoalesceClass::Scalar);
                    }
                }
                Instruction::Intrinsic {
                    dest: Some(d), op, ..
                } if op.vector_result_width().is_some() => {
                    classes.insert(d.0, CoalesceClass::Vector);
                }
                Instruction::Copy { dest, src } => {
                    if !allocas.contains(&dest.0) && !classes.contains_key(&dest.0) {
                        match src {
                            Operand::Value(v) => {
                                copy_dests_of.entry(v.0).or_default().push(dest.0);
                            }
                            Operand::Const(c) if scalar_const(*c) => {
                                classes.insert(dest.0, CoalesceClass::Scalar);
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let mut work: Vec<(u32, CoalesceClass)> = classes.iter().map(|(&id, &cls)| (id, cls)).collect();
    while let Some((id, cls)) = work.pop() {
        let Some(dests) = copy_dests_of.get(&id) else {
            continue;
        };
        for &dest in dests {
            if allocas.contains(&dest) || classes.contains_key(&dest) {
                continue;
            }
            classes.insert(dest, cls);
            work.push((dest, cls));
        }
    }

    for id in allocas {
        classes.remove(&id);
    }
    classes
}

/// Slots codegen re-reads around side effects (InlineAsm Phase 4).
fn collect_unsound_coalesce_ids(func: &IrFunction) -> FxHashSet<u32> {
    let mut ids = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::InlineAsm { outputs, .. } = inst {
                for (_, v, _) in outputs {
                    ids.insert(v.0);
                }
                for_each_operand_in_instruction(inst, |op| {
                    if let Operand::Value(v) = op {
                        ids.insert(v.0);
                    }
                });
                for_each_value_use_in_instruction(inst, |v| {
                    ids.insert(v.0);
                });
            }
        }
    }
    ids
}

fn collect_alloca_ids(func: &IrFunction) -> FxHashSet<u32> {
    let mut ids = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca { dest, .. } = inst {
                ids.insert(dest.0);
            }
        }
    }
    ids
}

fn for_each_instruction_value_use(inst: &Instruction, mut f: impl FnMut(u32)) {
    for_each_operand_in_instruction(inst, |op| {
        if let Operand::Value(v) = op {
            f(v.0);
        }
    });
    for_each_value_use_in_instruction(inst, |v| f(v.0));
}

fn for_each_terminator_value_use(term: &Terminator, mut f: impl FnMut(u32)) {
    for_each_operand_in_terminator(term, |op| {
        if let Operand::Value(v) = op {
            f(v.0);
        }
    });
}

fn instruction_uses_value(inst: &Instruction, id: u32) -> bool {
    let mut found = false;
    for_each_instruction_value_use(inst, |v| {
        if v == id {
            found = true;
        }
    });
    found
}

/// Successor lists matching `liveness.rs`: terminator edges + InlineAsm goto.
fn build_block_cfg(func: &IrFunction) -> (Vec<Vec<usize>>, Vec<Vec<usize>>) {
    let nblocks = func.blocks.len();
    let mut label_to_idx: FxHashMap<u32, usize> = FxHashMap::default();
    label_to_idx.reserve(nblocks);
    for (idx, block) in func.blocks.iter().enumerate() {
        label_to_idx.insert(block.label.0, idx);
    }

    let mut succs: Vec<Vec<usize>> = vec![Vec::new(); nblocks];
    let mut add_edge = |succs: &mut [Vec<usize>], from: usize, label: u32| {
        if let Some(&to) = label_to_idx.get(&label) {
            if !succs[from].contains(&to) {
                succs[from].push(to);
            }
        }
    };

    for (idx, block) in func.blocks.iter().enumerate() {
        match &block.terminator {
            Terminator::Branch(target) => add_edge(&mut succs, idx, target.0),
            Terminator::CondBranch {
                true_label,
                false_label,
                ..
            } => {
                add_edge(&mut succs, idx, true_label.0);
                add_edge(&mut succs, idx, false_label.0);
            }
            Terminator::IndirectBranch {
                possible_targets, ..
            } => {
                for t in possible_targets {
                    add_edge(&mut succs, idx, t.0);
                }
            }
            Terminator::Switch { cases, default, .. } => {
                add_edge(&mut succs, idx, default.0);
                for (_, label) in cases {
                    add_edge(&mut succs, idx, label.0);
                }
            }
            _ => {}
        }
        for inst in &block.instructions {
            if let Instruction::InlineAsm { goto_labels, .. } = inst {
                for (_, label) in goto_labels {
                    add_edge(&mut succs, idx, label.0);
                }
            }
        }
    }

    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); nblocks];
    for (src, dests) in succs.iter().enumerate() {
        for &d in dests {
            preds[d].push(src);
        }
    }
    (succs, preds)
}

fn instruction_def_id(inst: &Instruction) -> Option<u32> {
    if let Some(dest) = inst.dest() {
        return Some(dest.0);
    }
    match inst {
        Instruction::Copy { dest, .. } | Instruction::Phi { dest, .. } => Some(dest.0),
        _ => None,
    }
}

// ── Dense bitsets for liveness + interference ─────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
struct BitSet {
    words: Vec<u64>,
    nbits: usize,
}

impl BitSet {
    fn new(n: usize) -> Self {
        Self {
            words: vec![0; n.saturating_add(63) / 64],
            nbits: n,
        }
    }

    fn clear(&mut self) {
        for w in &mut self.words {
            *w = 0;
        }
    }

    #[inline]
    fn insert(&mut self, i: usize) {
        if i < self.nbits {
            self.words[i / 64] |= 1u64 << (i % 64);
        }
    }

    #[inline]
    fn remove(&mut self, i: usize) {
        if i < self.nbits {
            self.words[i / 64] &= !(1u64 << (i % 64));
        }
    }

    #[inline]
    fn contains(&self, i: usize) -> bool {
        if i >= self.nbits {
            return false;
        }
        self.words[i / 64] & (1u64 << (i % 64)) != 0
    }

    #[inline]
    fn union_with(&mut self, other: &Self) -> bool {
        let mut changed = false;
        for (a, b) in self.words.iter_mut().zip(other.words.iter()) {
            let n = *a | *b;
            changed |= n != *a;
            *a = n;
        }
        changed
    }

    #[inline]
    fn intersects(&self, other: &Self) -> bool {
        self.words
            .iter()
            .zip(other.words.iter())
            .any(|(a, b)| a & b != 0)
    }

    /// `self |= other & !mask`
    #[inline]
    fn or_and_not(&mut self, other: &Self, mask: &Self) {
        for ((a, b), m) in self
            .words
            .iter_mut()
            .zip(other.words.iter())
            .zip(mask.words.iter())
        {
            *a |= *b & !*m;
        }
    }

    fn for_each(&self, mut f: impl FnMut(usize)) {
        for (wi, &word) in self.words.iter().enumerate() {
            let mut bits = word;
            while bits != 0 {
                let b = bits.trailing_zeros() as usize;
                let i = wi * 64 + b;
                if i < self.nbits {
                    f(i);
                }
                bits &= bits - 1;
            }
        }
    }
}

fn uf_find(parent: &mut [usize], mut x: usize) -> usize {
    let mut root = x;
    while parent[root] != root {
        root = parent[root];
    }
    while parent[x] != root {
        let next = parent[x];
        parent[x] = root;
        x = next;
    }
    root
}

/// Backward liveness over real successor edges, then copy-aware interference:
/// `d = copy s` interferes with every value live after the copy except `s`.
///
/// Worklist + dense bitsets. The lattice is finite and the transfer is
/// monotone. A defensive bound of `|B|·(|V|+2)` updates is the lattice
/// height and only fires on an implementation bug.
fn cfg_copy_interference(
    func: &IrFunction,
    dense: &FxHashMap<u32, usize>,
) -> Option<Vec<BitSet>> {
    let n = dense.len();
    if n == 0 {
        return Some(Vec::new());
    }
    let nblocks = func.blocks.len();
    if nblocks == 0 {
        return Some(vec![BitSet::new(n); n]);
    }

    let (succs, preds) = build_block_cfg(func);

    let mut block_use: Vec<BitSet> = vec![BitSet::new(n); nblocks];
    let mut block_def: Vec<BitSet> = vec![BitSet::new(n); nblocks];
    for (block_idx, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            for_each_instruction_value_use(inst, |value| {
                if let Some(&id) = dense.get(&value) {
                    if !block_def[block_idx].contains(id) {
                        block_use[block_idx].insert(id);
                    }
                }
            });
            if let Some(dest) = instruction_def_id(inst) {
                if let Some(&id) = dense.get(&dest) {
                    block_def[block_idx].insert(id);
                }
            }
        }
        for_each_terminator_value_use(&block.terminator, |value| {
            if let Some(&id) = dense.get(&value) {
                if !block_def[block_idx].contains(id) {
                    block_use[block_idx].insert(id);
                }
            }
        });
    }

    // Unlowered phi incoming is a use at the predecessor terminator
    // (same contract as `liveness.rs`). After phi elim this is a no-op.
    let mut label_to_idx: FxHashMap<u32, usize> = FxHashMap::default();
    for (idx, block) in func.blocks.iter().enumerate() {
        label_to_idx.insert(block.label.0, idx);
    }
    for (block_idx, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            if let Instruction::Phi { dest, incoming, .. } = inst {
                if let Some(&id) = dense.get(&dest.0) {
                    block_def[block_idx].insert(id);
                }
                for (op, pred_label) in incoming {
                    if let Operand::Value(v) = op {
                        if let Some(&id) = dense.get(&v.0) {
                            if let Some(&pred) = label_to_idx.get(&pred_label.0) {
                                if !block_def[pred].contains(id) {
                                    block_use[pred].insert(id);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let mut live_in: Vec<BitSet> = vec![BitSet::new(n); nblocks];
    let mut live_out: Vec<BitSet> = vec![BitSet::new(n); nblocks];
    let mut work: Vec<usize> = (0..nblocks).rev().collect();
    let mut on_work = vec![true; nblocks];
    let max_steps = nblocks
        .saturating_mul(n.saturating_add(2))
        .saturating_add(16);
    let mut steps = 0usize;
    let mut scratch = BitSet::new(n);
    let mut out = BitSet::new(n);

    while let Some(block_idx) = work.pop() {
        on_work[block_idx] = false;
        steps += 1;
        if steps > max_steps {
            return None;
        }

        out.clear();
        for &succ in &succs[block_idx] {
            if succ < nblocks {
                out.union_with(&live_in[succ]);
            }
        }

        scratch.clone_from(&block_use[block_idx]);
        scratch.or_and_not(&out, &block_def[block_idx]);

        if out != live_out[block_idx] {
            live_out[block_idx].clone_from(&out);
        }
        if scratch != live_in[block_idx] {
            live_in[block_idx].clone_from(&scratch);
            for &p in &preds[block_idx] {
                if !on_work[p] {
                    on_work[p] = true;
                    work.push(p);
                }
            }
        }
    }

    let mut adj = vec![BitSet::new(n); n];
    let mut live = BitSet::new(n);
    for (block_idx, block) in func.blocks.iter().enumerate() {
        live.clone_from(&live_out[block_idx]);
        for_each_terminator_value_use(&block.terminator, |value| {
            if let Some(&id) = dense.get(&value) {
                live.insert(id);
            }
        });
        // Phi incoming of successors is live at this terminator.
        for &succ in &succs[block_idx] {
            if succ >= nblocks {
                continue;
            }
            for inst in &func.blocks[succ].instructions {
                if let Instruction::Phi { incoming, .. } = inst {
                    for (op, pred_label) in incoming {
                        if pred_label.0 == func.blocks[block_idx].label.0 {
                            if let Operand::Value(v) = op {
                                if let Some(&id) = dense.get(&v.0) {
                                    live.insert(id);
                                }
                            }
                        }
                    }
                }
            }
        }
        for inst in block.instructions.iter().rev() {
            let copy_source = match inst {
                Instruction::Copy {
                    src: Operand::Value(source),
                    ..
                } => dense.get(&source.0).copied(),
                _ => None,
            };
            if let Some(dest) = instruction_def_id(inst) {
                if let Some(&di) = dense.get(&dest) {
                    live.for_each(|other| {
                        if copy_source != Some(other) && other != di {
                            adj[di].insert(other);
                            adj[other].insert(di);
                        }
                    });
                    live.remove(di);
                }
            }
            for_each_instruction_value_use(inst, |value| {
                if let Some(&id) = dense.get(&value) {
                    live.insert(id);
                }
            });
        }
    }
    Some(adj)
}

/// CFG-aware stack-copy coalescing. Only the stack-slot alias decision;
/// phi lowering, RA and copy emission are unchanged.
fn build_cfg_copy_alias_map(
    func: &IrFunction,
    multi_def_values: &FxHashSet<u32>,
    reg_assigned: &FxHashMap<u32, PhysReg>,
) -> (FxHashMap<u32, u32>, FxHashSet<u32>) {
    let classes = collect_scalar_values(func);
    let unsound = collect_unsound_coalesce_ids(func);

    let mut candidates: Vec<(u32, u32)> = Vec::new();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Copy {
                dest,
                src: Operand::Value(source),
            } = inst
            {
                if dest.0 == source.0
                    || reg_assigned.contains_key(&dest.0)
                    || reg_assigned.contains_key(&source.0)
                    || unsound.contains(&dest.0)
                    || unsound.contains(&source.0)
                {
                    continue;
                }
                let (Some(&dest_cls), Some(&src_cls)) =
                    (classes.get(&dest.0), classes.get(&source.0))
                else {
                    continue;
                };
                // Never share an 8-byte scalar home with a 16/32-byte vector home.
                if dest_cls != src_cls {
                    continue;
                }
                candidates.push((dest.0, source.0));
            }
        }
    }
    // Prefer copies that touch a multi-def (phi-web / loop-carried) value.
    candidates.sort_unstable_by(|a, b| {
        let score = |dest: u32, src: u32| {
            u8::from(multi_def_values.contains(&dest)) + u8::from(multi_def_values.contains(&src))
        };
        score(b.0, b.1)
            .cmp(&score(a.0, a.1))
            .then(a.0.cmp(&b.0))
            .then(a.1.cmp(&b.1))
    });
    candidates.dedup();
    if candidates.is_empty() {
        return (FxHashMap::default(), FxHashSet::default());
    }

    let tracked: FxHashSet<u32> = candidates
        .iter()
        .flat_map(|(dest, source)| [*dest, *source])
        .collect();

    let mut val_of: Vec<u32> = Vec::with_capacity(tracked.len());
    let mut dense: FxHashMap<u32, usize> = FxHashMap::default();
    for &v in &tracked {
        dense.insert(v, val_of.len());
        val_of.push(v);
    }
    let n = val_of.len();

    let Some(adj) = cfg_copy_interference(func, &dense) else {
        return (FxHashMap::default(), FxHashSet::default());
    };

    let mut parent: Vec<usize> = (0..n).collect();
    let mut members: Vec<BitSet> = (0..n)
        .map(|i| {
            let mut s = BitSet::new(n);
            s.insert(i);
            s
        })
        .collect();
    let mut inter = adj;

    for (dest, source) in candidates {
        let Some(&d) = dense.get(&dest) else {
            continue;
        };
        let Some(&s) = dense.get(&source) else {
            continue;
        };
        let dest_root = uf_find(&mut parent, d);
        let source_root = uf_find(&mut parent, s);
        if dest_root == source_root {
            continue;
        }
        if inter[dest_root].intersects(&members[source_root])
            || inter[source_root].intersects(&members[dest_root])
        {
            continue;
        }
        // Multi-def phi dest owns the web (cross-block lifetime). Else lower ID.
        let dest_root_val = val_of[dest_root];
        let source_root_val = val_of[source_root];
        let owner = match (
            multi_def_values.contains(&dest_root_val),
            multi_def_values.contains(&source_root_val),
        ) {
            (true, false) => dest_root,
            (false, true) => source_root,
            _ => {
                if dest_root_val <= source_root_val {
                    dest_root
                } else {
                    source_root
                }
            }
        };
        let other = if owner == dest_root {
            source_root
        } else {
            dest_root
        };
        let other_members = members[other].clone();
        let other_inter = inter[other].clone();
        members[owner].union_with(&other_members);
        inter[owner].union_with(&other_inter);
        parent[other] = owner;
    }

    let mut groups: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    for i in 0..n {
        let root = uf_find(&mut parent, i);
        groups.entry(val_of[root]).or_default().push(val_of[i]);
    }

    let mut aliases = FxHashMap::default();
    let mut force_aliases = FxHashSet::default();
    for (root, mut group_members) in groups {
        group_members.sort_unstable();
        if group_members.len() < 2 {
            continue;
        }
        let preferred_root = group_members
            .iter()
            .copied()
            .filter(|id| multi_def_values.contains(id))
            .min()
            .unwrap_or(root);
        for member in group_members {
            if member != preferred_root {
                aliases.insert(member, preferred_root);
                force_aliases.insert(member);
            }
        }
    }

    if env_flag("CCC_DEBUG_SLOT_COALESCE") && !aliases.is_empty() {
        let mut pairs: Vec<(u32, u32)> = aliases.iter().map(|(&a, &b)| (a, b)).collect();
        pairs.sort_unstable();
        eprintln!(
            "[CFG-SLOT-COALESCE] fn={} candidates={} aliases={} pairs={:?}",
            func.name,
            tracked.len(),
            pairs.len(),
            pairs,
        );
    }
    (aliases, force_aliases)
}

/// Build dest_id → root_id for Copy instructions that can share a stack slot.
///
/// Returns `(copy_alias, phi_web_aliases, loop_phi_aliases)`:
/// - `phi_web_aliases` need force-overwrite in `resolve_copy_aliases`
/// - `loop_phi_aliases` are certified by `detect_phi_coalesce_groups` and
///   skip the generic def/last-use check (legacy path only)
pub(super) fn build_copy_alias_map(
    func: &IrFunction,
    def_block: &FxHashMap<u32, usize>,
    multi_def_values: &FxHashSet<u32>,
    reg_assigned: &FxHashMap<u32, PhysReg>,
    use_blocks_map: &FxHashMap<u32, Vec<usize>>,
    cached_liveness: &Option<crate::backend::liveness::LivenessResult>,
) -> (FxHashMap<u32, u32>, FxHashSet<u32>, FxHashSet<u32>) {
    if cfg_copy_coalesce_enabled() {
        let (aliases, force) = build_cfg_copy_alias_map(func, multi_def_values, reg_assigned);
        return (aliases, force, FxHashSet::default());
    }

    let classes = collect_scalar_values(func);
    let unsound = collect_unsound_coalesce_ids(func);

    let mut use_count: FxHashMap<u32, u32> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            for_each_instruction_value_use(inst, |v| {
                *use_count.entry(v).or_insert(0) += 1;
            });
        }
        for_each_terminator_value_use(&block.terminator, |v| {
            *use_count.entry(v).or_insert(0) += 1;
        });
    }

    let same_class = |a: u32, b: u32| -> bool {
        match (classes.get(&a), classes.get(&b)) {
            (Some(x), Some(y)) => x == y,
            _ => false,
        }
    };

    let mut raw_aliases: Vec<(u32, u32)> = Vec::new();
    for (blk_idx, block) in func.blocks.iter().enumerate() {
        for (inst_idx, inst) in block.instructions.iter().enumerate() {
            if let Instruction::Copy {
                dest,
                src: Operand::Value(src_val),
            } = inst
            {
                let d = dest.0;
                let s = src_val.0;
                if d == s
                    || multi_def_values.contains(&s)
                    || reg_assigned.contains_key(&d)
                    || reg_assigned.contains_key(&s)
                    || unsound.contains(&d)
                    || unsound.contains(&s)
                    || !same_class(d, s)
                {
                    continue;
                }
                let sole_use = use_count.get(&s).copied().unwrap_or(0) == 1;
                if !sole_use {
                    let src_only_in_this_block = use_blocks_map
                        .get(&s)
                        .map(|blks| blks.iter().all(|&b| b == blk_idx))
                        .unwrap_or(true);
                    if !src_only_in_this_block {
                        continue;
                    }
                    let mut used_after = false;
                    for later_inst in &block.instructions[inst_idx + 1..] {
                        if instruction_uses_value(later_inst, s) {
                            used_after = true;
                            break;
                        }
                    }
                    if !used_after {
                        for_each_terminator_value_use(&block.terminator, |v| {
                            if v == s {
                                used_after = true;
                            }
                        });
                    }
                    if used_after {
                        continue;
                    }
                }

                let src_in_copy_block = def_block.get(&s).copied() == Some(blk_idx);
                let dest_cross_block = use_blocks_map
                    .get(&d)
                    .map(|blks| blks.iter().any(|&b| b != blk_idx))
                    .unwrap_or(false);

                if src_in_copy_block && dest_cross_block {
                    // Phi-copy: src dies here, dest is the wider-live slot owner.
                    raw_aliases.push((s, d));
                    continue;
                }

                if multi_def_values.contains(&d) || dest_cross_block {
                    continue;
                }
                raw_aliases.push((d, s));
            }
        }
    }

    // Phi-web: coalesce src → dest for multi-def dest iff src is not live at
    // any rival dest-write. Only web members may join (external feeds keep
    // their own home — otherwise the elided copy leaves a stale value).
    let mut phi_web_aliases: FxHashSet<u32> = FxHashSet::default();
    if !env_flag("CCC_NO_PHI_WEB_COALESCE") {
        let mut phi_copies: FxHashMap<u32, Vec<(u32, usize, u32)>> = FxHashMap::default();
        let mut dest_copy_points: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
        let already_aliased: FxHashSet<u32> = raw_aliases.iter().map(|&(a, _)| a).collect();
        {
            let mut pp: u32 = 0;
            for (blk_idx, block) in func.blocks.iter().enumerate() {
                for inst in &block.instructions {
                    if let Instruction::Copy {
                        dest,
                        src: Operand::Value(src_val),
                    } = inst
                    {
                        let d = dest.0;
                        let s = src_val.0;
                        if multi_def_values.contains(&d)
                            && !reg_assigned.contains_key(&s)
                            && !reg_assigned.contains_key(&d)
                            && !already_aliased.contains(&s)
                            && !unsound.contains(&d)
                            && !unsound.contains(&s)
                            && same_class(d, s)
                        {
                            phi_copies.entry(d).or_default().push((s, blk_idx, pp));
                            dest_copy_points.entry(d).or_default().push(pp);
                        }
                    }
                    pp = pp.saturating_add(1);
                }
                pp = pp.saturating_add(1);
            }
        }

        // Program-point numbering matches `liveness.rs` (1 per inst + 1 per term).
        let interval_map: FxHashMap<u32, (u32, u32)> = cached_liveness
            .as_ref()
            .map(|lr| {
                lr.intervals
                    .iter()
                    .map(|iv| (iv.value_id, (iv.start, iv.end)))
                    .collect()
            })
            .unwrap_or_default();

        for (dest_id, sources) in &phi_copies {
            if sources.len() < 2 || reg_assigned.contains_key(dest_id) {
                continue;
            }
            let Some(all_copy_points) = dest_copy_points.get(dest_id) else {
                continue;
            };

            for &(src_id, _src_blk, src_copy_pp) in sources {
                if !phi_copies.contains_key(&src_id) && !multi_def_values.contains(&src_id) {
                    continue;
                }

                let other_copy_points: Vec<u32> = all_copy_points
                    .iter()
                    .copied()
                    .filter(|&pp| pp != src_copy_pp)
                    .collect();

                let interferes = if let Some(&(start, end)) = interval_map.get(&src_id) {
                    other_copy_points
                        .iter()
                        .any(|&pp| start <= pp && pp <= end)
                } else {
                    let other_def_blks: FxHashSet<usize> = sources
                        .iter()
                        .filter(|&&(_, _, pp)| pp != src_copy_pp)
                        .map(|&(_, blk, _)| blk)
                        .collect();

                    let mut interferes_fallback = false;
                    'blocks: for (blk_idx, block) in func.blocks.iter().enumerate() {
                        if !other_def_blks.contains(&blk_idx) {
                            continue;
                        }
                        for inst in &block.instructions {
                            if let Instruction::Copy {
                                dest: copy_dest,
                                src: Operand::Value(copy_src),
                            } = inst
                            {
                                if copy_dest.0 == *dest_id && copy_src.0 == src_id {
                                    continue;
                                }
                            }
                            if instruction_uses_value(inst, src_id) {
                                interferes_fallback = true;
                                break 'blocks;
                            }
                        }
                        for_each_terminator_value_use(&block.terminator, |v| {
                            if v == src_id {
                                interferes_fallback = true;
                            }
                        });
                        if interferes_fallback {
                            break;
                        }
                    }
                    interferes_fallback
                };

                if !interferes {
                    raw_aliases.push((src_id, *dest_id));
                    phi_web_aliases.insert(src_id);
                }
            }
        }
    }

    // Loop-backedge phi: const-init + one backedge Copy never enters the
    // 2-source phi-web. `detect_phi_coalesce_groups` proves dest is dead
    // after the backedge source def, so they may share the dest slot.
    let mut loop_phi_aliases: FxHashSet<u32> = FxHashSet::default();
    if !env_flag("CCC_NO_LOOP_PHI_SLOT") {
        if let Some(liveness) = cached_liveness {
            let mut def_ty_ok: FxHashMap<u32, bool> = FxHashMap::default();
            let mut copy_of: Vec<(u32, u32)> = Vec::new();
            for block in &func.blocks {
                for inst in &block.instructions {
                    let (d, ok) = match inst {
                        Instruction::BinOp { dest, ty, .. }
                        | Instruction::UnaryOp { dest, ty, .. }
                        | Instruction::Load { dest, ty, .. }
                        | Instruction::Select { dest, ty, .. }
                        | Instruction::AtomicLoad { dest, ty, .. }
                        | Instruction::AtomicRmw { dest, ty, .. }
                        | Instruction::AtomicCmpxchg { dest, ty, .. }
                        | Instruction::ParamRef { dest, ty, .. } => (dest.0, scalar_type(*ty)),
                        Instruction::Cmp { dest, .. }
                        | Instruction::GetElementPtr { dest, .. }
                        | Instruction::GlobalAddr { dest, .. }
                        | Instruction::LabelAddr { dest, .. } => (dest.0, true),
                        Instruction::Cast {
                            dest, to_ty, from_ty, ..
                        } => (dest.0, scalar_type(*to_ty) && scalar_type(*from_ty)),
                        Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
                            match info.dest {
                                Some(d) => (d.0, scalar_type(info.return_type)),
                                None => continue,
                            }
                        }
                        Instruction::Copy {
                            dest,
                            src: Operand::Const(c),
                        } if scalar_const(*c) => (dest.0, true),
                        Instruction::Copy {
                            dest,
                            src: Operand::Value(src),
                        } => {
                            copy_of.push((dest.0, src.0));
                            continue;
                        }
                        _ => continue,
                    };
                    def_ty_ok.insert(d, ok);
                }
            }
            let mut changed = true;
            while changed {
                changed = false;
                for &(d, s) in &copy_of {
                    let Some(&ok) = def_ty_ok.get(&s) else {
                        continue;
                    };
                    match def_ty_ok.get(&d).copied() {
                        None => {
                            def_ty_ok.insert(d, ok);
                            changed = true;
                        }
                        Some(true) if !ok => {
                            def_ty_ok.insert(d, false);
                            changed = true;
                        }
                        _ => {}
                    }
                }
            }

            let already_aliased: FxHashSet<u32> = raw_aliases.iter().map(|&(a, _)| a).collect();
            let mut claimed_dests: FxHashSet<u32> = FxHashSet::default();
            let debug_loop_phi = env_flag("CCC_DEBUG_LOOP_PHI");
            for cand in detect_phi_coalesce_groups(func, liveness) {
                let (phi_dest, backedge_src) = (cand.phi_dest, cand.backedge_src);
                let src_ty_ok = def_ty_ok.get(&backedge_src).copied().unwrap_or(false);
                if debug_loop_phi {
                    eprintln!(
                        "[LOOP_PHI] func={} pair dest=v{} src=v{} dest_reg={} src_reg={} aliased={} ty_ok={}",
                        func.name,
                        phi_dest,
                        backedge_src,
                        reg_assigned.contains_key(&phi_dest),
                        reg_assigned.contains_key(&backedge_src),
                        already_aliased.contains(&backedge_src),
                        src_ty_ok,
                    );
                }
                if reg_assigned.contains_key(&phi_dest)
                    || reg_assigned.contains_key(&backedge_src)
                    || unsound.contains(&phi_dest)
                    || unsound.contains(&backedge_src)
                {
                    continue;
                }
                if already_aliased.contains(&backedge_src) {
                    if src_ty_ok {
                        loop_phi_aliases.insert(backedge_src);
                    }
                    continue;
                }
                if !src_ty_ok {
                    continue;
                }
                if !claimed_dests.insert(phi_dest) {
                    continue;
                }
                raw_aliases.push((backedge_src, phi_dest));
                phi_web_aliases.insert(backedge_src);
                loop_phi_aliases.insert(backedge_src);
            }
        }
    }

    const MAX_ALIAS_CHAIN_DEPTH: usize = 64;
    let mut copy_alias: FxHashMap<u32, u32> = FxHashMap::default();
    for (dest_id, src_id) in raw_aliases {
        if dest_id == src_id {
            continue;
        }
        let origin = src_id;
        let mut root = src_id;
        let mut depth = 0usize;
        let mut refuse = false;
        while let Some(&parent) = copy_alias.get(&root) {
            if parent == dest_id || parent == root || parent == origin {
                refuse = true;
                break;
            }
            root = parent;
            depth += 1;
            if depth > MAX_ALIAS_CHAIN_DEPTH {
                refuse = true;
                break;
            }
        }
        if !refuse && root != dest_id {
            copy_alias.insert(dest_id, root);
        }
    }

    let alloca_ids = collect_alloca_ids(func);
    if !alloca_ids.is_empty() {
        copy_alias.retain(|dest_id, root_id| {
            !alloca_ids.contains(root_id) && !alloca_ids.contains(dest_id)
        });
    }
    if !unsound.is_empty() {
        copy_alias.retain(|dest_id, root_id| !unsound.contains(dest_id) && !unsound.contains(root_id));
    }

    loop_phi_aliases.retain(|v| copy_alias.contains_key(v));
    phi_web_aliases.retain(|v| copy_alias.contains_key(v));

    if env_flag("CCC_DEBUG_SLOT_COALESCE") && !copy_alias.is_empty() {
        let mut aliases: Vec<(u32, u32)> = copy_alias
            .iter()
            .map(|(&dest, &root)| (dest, root))
            .collect();
        aliases.sort_unstable();
        eprintln!(
            "[SLOT-COALESCE] fn={} aliases={} phi_web_aliases={} loop_phi_aliases={} pairs={:?}",
            func.name,
            aliases.len(),
            phi_web_aliases.len(),
            loop_phi_aliases.len(),
            aliases,
        );
    }

    (copy_alias, phi_web_aliases, loop_phi_aliases)
}

/// Values that can skip a stack slot: produced into the GPR accumulator and
/// consumed as the first operand of the next instruction (or the terminator).
///
/// `lhs_first_binop` is true on backends that always load BinOp/Cmp lhs
/// before rhs (RISC-V). x86/ARM rhs-conflict / operand-swap paths are unsafe.
pub(super) fn compute_immediately_consumed(
    func: &IrFunction,
    lhs_first_binop: bool,
) -> FxHashSet<u32> {
    let mut result = FxHashSet::default();

    let mut operand_use_count: FxHashMap<u32, u32> = FxHashMap::default();
    let mut has_value_ref_use: FxHashSet<u32> = FxHashSet::default();
    let mut copy_alias_roots: FxHashSet<u32> = FxHashSet::default();

    for block in &func.blocks {
        for inst in &block.instructions {
            for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    *operand_use_count.entry(v.0).or_insert(0) += 1;
                }
            });
            for_each_value_use_in_instruction(inst, |v| {
                has_value_ref_use.insert(v.0);
            });
            if let Instruction::Copy {
                src: Operand::Value(v),
                ..
            } = inst
            {
                copy_alias_roots.insert(v.0);
            }
        }
        for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                *operand_use_count.entry(v.0).or_insert(0) += 1;
            }
        });
    }

    for block in &func.blocks {
        let insts = &block.instructions;
        for (i, inst) in insts.iter().enumerate() {
            let Some(dest) = inst.dest() else {
                continue;
            };
            if !is_gpr_acc_preserving_producer(inst) {
                continue;
            }
            if has_value_ref_use.contains(&dest.0) || copy_alias_roots.contains(&dest.0) {
                continue;
            }
            if operand_use_count.get(&dest.0).copied().unwrap_or(0) != 1 {
                continue;
            }

            if i + 1 < insts.len() {
                if is_safe_sole_consumer(&insts[i + 1], dest.0, lhs_first_binop) {
                    result.insert(dest.0);
                }
            } else if is_sole_operand_of_terminator(&block.terminator, dest.0) {
                result.insert(dest.0);
            }
        }
    }

    result
}

/// Producer leaves its GPR result in the accumulator cache.
fn is_gpr_acc_preserving_producer(inst: &Instruction) -> bool {
    match inst {
        Instruction::Load { ty, .. }
        | Instruction::BinOp { ty, .. }
        | Instruction::UnaryOp { ty, .. }
        | Instruction::Select { ty, .. } => ty_lives_in_gpr_cache(*ty),
        Instruction::Cast { from_ty, to_ty, .. } => {
            ty_lives_in_gpr_cache(*to_ty) && !from_ty.is_128bit() && !from_ty.is_long_double()
        }
        Instruction::Cmp { ty, .. } => ty_lives_in_gpr_cache(*ty),
        Instruction::GetElementPtr { .. }
        | Instruction::GlobalAddr { .. }
        | Instruction::LabelAddr { .. } => true,
        _ => false,
    }
}

/// `value_id` is the first (and, for default targets, only) operand loaded.
fn is_safe_sole_consumer(inst: &Instruction, value_id: u32, lhs_first_binop: bool) -> bool {
    match inst {
        Instruction::Store {
            val: Operand::Value(v),
            ..
        } => v.0 == value_id,
        Instruction::Cast {
            src: Operand::Value(v),
            ..
        } => v.0 == value_id,
        Instruction::UnaryOp {
            src: Operand::Value(v),
            ..
        } => v.0 == value_id,
        Instruction::Copy {
            src: Operand::Value(v),
            ..
        } => v.0 == value_id,
        Instruction::BinOp {
            lhs: Operand::Value(v),
            ..
        } if lhs_first_binop => v.0 == value_id,
        Instruction::Cmp {
            lhs: Operand::Value(v),
            ..
        } if lhs_first_binop => v.0 == value_id,
        _ => false,
    }
}

fn is_sole_operand_of_terminator(term: &Terminator, value_id: u32) -> bool {
    let mut saw = false;
    let mut extra = false;
    for_each_terminator_value_use(term, |id| {
        if id == value_id {
            saw = true;
        } else {
            extra = true;
        }
    });
    saw && !extra
}

#[cfg(test)]
mod cfg_copy_coalesce_tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, IrBinOp, Value};

    fn block(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions,
            source_spans: Vec::new(),
            terminator,
        }
    }

    fn scalar_def(dest: u32, value: i32) -> Instruction {
        Instruction::BinOp {
            dest: Value(dest),
            op: IrBinOp::Add,
            lhs: Operand::Const(IrConst::I32(value)),
            rhs: Operand::Const(IrConst::I32(0)),
            ty: IrType::I32,
        }
    }

    #[test]
    fn cfg_copy_coalesces_a_straight_line_copy() {
        let mut func = IrFunction::new("straight".to_string(), IrType::I32, vec![], false);
        func.blocks.push(block(
            0,
            vec![
                scalar_def(0, 7),
                Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Value(Value(0)),
                },
            ],
            Terminator::Return(Some(Operand::Value(Value(1)))),
        ));
        let (aliases, force) =
            build_cfg_copy_alias_map(&func, &FxHashSet::default(), &FxHashMap::default());
        assert_eq!(aliases.get(&1), Some(&0));
        assert!(force.contains(&1));
    }

    #[test]
    fn cfg_copy_rejects_a_phi_edge_source_live_on_another_path() {
        // d is a lowered phi: d = x on the left, d = y on the right.
        // x is used after the join, so x and d must not share a slot.
        let mut func = IrFunction::new("diamond".to_string(), IrType::I32, vec![], false);
        func.blocks = vec![
            block(
                0,
                vec![scalar_def(0, 11)],
                Terminator::CondBranch {
                    cond: Operand::Const(IrConst::I32(1)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            block(
                1,
                vec![Instruction::Copy {
                    dest: Value(2),
                    src: Operand::Value(Value(0)),
                }],
                Terminator::Branch(BlockId(3)),
            ),
            block(
                2,
                vec![
                    scalar_def(1, 22),
                    Instruction::Copy {
                        dest: Value(2),
                        src: Operand::Value(Value(1)),
                    },
                ],
                Terminator::Branch(BlockId(3)),
            ),
            block(
                3,
                vec![Instruction::BinOp {
                    dest: Value(3),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Value(Value(2)),
                    ty: IrType::I32,
                }],
                Terminator::Return(Some(Operand::Value(Value(3)))),
            ),
        ];
        let mut multi_def = FxHashSet::default();
        multi_def.insert(2);
        let (aliases, _) = build_cfg_copy_alias_map(&func, &multi_def, &FxHashMap::default());
        assert_ne!(aliases.get(&0), Some(&2));
        assert_ne!(aliases.get(&2), Some(&0));
        assert_eq!(aliases.get(&1), Some(&2));
    }

    #[test]
    fn cfg_copy_coalesces_loop_carried_phi_sources() {
        // init is dead after its edge copy and may share the phi home.
        // next is redefined at the header; CFG liveness keeps it distinct
        // because the exit still reads the pre-increment phi.
        let mut func = IrFunction::new("loop_phi".to_string(), IrType::I32, vec![], false);
        func.blocks = vec![
            block(
                0,
                vec![
                    scalar_def(0, 3),
                    Instruction::Copy {
                        dest: Value(2),
                        src: Operand::Value(Value(0)),
                    },
                ],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![Instruction::BinOp {
                    dest: Value(1),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                }],
                Terminator::CondBranch {
                    cond: Operand::Const(IrConst::I32(1)),
                    true_label: BlockId(2),
                    false_label: BlockId(3),
                },
            ),
            block(
                2,
                vec![Instruction::Copy {
                    dest: Value(2),
                    src: Operand::Value(Value(1)),
                }],
                Terminator::Branch(BlockId(1)),
            ),
            block(3, vec![], Terminator::Return(Some(Operand::Value(Value(2))))),
        ];
        let mut multi_def = FxHashSet::default();
        multi_def.insert(2);
        let (aliases, _) = build_cfg_copy_alias_map(&func, &multi_def, &FxHashMap::default());
        assert_eq!(aliases.get(&0), Some(&2));
        assert_ne!(aliases.get(&1), Some(&2));
    }

    #[test]
    fn cfg_copy_excludes_i128_from_scalar_slot_aliasing() {
        let mut func = IrFunction::new("wide".to_string(), IrType::I32, vec![], false);
        func.blocks.push(block(
            0,
            vec![
                Instruction::BinOp {
                    dest: Value(0),
                    op: IrBinOp::Add,
                    lhs: Operand::Const(IrConst::I128(1)),
                    rhs: Operand::Const(IrConst::I128(2)),
                    ty: IrType::I128,
                },
                Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Value(Value(0)),
                },
            ],
            Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
        ));
        let (aliases, _) =
            build_cfg_copy_alias_map(&func, &FxHashSet::default(), &FxHashMap::default());
        assert!(aliases.is_empty());
    }

    #[test]
    fn cfg_copy_refuses_self_copy() {
        let mut func = IrFunction::new("self".to_string(), IrType::I32, vec![], false);
        func.blocks.push(block(
            0,
            vec![
                scalar_def(0, 1),
                Instruction::Copy {
                    dest: Value(0),
                    src: Operand::Value(Value(0)),
                },
            ],
            Terminator::Return(Some(Operand::Value(Value(0)))),
        ));
        let (aliases, _) =
            build_cfg_copy_alias_map(&func, &FxHashSet::default(), &FxHashMap::default());
        assert!(aliases.is_empty());
    }

    #[test]
    fn immediately_consumed_picks_adjacent_cast() {
        let mut func = IrFunction::new("adj".to_string(), IrType::I32, vec![], false);
        func.blocks.push(block(
            0,
            vec![
                scalar_def(0, 9),
                Instruction::Cast {
                    dest: Value(1),
                    src: Operand::Value(Value(0)),
                    from_ty: IrType::I32,
                    to_ty: IrType::I64,
                },
            ],
            Terminator::Return(Some(Operand::Value(Value(1)))),
        ));
        let skip = compute_immediately_consumed(&func, false);
        assert!(skip.contains(&0));
        assert!(!skip.contains(&1));
    }

    #[test]
    fn immediately_consumed_rejects_two_uses() {
        let mut func = IrFunction::new("two".to_string(), IrType::I32, vec![], false);
        func.blocks.push(block(
            0,
            vec![
                scalar_def(0, 9),
                Instruction::Cast {
                    dest: Value(1),
                    src: Operand::Value(Value(0)),
                    from_ty: IrType::I32,
                    to_ty: IrType::I64,
                },
                Instruction::UnaryOp {
                    dest: Value(2),
                    op: crate::ir::reexports::IrUnaryOp::Neg,
                    src: Operand::Value(Value(0)),
                    ty: IrType::I32,
                },
            ],
            Terminator::Return(Some(Operand::Value(Value(1)))),
        ));
        let skip = compute_immediately_consumed(&func, false);
        assert!(!skip.contains(&0));
    }

    #[test]
    fn immediately_consumed_rejects_float_producer() {
        let mut func = IrFunction::new("fp".to_string(), IrType::F64, vec![], false);
        func.blocks.push(block(
            0,
            vec![
                Instruction::BinOp {
                    dest: Value(0),
                    op: IrBinOp::Add,
                    lhs: Operand::Const(IrConst::I64(0)),
                    rhs: Operand::Const(IrConst::I64(0)),
                    ty: IrType::F64,
                },
                Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Value(Value(0)),
                },
            ],
            Terminator::Return(Some(Operand::Value(Value(1)))),
        ));
        let skip = compute_immediately_consumed(&func, false);
        assert!(!skip.contains(&0));
    }
}

/// Codegen reads args outside `sse_load_arg` / `avx_load_arg_to` and would
/// observe a deferred (never-written) slot.
pub(crate) fn is_raw_reader_intrinsic(op: &crate::ir::intrinsics::IntrinsicOp) -> bool {
    use crate::ir::intrinsics::IntrinsicOp as O;
    matches!(
        op,
        O::Pblendvb128
            | O::Pblendw128
            | O::Loadldi128
            | O::Storeldi128
            | O::FmaF64x2
            | O::FmaF64x4
            | O::FmaF64x4Hoisted
            | O::FmaF64x4SIB
            | O::BroadcastLoadF64
            | O::LoadF64x2
            | O::LoadF64x4
            | O::LoadI32x4
            | O::LoadI32x8
            | O::HorizontalAddF64x2
            | O::HorizontalAddF64x4
            | O::HorizontalAddI32x4
            | O::HorizontalAddI32x8
            | O::VecLoadF64x2
            | O::VecLoadF64x4
            | O::VecLoadI32x4
            | O::VecLoadI32x8
            | O::VecLoadF32x4
            | O::VecLoadF32x8
            | O::VecHorizontalAddF64x2
            | O::VecHorizontalAddF64x4
            | O::VecHorizontalAddI32x4
            | O::VecHorizontalAddI32x8
            | O::VecHorizontalAddF32x4
            | O::VecHorizontalAddF32x8
            | O::VecZeroF64x2
            | O::VecZeroF64x4
            | O::VecZeroI32x4
            | O::VecZeroI32x8
            | O::VecZeroF32x4
            | O::VecZeroF32x8
    )
}

/// Two-operand emitters that load the last-stored operand first into
/// `%xmm1`/`%ymm1` when `args[1]` is still in `%xmm0`/`%ymm0`.
fn is_two_operand_binary(op: &crate::ir::intrinsics::IntrinsicOp) -> bool {
    use crate::ir::intrinsics::IntrinsicOp as O;
    matches!(
        op,
        O::Pcmpeqb128
            | O::Pcmpeqd128
            | O::Psubusb128
            | O::Psubsb128
            | O::Por128
            | O::Pand128
            | O::Pxor128
            | O::AddPs128
            | O::SubPs128
            | O::MulPs128
            | O::AddPd128
            | O::SubPd128
            | O::MulPd128
            | O::Paddw128
            | O::Psubw128
            | O::Pmulhw128
            | O::Pmullw128
            | O::Pmuludq128
            | O::Pmuldq128
            | O::Pmulld128
            | O::Pmaddwd128
            | O::Pmaddubsw128
            | O::Pcmpgtw128
            | O::Pcmpgtb128
            | O::Paddd128
            | O::Psubd128
            | O::Paddb128
            | O::Psubb128
            | O::Psubusw128
            | O::Psadbw128
            | O::Pshufb128
            | O::Pmaxub128
            | O::Pminub128
            | O::Pmovzxbw128
            | O::Pmovzxwd128
            | O::Packssdw128
            | O::Packsswb128
            | O::Packuswb128
            | O::Punpcklbw128
            | O::Punpckhbw128
            | O::Punpcklwd128
            | O::Punpckhwd128
            | O::Aesenc128
            | O::Aesenclast128
            | O::Aesdec128
            | O::Aesdeclast128
            | O::AddF64x2
            | O::MulF64x2
            | O::AddI32x4
            | O::Paddb256
            | O::Paddw256
            | O::Paddd256
            | O::Psubb256
            | O::Psubw256
            | O::Psubusw256
            | O::Psadbw256
            | O::Pmaddubsw256
            | O::Pmaddwd256
            | O::Pcmpeqb256
            | O::Pcmpgtb256
            | O::Pshufb256
            | O::Pmaxub256
            | O::Pminub256
            | O::Pxor256
            | O::Por256
            | O::Pand256
            | O::AddF64x4
            | O::MulF64x4
            | O::AddI32x8
            | O::VecAddF64x4
            | O::VecMulF64x4
            | O::VecFmaF64x4
            | O::VecAddI32x8
            | O::VecMulI32x8
            | O::VecBroadcastI32x8
            | O::VecBroadcastF32x8
            | O::VecMulF32x8
            | O::VecAddF32x8
            | O::VecFmaF32x8
            | O::VecAddF64x2
            | O::VecMulF64x2
            | O::VecAddI32x4
            | O::VecAddF32x4
            | O::VecMulF32x4
    )
}

fn is_vec_ssa_producer(op: &crate::ir::intrinsics::IntrinsicOp) -> bool {
    use crate::ir::intrinsics::IntrinsicOp as O;
    matches!(
        op,
        O::VecLoadF64x2
            | O::VecLoadF64x4
            | O::VecLoadI32x4
            | O::VecLoadI32x8
            | O::VecLoadF32x4
            | O::VecLoadF32x8
            | O::VecAddF64x2
            | O::VecAddF64x4
            | O::VecFmaF64x4
            | O::VecMaddF64x4
            | O::VecAddI32x4
            | O::VecAddI32x8
            | O::VecMulI32x8
            | O::VecBroadcastI32x8
            | O::VecBroadcastF32x8
            | O::VecMulF32x8
            | O::VecAddF32x8
            | O::VecFmaF32x8
            | O::VecMaddF32x8
            | O::VecAddF32x4
            | O::VecMulF64x2
            | O::VecMulF64x4
            | O::VecMulF32x4
            | O::VecZeroF64x2
            | O::VecZeroF64x4
            | O::VecZeroI32x4
            | O::VecZeroI32x8
            | O::VecZeroF32x4
            | O::VecZeroF32x8
    )
}

fn is_user_store_intrinsic(op: &crate::ir::intrinsics::IntrinsicOp) -> bool {
    use crate::ir::intrinsics::IntrinsicOp as O;
    matches!(
        op,
        O::Storedqu | O::Storeu256 | O::Store256 | O::Storeldi128 | O::Movntdq | O::Movntpd
    )
}

/// Vector-result stores that can be skipped: the dest is consumed once from
/// the last-store peephole by a cache-aware intrinsic in the same block.
///
/// All def sites of the slot must qualify. Codegen skips the store at every
/// writer, so one non-adjacent site would make its consumer read a
/// never-written slot.
pub(super) fn compute_vector_defer_values(func: &IrFunction) -> FxHashSet<u32> {
    let mut result = FxHashSet::default();

    let mut allocas: FxHashSet<u32> = FxHashSet::default();
    let mut volatile_allocas: FxHashSet<u32> = FxHashSet::default();
    let mut copy_alias_roots: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Alloca {
                    dest,
                    volatile,
                    semantic_volatile,
                    ..
                } => {
                    allocas.insert(dest.0);
                    if *volatile || *semantic_volatile {
                        volatile_allocas.insert(dest.0);
                    }
                }
                Instruction::Copy {
                    src: Operand::Value(v),
                    ..
                } => {
                    copy_alias_roots.insert(v.0);
                }
                _ => {}
            }
        }
    }

    let mut uses: FxHashMap<u32, Vec<(usize, usize)>> = FxHashMap::default();
    let mut has_value_ref_use: FxHashSet<u32> = FxHashSet::default();
    let mut non_intrinsic_arg_use: FxHashSet<u32> = FxHashSet::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            match inst {
                Instruction::Intrinsic { args, .. } => {
                    for arg in args {
                        if let Operand::Value(v) = arg {
                            uses.entry(v.0).or_default().push((bi, ii));
                        }
                    }
                }
                other => {
                    for_each_operand_in_instruction(other, |op| {
                        if let Operand::Value(v) = op {
                            uses.entry(v.0).or_default().push((bi, ii));
                            non_intrinsic_arg_use.insert(v.0);
                        }
                    });
                    for_each_value_use_in_instruction(other, |v| {
                        has_value_ref_use.insert(v.0);
                    });
                }
            }
        }
        for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                uses.entry(v.0).or_default().push((bi, usize::MAX));
                non_intrinsic_arg_use.insert(v.0);
            }
        });
    }

    let is_vec_invalidator = |inst: &Instruction| -> bool {
        match inst {
            Instruction::Call { .. }
            | Instruction::CallIndirect { .. }
            | Instruction::InlineAsm { .. }
            | Instruction::Memcpy { .. }
            | Instruction::DynAlloca { .. }
            | Instruction::Store { .. }
            | Instruction::AtomicLoad { .. }
            | Instruction::AtomicStore { .. }
            | Instruction::AtomicRmw { .. }
            | Instruction::AtomicCmpxchg { .. }
            | Instruction::AtomicInc { .. } => true,
            Instruction::Intrinsic { op, .. } => is_raw_reader_intrinsic(op),
            _ => false,
        }
    };

    let mut defs: FxHashMap<u32, Vec<(usize, usize)>> = FxHashMap::default();
    let mut def_blocks: FxHashMap<u32, FxHashSet<usize>> = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            match inst {
                Instruction::Intrinsic {
                    dest_ptr: Some(d),
                    op,
                    ..
                } if !is_user_store_intrinsic(op) && allocas.contains(&d.0) => {
                    defs.entry(d.0).or_default().push((bi, ii));
                    def_blocks.entry(d.0).or_default().insert(bi);
                }
                Instruction::Intrinsic {
                    dest: Some(d), op, ..
                } if is_vec_ssa_producer(op) => {
                    defs.entry(d.0).or_default().push((bi, ii));
                    def_blocks.entry(d.0).or_default().insert(bi);
                }
                _ => {}
            }
        }
    }

    let debug_vdefer = env_flag("CCC_DEBUG_VDEFER");

    for (&slot, sites) in &defs {
        if volatile_allocas.contains(&slot)
            || copy_alias_roots.contains(&slot)
            || has_value_ref_use.contains(&slot)
            || non_intrinsic_arg_use.contains(&slot)
        {
            continue;
        }

        if let Some(use_sites) = uses.get(&slot) {
            let written = def_blocks.get(&slot);
            let orphan = use_sites
                .iter()
                .any(|&(ubi, _)| !written.map(|b| b.contains(&ubi)).unwrap_or(false));
            if orphan {
                continue;
            }
        }

        let mut sites_sorted = sites.clone();
        sites_sorted.sort_unstable();

        let mut all_sites_ok = true;
        for &(bi, i) in &sites_sorted {
            let insts = &func.blocks[bi].instructions;

            // Use at or before the def ⇒ backedge / RMW; slot must stay live.
            let earlier_use = uses
                .get(&slot)
                .is_some_and(|u| u.iter().any(|&(ubi, uii)| ubi == bi && uii <= i));
            if earlier_use {
                all_sites_ok = false;
                break;
            }

            let next_def_in_block = sites_sorted
                .iter()
                .find(|&&(db, di)| db == bi && di > i)
                .map(|&(_, di)| di);
            let window_uses: Vec<usize> = uses
                .get(&slot)
                .map(|u| {
                    u.iter()
                        .filter(|&&(ubi, uii)| {
                            ubi == bi
                                && uii > i
                                && next_def_in_block.map(|nd| uii < nd).unwrap_or(true)
                        })
                        .map(|&(_, uii)| uii)
                        .collect()
                })
                .unwrap_or_default();

            if window_uses.len() != 1 {
                all_sites_ok = false;
                break;
            }
            let u = window_uses[0];
            if u >= insts.len() || u <= i {
                all_sites_ok = false;
                break;
            }

            let mut ok = true;
            for ik in &insts[(i + 1)..u] {
                if is_vec_invalidator(ik) || matches!(ik, Instruction::Intrinsic { .. }) {
                    ok = false;
                    break;
                }
            }
            if !ok {
                all_sites_ok = false;
                break;
            }

            let Some(Instruction::Intrinsic {
                op: cop, args: cargs, ..
            }) = insts.get(u)
            else {
                all_sites_ok = false;
                break;
            };
            let pos = cargs
                .iter()
                .position(|a| matches!(a, Operand::Value(v) if v.0 == slot));
            // pos 0: first vector load. pos 1: two-operand emitter load-order swap.
            // pos ≥ 2 (FMA/etc.) is not covered by that swap.
            let cache_aware = match pos {
                Some(0) => !is_raw_reader_intrinsic(cop),
                Some(1) => is_two_operand_binary(cop),
                _ => false,
            };
            if !cache_aware {
                all_sites_ok = false;
                break;
            }
            if debug_vdefer {
                eprintln!(
                    "[VDEFER] slot={} def=({},{})(uses_in_window={:?}) consumer=({},{})({:?}) site-ok",
                    slot, bi, i, window_uses, bi, u, cop
                );
            }
        }
        if all_sites_ok {
            result.insert(slot);
        }
    }

    result
}
