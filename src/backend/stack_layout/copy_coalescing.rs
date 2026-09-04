//! Copy coalescing, immediately-consumed GPR analysis, vector-store deferral.
//!
//! Stack coalescing is frequency-weighted conservative move coalescing on a
//! CFG-precise, copy-aware interference graph (George/Appel; Chaitin/Briggs
//! move rule). For `d = copy s`, `d` interferes with every value live after
//! the copy except `s`. Unlimited stack homes ⇒ the George test collapses to
//! “no edge”: greedy merge by execution weight is the practical maximum-
//! weight coalescing heuristic (exact MWCP is NP-hard).
//!
//! Weights use the same `10^depth` loop scale as RA so inner-loop copies
//! win ties against cold ones. CFG matches `liveness.rs` (terminators +
//! asm-goto). Safe degradation is extra slots, never a wrong merge.

use std::sync::OnceLock;

use crate::backend::liveness::{
    for_each_operand_in_instruction, for_each_operand_in_terminator,
    for_each_value_use_in_instruction, LivenessResult,
};
use crate::backend::regalloc::{detect_phi_coalesce_groups, PhysReg};
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::reexports::{Instruction, IrBinOp, IrConst, IrFunction, Operand, Terminator};

fn cfg_copy_coalesce_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("CCC_NO_CFG_COPY_COALESCE").is_none())
}

fn env_flag(name: &'static str) -> bool {
    std::env::var_os(name).is_some()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CoalesceClass {
    Scalar,
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

fn ty_lives_in_gpr_cache(ty: IrType) -> bool {
    scalar_type(ty) && !ty.is_float() && !ty.is_128bit() && !ty.is_long_double()
}

/// `10^min(depth,6)` so an inner-loop copy outranks any nest of cold ones.
/// Multi-def (phi / loop-carried) gets another ×10: those copies execute
/// once per iteration and dominate stack traffic on spilled IVs.
fn copy_weight(loop_depth: u32, touches_multi_def: bool) -> u32 {
    let mut w = 1u32;
    for _ in 0..loop_depth.min(6) {
        w = w.saturating_mul(10);
    }
    if touches_multi_def {
        w = w.saturating_mul(10);
    }
    w
}

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
                | Instruction::Cast {
                    dest, to_ty: ty, ..
                }
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
                Instruction::Phi { dest, ty, .. } if scalar_type(*ty) => {
                    classes.insert(dest.0, CoalesceClass::Scalar);
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

fn instruction_def_id(inst: &Instruction) -> Option<u32> {
    if let Some(dest) = inst.dest() {
        return Some(dest.0);
    }
    match inst {
        Instruction::Copy { dest, .. } | Instruction::Phi { dest, .. } => Some(dest.0),
        _ => None,
    }
}

fn operand_is_value(op: &Operand, id: u32) -> bool {
    matches!(op, Operand::Value(v) if v.0 == id)
}

fn operand_is_const(op: &Operand) -> bool {
    matches!(op, Operand::Const(_))
}

/// CFG identical to `liveness.rs`: terminator edges + InlineAsm goto.
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

fn cfg_postorder(succs: &[Vec<usize>]) -> Vec<usize> {
    let n = succs.len();
    let mut state = vec![0u8; n];
    let mut post = Vec::with_capacity(n);
    let mut next = vec![0usize; n];
    for start in 0..n {
        if state[start] != 0 {
            continue;
        }
        state[start] = 1;
        let mut stack = vec![start];
        while let Some(&b) = stack.last() {
            let i = next[b];
            if i < succs[b].len() {
                next[b] = i + 1;
                let s = succs[b][i];
                if s < n && state[s] == 0 {
                    state[s] = 1;
                    stack.push(s);
                }
            } else {
                state[b] = 2;
                post.push(b);
                stack.pop();
            }
        }
    }
    post
}

/// Natural-loop depth, one increment per header (two latches ≠ nest).
fn cheap_loop_depth(succs: &[Vec<usize>], preds: &[Vec<usize>]) -> Vec<u32> {
    let n = succs.len();
    let mut depth = vec![0u32; n];
    if n == 0 {
        return depth;
    }

    let mut state = vec![0u8; n];
    let mut next = vec![0usize; n];
    let mut back_headers: FxHashMap<usize, Vec<usize>> = FxHashMap::default();
    for start in 0..n {
        if state[start] != 0 {
            continue;
        }
        state[start] = 1;
        let mut stack = vec![start];
        while let Some(&b) = stack.last() {
            let i = next[b];
            if i < succs[b].len() {
                next[b] = i + 1;
                let s = succs[b][i];
                if s >= n {
                    continue;
                }
                match state[s] {
                    0 => {
                        state[s] = 1;
                        stack.push(s);
                    }
                    1 => back_headers.entry(s).or_default().push(b),
                    _ => {}
                }
            } else {
                state[b] = 2;
                stack.pop();
            }
        }
    }

    for (&header, tails) in &back_headers {
        let mut seen = vec![false; n];
        seen[header] = true;
        depth[header] = depth[header].saturating_add(1);
        let mut work: Vec<usize> = Vec::new();
        for &tail in tails {
            if tail == header || seen[tail] {
                continue;
            }
            seen[tail] = true;
            depth[tail] = depth[tail].saturating_add(1);
            work.push(tail);
        }
        while let Some(b) = work.pop() {
            for &p in &preds[b] {
                if p < n && !seen[p] {
                    seen[p] = true;
                    depth[p] = depth[p].saturating_add(1);
                    work.push(p);
                }
            }
        }
    }
    depth
}

fn loop_depth_for(func: &IrFunction, liveness: Option<&LivenessResult>) -> Vec<u32> {
    if let Some(lr) = liveness {
        if lr.block_loop_depth.len() == func.blocks.len() {
            return lr.block_loop_depth.clone();
        }
    }
    let (succs, preds) = build_block_cfg(func);
    cheap_loop_depth(&succs, &preds)
}

// ── Dense bitsets ─────────────────────────────────────────────────────────

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
        i < self.nbits && self.words[i / 64] & (1u64 << (i % 64)) != 0
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

/// Worklist liveness seeded in reverse postorder (exits first), then
/// copy-aware interference. Lattice-height cap is an impl-bug guard only.
fn cfg_copy_interference(func: &IrFunction, dense: &FxHashMap<u32, usize>) -> Option<Vec<BitSet>> {
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

    let mut label_to_idx: FxHashMap<u32, usize> = FxHashMap::default();
    for (idx, block) in func.blocks.iter().enumerate() {
        label_to_idx.insert(block.label.0, idx);
    }
    // Unlowered phi: incoming is a use at the predecessor terminator, dest
    // is a def at the header (same contract as liveness.rs).
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
    let post = cfg_postorder(&succs);
    let mut work: Vec<usize> = post.iter().rev().copied().collect();
    let mut on_work = vec![false; nblocks];
    for &b in &work {
        on_work[b] = true;
    }
    for b in 0..nblocks {
        if !on_work[b] {
            on_work[b] = true;
            work.push(b);
        }
    }

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

struct Affinity {
    dest: u32,
    src: u32,
    weight: u32,
}

fn try_push_affinity(
    out: &mut Vec<Affinity>,
    dest: u32,
    src: u32,
    weight: u32,
    classes: &FxHashMap<u32, CoalesceClass>,
    unsound: &FxHashSet<u32>,
    reg_assigned: &FxHashMap<u32, PhysReg>,
) {
    if dest == src
        || reg_assigned.contains_key(&dest)
        || reg_assigned.contains_key(&src)
        || unsound.contains(&dest)
        || unsound.contains(&src)
    {
        return;
    }
    let (Some(&dc), Some(&sc)) = (classes.get(&dest), classes.get(&src)) else {
        return;
    };
    if dc != sc {
        return;
    }
    out.push(Affinity { dest, src, weight });
}

fn build_cfg_copy_alias_map(
    func: &IrFunction,
    multi_def_values: &FxHashSet<u32>,
    reg_assigned: &FxHashMap<u32, PhysReg>,
    liveness: Option<&LivenessResult>,
) -> (FxHashMap<u32, u32>, FxHashSet<u32>) {
    let classes = collect_scalar_values(func);
    let unsound = collect_unsound_coalesce_ids(func);
    let depths = loop_depth_for(func, liveness);

    let mut affinities: Vec<Affinity> = Vec::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        let depth = depths.get(bi).copied().unwrap_or(0);
        for inst in &block.instructions {
            match inst {
                Instruction::Copy {
                    dest,
                    src: Operand::Value(source),
                } => {
                    let multi =
                        multi_def_values.contains(&dest.0) || multi_def_values.contains(&source.0);
                    try_push_affinity(
                        &mut affinities,
                        dest.0,
                        source.0,
                        copy_weight(depth, multi),
                        &classes,
                        &unsound,
                        reg_assigned,
                    );
                }
                // Virtual copies: unlowered phi incoming ≡ copy at the pred edge.
                Instruction::Phi { dest, incoming, .. } => {
                    for (op, pred_label) in incoming {
                        let Operand::Value(source) = op else {
                            continue;
                        };
                        let pred_depth = func
                            .blocks
                            .iter()
                            .position(|b| b.label.0 == pred_label.0)
                            .and_then(|i| depths.get(i).copied())
                            .unwrap_or(depth);
                        try_push_affinity(
                            &mut affinities,
                            dest.0,
                            source.0,
                            copy_weight(pred_depth, true),
                            &classes,
                            &unsound,
                            reg_assigned,
                        );
                    }
                }
                _ => {}
            }
        }
    }

    // Loop-phi pairs the linear-scan interval check would reject after the
    // fact: feed them through the *same* graph. If they interfere here they
    // are not merged (CFG proof wins). High weight so a legal pair is not
    // blocked by an earlier cold merge.
    if let Some(lr) = liveness {
        for cand in detect_phi_coalesce_groups(func, lr) {
            let depth = 4;
            try_push_affinity(
                &mut affinities,
                cand.phi_dest,
                cand.backedge_src,
                copy_weight(depth, true),
                &classes,
                &unsound,
                reg_assigned,
            );
        }
    }

    affinities.sort_unstable_by(|a, b| {
        b.weight
            .cmp(&a.weight)
            .then(a.dest.cmp(&b.dest))
            .then(a.src.cmp(&b.src))
    });
    affinities.dedup_by(|a, b| a.dest == b.dest && a.src == b.src);
    if affinities.is_empty() {
        return (FxHashMap::default(), FxHashSet::default());
    }

    let mut tracked: Vec<u32> = affinities.iter().flat_map(|a| [a.dest, a.src]).collect();
    tracked.sort_unstable();
    tracked.dedup();

    let mut dense: FxHashMap<u32, usize> = FxHashMap::default();
    dense.reserve(tracked.len());
    for (i, &v) in tracked.iter().enumerate() {
        dense.insert(v, i);
    }
    let n = tracked.len();

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

    for aff in &affinities {
        let Some(&d) = dense.get(&aff.dest) else {
            continue;
        };
        let Some(&s) = dense.get(&aff.src) else {
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
        let dest_root_val = tracked[dest_root];
        let source_root_val = tracked[source_root];
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
        groups.entry(tracked[root]).or_default().push(tracked[i]);
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

/// dest_id → root_id for Copy instructions that can share a stack slot.
///
/// Returns `(copy_alias, phi_web_aliases, loop_phi_aliases)`:
/// - `phi_web_aliases` need force-overwrite in `resolve_copy_aliases`
/// - `loop_phi_aliases` skip the generic def/last-use check (legacy path)
pub(super) fn build_copy_alias_map(
    func: &IrFunction,
    def_block: &FxHashMap<u32, usize>,
    multi_def_values: &FxHashSet<u32>,
    reg_assigned: &FxHashMap<u32, PhysReg>,
    use_blocks_map: &FxHashMap<u32, Vec<usize>>,
    cached_liveness: &Option<LivenessResult>,
) -> (FxHashMap<u32, u32>, FxHashSet<u32>, FxHashSet<u32>) {
    if cfg_copy_coalesce_enabled() {
        let (aliases, force) = build_cfg_copy_alias_map(
            func,
            multi_def_values,
            reg_assigned,
            cached_liveness.as_ref(),
        );
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
                    other_copy_points.iter().any(|&pp| start <= pp && pp <= end)
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
                            dest,
                            to_ty,
                            from_ty,
                            ..
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
        copy_alias
            .retain(|dest_id, root_id| !unsound.contains(dest_id) && !unsound.contains(root_id));
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

/// GPR values that can skip a stack slot: produced into the accumulator and
/// consumed as the first (or uniquely-loaded) operand of the next instruction.
///
/// Copy is acc-preserving (`store_rax_to` leaves the cache armed). A copy
/// source is excluded from the skip set only when some dest still needs the
/// home — if every dest is itself skipped, the whole rename chain stays in
/// the accumulator (0 stores).
pub(crate) fn compute_immediately_consumed(
    func: &IrFunction,
    lhs_first_binop: bool,
) -> FxHashSet<u32> {
    let mut operand_use_count: FxHashMap<u32, u32> = FxHashMap::default();
    let mut has_value_ref_use: FxHashSet<u32> = FxHashSet::default();
    let mut copy_dests_of: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    let mut non_gpr: FxHashSet<u32> = FxHashSet::default();

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
            match inst {
                Instruction::BinOp { dest, ty, .. }
                | Instruction::UnaryOp { dest, ty, .. }
                | Instruction::Load { dest, ty, .. }
                | Instruction::Select { dest, ty, .. }
                    if !ty_lives_in_gpr_cache(*ty) =>
                {
                    non_gpr.insert(dest.0);
                }
                Instruction::Cast {
                    dest,
                    to_ty,
                    from_ty,
                    ..
                } if !ty_lives_in_gpr_cache(*to_ty)
                    || from_ty.is_128bit()
                    || from_ty.is_long_double() =>
                {
                    non_gpr.insert(dest.0);
                }
                Instruction::Copy {
                    dest,
                    src: Operand::Value(v),
                } => {
                    copy_dests_of.entry(v.0).or_default().push(dest.0);
                }
                _ => {}
            }
        }
        for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                *operand_use_count.entry(v.0).or_insert(0) += 1;
            }
        });
    }

    let mut work: Vec<u32> = non_gpr.iter().copied().collect();
    while let Some(id) = work.pop() {
        if let Some(dests) = copy_dests_of.get(&id) {
            for &d in dests {
                if non_gpr.insert(d) {
                    work.push(d);
                }
            }
        }
    }

    let mut result = FxHashSet::default();
    for block in &func.blocks {
        let insts = &block.instructions;
        for (i, inst) in insts.iter().enumerate() {
            let Some(dest) = inst.dest().or_else(|| match inst {
                Instruction::Copy { dest, .. } => Some(*dest),
                _ => None,
            }) else {
                continue;
            };
            if non_gpr.contains(&dest.0) || has_value_ref_use.contains(&dest.0) {
                continue;
            }
            if !is_gpr_acc_preserving_producer(inst) {
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

    // Copy-source keeps a slot if any dest still needs a home. Snapshot the
    // skip set first — retain+contains is a double borrow.
    let drop_srcs: Vec<u32> = copy_dests_of
        .iter()
        .filter_map(|(&src, dests)| {
            if result.contains(&src) && dests.iter().any(|d| !result.contains(d)) {
                Some(src)
            } else {
                None
            }
        })
        .collect();
    for id in drop_srcs {
        result.remove(&id);
    }

    result
}

fn is_gpr_acc_preserving_producer(inst: &Instruction) -> bool {
    match inst {
        Instruction::Load { ty, .. }
        | Instruction::BinOp { ty, .. }
        | Instruction::UnaryOp { ty, .. }
        | Instruction::Select { ty, .. } => ty_lives_in_gpr_cache(*ty),
        Instruction::Cast { from_ty, to_ty, .. } => {
            ty_lives_in_gpr_cache(*to_ty) && !from_ty.is_128bit() && !from_ty.is_long_double()
        }
        // Predicate lands in a GPR via setcc; compared type may be float.
        Instruction::Cmp { .. } => true,
        Instruction::GetElementPtr { .. }
        | Instruction::GlobalAddr { .. }
        | Instruction::LabelAddr { .. } => true,
        // store_rax_to leaves the cache holding dest.
        Instruction::Copy {
            src: Operand::Value(_),
            ..
        }
        | Instruction::Copy {
            src: Operand::Const(_),
            ..
        } => true,
        _ => false,
    }
}

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
        Instruction::BinOp { lhs, rhs, ty, .. } => {
            if ty.is_float() || ty.is_long_double() {
                return false;
            }
            // A value is consumed from the accumulator only when it is the LHS
            // AND the consumer reads the LHS first: either the RHS is a
            // constant (an immediate / materialised-const form, so the LHS is
            // the operand staged in the accumulator), or the backend is
            // lhs_first_binop (x86-64, RISC-V load the LHS before the RHS).
            // A value that is the RHS of `Sub(Const, v)` is NOT safe: x86-64
            // lowers it to `movq $0,%rax; subl <v>,%rax` where <v> is loaded
            // into %rcx or a memory operand from its HOME. Skipping that home
            // made the load read zero (alu_peepholes sdivm3: `0 - (v/3)`
            // returned 0 instead of the negated quotient).
            operand_is_value(lhs, value_id) && (operand_is_const(rhs) || lhs_first_binop)
        }
        Instruction::Cmp { lhs, rhs, ty, .. } => {
            if ty.is_float() || ty.is_long_double() {
                return false;
            }
            operand_is_value(lhs, value_id) && (operand_is_const(rhs) || lhs_first_binop)
        }
        _ => false,
    }
}

fn is_sole_operand_of_terminator(term: &Terminator, value_id: u32) -> bool {
    // A value whose single use is `Return` must NOT be skipped on backends
    // whose return path materialises through the normal location machinery
    // (ABI return register / slot) — a skipped home breaks the contract
    // there.  i686 is the exception: its scalar-int return path IS the
    // accumulator (emit_return_default → operand_to_eax → int-to-reg no-op),
    // and the acc-preserving producer (the gate in compute_immediately_
    // consumed) already left the value in %eax with a live cache entry —
    // the Return consumes it with zero instructions and no home.  strlen's
    // `p - s` tail, every leaf computation returning a folded expression.
    // Wide/float returns are safe too: their types fail the producer gate
    // (ty_lives_in_gpr_cache) long before reaching this point.
    if matches!(term, Terminator::Return(_)) && !crate::common::types::target_is_32bit() {
        return false;
    }
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
            build_cfg_copy_alias_map(&func, &FxHashSet::default(), &FxHashMap::default(), None);
        assert_eq!(aliases.get(&1), Some(&0));
        assert!(force.contains(&1));
    }

    #[test]
    fn cfg_copy_rejects_a_phi_edge_source_live_on_another_path() {
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
        let (aliases, _) = build_cfg_copy_alias_map(&func, &multi_def, &FxHashMap::default(), None);
        assert_ne!(aliases.get(&0), Some(&2));
        assert_ne!(aliases.get(&2), Some(&0));
        assert_eq!(aliases.get(&1), Some(&2));
    }

    #[test]
    fn cfg_copy_coalesces_loop_carried_phi_sources() {
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
            block(
                3,
                vec![],
                Terminator::Return(Some(Operand::Value(Value(2)))),
            ),
        ];
        let mut multi_def = FxHashSet::default();
        multi_def.insert(2);
        let (aliases, _) = build_cfg_copy_alias_map(&func, &multi_def, &FxHashMap::default(), None);
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
            build_cfg_copy_alias_map(&func, &FxHashSet::default(), &FxHashMap::default(), None);
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
            build_cfg_copy_alias_map(&func, &FxHashSet::default(), &FxHashMap::default(), None);
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

    #[test]
    fn immediately_consumed_binop_const_rhs_on_x86() {
        let mut func = IrFunction::new("imm".to_string(), IrType::I32, vec![], false);
        func.blocks.push(block(
            0,
            vec![
                scalar_def(0, 1),
                Instruction::BinOp {
                    dest: Value(1),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                },
            ],
            Terminator::Return(Some(Operand::Value(Value(1)))),
        ));
        let skip = compute_immediately_consumed(&func, false);
        assert!(skip.contains(&0));
    }

    #[test]
    fn immediately_consumed_copy_rename_chain() {
        let mut func = IrFunction::new("rename".to_string(), IrType::I32, vec![], false);
        func.blocks.push(block(
            0,
            vec![
                scalar_def(0, 4),
                Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Value(Value(0)),
                },
                Instruction::Cast {
                    dest: Value(2),
                    src: Operand::Value(Value(1)),
                    from_ty: IrType::I32,
                    to_ty: IrType::I64,
                },
            ],
            Terminator::Return(Some(Operand::Value(Value(2)))),
        ));
        let skip = compute_immediately_consumed(&func, false);
        assert!(skip.contains(&0), "src of dead copy should stay in acc");
        assert!(skip.contains(&1), "copy dest consumed by cast");
    }
}

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
            | O::FmaF64x4HoistedSIB
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
            // v12 Fix F: VecMaxI32x8 is a two-operand binary (vpmaxsd) —
            // the consumer reads the deferred vec_load from %ymm0 and folds
            // it into the 3-operand form. Without this, the vec_load is
            // stored to a slot every iteration (a dead 256-bit store that
            // made the vectorized find_max SLOWER than scalar).
            | O::VecMaxI32x8
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

/// i686 x87 top-of-stack deferral (`state.x87_defer_values`): an F64 binop
/// result whose ONLY use is as an operand of the immediately-following F64
/// binop, in the same block. The emitter may then store the result with a
/// non-popping `fstl` and let the consumer take it from st(0) (`fld %st(0)`
/// dup or an in-place non-popping arith form) instead of reloading the slot.
/// Sound because adjacency + single-use means no other instruction can run
/// between the store and the consumption: nothing can clobber the slot or
/// push/pop x87 within the window (any such instruction would be a second
/// use or would break adjacency). The emitter still flushes defensively at
/// every boundary, so a stale entry only ever costs a redundant store.
pub(super) fn compute_x87_defer_values(func: &IrFunction) -> FxHashSet<u32> {
    let mut result = FxHashSet::default();

    // Total use count per value, across all instructions and terminators.
    let mut uses: FxHashMap<u32, u32> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    *uses.entry(v.0).or_insert(0) += 1;
                }
            });
        }
        crate::backend::liveness::for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                *uses.entry(v.0).or_insert(0) += 1;
            }
        });
    }

    // Adjacency window: BinOp(F64) immediately followed by BinOp(F64) that
    // consumes the first result as one operand. classify_float_binop maps
    // Add/Sub/Mul/SDiv/UDiv to float ops; any other op on F64 would have
    // panicked in emit_binop, so the whitelist mirrors it exactly.
    for block in &func.blocks {
        let insts = &block.instructions;
        for w in insts.windows(2) {
            let (a, b) = (&w[0], &w[1]);
            let dest = match a {
                Instruction::BinOp {
                    dest,
                    op,
                    ty: IrType::F64,
                    ..
                } => {
                    if !matches!(
                        op,
                        IrBinOp::Add | IrBinOp::Sub | IrBinOp::Mul | IrBinOp::SDiv | IrBinOp::UDiv
                    ) {
                        continue;
                    }
                    dest.0
                }
                _ => continue,
            };
            let consumed = match b {
                Instruction::BinOp {
                    op: b_op,
                    lhs,
                    rhs,
                    ty: IrType::F64,
                    ..
                } => {
                    if !matches!(
                        b_op,
                        IrBinOp::Add | IrBinOp::Sub | IrBinOp::Mul | IrBinOp::SDiv | IrBinOp::UDiv
                    ) {
                        continue;
                    }
                    match (lhs, rhs) {
                        (Operand::Value(l), Operand::Value(r)) => l.0 == dest || r.0 == dest,
                        (Operand::Value(l), _) => l.0 == dest,
                        (_, Operand::Value(r)) => r.0 == dest,
                        (_, _) => false,
                    }
                }
                _ => continue,
            };
            if consumed && uses.get(&dest).copied().unwrap_or(0) == 1 {
                result.insert(dest);
            }
        }
    }

    result
}

/// Pure vector loads: they write no memory and define no GPR (their address
/// is either an RA-homed GPR pair or the `%rax`/`%rcx` scratch pair).
pub(crate) fn is_pure_vec_load(op: &crate::ir::intrinsics::IntrinsicOp) -> bool {
    use crate::ir::intrinsics::IntrinsicOp as O;
    matches!(
        op,
        O::VecLoadF64x4
            | O::VecLoadF32x8
            | O::VecLoadI32x8
            | O::VecLoadF64x2
            | O::VecLoadF32x4
            | O::VecLoadI32x4
            | O::VecLoadI64x2
    )
}

/// 256-bit loads eligible for source-operand folding (VLFOLD).
pub(crate) fn is_memfold_vec_load(op: &crate::ir::intrinsics::IntrinsicOp) -> bool {
    use crate::ir::intrinsics::IntrinsicOp as O;
    matches!(op, O::VecLoadF64x4 | O::VecLoadF32x8 | O::VecLoadI32x8)
}

/// Map FMA intrinsics `VecMadd*(input, scale, bias)` (`emit_avx_map_fma`):
/// the bias folds through the 213 form (`vfmadd213ps mem, %scale, %ymm0` =
/// scale*input + mem) and the input through the 231 form
/// (`vfmadd231ps mem, %scale, %ymm0` = scale*mem + bias).
pub(crate) fn memfold_consumer_madd_256(op: &crate::ir::intrinsics::IntrinsicOp) -> bool {
    use crate::ir::intrinsics::IntrinsicOp as O;
    matches!(op, O::VecMaddF64x4 | O::VecMaddF32x8)
}

/// Two-operand 256-bit VEX arithmetic intrinsics that go through
/// `emit_avx_binary_256` and therefore can take a memory operand.
/// Returns `Some(commutative)`.
pub(crate) fn memfold_consumer_256(op: &crate::ir::intrinsics::IntrinsicOp) -> Option<bool> {
    use crate::ir::intrinsics::IntrinsicOp as O;
    match op {
        O::VecAddF64x4
        | O::VecMulF64x4
        | O::VecAddF32x8
        | O::VecMulF32x8
        | O::VecAddI32x8
        | O::VecMulI32x8
        | O::VecMaxI32x8 => Some(true),
        O::VecSubF64x4 | O::VecSubF32x8 | O::VecDivF64x4 | O::VecDivF32x8 => Some(false),
        _ => None,
    }
}

/// VLFOLD (the general form of IS-05): a single-use 256-bit `VecLoad*` whose
/// only consumer is the next — or next-but-one, across another pure
/// `VecLoad*` — two-operand VEX arithmetic intrinsic in the same block is
/// elided entirely; the consumer folds the load's *source* memory operand
/// (`vpaddd (%rdx,%r13), %ymm0, %ymm0`) exactly like ICX/GCC/Clang emit for
/// `c[i] = a[i] + b[i]` and `s += a[i]`.  Before this analysis the first of
/// two streamed loads was deferred, then flushed to its home slot when the
/// second load claimed `%ymm0`, and the consumer re-read the slot: one
/// store + one stack load per iteration in every map/reduction body.
///
/// Soundness argument (mirrors the deferred-store whitelist above):
/// * the only instructions allowed between load and consumer are pure
///   vector loads, which write no memory and define no GPR, so the elided
///   load's RA-homed base/index registers still hold the same values when
///   the consumer re-materialises the memory operand;
/// * the emitter additionally requires base/index to be RA-homed GPRs (or a
///   zero constant): scratch `%rax`/`%rcx` addressing would not survive the
///   intervening load — it falls back to the ordinary load otherwise;
/// * VEX arithmetic imposes no alignment on memory operands, so unaligned
///   streams are legal; legacy-SSE 128-bit forms (`paddd m128`) would fault,
///   hence only the 256-bit family is eligible;
/// * a non-commutative consumer may only fold its second operand (AT&T
///   `op mem, %src1, %dst` computes `src1 op mem`);
/// * every consumer path either uses the memory operand or materialises the
///   load (`avx_load_arg_to`), and a safety net in `emit_intrinsic_impl`
///   materialises a pending fold before any unexpected intrinsic.
///
/// When both operands of a consumer are adjacent loads, the farther one is
/// folded: the nearer load then streams through `%ymm0` under the existing
/// deferral and the consumer becomes `op mem, %ymm0, %ymm0`.
/// Kill switch: `CCC_NO_VLFOLD=1`.
pub(super) fn compute_vector_memfold_values(func: &IrFunction) -> FxHashSet<u32> {
    let mut result = FxHashSet::default();
    if env_flag("CCC_NO_VLFOLD") {
        return result;
    }

    // Exact use census: intrinsic-argument uses are counted, every other
    // kind of reference (non-intrinsic operand, terminator, value ref,
    // dest_ptr) poisons the value.
    let mut use_count: FxHashMap<u32, u32> = FxHashMap::default();
    let mut poisoned: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Intrinsic { args, dest_ptr, .. } => {
                    for a in args {
                        if let Operand::Value(v) = a {
                            *use_count.entry(v.0).or_default() += 1;
                        }
                    }
                    if let Some(p) = dest_ptr {
                        poisoned.insert(p.0);
                    }
                }
                other => {
                    for_each_operand_in_instruction(other, |op| {
                        if let Operand::Value(v) = op {
                            poisoned.insert(v.0);
                        }
                    });
                    for_each_value_use_in_instruction(other, |v| {
                        poisoned.insert(v.0);
                    });
                }
            }
        }
        for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                poisoned.insert(v.0);
            }
        });
    }

    let debug = env_flag("CCC_DEBUG_VLFOLD");
    for (bi, block) in func.blocks.iter().enumerate() {
        let insts = &block.instructions;
        let load_at = |k: usize| -> Option<u32> {
            match insts.get(k) {
                Some(Instruction::Intrinsic {
                    dest: Some(d), op, ..
                }) if is_memfold_vec_load(op) => Some(d.0),
                _ => None,
            }
        };
        let pure_load_at = |k: usize| -> bool {
            matches!(insts.get(k), Some(Instruction::Intrinsic { op, .. }) if is_pure_vec_load(op))
        };
        for j in 0..insts.len() {
            let Instruction::Intrinsic {
                dest: Some(_),
                op: cop,
                args: cargs,
                ..
            } = &insts[j]
            else {
                continue;
            };
            // Madd `a*b + c`: every position may fold (the multiplicands
            // commute; the emitter picks the 213/231 form and keeps the
            // XMM-homed broadcast as the register source).
            let madd = memfold_consumer_madd_256(cop);
            let (commutative, a0, a1) = if madd {
                if cargs.len() != 3 {
                    continue;
                }
                let (Operand::Value(a0), Operand::Value(_), Operand::Value(a2)) =
                    (&cargs[0], &cargs[1], &cargs[2])
                else {
                    continue;
                };
                (true, a0, a2)
            } else {
                let Some(commutative) = memfold_consumer_256(cop) else {
                    continue;
                };
                if cargs.len() != 2 {
                    continue;
                }
                let (Operand::Value(a0), Operand::Value(a1)) = (&cargs[0], &cargs[1]) else {
                    continue;
                };
                (commutative, a0, a1)
            };
            if a0.0 == a1.0 {
                continue;
            }
            let eligible = |d: u32, is_second: bool| -> bool {
                (is_second || commutative)
                    && !poisoned.contains(&d)
                    && use_count.get(&d).copied() == Some(1)
            };
            let a_mid = match (madd, &cargs[1]) {
                (true, Operand::Value(v)) => Some(v.0),
                _ => None,
            };
            let pick_from = |d: u32| -> Option<u32> {
                if d == a1.0 && eligible(d, true) {
                    Some(d)
                } else if (d == a0.0 || a_mid == Some(d)) && eligible(d, false) {
                    Some(d)
                } else {
                    None
                }
            };
            let mut pick = None;
            if j >= 2 && pure_load_at(j - 1) {
                if let Some(d) = load_at(j - 2) {
                    pick = pick_from(d);
                }
            }
            if pick.is_none() && j >= 1 {
                if let Some(d) = load_at(j - 1) {
                    pick = pick_from(d);
                }
            }
            if let Some(d) = pick {
                if debug {
                    eprintln!("[VLFOLD-IR] {} b{} i{} fold load %{} into {:?}", func.name, bi, j, d, cop);
                }
                result.insert(d);
            }
        }
    }
    result
}

/// Defer a vector home-slot store when every def is consumed once from the
/// last-store peephole by a cache-aware intrinsic in the same block.
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

    // A deferred result lives only in an x86 SIMD scratch register until its
    // consuming intrinsic. Do not carry it across an unrelated instruction
    // that can use the same scratch registers. This is deliberately a
    // whitelist: Copy has no type field and can be a vector/FP copy, while
    // FP operations spill through xmm0/xmm1 under register pressure. Integer
    // address/arithmetic instructions and integer loads are safe to cross.
    // FP/vector loads are not: their generic lowering can use the same scratch
    // registers. Candidates are non-escaping allocas, so the alias checks
    // below independently reject derivations or non-intrinsic slot uses.
    let is_vec_invalidator = crate::backend::generation::instruction_may_clobber_vector_scratch;

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
                op: cop,
                args: cargs,
                ..
            }) = insts.get(u)
            else {
                all_sites_ok = false;
                break;
            };
            let pos = cargs
                .iter()
                .position(|a| matches!(a, Operand::Value(v) if v.0 == slot));
            // VLFOLD: `emit_avx_map_fma` streams whichever multiplicand or
            // bias is not folded/homed through `avx_load_arg`, which is a
            // no-op for the deferred value (positions 1 and 2 of VecMadd*).
            let cache_aware = match pos {
                Some(0) => !is_raw_reader_intrinsic(cop),
                Some(1) => is_two_operand_binary(cop) || memfold_consumer_madd_256(cop),
                Some(2) => memfold_consumer_madd_256(cop),
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
