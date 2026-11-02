//! Path-duplicating jump threading for branches decided by a dominating
//! test (GCC's backward threader, LLVM's JumpThreading).
//!
//! A conditional branch on a value that flows through phis from a
//! predecessor where that value is already known is redundant on that path.
//! The shape that motivated the pass is the two-list merge loop
//! (Linux `lib/list_sort.c` `merge`):
//!
//! ```text
//! H:   a = phi [a0, E], [a'', J]      ; b = phi ...
//!      br a ? T : X                   ; while (a && b)
//! T:   br b ? C : X
//! C:   ... cmp ...  br le ? PA : PB
//! PA:  a' = load a->next  ; br J      ; advance a; b unchanged (non-null)
//! PB:  b' = load b->next  ; br J      ; advance b; a unchanged (non-null)
//! J:   a'' = phi [a', PA], [a, PB]    ; b'' = phi [b, PA], [b', PB]
//!      br H
//! ```
//!
//! On the path `PB -> J -> H`, `a` is known non-null (`PB` lies under the
//! true edge of `H`'s test), so `H`'s branch is decided; on `PA -> J -> H ->
//! T`, `T`'s test of `b` is.  Duplicating the (instruction-free) path blocks
//! for that predecessor and replacing the decided branch by a jump leaves
//! each arm testing only the pointer it advanced, and gives the data
//! dependent compare branch a history-bearing layout (measured on the
//! 14700KF report: 9.58M vs GCC's 3.64M P-core branch misses before).
//!
//! # Algorithm
//!
//! For each block `Bk` ending in `CondBranch(c)` (or on `c ==/!= 0`), walk
//! back from `Bk` through a chain of at most [`MAX_PATH_BLOCKS`] blocks:
//! a value defined by a phi of the chain head is traced into the head's
//! predecessors; a value defined above the head extends the chain through
//! the head's single predecessor.  A predecessor `P` whose incoming value
//! is a constant or is fixed by a dominating `CondBranch` on that value
//! (the edge into a single-predecessor block dominating `P`) is a thread:
//!
//! 1. Clone every chain block for `P`: phis become `Copy`s of their arm on
//!    the path edge, other instructions get fresh destinations, the path
//!    edges point to the next clone, off-path edges keep their targets and
//!    the targets' phis gain an arm for the clone; the last clone jumps to
//!    the decided successor.
//! 2. Retarget `P` to the first clone and drop `P`'s arm from the head.
//! 3. Repair SSA: every value defined in the chain now has two definitions.
//!    Phis are placed at the iterated dominance frontier of the two
//!    definition blocks restricted to blocks where the value is live-in
//!    (pruned SSA); every other use reads the nearest definition or placed
//!    phi on its dominator chain, which in pruned SSA is the unique
//!    reaching definition.
//!
//! The rewrite is done on a copy of the function and committed only when
//! every step succeeded, so a refusal anywhere leaves the input untouched.
//!
//! Cost model: duplicated blocks may hold at most [`MAX_PATH_INSTS`]
//! non-phi instructions in total, all of plain value or memory kinds (no
//! calls, asm, stack or label manipulation), and a function takes at most
//! [`MAX_THREADS_PER_FUNCTION`] threads.  Every thread removes one
//! executed conditional branch from its path.  Not run at -Os/-Oz.
//!
//! Disable with `CCC_DISABLE_PASSES=jump_thread`.

use super::loop_analysis::{NaturalLoop, find_merged_natural_loops};
use super::loop_unroll::rename_inst_dest;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::source::Span;
use crate::common::types::IrType;
use crate::ir::analysis;
use crate::ir::reexports::{
    BasicBlock, BlockId, Instruction, IrCmpOp, IrFunction, Operand, Terminator, Value,
    replace_values_in_inst_map, replace_values_in_terminator_map,
};

/// Longest chain of duplicated blocks.
const MAX_PATH_BLOCKS: usize = 4;
/// Non-phi instructions one thread may duplicate.
const MAX_PATH_INSTS: usize = 8;
/// Threads per function (bounds the growth of repeated threading).
const MAX_THREADS_PER_FUNCTION: usize = 16;
/// Largest loop-header block copied into its latches ([`tail_cost`]).
const MAX_TAIL_DUP_COST: usize = 4;
/// Latches one header may be copied into.
const MAX_TAIL_DUP_LATCHES: usize = 2;
/// Latch copies per function.
const MAX_TAIL_DUPS_PER_FUNCTION: usize = 8;

struct Cfg {
    labels: FxHashMap<BlockId, usize>,
    preds: Vec<Vec<usize>>,
    idom: Vec<usize>,
}

impl Cfg {
    fn build(func: &IrFunction) -> Self {
        let labels = analysis::build_label_map(func);
        let (p, s) = analysis::build_cfg(func, &labels);
        let n = func.blocks.len();
        let idom = analysis::compute_dominators(n, &p, &s);
        let preds = (0..n)
            .map(|b| p.row(b).iter().map(|&x| x as usize).collect())
            .collect();
        Cfg {
            labels,
            preds,
            idom,
        }
    }

    fn reachable(&self, b: usize) -> bool {
        b == 0 || self.idom.get(b).is_some_and(|&d| d != usize::MAX)
    }
}

/// A path cloned for `pred`.  With a `target` the last block's branch is
/// decided and becomes a jump there; without one (latch tail duplication)
/// the clone keeps the original branch.
#[derive(Clone, PartialEq, Eq, Hash)]
struct Thread {
    pred: usize,
    path: Vec<usize>,
    target: Option<BlockId>,
}

/// `Cmp {==, !=} x, 0` definitions: dest -> (x, true-when-nonzero).
fn zero_tests(func: &IrFunction) -> FxHashMap<u32, (Value, bool)> {
    let mut m = FxHashMap::default();
    for inst in func.blocks.iter().flat_map(|b| &b.instructions) {
        if let Instruction::Cmp {
            dest,
            op,
            lhs: Operand::Value(x),
            rhs: Operand::Const(k),
            ..
        } = inst
            && k.is_zero()
        {
            match op {
                IrCmpOp::Ne => {
                    m.insert(dest.0, (*x, true));
                }
                IrCmpOp::Eq => {
                    m.insert(dest.0, (*x, false));
                }
                _ => {}
            }
        }
    }
    m
}

/// Integer comparisons by destination: `(op, lhs, rhs, type)`.  Float
/// compares are left out: with NaN, `!(a < b)` is not `a >= b`.
type CmpFacts = FxHashMap<u32, (IrCmpOp, Operand, Operand, IrType)>;

fn cmp_facts(func: &IrFunction) -> CmpFacts {
    let mut m = FxHashMap::default();
    for inst in func.blocks.iter().flat_map(|b| &b.instructions) {
        if let Instruction::Cmp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        } = inst
            && !ty.is_float()
            && !ty.is_long_double()
        {
            m.insert(dest.0, (*op, *lhs, *rhs, *ty));
        }
    }
    m
}

fn negate_cmp(op: IrCmpOp) -> IrCmpOp {
    use IrCmpOp::*;
    match op {
        Eq => Ne,
        Ne => Eq,
        Slt => Sge,
        Sge => Slt,
        Sle => Sgt,
        Sgt => Sle,
        Ult => Uge,
        Uge => Ult,
        Ule => Ugt,
        Ugt => Ule,
    }
}

/// `a op b` == `b swap(op) a`.
fn swap_cmp(op: IrCmpOp) -> IrCmpOp {
    use IrCmpOp::*;
    match op {
        Eq => Eq,
        Ne => Ne,
        Slt => Sgt,
        Sgt => Slt,
        Sle => Sge,
        Sge => Sle,
        Ult => Ugt,
        Ugt => Ult,
        Ule => Uge,
        Uge => Ule,
    }
}

/// Does `a f b` imply `a q b` (same operands, same order)?
fn cmp_implies(f: IrCmpOp, q: IrCmpOp) -> bool {
    use IrCmpOp::*;
    f == q
        || matches!(
            (f, q),
            (Slt, Sle | Ne)
                | (Sgt, Sge | Ne)
                | (Ult, Ule | Ne)
                | (Ugt, Uge | Ne)
                | (Eq, Sle | Sge | Ule | Uge)
        )
}

/// The truth of comparison `q` given that comparison `fact` evaluated to
/// `truth`, when it follows from the operands alone.
fn implied_cmp(
    fact: (IrCmpOp, Operand, Operand, IrType),
    truth: bool,
    q: (IrCmpOp, Operand, Operand, IrType),
) -> Option<bool> {
    let (fop, fa, fb, fty) = fact;
    let (qop, qa, qb, qty) = q;
    if fty != qty {
        return None;
    }
    // Restate the fact as a true relation `fa holds fb`, oriented like `q`.
    let holds = if truth { fop } else { negate_cmp(fop) };
    let holds = if (qa, qb) == (fa, fb) {
        holds
    } else if (qa, qb) == (fb, fa) {
        swap_cmp(holds)
    } else {
        return None;
    };
    if cmp_implies(holds, qop) {
        Some(true)
    } else if cmp_implies(holds, negate_cmp(qop)) {
        Some(false)
    } else {
        None
    }
}

/// Is `v` known non-zero (`Some(true)`) or zero (`Some(false)`) at the end
/// of block `at`?  Facts come from the edge into a single-predecessor block
/// dominating `at` whose predecessor branches on `v` (or on `v ==/!= 0`, or
/// on an integer comparison of `v`'s own operands that implies it).
fn known_truth(
    func: &IrFunction,
    cfg: &Cfg,
    tests: &FxHashMap<u32, (Value, bool)>,
    cmps: &CmpFacts,
    v: Value,
    at: usize,
) -> Option<bool> {
    let mut x = at;
    loop {
        if let [d] = cfg.preds[x][..]
            && let Terminator::CondBranch {
                cond: Operand::Value(c),
                true_label,
                false_label,
            } = &func.blocks[d].terminator
            && true_label != false_label
        {
            let on_true = *true_label == func.blocks[x].label;
            if *c == v {
                return Some(on_true);
            }
            if let Some(&(t, nonzero_on_true)) = tests.get(&c.0)
                && t == v
            {
                return Some(on_true == nonzero_on_true);
            }
            if let (Some(&fact), Some(&q)) = (cmps.get(&c.0), cmps.get(&v.0))
                && let Some(t) = implied_cmp(fact, on_true, q)
            {
                return Some(t);
            }
        }
        let up = *cfg.idom.get(x)?;
        if up == x || up == usize::MAX {
            return None;
        }
        x = up;
    }
}

/// Instructions a thread may duplicate.
fn duplicable(inst: &Instruction) -> bool {
    matches!(
        inst,
        Instruction::Phi { .. }
            | Instruction::Load { .. }
            | Instruction::Store { .. }
            | Instruction::BinOp { .. }
            | Instruction::UnaryOp { .. }
            | Instruction::Cmp { .. }
            | Instruction::GetElementPtr { .. }
            | Instruction::Cast { .. }
            | Instruction::Copy { .. }
            | Instruction::GlobalAddr { .. }
            | Instruction::Select { .. }
    )
}

fn non_phi_count(b: &BasicBlock) -> usize {
    b.instructions
        .iter()
        .filter(|i| !matches!(i, Instruction::Phi { .. }))
        .count()
}

/// Number of edges from `term` to `to`; `None` for unsupported terminators.
fn edges_to(term: &Terminator, to: BlockId) -> Option<usize> {
    match term {
        Terminator::Branch(t) => Some(usize::from(*t == to)),
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => Some(usize::from(*true_label == to) + usize::from(*false_label == to)),
        _ => None,
    }
}

fn unique_arm(incoming: &[(Operand, BlockId)], from: BlockId) -> Option<Operand> {
    let mut it = incoming.iter().filter(|(_, l)| *l == from);
    let first = it.next()?.0;
    it.next().is_none().then_some(first)
}

fn phi_def<'a>(b: &'a BasicBlock, v: Value) -> Option<&'a [(Operand, BlockId)]> {
    b.instructions.iter().find_map(|i| match i {
        Instruction::Phi { dest, incoming, .. } if *dest == v => Some(&incoming[..]),
        _ => None,
    })
}

struct Search<'a> {
    func: &'a IrFunction,
    cfg: &'a Cfg,
    tests: FxHashMap<u32, (Value, bool)>,
    cmps: CmpFacts,
    refused: &'a FxHashSet<Thread>,
}

impl Search<'_> {
    fn first(&self) -> Option<Thread> {
        for (bk, block) in self.func.blocks.iter().enumerate() {
            if !self.cfg.reachable(bk) {
                continue;
            }
            let Terminator::CondBranch {
                cond: Operand::Value(c),
                true_label,
                false_label,
            } = &block.terminator
            else {
                continue;
            };
            if true_label == false_label {
                continue;
            }
            let (x, nonzero_takes_true) = match self.tests.get(&c.0) {
                Some(&(x, nz)) => (x, nz),
                None => (*c, true),
            };
            let decide = |truth: bool| {
                if truth == nonzero_takes_true {
                    *true_label
                } else {
                    *false_label
                }
            };
            if let Some(t) = self.trace(vec![bk], x, &decide) {
                return Some(t);
            }
            if let Some(t) = self.implied_by_pred(bk, *c, *true_label, *false_label) {
                return Some(t);
            }
        }
        None
    }

    /// The head's own comparison `c` (operands defined above the head) is
    /// decided at the end of a predecessor by an implying edge fact: the
    /// mismatch exit of `while (j >= 0 && a[j] == b[j]) j--;` re-tests
    /// `j < 0` where `j >= 0` is known.  Clone the head for that edge.
    fn implied_by_pred(
        &self,
        head: usize,
        c: Value,
        true_label: BlockId,
        false_label: BlockId,
    ) -> Option<Thread> {
        let &(_, lhs, rhs, _) = self.cmps.get(&c.0)?;
        let block = &self.func.blocks[head];
        let defined_here = |op: Operand| {
            matches!(op, Operand::Value(v)
                if block.instructions.iter().any(|i| i.dest() == Some(v)))
        };
        if defined_here(lhs)
            || defined_here(rhs)
            || non_phi_count(block) > MAX_PATH_INSTS
            || !block.instructions.iter().all(duplicable)
        {
            return None;
        }
        for &p in &self.cfg.preds[head] {
            if p == head
                || !self.cfg.reachable(p)
                || edges_to(&self.func.blocks[p].terminator, block.label) != Some(1)
            {
                continue;
            }
            let Some(truth) = self
                .edge_truth(p, block.label, c)
                .or_else(|| known_truth(self.func, self.cfg, &self.tests, &self.cmps, c, p))
            else {
                continue;
            };
            let thread = Thread {
                pred: p,
                path: vec![head],
                target: Some(if truth { true_label } else { false_label }),
            };
            if !self.refused.contains(&thread) {
                return Some(thread);
            }
        }
        None
    }

    fn trace(
        &self,
        path: Vec<usize>,
        val: Value,
        decide: &dyn Fn(bool) -> BlockId,
    ) -> Option<Thread> {
        let head = path[0];
        let insts: usize = path
            .iter()
            .map(|&b| non_phi_count(&self.func.blocks[b]))
            .sum();
        let head_block = &self.func.blocks[head];
        if path.len() > MAX_PATH_BLOCKS
            || insts > MAX_PATH_INSTS
            || !head_block.instructions.iter().all(duplicable)
            || (path.len() > 1 && !self.chain_edge_ok(head, path[1]))
        {
            return None;
        }
        if let Some(incoming) = phi_def(head_block, val) {
            for &p in &self.cfg.preds[head] {
                if path.contains(&p) || !self.cfg.reachable(p) {
                    continue;
                }
                let pl = self.func.blocks[p].label;
                if edges_to(&self.func.blocks[p].terminator, head_block.label) != Some(1) {
                    continue;
                }
                let Some(w) = unique_arm(incoming, pl) else {
                    continue;
                };
                let truth = match w {
                    Operand::Const(k) => Some(!k.is_zero()),
                    Operand::Value(wv) => {
                        known_truth(self.func, self.cfg, &self.tests, &self.cmps, wv, p)
                    }
                };
                if let Some(t) = truth {
                    let thread = Thread {
                        pred: p,
                        path: path.clone(),
                        target: Some(decide(t)),
                    };
                    if !self.refused.contains(&thread) {
                        return Some(thread);
                    }
                }
                if let Operand::Value(wv) = w
                    && phi_def(&self.func.blocks[p], wv).is_some()
                {
                    let mut longer = vec![p];
                    longer.extend_from_slice(&path);
                    if let Some(t) = self.trace(longer, wv, decide) {
                        return Some(t);
                    }
                }
            }
            return None;
        }
        if head_block
            .instructions
            .iter()
            .any(|i| i.dest() == Some(val))
        {
            return None;
        }
        // Defined above the head: extend through a unique predecessor.
        if let [q] = self.cfg.preds[head][..]
            && !path.contains(&q)
            && edges_to(&self.func.blocks[q].terminator, head_block.label) == Some(1)
        {
            let mut longer = vec![q];
            longer.extend_from_slice(&path);
            return self.trace(longer, val, decide);
        }
        None
    }

    /// The truth of comparison `c` on the edge `p -> to` implied by `p`'s own
    /// branch (the loop-exit edge of `while (j >= 0 ...)` knows `j < 0`).
    fn edge_truth(&self, p: usize, to: BlockId, c: Value) -> Option<bool> {
        let Terminator::CondBranch {
            cond: Operand::Value(pc),
            true_label,
            false_label,
        } = &self.func.blocks[p].terminator
        else {
            return None;
        };
        if true_label == false_label {
            return None;
        }
        let on_true = *true_label == to;
        if *pc == c {
            return Some(on_true);
        }
        implied_cmp(*self.cmps.get(&pc.0)?, on_true, *self.cmps.get(&c.0)?)
    }

    /// A chain block must reach the next one by exactly one edge.
    fn chain_edge_ok(&self, from: usize, to: usize) -> bool {
        edges_to(
            &self.func.blocks[from].terminator,
            self.func.blocks[to].label,
        ) == Some(1)
    }
}

fn fresh_value(func: &mut IrFunction) -> Value {
    let id = func
        .next_value_id
        .max(func.max_value_id().saturating_add(1));
    func.next_value_id = id + 1;
    Value(id)
}

fn fresh_label(func: &mut IrFunction) -> BlockId {
    let live = func.blocks.iter().map(|b| b.label.0).max().unwrap_or(0);
    let id = func.next_label.max(live.saturating_add(1));
    func.next_label = id + 1;
    BlockId(id)
}

fn map_operand(op: Operand, map: &FxHashMap<u32, u32>) -> Operand {
    match op {
        Operand::Value(v) => map.get(&v.0).map_or(op, |&n| Operand::Value(Value(n))),
        Operand::Const(_) => op,
    }
}

fn redirect(term: &mut Terminator, from: BlockId, to: BlockId) {
    match term {
        Terminator::Branch(t) if *t == from => *t = to,
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => {
            if *true_label == from {
                *true_label = to;
            }
            if *false_label == from {
                *false_label = to;
            }
        }
        _ => {}
    }
}

fn successors(term: &Terminator) -> Vec<BlockId> {
    match term {
        Terminator::Branch(t) => vec![*t],
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => vec![*true_label, *false_label],
        _ => Vec::new(),
    }
}

/// Apply `t` to `func`; `None` (and `func` must then be discarded) when any
/// step cannot be completed soundly.
fn apply(func: &mut IrFunction, t: &Thread) -> Option<()> {
    let pred_label = func.blocks[t.pred].label;
    let orig_labels: Vec<BlockId> = t.path.iter().map(|&b| func.blocks[b].label).collect();
    let clone_labels: Vec<BlockId> = t.path.iter().map(|_| fresh_label(func)).collect();
    let mut map: FxHashMap<u32, u32> = FxHashMap::default();
    let mut types: FxHashMap<u32, IrType> = FxHashMap::default();
    let mut clones = Vec::with_capacity(t.path.len());
    // Clone label -> length of its leading phi-copy group (see `repair_ssa`).
    let mut phi_prefix: FxHashMap<u32, usize> = FxHashMap::default();
    // (successor label, original block label, clone label, mapped arm values).
    let mut extra_arms: Vec<(BlockId, BlockId, BlockId, FxHashMap<u32, u32>)> = Vec::new();

    for (i, &bi) in t.path.iter().enumerate() {
        let from = if i == 0 {
            pred_label
        } else {
            orig_labels[i - 1]
        };
        let block = func.blocks[bi].clone();
        let spans =
            (block.source_spans.len() == block.instructions.len()).then_some(&block.source_spans);
        let mut insts = Vec::with_capacity(block.instructions.len());
        let mut out_spans = Vec::new();
        // Phis read the values at the end of the path edge: map every arm
        // before this block's own renames take effect.
        let mut phi_renames = Vec::new();
        for (k, inst) in block.instructions.iter().enumerate() {
            if let Instruction::Phi { dest, ty, incoming } = inst {
                let arm = map_operand(unique_arm(incoming, from)?, &map);
                let nd = fresh_value(func);
                types.insert(dest.0, *ty);
                phi_renames.push((dest.0, nd.0));
                insts.push(Instruction::Copy { dest: nd, src: arm });
                if let Some(s) = spans {
                    out_spans.push(s[k]);
                }
            }
        }
        phi_prefix.insert(clone_labels[i].0, phi_renames.len());
        map.extend(phi_renames);
        for (k, inst) in block.instructions.iter().enumerate() {
            if matches!(inst, Instruction::Phi { .. }) {
                continue;
            }
            let mut c = inst.clone();
            replace_values_in_inst_map(&mut c, &map);
            if let Some(d) = inst.dest() {
                let nd = fresh_value(func);
                let mut one = FxHashMap::default();
                one.insert(d.0, nd.0);
                rename_inst_dest(&mut c, &one);
                if let Some(ty) = inst.result_type() {
                    types.insert(d.0, ty);
                }
                map.insert(d.0, nd.0);
            }
            insts.push(c);
            if let Some(s) = spans {
                out_spans.push(s[k]);
            }
        }
        let terminator = if i + 1 < t.path.len() {
            let mut term = block.terminator.clone();
            replace_values_in_terminator_map(&mut term, &map);
            redirect(&mut term, orig_labels[i + 1], clone_labels[i + 1]);
            term
        } else if let Some(target) = t.target {
            Terminator::Branch(target)
        } else {
            let mut term = block.terminator.clone();
            replace_values_in_terminator_map(&mut term, &map);
            term
        };
        for s in successors(&terminator) {
            if !clone_labels.contains(&s) {
                extra_arms.push((s, orig_labels[i], clone_labels[i], map.clone()));
            }
        }
        clones.push(BasicBlock {
            label: clone_labels[i],
            instructions: insts,
            terminator,
            source_spans: out_spans,
        });
    }

    // Off-path successors read the clone's values on the new edges.
    let labels = analysis::build_label_map(func);
    for (succ, orig, clone, m) in &extra_arms {
        let &si = labels.get(succ)?;
        for inst in &mut func.blocks[si].instructions {
            if let Instruction::Phi { incoming, .. } = inst {
                let arm = map_operand(unique_arm(incoming, *orig)?, m);
                incoming.push((arm, *clone));
            }
        }
    }
    // Predecessor -> first clone; its arm leaves the original head.
    redirect(
        &mut func.blocks[t.pred].terminator,
        orig_labels[0],
        clone_labels[0],
    );
    for inst in &mut func.blocks[t.path[0]].instructions {
        if let Instruction::Phi { incoming, .. } = inst {
            incoming.retain(|(_, l)| *l != pred_label);
        }
    }
    let at = t.pred + 1;
    for (k, c) in clones.into_iter().enumerate() {
        func.blocks.insert(at + k, c);
    }
    super::cfg_simplify::eliminate_unreachable_blocks(func);
    repair_ssa(func, &map, &types, &phi_prefix)
}

/// Restore single-reaching-definition form for every `orig -> clone` pair.
///
/// A clone block starts with the copies its phis became.  Those are a
/// *parallel* assignment reading the values on the incoming edge, so every
/// use inside that prefix (`phi_prefix[label]` instructions) reads the
/// block's entry value even when a clone definition precedes it there --
/// `b' = a` after `a' = ...` in a rotation (`h = g, g = f, ...`) still means
/// the previous `a`.
fn repair_ssa(
    func: &mut IrFunction,
    pairs: &FxHashMap<u32, u32>,
    types: &FxHashMap<u32, IrType>,
    phi_prefix: &FxHashMap<u32, usize>,
) -> Option<()> {
    let cfg = Cfg::build(func);
    let n = func.blocks.len();
    let (fp, _) = analysis::build_cfg(func, &cfg.labels);
    let df = analysis::compute_dominance_frontiers(n, &fp, &cfg.idom);
    let mut def_block: FxHashMap<u32, usize> = FxHashMap::default();
    for (bi, b) in func.blocks.iter().enumerate() {
        for inst in &b.instructions {
            if let Some(d) = inst.dest() {
                def_block.insert(d.0, bi);
            }
        }
    }
    let mut order: Vec<(&u32, &u32)> = pairs.iter().collect();
    order.sort_unstable();
    for (&orig, &clone) in order {
        let defs: FxHashMap<usize, u32> = [orig, clone]
            .iter()
            .filter_map(|v| def_block.get(v).map(|&b| (b, *v)))
            .collect();
        if !uses_exist(func, orig) {
            continue;
        }
        if !defs.values().any(|&v| v == clone) {
            // The clone died with an unreachable block: nothing to merge.
            continue;
        }
        // Live-in blocks of `orig` (pruned SSA).
        let mut live_in = vec![false; n];
        let mut work = Vec::new();
        for (bi, b) in func.blocks.iter().enumerate() {
            if !cfg.reachable(bi) {
                continue;
            }
            // Is `orig` read before any definition in this block (live-in)?
            let mut read_on_entry = false;
            let mut defined = false;
            let prefix = phi_prefix.get(&b.label.0).copied().unwrap_or(0);
            for (k, inst) in b.instructions.iter().enumerate() {
                if let Instruction::Phi { incoming, .. } = inst {
                    for (op, from) in incoming {
                        if matches!(op, Operand::Value(v) if v.0 == orig)
                            && let Some(&q) = cfg.labels.get(from)
                            && !defs.contains_key(&q)
                        {
                            work.push(q);
                        }
                    }
                } else if !defined || k < prefix {
                    inst.for_each_used_value(|v| read_on_entry |= v == orig);
                }
                defined |= inst.dest().is_some_and(|d| d.0 == orig || d.0 == clone);
            }
            if !defined {
                b.terminator
                    .for_each_used_value(|v| read_on_entry |= v == orig);
            }
            if read_on_entry {
                if defs.contains_key(&bi) {
                    // A definition block read before its definition (a
                    // clone's phi copies): live-out of its predecessors.
                    work.extend(
                        cfg.preds[bi]
                            .iter()
                            .copied()
                            .filter(|q| !defs.contains_key(q)),
                    );
                } else {
                    work.push(bi);
                }
            }
        }
        while let Some(b) = work.pop() {
            if live_in[b] || defs.contains_key(&b) {
                continue;
            }
            live_in[b] = true;
            work.extend(cfg.preds[b].iter().copied());
        }
        // Iterated dominance frontier of the definitions, pruned.
        let mut phi_at: FxHashMap<usize, u32> = FxHashMap::default();
        let mut in_idf = FxHashSet::default();
        let mut wl: Vec<usize> = defs.keys().copied().collect();
        while let Some(d) = wl.pop() {
            for &z in df.get(d)? {
                if in_idf.insert(z) {
                    wl.push(z);
                }
            }
        }
        let mut idf: Vec<usize> = in_idf.into_iter().filter(|&z| live_in[z]).collect();
        idf.sort_unstable();
        // A merge phi needs the value's type; `Copy` results carry none.
        let ty = match (idf.is_empty(), types.get(&orig)) {
            (true, _) => IrType::I64,
            (false, Some(&t)) => t,
            (false, None) => return None,
        };
        for &z in &idf {
            phi_at.insert(z, fresh_value(func).0);
        }
        let at_end = |mut x: usize| -> Option<u32> {
            loop {
                if let Some(&v) = defs.get(&x) {
                    return Some(v);
                }
                if let Some(&p) = phi_at.get(&x) {
                    return Some(p);
                }
                let up = *cfg.idom.get(x)?;
                if up == x || up == usize::MAX {
                    return None;
                }
                x = up;
            }
        };
        let at_entry = |x: usize| -> Option<u32> {
            if let Some(&p) = phi_at.get(&x) {
                return Some(p);
            }
            let up = *cfg.idom.get(x)?;
            if up == x || up == usize::MAX {
                return None;
            }
            at_end(up)
        };
        // Build the placed phis' arms before rewriting anything.
        let mut new_phis: Vec<(usize, Instruction)> = Vec::new();
        for &z in &idf {
            let mut incoming = Vec::new();
            let mut seen = FxHashSet::default();
            for &q in &cfg.preds[z] {
                if !seen.insert(q) {
                    return None;
                }
                incoming.push((Operand::Value(Value(at_end(q)?)), func.blocks[q].label));
            }
            new_phis.push((
                z,
                Instruction::Phi {
                    dest: Value(phi_at[&z]),
                    ty,
                    incoming,
                },
            ));
        }
        // Rewrite the uses of `orig`.  Phi arms read at the end of their
        // edge.  Elsewhere a use after a definition in its own block reads
        // that definition; a use at or before it reads the block's entry
        // value -- in a clone the phi-turned-`Copy` reads sit before the
        // clone's definitions and still mean "the value on the edge" (the
        // previous iteration's instance of a header phi on a back edge).
        for bi in 0..n {
            if !cfg.reachable(bi) {
                continue;
            }
            let block = &mut func.blocks[bi];
            let local_def = block
                .instructions
                .iter()
                .position(|i| i.dest().is_some_and(|d| d.0 == orig || d.0 == clone))
                .map(|k| (k, block.instructions[k].dest().map_or(orig, |d| d.0)));
            let mut entry_val = None;
            let prefix = phi_prefix.get(&block.label.0).copied().unwrap_or(0);
            for (k, inst) in block.instructions.iter_mut().enumerate() {
                if let Instruction::Phi { incoming, .. } = inst {
                    for (op, from) in incoming.iter_mut() {
                        if matches!(op, Operand::Value(v) if v.0 == orig) {
                            let &q = cfg.labels.get(from)?;
                            *op = Operand::Value(Value(at_end(q)?));
                        }
                    }
                    continue;
                }
                let mut uses = false;
                inst.for_each_used_value(|v| uses |= v == orig);
                if !uses {
                    continue;
                }
                let to = match local_def {
                    Some((d, val)) if k > d && k >= prefix => val,
                    _ => match entry_val {
                        Some(e) => e,
                        None => {
                            let e = at_entry(bi)?;
                            entry_val = Some(e);
                            e
                        }
                    },
                };
                if to != orig {
                    let mut m = FxHashMap::default();
                    m.insert(orig, to);
                    replace_values_in_inst_map(inst, &m);
                }
            }
            let mut uses = false;
            block.terminator.for_each_used_value(|v| uses |= v == orig);
            if uses {
                let to = match local_def {
                    Some((_, val)) => val,
                    None => at_entry(bi)?,
                };
                if to != orig {
                    let mut m = FxHashMap::default();
                    m.insert(orig, to);
                    replace_values_in_terminator_map(&mut block.terminator, &m);
                }
            }
        }
        for (z, phi) in new_phis {
            let block = &mut func.blocks[z];
            block.instructions.insert(0, phi);
            if !block.source_spans.is_empty() {
                block.source_spans.insert(0, Span::dummy());
            }
        }
    }
    Some(())
}

fn uses_exist(func: &IrFunction, v: u32) -> bool {
    func.blocks.iter().any(|b| {
        let mut hit = false;
        for inst in &b.instructions {
            inst.for_each_used_value(|u| hit |= u == v);
        }
        b.terminator.for_each_used_value(|u| hit |= u == v);
        hit
    })
}

/// Cost of a block for tail duplication: instructions that survive to
/// machine code (constant-offset GEPs fold into addressing, copies vanish).
fn tail_cost(b: &BasicBlock) -> usize {
    b.instructions
        .iter()
        .filter(|i| match i {
            Instruction::Phi { .. } | Instruction::Copy { .. } => false,
            Instruction::GetElementPtr { offset, .. } => !matches!(offset, Operand::Const(_)),
            _ => true,
        })
        .count()
}

/// Does `b`'s conditional branch depend, within `b`, on a value `b` loads?
fn branch_reads_block_load(b: &BasicBlock) -> bool {
    let mut tainted: FxHashSet<u32> = FxHashSet::default();
    for inst in &b.instructions {
        let mut reads = matches!(inst, Instruction::Load { .. });
        if !reads {
            inst.for_each_used_value(|v| reads |= tainted.contains(&v));
        }
        if reads && let Some(d) = inst.dest() {
            tainted.insert(d.0);
        }
    }
    let mut cond_tainted = false;
    b.terminator
        .for_each_used_value(|v| cond_tainted |= tainted.contains(&v));
    cond_tainted
}

/// Is `h`'s branch a choice *within* a loop iteration rather than an exit?
/// Take `L`, the smallest natural loop holding `h` and both successors.
/// It qualifies when `L` is headed by `h`, or when every loop nested in `L`
/// holds `h` or sits inside one that does: the arm that leaves `h`'s own
/// cycle comes back without running other loop-level work.  In the list
/// merge, the take-`b` arm returns through the enclosing `b != NULL` test
/// (`L`'s only child loop holds the compare); in fannkuch's
/// `while ((k = perm[0]) != 0)`, the exit arm reaches `h` again only around
/// the outer `for (;;)`, past sibling loops, and copying it is plain
/// rotation (+4.4% Ir from the rotated copy's register pressure).
///
/// An arm leaving `h`'s own loop must also be private to `h` (`h` its only
/// predecessor), like the merge's take-`b` path.  A shared exit join -- the
/// three tests of `while (*a && *b && *a == *b)` all exit to one block --
/// already merges values in phis, and every cloned exit edge then carries
/// copies into them (`strlen_bench`'s inlined `my_strstr`, +2.3% Ir).
fn branch_chooses_within_iteration(
    loops: &[NaturalLoop],
    preds: &[Vec<usize>],
    h: usize,
    t: usize,
    f: usize,
) -> bool {
    let Some(l) = loops
        .iter()
        .filter(|m| m.contains(h) && m.contains(t) && m.contains(f))
        .min_by_key(|m| m.len())
    else {
        return false;
    };
    if l.header == h {
        return true;
    }
    let own = loops.iter().find(|m| m.header == h);
    let private = |x: usize| own.is_some_and(|m| m.contains(x)) || preds[x][..] == [h];
    if !private(t) || !private(f) {
        return false;
    }
    let holds_h = |m: &NaturalLoop| m.contains(h) && m.is_subloop_of(l);
    loops
        .iter()
        .filter(|m| m.is_subloop_of(l))
        .all(|m| m.contains(h) || loops.iter().any(|n| holds_h(n) && m.is_subloop_of(n)))
}

/// Latch tail duplication (GCC bb-reorder's small-block copy, `copy_bb_p`):
/// a small loop-header block ending in a conditional branch is copied into
/// each latch that jumps to it unconditionally.  The latch loses its
/// `jmp`, and each copy is its own branch site, so a data-dependent header
/// branch is predicted with "which latch ran" as history -- the list merge
/// compare drops from 25.0M to the two-site split GCC gets (Callgrind
/// branch simulation, 19.1M for GCC's two sites).
///
/// Only a branch on a value **loaded in the header block** qualifies
/// ([`branch_reads_block_load`]): splitting a site helps only a
/// data-dependent branch.  An induction-variable or pointer-null exit test is
/// predicted near-perfectly from one site, and both arms of the copied
/// branch must choose within an iteration
/// ([`branch_chooses_within_iteration`]): copying an exit test, even a
/// loaded one, is plain loop rotation, which `loop_invert` already performs later on the shape the
/// rest of the pipeline expects.  Copying those early anyway was measured to
/// cost Ir with no prediction gain: +8.4% `bitops`, +12% `mandelbrot`, +13%
/// `linux_find_bit`, +33% `percpu_ptr_walk` (Callgrind corpus screen; one
/// extra `mov` per iteration and three more callee-saved registers on
/// `bitops`).
fn tail_duplicate_latches(func: &mut IrFunction) -> usize {
    // Each copy turns one unconditional latch edge into conditional ones,
    // so rounds terminate; the budget bounds code growth.
    let mut total = 0;
    while total < MAX_TAIL_DUPS_PER_FUNCTION {
        let n = tail_duplicate_round(func, MAX_TAIL_DUPS_PER_FUNCTION - total);
        if n == 0 {
            break;
        }
        total += n;
    }
    total
}

fn tail_duplicate_round(func: &mut IrFunction, budget: usize) -> usize {
    let cfg = Cfg::build(func);
    let loops = {
        let (p, s) = analysis::build_cfg(func, &cfg.labels);
        find_merged_natural_loops(func.blocks.len(), &p, &s, &cfg.idom)
    };
    let mut cands: Vec<(BlockId, BlockId)> = Vec::new();
    for (bi, b) in func.blocks.iter().enumerate() {
        let Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } = &b.terminator
        else {
            continue;
        };
        if true_label == false_label
            || !cfg.reachable(bi)
            || tail_cost(b) > MAX_TAIL_DUP_COST
            || !b.instructions.iter().all(duplicable)
        {
            continue;
        }
        let latches: Vec<usize> = cfg.preds[bi]
            .iter()
            .copied()
            .filter(|&p| {
                p != bi
                    && analysis::dominates_idom(&cfg.idom, bi, p)
                    && matches!(func.blocks[p].terminator, Terminator::Branch(t) if t == b.label)
            })
            .collect();
        let entries = cfg.preds[bi].len() - latches.len();
        if latches.is_empty()
            || entries == 0
            || latches.len() > MAX_TAIL_DUP_LATCHES
            || !branch_reads_block_load(b)
            || !cfg
                .labels
                .get(true_label)
                .zip(cfg.labels.get(false_label))
                .is_some_and(|(&t, &f)| {
                    branch_chooses_within_iteration(&loops, &cfg.preds, bi, t, f)
                })
        {
            continue;
        }
        cands.extend(latches.iter().map(|&p| (func.blocks[p].label, b.label)));
    }
    let mut applied = 0;
    for (pl, bl) in cands {
        if applied == budget {
            break;
        }
        let labels = analysis::build_label_map(func);
        let (Some(&p), Some(&b)) = (labels.get(&pl), labels.get(&bl)) else {
            continue;
        };
        if !matches!(func.blocks[p].terminator, Terminator::Branch(t) if t == bl) {
            continue;
        }
        let thread = Thread {
            pred: p,
            path: vec![b],
            target: None,
        };
        let mut trial = func.clone();
        if apply(&mut trial, &thread).is_some() {
            *func = trial;
            applied += 1;
        }
    }
    applied
}

/// Run the pass on one function: thread decided branches, then copy small
/// loop headers into their latches.  Returns the number of rewrites.
pub(crate) fn run(func: &mut IrFunction) -> usize {
    if func.blocks.len() < 2 {
        return 0;
    }
    thread_decided_branches(func) + tail_duplicate_latches(func)
}

/// Does `thread` copy blocks of a loop `L` for a predecessor outside `L` and
/// leave the copy by an edge into `L` other than to its header?
///
/// Such a thread decides a branch for the loop ENTRY only: the saving is one
/// branch per entry, while the clone becomes a second way into the loop body
/// and SSA repair must merge every value the path defines at the blocks where
/// the clone and the original meet, which now lie inside `L` and run every
/// iteration.  count_zero_intermediate_casts.c's driver loop (`i == 0 ? 2 :
/// ...` tested at the top of `for (i = 0; ...)`) took two such phis for `i`
/// and the LCG state and spent four more instructions per iteration (303,127
/// -> 319,501 Ir in `main` under Callgrind).  LLVM's JumpThreading refuses
/// loop headers for the same reason.  A thread whose every exit leaves `L`
/// (or re-enters at its header) bypasses or enters the loop whole and keeps
/// the merges outside it, so it stays allowed, as do threads whose
/// predecessor is inside `L` (the list-merge latch threads).
fn enters_loop_body(
    succs: &analysis::FlatAdj,
    cfg: &Cfg,
    loops: &[NaturalLoop],
    thread: &Thread,
) -> bool {
    let index = |l: &BlockId| cfg.labels.get(l).copied();
    loops.iter().any(|l| {
        if l.contains(thread.pred) || !thread.path.iter().any(|&b| l.contains(b)) {
            return false;
        }
        // A thread that iterates an enclosing loop -- its source inside that
        // loop, its path through that loop's header, i.e. across its back
        // edge -- reaches `l` from inside the enclosing loop.  That is the
        // latch thread this pass exists for (the merge loop: once one arm
        // is threaded its copy closes an inner loop, and the other arm's
        // thread runs through the outer header into that inner body), not
        // an entry-edge rotation of `l`.
        let iterates_enclosing = loops.iter().any(|m| {
            m.header != l.header
                && m.contains(thread.pred)
                && l.is_subloop_of(m)
                && thread.path.contains(&m.header)
        });
        if iterates_enclosing {
            return false;
        }
        let mut exits: Vec<usize> = Vec::new();
        for (i, &b) in thread.path.iter().enumerate() {
            let next = thread.path.get(i + 1).copied();
            let last = next.is_none();
            if last && let Some(t) = &thread.target {
                exits.extend(index(t));
                continue;
            }
            exits.extend(
                succs
                    .row(b)
                    .iter()
                    .map(|&si| si as usize)
                    .filter(|&si| Some(si) != next),
            );
        }
        exits.iter().any(|&e| l.contains(e) && e != l.header)
    })
}

/// Apply decided-branch threads until none is left (or the budget is spent).
fn thread_decided_branches(func: &mut IrFunction) -> usize {
    let mut refused = FxHashSet::default();
    let mut applied = 0;
    while applied < MAX_THREADS_PER_FUNCTION {
        let cfg = Cfg::build(func);
        let search = Search {
            func,
            cfg: &cfg,
            tests: zero_tests(func),
            cmps: cmp_facts(func),
            refused: &refused,
        };
        let Some(thread) = search.first() else {
            break;
        };
        let (p, s) = analysis::build_cfg(func, &cfg.labels);
        let loops = find_merged_natural_loops(func.blocks.len(), &p, &s, &cfg.idom);
        if enters_loop_body(&s, &cfg, &loops, &thread) {
            refused.insert(thread);
            continue;
        }
        let mut trial = func.clone();
        if apply(&mut trial, &thread).is_some() {
            *func = trial;
            applied += 1;
        } else {
            refused.insert(thread);
        }
    }
    applied
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::AddressSpace;

    fn block(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions,
            terminator,
            source_spans: Vec::new(),
        }
    }

    fn v(n: u32) -> Operand {
        Operand::Value(Value(n))
    }

    fn phi(dest: u32, arms: &[(u32, u32)]) -> Instruction {
        Instruction::Phi {
            dest: Value(dest),
            ty: IrType::Ptr,
            incoming: arms.iter().map(|&(x, b)| (v(x), BlockId(b))).collect(),
        }
    }

    fn load(dest: u32, ptr: u32) -> Instruction {
        Instruction::Load {
            dest: Value(dest),
            ptr: Value(ptr),
            ty: IrType::Ptr,
            seg_override: AddressSpace::Default,
            volatile: false,
        }
    }

    fn cbr(c: u32, t: u32, f: u32) -> Terminator {
        Terminator::CondBranch {
            cond: v(c),
            true_label: BlockId(t),
            false_label: BlockId(f),
        }
    }

    /// The list merge loop of the module docs.
    /// B0: a0=v0, b0=v1 -> H1.  H1: a=v10 phi[v0@0, v20@6], b=v11 phi[v1@0, v21@6],
    /// br a ? T2 : X7.  T2: br b ? C3 : X7.  C3: c=v12 (opaque) br c ? PA4 : PB5.
    /// PA4: v13 = load a -> J6.  PB5: v14 = load b -> J6.
    /// J6: v20 = phi[v13@4, v10@5], v21 = phi[v11@4, v14@5] -> H1.
    /// X7: return a.
    fn merge_loop() -> IrFunction {
        let mut f = IrFunction::new("m".into(), IrType::Ptr, vec![], false);
        let param = |d: u32| Instruction::ParamRef {
            dest: Value(d),
            param_idx: d as usize,
            ty: IrType::Ptr,
        };
        f.blocks = vec![
            block(
                0,
                vec![param(0), param(1), param(2)],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![phi(10, &[(0, 0), (20, 6)]), phi(11, &[(1, 0), (21, 6)])],
                cbr(10, 2, 7),
            ),
            block(2, vec![], cbr(11, 3, 7)),
            block(3, vec![], cbr(2, 4, 5)),
            block(4, vec![load(13, 10)], Terminator::Branch(BlockId(6))),
            block(5, vec![load(14, 11)], Terminator::Branch(BlockId(6))),
            block(
                6,
                vec![phi(20, &[(13, 4), (10, 5)]), phi(21, &[(11, 4), (14, 5)])],
                Terminator::Branch(BlockId(1)),
            ),
            block(7, vec![], Terminator::Return(Some(v(10)))),
        ];
        f
    }

    /// Reference interpreter for the integer subset these tests use, so a
    /// rewrite is checked against the original's *behaviour* (a dominating
    /// but stale value instance passes structural checks).
    fn eval(f: &IrFunction, params: &[i64], mem: &mut FxHashMap<i64, i64>) -> Option<i64> {
        use crate::ir::reexports::IrBinOp;
        let labels = analysis::build_label_map(f);
        let mut env: FxHashMap<u32, i64> = FxHashMap::default();
        let val = |env: &FxHashMap<u32, i64>, op: &Operand| -> Option<i64> {
            match op {
                Operand::Value(x) => env.get(&x.0).copied(),
                Operand::Const(k) => k.to_i64(),
            }
        };
        let (mut cur, mut prev) = (0usize, None::<BlockId>);
        for _ in 0..100_000 {
            let b = &f.blocks[cur];
            let mut phis = Vec::new();
            for inst in &b.instructions {
                if let Instruction::Phi { dest, incoming, .. } = inst {
                    let arm = incoming.iter().find(|(_, l)| Some(*l) == prev)?;
                    phis.push((dest.0, val(&env, &arm.0)?));
                }
            }
            env.extend(phis);
            for inst in &b.instructions {
                let r = match inst {
                    Instruction::Phi { .. } => continue,
                    Instruction::ParamRef { param_idx, .. } => *params.get(*param_idx)?,
                    Instruction::Copy { src, .. } => val(&env, src)?,
                    Instruction::BinOp { op, lhs, rhs, .. } => {
                        let (l, r) = (val(&env, lhs)?, val(&env, rhs)?);
                        match op {
                            IrBinOp::Add => l.wrapping_add(r),
                            IrBinOp::Sub => l.wrapping_sub(r),
                            IrBinOp::Mul => l.wrapping_mul(r),
                            _ => return None,
                        }
                    }
                    Instruction::Cmp { op, lhs, rhs, .. } => {
                        let (l, r) = (val(&env, lhs)?, val(&env, rhs)?);
                        i64::from(op.eval_i64(l, r))
                    }
                    Instruction::GetElementPtr { base, offset, .. } => {
                        env.get(&base.0)?.wrapping_add(val(&env, offset)?)
                    }
                    Instruction::Load { ptr, .. } => *mem.get(env.get(&ptr.0)?).unwrap_or(&0),
                    Instruction::Store { val: x, ptr, .. } => {
                        let (a, x) = (*env.get(&ptr.0)?, val(&env, x)?);
                        mem.insert(a, x);
                        continue;
                    }
                    _ => return None,
                };
                env.insert(inst.dest()?.0, r);
            }
            prev = Some(b.label);
            cur = match &b.terminator {
                Terminator::Branch(t) => *labels.get(t)?,
                Terminator::CondBranch {
                    cond,
                    true_label,
                    false_label,
                } => *labels.get(if val(&env, cond)? != 0 {
                    true_label
                } else {
                    false_label
                })?,
                Terminator::Return(r) => return r.as_ref().map_or(Some(0), |op| val(&env, op)),
                _ => return None,
            };
        }
        None
    }

    /// Run `f` before and after the pass on each input; results and final
    /// memory must agree.
    fn assert_same_behaviour(f: &IrFunction, inputs: &[(Vec<i64>, Vec<(i64, i64)>)]) {
        let mut g = f.clone();
        assert!(
            run(&mut g) > 0,
            "the pass must fire for the check to mean anything"
        );
        for (params, init) in inputs {
            let mut m1: FxHashMap<i64, i64> = init.iter().copied().collect();
            let mut m2 = m1.clone();
            let r1 = eval(f, params, &mut m1);
            assert!(r1.is_some(), "reference run failed for {params:?}");
            assert_eq!(
                r1,
                eval(&g, params, &mut m2),
                "result differs for {params:?}"
            );
            assert_eq!(m1, m2, "memory differs for {params:?}");
        }
    }

    /// Copy header `h` into each latch with the pass's own clone-and-repair
    /// step, bypassing the profitability gate, and check behaviour.
    fn assert_latch_copies_behave(
        f: &IrFunction,
        h: u32,
        latches: &[u32],
        inputs: &[(Vec<i64>, Vec<(i64, i64)>)],
    ) {
        let mut g = f.clone();
        for &l in latches {
            let labels = analysis::build_label_map(&g);
            let thread = Thread {
                pred: labels[&BlockId(l)],
                path: vec![labels[&BlockId(h)]],
                target: None,
            };
            assert!(apply(&mut g, &thread).is_some(), "copy into latch {l}");
        }
        for (params, init) in inputs {
            let mut m1: FxHashMap<i64, i64> = init.iter().copied().collect();
            let mut m2 = m1.clone();
            let r1 = eval(f, params, &mut m1);
            assert!(r1.is_some(), "reference run failed for {params:?}");
            assert_eq!(
                r1,
                eval(&g, params, &mut m2),
                "result differs for {params:?}"
            );
            assert_eq!(m1, m2, "memory differs for {params:?}");
        }
    }

    /// Two lists merged by a per-node flag at offset 8; the accumulator
    /// records the order the arms ran in.
    fn flagged_merge_loop() -> IrFunction {
        let mut f = merge_loop();
        let gep = |d: u32, base: u32| Instruction::GetElementPtr {
            dest: Value(d),
            base: Value(base),
            offset: Operand::Const(crate::ir::reexports::IrConst::I64(8)),
            ty: IrType::Ptr,
        };
        let bin = |d: u32, l: u32, k: i64| Instruction::BinOp {
            dest: Value(d),
            op: crate::ir::reexports::IrBinOp::Mul,
            lhs: v(l),
            rhs: Operand::Const(crate::ir::reexports::IrConst::I64(k)),
            ty: IrType::I64,
        };
        let add = |d: u32, l: u32, k: i64| Instruction::BinOp {
            dest: Value(d),
            op: crate::ir::reexports::IrBinOp::Add,
            lhs: v(l),
            rhs: Operand::Const(crate::ir::reexports::IrConst::I64(k)),
            ty: IrType::I64,
        };
        // H accumulates s; C3 branches on a's flag.
        f.blocks[1].instructions.push(phi(30, &[(40, 0), (33, 6)]));
        f.blocks[0].instructions.push(Instruction::Copy {
            dest: Value(40),
            src: Operand::Const(crate::ir::reexports::IrConst::I64(0)),
        });
        f.blocks[3].instructions = vec![gep(15, 10), load(16, 15)];
        f.blocks[3].terminator = cbr(16, 4, 5);
        f.blocks[4]
            .instructions
            .extend([bin(31, 30, 3), add(34, 31, 1)]);
        f.blocks[5]
            .instructions
            .extend([bin(32, 30, 3), add(35, 32, 2)]);
        f.blocks[6].instructions.push(phi(33, &[(34, 4), (35, 5)]));
        f.blocks[7].terminator = Terminator::Return(Some(v(30)));
        f
    }

    #[test]
    fn threaded_merge_loop_behaves_like_the_original() {
        let f = flagged_merge_loop();
        // Lists at 100.. and 500..: node = [next, flag].
        let list = |base: i64, flags: &[i64]| -> Vec<(i64, i64)> {
            let mut m = Vec::new();
            for (k, &fl) in flags.iter().enumerate() {
                let at = base + 16 * k as i64;
                let next = if k + 1 < flags.len() { at + 16 } else { 0 };
                m.push((at, next));
                m.push((at + 8, fl));
            }
            m
        };
        let mut inputs = Vec::new();
        for (fa, fb) in [
            (vec![1, 1, 0, 1], vec![0, 1, 0]),
            (vec![0, 0, 0], vec![1, 1, 1, 1]),
            (vec![1], vec![0]),
            (vec![1, 0, 1, 0, 1, 0], vec![1, 0, 1]),
        ] {
            let mut m = list(100, &fa);
            m.extend(list(500, &fb));
            inputs.push((vec![100, 500, 0], m));
        }
        inputs.push((vec![0, 500, 0], list(500, &[1, 0])));
        inputs.push((vec![100, 0, 0], list(100, &[1, 0])));
        assert_same_behaviour(&f, &inputs);
    }

    /// `main`'s list-building loop: the header's `p` reads the header's own
    /// `q` across the back edge (`p = phi[0, q]; q = phi[base, q + 24]`).
    /// Tail-duplicating the header into the latch must make the clone's
    /// copy of `p` read the *current* `q`, not the stale header instance.
    #[test]
    fn a_header_phi_read_across_the_back_edge_keeps_its_instance() {
        let mut f = IrFunction::new("r".into(), IrType::I64, vec![], false);
        let param = |d: u32, i: usize| Instruction::ParamRef {
            dest: Value(d),
            param_idx: i,
            ty: IrType::I64,
        };
        let k = |x: i64| Operand::Const(crate::ir::reexports::IrConst::I64(x));
        let ult = |d: u32, l: u32, r: u32| Instruction::Cmp {
            dest: Value(d),
            op: IrCmpOp::Ult,
            lhs: v(l),
            rhs: v(r),
            ty: IrType::I64,
        };
        let store = || Instruction::Store {
            val: v(10),
            ptr: Value(11),
            ty: IrType::I64,
            seg_override: AddressSpace::Default,
            volatile: false,
        };
        let gep = |d: u32, off: i64| Instruction::GetElementPtr {
            dest: Value(d),
            base: Value(11),
            offset: k(off),
            ty: IrType::Ptr,
        };
        let inc = |d: u32| Instruction::BinOp {
            dest: Value(d),
            op: crate::ir::reexports::IrBinOp::Add,
            lhs: v(12),
            rhs: k(1),
            ty: IrType::I64,
        };
        let phi3 = |d: u32, init: Operand, a4: u32, a5: u32| Instruction::Phi {
            dest: Value(d),
            ty: IrType::I64,
            incoming: vec![(init, BlockId(0)), (v(a4), BlockId(4)), (v(a5), BlockId(5))],
        };
        // Two latches (4, 5), each receiving a copy of the header.
        f.blocks = vec![
            block(
                0,
                vec![param(0, 0), param(1, 1), param(2, 2), param(3, 3)],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![
                    phi3(10, k(0), 11, 11),
                    phi3(11, v(0), 13, 17),
                    phi3(12, k(0), 14, 18),
                    load(19, 3),
                    ult(15, 12, 19),
                ],
                cbr(15, 2, 3),
            ),
            block(2, vec![ult(16, 12, 2)], cbr(16, 4, 5)),
            block(
                4,
                vec![store(), gep(13, 24), inc(14)],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                5,
                vec![store(), gep(17, 8), inc(18)],
                Terminator::Branch(BlockId(1)),
            ),
            block(3, vec![], Terminator::Return(Some(v(10)))),
        ];
        let mut inputs: Vec<(Vec<i64>, Vec<(i64, i64)>)> = Vec::new();
        // The loop bound `n` is loaded from address 5000 in the header.
        for n in [0, 1, 2, 5] {
            for m in [0, 2, 9] {
                inputs.push((vec![1000, n, m, 5000], vec![(5000, n)]));
            }
        }
        // A loaded exit test is refused by the gate (plain rotation); the
        // copy itself must still be sound.
        assert_eq!(tail_duplicate_latches(&mut f.clone()), 0);
        assert_latch_copies_behave(&f, 1, &[4, 5], &inputs);
    }

    /// SHA-256's rotation (`c = b, b = a, a = t`) in a loop whose header is
    /// copied into the latch: the clone's phi copies form a parallel
    /// assignment, so `b' = a` must read the previous `a` even though `a'`
    /// is defined just before it in the clone.
    #[test]
    fn a_rotation_copied_into_its_latch_keeps_parallel_copy_semantics() {
        use crate::ir::reexports::{IrBinOp, IrConst};
        let mut f = IrFunction::new("rot".into(), IrType::I64, vec![], false);
        let param = |d: u32, i: usize| Instruction::ParamRef {
            dest: Value(d),
            param_idx: i,
            ty: IrType::I64,
        };
        let k = |x: i64| Operand::Const(IrConst::I64(x));
        let bin = |d: u32, op: IrBinOp, l: Operand, r: Operand| Instruction::BinOp {
            dest: Value(d),
            op,
            lhs: l,
            rhs: r,
            ty: IrType::I64,
        };
        let ult = |d: u32, l: u32, r: u32| Instruction::Cmp {
            dest: Value(d),
            op: IrCmpOp::Ult,
            lhs: v(l),
            rhs: v(r),
            ty: IrType::I64,
        };
        let phi3 = |d: u32, init: Operand, a4: u32, a5: u32| Instruction::Phi {
            dest: Value(d),
            ty: IrType::I64,
            incoming: vec![(init, BlockId(0)), (v(a4), BlockId(4)), (v(a5), BlockId(5))],
        };
        // a = t, b = a, c = b over two latches (4, 5) with different `t`.
        f.blocks = vec![
            block(
                0,
                vec![param(0, 0), param(1, 1), param(2, 2), param(3, 3)],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![
                    phi3(10, k(1), 20, 23),
                    phi3(11, k(2), 10, 10),
                    phi3(12, v(0), 11, 11),
                    phi3(13, k(0), 21, 24),
                    load(17, 3),
                    ult(14, 13, 17),
                ],
                cbr(14, 2, 3),
            ),
            block(2, vec![ult(16, 13, 2)], cbr(16, 4, 5)),
            block(
                4,
                vec![
                    bin(19, IrBinOp::Mul, v(12), k(3)),
                    bin(20, IrBinOp::Add, v(19), v(10)),
                    bin(21, IrBinOp::Add, v(13), k(1)),
                ],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                5,
                vec![
                    bin(22, IrBinOp::Mul, v(12), k(5)),
                    bin(23, IrBinOp::Add, v(22), v(10)),
                    bin(24, IrBinOp::Add, v(13), k(1)),
                ],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                3,
                vec![
                    bin(30, IrBinOp::Mul, v(11), k(1000)),
                    bin(31, IrBinOp::Add, v(30), v(10)),
                    bin(32, IrBinOp::Mul, v(12), k(1_000_000)),
                    bin(33, IrBinOp::Add, v(31), v(32)),
                ],
                Terminator::Return(Some(v(33))),
            ),
        ];
        let mut inputs: Vec<(Vec<i64>, Vec<(i64, i64)>)> = Vec::new();
        // The trip bound `n` is loaded from address 5000 in the header.
        for n in [0, 1, 2, 3, 7] {
            for m in [0, 2, 9] {
                inputs.push((vec![5, n, m, 5000], vec![(5000, n)]));
            }
        }
        assert_latch_copies_behave(&f, 1, &[4, 5], &inputs);
    }

    /// An induction-variable exit test: one site predicts it, and the copy
    /// would only rotate the loop, which `loop_invert` does later.
    #[test]
    fn an_induction_variable_header_is_not_duplicated() {
        let mut f = IrFunction::new("one".into(), IrType::I64, vec![], false);
        let k = |x: i64| Operand::Const(crate::ir::reexports::IrConst::I64(x));
        f.blocks = vec![
            block(
                0,
                vec![Instruction::ParamRef {
                    dest: Value(1),
                    param_idx: 0,
                    ty: IrType::I64,
                }],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![
                    Instruction::Phi {
                        dest: Value(10),
                        ty: IrType::I64,
                        incoming: vec![(k(0), BlockId(0)), (v(11), BlockId(2))],
                    },
                    Instruction::Cmp {
                        dest: Value(12),
                        op: IrCmpOp::Ult,
                        lhs: v(10),
                        rhs: v(1),
                        ty: IrType::I64,
                    },
                ],
                cbr(12, 2, 3),
            ),
            block(
                2,
                vec![Instruction::BinOp {
                    dest: Value(11),
                    op: crate::ir::reexports::IrBinOp::Add,
                    lhs: v(10),
                    rhs: k(1),
                    ty: IrType::I64,
                }],
                Terminator::Branch(BlockId(1)),
            ),
            block(3, vec![], Terminator::Return(Some(v(10)))),
        ];
        assert_eq!(tail_duplicate_latches(&mut f), 0);
        assert_eq!(f.blocks.len(), 4);
    }

    /// The tail-duplication gate on loop nests: `h` = 1, arms 2 and 3.
    #[test]
    fn only_a_choice_within_an_iteration_is_tail_duplicated() {
        let lp = |header: usize, body: &[usize]| NaturalLoop {
            header,
            body: body.iter().copied().collect(),
        };
        // Arm 3's predecessors: private to `h` = 1, or a shared exit join.
        let private: Vec<Vec<usize>> = vec![vec![], vec![0], vec![1], vec![1], vec![2], vec![]];
        let mut shared = private.clone();
        shared[3] = vec![1, 2];
        // `h` heads the loop holding both arms.
        assert!(branch_chooses_within_iteration(
            &[lp(1, &[1, 2, 3])],
            &shared,
            1,
            2,
            3
        ));
        // The list merge: arm 3 returns via the enclosing test 0, whose
        // loop's only child loop holds `h`.
        let merge = [lp(0, &[0, 1, 2, 3, 4]), lp(1, &[1, 2, 4])];
        assert!(branch_chooses_within_iteration(&merge, &private, 1, 2, 3));
        // ... but not when arm 3 is a join shared with another loop exit.
        assert!(!branch_chooses_within_iteration(&merge, &shared, 1, 2, 3));
        // ... also with a loop nested inside `h`'s own.
        let nested = [
            lp(0, &[0, 1, 2, 3, 4, 5]),
            lp(1, &[1, 2, 4, 5]),
            lp(5, &[5]),
        ];
        assert!(branch_chooses_within_iteration(&nested, &private, 1, 2, 3));
        // fannkuch: arm 3 exits `h`'s loop into an outer loop that also runs
        // a sibling loop (6) before coming back.
        let exit = [lp(0, &[0, 1, 2, 3, 4, 6]), lp(1, &[1, 2, 4]), lp(6, &[6])];
        assert!(!branch_chooses_within_iteration(&exit, &private, 1, 2, 3));
        // An exit out of every loop.
        assert!(!branch_chooses_within_iteration(
            &[lp(1, &[1, 2])],
            &private,
            1,
            2,
            3
        ));
    }

    #[test]
    fn comparison_facts_imply_negated_swapped_and_weaker_forms() {
        use IrCmpOp::*;
        let k = |x: i64| Operand::Const(crate::ir::reexports::IrConst::I64(x));
        let (a, b) = (v(1), v(2));
        let t = IrType::I64;
        // j >= 0 known true: j < 0 is false, 0 <= j... is `0 Sle j` true.
        assert_eq!(
            implied_cmp((Sge, a, k(0), t), true, (Slt, a, k(0), t)),
            Some(false)
        );
        assert_eq!(
            implied_cmp((Sge, a, k(0), t), true, (Sle, k(0), a, t)),
            Some(true)
        );
        // a < b false: a >= b, so b <= a true and a == b undecided.
        assert_eq!(
            implied_cmp((Slt, a, b, t), false, (Sle, b, a, t)),
            Some(true)
        );
        assert_eq!(implied_cmp((Slt, a, b, t), false, (Eq, a, b, t)), None);
        // Strict implies non-strict and inequality.
        assert_eq!(
            implied_cmp((Ult, a, b, t), true, (Ule, a, b, t)),
            Some(true)
        );
        assert_eq!(
            implied_cmp((Ult, a, b, t), true, (Eq, a, b, t)),
            Some(false)
        );
        assert_eq!(
            implied_cmp((Eq, a, b, t), true, (Ugt, b, a, t)),
            Some(false)
        );
        // Signedness is part of the predicate: Slt says nothing about Ult.
        assert_eq!(implied_cmp((Slt, a, b, t), true, (Ult, a, b, t)), None);
        // Different compare widths or operands decide nothing.
        assert_eq!(
            implied_cmp((Slt, a, b, IrType::I32), true, (Slt, a, b, t)),
            None
        );
        assert_eq!(implied_cmp((Slt, a, b, t), true, (Slt, a, v(3), t)), None);
    }

    /// `while (j >= 0 && a[j] == b[j]) j--; if (j < 0) ...`: the mismatch
    /// exit leaves the body where `j >= 0` holds, so its `j < 0` re-test is
    /// decided and the edge goes straight to the continuation.
    #[test]
    fn an_implied_retest_on_the_mismatch_exit_is_threaded() {
        use crate::ir::reexports::{IrBinOp, IrConst};
        let mut f = IrFunction::new("scan".into(), IrType::I64, vec![], false);
        let k = |x: i64| Operand::Const(IrConst::I64(x));
        let param = |d: u32, i: usize| Instruction::ParamRef {
            dest: Value(d),
            param_idx: i,
            ty: IrType::I64,
        };
        let cmp = |d: u32, op: IrCmpOp, l: Operand, r: Operand| Instruction::Cmp {
            dest: Value(d),
            op,
            lhs: l,
            rhs: r,
            ty: IrType::I64,
        };
        let gep = |d: u32, base: u32| Instruction::GetElementPtr {
            dest: Value(d),
            base: Value(base),
            offset: v(10),
            ty: IrType::Ptr,
        };
        f.blocks = vec![
            block(
                0,
                vec![param(0, 0), param(1, 1), param(2, 2)],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![
                    Instruction::Phi {
                        dest: Value(10),
                        ty: IrType::I64,
                        incoming: vec![(v(0), BlockId(0)), (v(16), BlockId(3))],
                    },
                    cmp(11, IrCmpOp::Sge, v(10), k(0)),
                ],
                cbr(11, 2, 4),
            ),
            block(
                2,
                vec![
                    gep(12, 1),
                    load(13, 12),
                    gep(14, 2),
                    load(15, 14),
                    cmp(17, IrCmpOp::Eq, v(13), v(15)),
                ],
                cbr(17, 3, 4),
            ),
            block(
                3,
                vec![Instruction::BinOp {
                    dest: Value(16),
                    op: IrBinOp::Sub,
                    lhs: v(10),
                    rhs: k(1),
                    ty: IrType::I64,
                }],
                Terminator::Branch(BlockId(1)),
            ),
            block(4, vec![cmp(18, IrCmpOp::Slt, v(10), k(0))], cbr(18, 5, 6)),
            block(5, vec![], Terminator::Return(Some(k(-1)))),
            block(6, vec![], Terminator::Return(Some(v(10)))),
        ];
        let mut g = f.clone();
        assert!(thread_decided_branches(&mut g) > 0);
        let compare = block_of(&g, 2).expect("compare block");
        assert!(
            matches!(compare.terminator, Terminator::CondBranch { false_label, .. } if false_label != BlockId(4)),
            "the mismatch edge must bypass the re-test"
        );
        let header = block_of(&g, 1).expect("header");
        assert!(
            matches!(header.terminator, Terminator::CondBranch { false_label, .. } if false_label != BlockId(4)),
            "the exit edge (j < 0 known) must bypass the re-test"
        );
        // a = 100.., b = 200..; mismatches at every position, and none.
        let mut inputs: Vec<(Vec<i64>, Vec<(i64, i64)>)> = Vec::new();
        for j0 in [-1, 0, 1, 3] {
            for miss in [-1, 0, 1, 2, 3] {
                let mut mem = Vec::new();
                for i in 0..4 {
                    mem.push((100 + i, 7 * i));
                    mem.push((200 + i, if i == miss { 99 } else { 7 * i }));
                }
                inputs.push((vec![j0, 100, 200], mem));
            }
        }
        assert_same_behaviour(&f, &inputs);
    }

    fn block_of(f: &IrFunction, label: u32) -> Option<&BasicBlock> {
        f.blocks.iter().find(|b| b.label == BlockId(label))
    }

    /// Conditional branches an arm executes before reaching a copy of the
    /// compare (the block branching on v2), following true edges.
    fn tests_until_compare(f: &IrFunction, from: u32) -> Option<usize> {
        let labels = analysis::build_label_map(f);
        let mut cur = *labels.get(&BlockId(from))?;
        let mut tests = 0;
        for _ in 0..16 {
            match &f.blocks[cur].terminator {
                Terminator::CondBranch {
                    cond: Operand::Value(Value(2)),
                    ..
                } if cur != *labels.get(&BlockId(from))? => return Some(tests),
                Terminator::Branch(t) => cur = *labels.get(t)?,
                Terminator::CondBranch { true_label, .. } => {
                    tests += 1;
                    cur = *labels.get(true_label)?;
                }
                _ => return None,
            }
        }
        None
    }

    #[test]
    fn each_merge_arm_tests_only_the_pointer_it_advanced() {
        let mut f = merge_loop();
        assert_eq!(tests_until_compare(&f, 4), Some(2));
        assert_eq!(tests_until_compare(&f, 5), Some(2));
        let n = run(&mut f);
        assert!(n >= 2, "both arms thread, got {n}");
        assert_eq!(tests_until_compare(&f, 4), Some(1));
        assert_eq!(tests_until_compare(&f, 5), Some(1));
    }

    #[test]
    fn each_arm_gets_its_own_copy_of_the_compare() {
        let mut f = flagged_merge_loop();
        run(&mut f);
        // The compare branches on a flag its block loads; count those sites.
        let sites = f
            .blocks
            .iter()
            .filter(|b| {
                let Terminator::CondBranch {
                    cond: Operand::Value(c),
                    ..
                } = &b.terminator
                else {
                    return false;
                };
                b.instructions
                    .iter()
                    .any(|i| matches!(i, Instruction::Load { dest, .. } if dest == c))
            })
            .count();
        assert!(
            sites >= 2,
            "the arms do not share one compare site ({sites})"
        );
    }

    #[test]
    fn every_use_reads_a_dominating_definition_after_repair() {
        let mut f = merge_loop();
        run(&mut f);
        let cfg = Cfg::build(&f);
        let mut def_at: FxHashMap<u32, usize> = FxHashMap::default();
        for (bi, b) in f.blocks.iter().enumerate() {
            for inst in &b.instructions {
                if let Some(d) = inst.dest() {
                    assert!(def_at.insert(d.0, bi).is_none(), "v{} defined twice", d.0);
                }
            }
        }
        for (bi, b) in f.blocks.iter().enumerate() {
            let check = |user: usize, u: u32| {
                if let Some(&d) = def_at.get(&u) {
                    assert!(
                        d == user || analysis::dominates_idom(&cfg.idom, d, user),
                        "v{u} (block {d}) does not dominate its use in block {user}"
                    );
                }
            };
            for inst in &b.instructions {
                if let Instruction::Phi { incoming, .. } = inst {
                    for (op, from) in incoming {
                        if let Operand::Value(x) = op {
                            check(cfg.labels[from], x.0);
                        }
                    }
                } else {
                    inst.for_each_used_value(|u| check(bi, u));
                }
            }
            b.terminator.for_each_used_value(|u| check(bi, u));
        }
    }

    #[test]
    fn unknown_incoming_values_are_not_threaded() {
        // Without the dominating test on b (T2 removed), PA's incoming b is
        // unknown; and a is never known on PA's path.
        let mut f = merge_loop();
        f.blocks[1].terminator = cbr(10, 3, 7);
        f.blocks.remove(2);
        let n = thread_decided_branches(&mut f);
        // Only PB -> J -> H (a known non-null) threads.
        assert_eq!(n, 1);
        assert_eq!(tests_until_compare(&f, 4), Some(1));
        assert_eq!(tests_until_compare(&f, 5), Some(0));
    }

    #[test]
    fn calls_on_the_path_and_oversized_paths_are_refused() {
        let mut f = merge_loop();
        block_index_push(
            &mut f,
            6,
            Instruction::Call {
                func: "g".into(),
                info: Default::default(),
            },
        );
        run(&mut f);
        assert_eq!(tests_until_compare(&f, 4), Some(2));
        assert_eq!(tests_until_compare(&f, 5), Some(2));
        let mut f = merge_loop();
        for k in 0..=MAX_PATH_INSTS as u32 {
            block_index_push(
                &mut f,
                6,
                Instruction::Copy {
                    dest: Value(100 + k),
                    src: v(10),
                },
            );
        }
        run(&mut f);
        assert_eq!(tests_until_compare(&f, 4), Some(2));
        assert_eq!(tests_until_compare(&f, 5), Some(2));
    }

    fn block_index_push(f: &mut IrFunction, label: u32, inst: Instruction) {
        if let Some(b) = f.blocks.iter_mut().find(|b| b.label == BlockId(label)) {
            b.instructions.push(inst);
        }
    }

    #[test]
    fn constant_incoming_values_decide_the_branch() {
        // B0 -> M2 with flag 1, B1 -> M2 with v5; M2 branches on the phi.
        let mut f = IrFunction::new("k".into(), IrType::I32, vec![], false);
        f.blocks = vec![
            block(
                0,
                vec![Instruction::ParamRef {
                    dest: Value(5),
                    param_idx: 0,
                    ty: IrType::I32,
                }],
                cbr(5, 1, 2),
            ),
            block(1, vec![], Terminator::Branch(BlockId(2))),
            block(
                2,
                vec![Instruction::Phi {
                    dest: Value(6),
                    ty: IrType::I32,
                    incoming: vec![
                        (
                            Operand::Const(crate::ir::reexports::IrConst::I32(0)),
                            BlockId(0),
                        ),
                        (v(5), BlockId(1)),
                    ],
                }],
                cbr(6, 3, 4),
            ),
            block(3, vec![], Terminator::Return(Some(v(5)))),
            block(4, vec![], Terminator::Return(None)),
        ];
        assert!(run(&mut f) >= 1);
        let b0 = &f.blocks[0];
        // B0's false edge (flag 0) now reaches B4 without testing the phi.
        let Terminator::CondBranch { false_label, .. } = b0.terminator else {
            panic!("B0 keeps its own test");
        };
        let labels = analysis::build_label_map(&f);
        let tgt = &f.blocks[labels[&false_label]];
        assert!(matches!(tgt.terminator, Terminator::Branch(BlockId(4))));
        // Both predecessors threaded: the merge block is gone.
        assert!(block_of(&f, 2).is_none());
    }

    /// `for (i = 0; i < n; i++) { if (i == 0) A else B; }` with the header
    /// testing `i == 0` (exit edge `exit_on_zero`): blocks 0 entry, 1 header,
    /// 2/3 the arms, 4 latch (`i + 1 < n`), 5 exit.
    fn entry_decided_loop(exit_on_zero: bool) -> IrFunction {
        use crate::ir::reexports::{IrBinOp, IrConst};
        let mut f = IrFunction::new("entry".into(), IrType::I64, vec![], false);
        let k = |x: i64| Operand::Const(IrConst::I64(x));
        let (t, e) = if exit_on_zero { (5, 2) } else { (2, 3) };
        f.blocks = vec![
            block(
                0,
                vec![Instruction::ParamRef {
                    dest: Value(1),
                    param_idx: 0,
                    ty: IrType::I64,
                }],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![
                    Instruction::Phi {
                        dest: Value(10),
                        ty: IrType::I64,
                        incoming: vec![(k(0), BlockId(0)), (v(11), BlockId(4))],
                    },
                    Instruction::Cmp {
                        dest: Value(12),
                        op: IrCmpOp::Eq,
                        lhs: v(10),
                        rhs: k(0),
                        ty: IrType::I64,
                    },
                ],
                cbr(12, t, e),
            ),
            block(2, vec![], Terminator::Branch(BlockId(4))),
            block(3, vec![], Terminator::Branch(BlockId(4))),
            block(
                4,
                vec![
                    Instruction::BinOp {
                        dest: Value(11),
                        op: IrBinOp::Add,
                        lhs: v(10),
                        rhs: k(1),
                        ty: IrType::I64,
                    },
                    Instruction::Cmp {
                        dest: Value(13),
                        op: IrCmpOp::Ult,
                        lhs: v(11),
                        rhs: v(1),
                        ty: IrType::I64,
                    },
                ],
                cbr(13, 1, 5),
            ),
            block(5, vec![], Terminator::Return(Some(v(10)))),
        ];
        f
    }

    /// The entry edge knows `i == 0`, but its decided successor is inside the
    /// loop: threading it would save one branch per entry and leave merge
    /// phis for `i` inside the loop, paid every iteration.
    #[test]
    fn an_entry_decided_test_is_not_threaded_into_the_loop_body() {
        let mut f = entry_decided_loop(false);
        assert_eq!(thread_decided_branches(&mut f), 0);
        assert_eq!(f.blocks.len(), 6);
        let phis = f
            .blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .filter(|i| matches!(i, Instruction::Phi { .. }))
            .count();
        assert_eq!(phis, 1);
    }

    /// When the decided successor leaves the loop the thread bypasses it
    /// whole (the merge phis sit at the exit), so it is still taken.
    #[test]
    fn an_entry_decided_bypass_of_the_loop_is_threaded() {
        let f = entry_decided_loop(true);
        assert_same_behaviour(
            &f,
            &[(vec![0], vec![]), (vec![1], vec![]), (vec![5], vec![])],
        );
    }
}
