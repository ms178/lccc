//! Latency-driven reassociation of associative integer trees in loops.
//!
//! A loop is a set of recurrences: its speed is bounded by the longest
//! loop-carried dependence chain, not by its instruction count.  Source code
//! (and every earlier pass) builds sums in source order, so a chain such as
//! SHA-256's
//!
//! ```text
//!     t1 = h + Σ1(e) + Ch(e, f, g) + K[i] + W[i]      // e' = d + t1
//! ```
//!
//! reaches the backend as `((((h + Σ1) + Ch) + K[i]) + W[i])`: after the
//! loop-carried `e` is known, Σ1 (3 ops), then four serial adds, then `d +`,
//! lie on the recurrence, although `h + K[i] + W[i]` does not depend on this
//! round's `e` at all (the loads are addressed by the induction variable,
//! which an out-of-order core runs iterations ahead).  Combining the
//! operands in order of availability instead,
//!
//! ```text
//!     t1 = ((K[i] + W[i]) + h) + Ch  ...  + Σ1        // Σ1 last
//! ```
//!
//! shortens the e→e recurrence from 8 to 5 operations; llvm-mca 19 measures
//! the whole round loop at 13.35 → 8.04 cycles (znver4) and 15.01 → 9.03
//! (raptorlake), ahead of GCC 16.2 (9.69 / 12.01) and Clang 23.1 (12.03 /
//! 14.02) at `-O2`.
//!
//! # Model
//!
//! Every value of a block `B` gets an availability pair `(r, l)` compared
//! lexicographically:
//!
//! * `r`: time since the start of the iteration along loop-carried
//!   dependences of `B`'s innermost natural loop `L`.  Header phis of `L`
//!   that carry a value around the loop and are not induction variables
//!   start at 0; values defined outside `L` (invariants), induction
//!   variables, constants and everything computed only from them are
//!   *free* (`None`, earlier than any time).  Outside loops every value is
//!   free, so only `l` matters there — but see *Scope*.
//! * `l`: plain dependence depth from the start of `B` (values defined
//!   elsewhere are 0).  It orders the free operands (a load must not be
//!   added before an argument that is ready) and breaks ties.
//!
//! Latencies are microarchitecture-neutral and deliberately coarse: 1 for
//! integer ALU ops (add/logic/shift/rotate — `rorx`/`rol` included), 3 for
//! multiply, 5 for a load, 0 for copies and integer narrowing, and 0 for a
//! bitwise `Not` all of whose uses are `And`s (BMI1 `andn` absorbs it — the
//! default x86-64-v3 target — so SHA's `Ch(e,f,g) = (e&f) ^ (~e&g)` is 2
//! deep from `e`, not 3, and is added before the 3-deep Σ1; without BMI1
//! both are 3 deep and their order does not change the bound).
//!
//! ## Rotation lags
//!
//! SHA-256-style state rotation (`h = g; g = f; f = e; e = d + t1`) makes
//! some carried phis pure copies of other carried phis.  Such a phi holds a
//! value computed `lag` iterations ago (h: 3, g: 2, f: 1, e: 0), so it is
//! ready long before this iteration's computed words: it starts at
//! `r = -lag`.  Timing every carried phi at 0 instead made the pass combine
//! `((e + f) + g) + (k + h)` in the rotation loop of
//! `tests/regression/check_phi_acyclic_order.sh` -- the recurrence did not
//! get shorter, but the folded `add (mem), reg` was lost (55 -> 56
//! instructions, one more than GCC 13.2; that is the PR #638 CI failure).
//! Lags compose along copy chains; a phi on a pure copy cycle (a genuine
//! swap) has no computed source and keeps lag 0.
//!
//! # Transformation
//!
//! A *tree* is a maximal set of `BinOp`s with the same associative,
//! commutative operator (`Add`, `And`, `Or`, `Xor`) and the same integer
//! type in one block, where every non-root node has exactly one use, by
//! another node of the tree.  Its leaves are rebuilt greedily: repeatedly
//! combine the two earliest-available operands (the Huffman construction
//! with `max` for `+`), which minimises the finish time of the tree for
//! unit-latency operators.  Free operands (off the recurrence) are first
//! folded in source order, which keeps the source's register pressure; the
//! greedy then orders the partial sum and the carried operands.  A tree is
//! rewritten only if it has a carried operand and the new root's recurrence
//! time is *strictly* earlier; the pass is therefore idempotent and leaves
//! every tree it cannot improve byte-identical.
//!
//! New nodes are placed right after the later of their operands (never
//! before the first node of the original tree), and the root never moves
//! later.  A leaf's pure, single-use expression chain (Σ1's rotates and
//! xors, Ch's and/andn/xor) is sunk to just before the node that now
//! consumes it: the reassociated order consumes the late operands last, so
//! leaving their definitions at their source positions would keep them live
//! across the early part of the tree (measured: one GPR spill in the
//! SHA-256 round loop).  Sinking a side-effect-free instruction within its
//! block to just before its only user is always legal; loads, calls and
//! trapping divisions never move.
//!
//! # Soundness
//!
//! Integer `Add` in this IR is two's-complement wrapping for every integer
//! type (the IR has no no-signed-wrap flags), so any association of a sum
//! yields the same bits; `And`/`Or`/`Xor` are associative and commutative
//! outright.  Signed-overflow reasoning (IV widening, CVP, SCCP) runs
//! strictly *before* this pass — it is the last scalar IR transform — so no
//! later analysis can observe an intermediate value that the source did not
//! compute.  Pointer arithmetic (GEP) is not touched.
//!
//! # Scope
//!
//! Only trees on a loop-carried recurrence are rewritten: latency matters
//! where it repeats serially.  Straight-line code and off-recurrence trees
//! in loops keep their source shape — the out-of-order core already
//! overlaps independent iterations, and reshaping them for depth only costs
//! registers (a 9-product 3x3 convolution sum balanced by depth spilled:
//! +24% dynamic instructions before this rule).
//!
//! Each node puts a single-use load leaf second, the operand position x86
//! folds into `add mem, reg`.  Rewrites repeat to a fixpoint (at most
//! `MAX_ROUNDS`): once `t1` is earlier, `a' = t1 + t2` may be worth
//! re-planning.  Every accepted rewrite strictly lowers its root's
//! recurrence time, so the rounds terminate on their own.
//!
//! # Register residency
//!
//! The model times a *register-resident* recurrence.  When the values live
//! on the loop's recurrences do not fit the allocatable GPRs, the
//! recurrence runs through stack slots and store-to-load forwarding the
//! model cannot see, and a reshaped tree moves spill traffic onto it:
//! i686 SHA-256 (8 rotating words, 6 GPRs) measured 1.058 [1.034, 1.070]
//! of the source shape.  A tree is therefore left alone when the live
//! recurrent values (exact SSA liveness restricted to the loop's tainted
//! values, weighted by the GPRs each occupies) exceed the GPR budget at
//! *every* point of its span: somewhere in the span the recurrence must be
//! partly memory-resident.  Measured separation (recurrent / budget):
//! i686 SHA 8/6 and the rotation loop 8/6 lose; i686 bitops 6/6, x86-64
//! lz4_compress 4/13 and SHA 8/13 gain.  Block peak pressure, the integral
//! of excess pressure, and total (not just recurrent) span pressure were
//! tried and each misclassified a measured case.
//!
//! Pass name for CCC_DISABLE_PASSES: "reassoc_lat".

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::analysis;
use crate::ir::reexports::{Instruction, IrBinOp, IrFunction, Operand, Value};
use crate::passes::loop_analysis;

/// Largest tree the pass rebuilds.  The greedy is quadratic in the leaf
/// count; real associative chains have a handful of leaves.
const MAX_LEAVES: usize = 64;

/// Fixpoint bound.  Each accepted rewrite strictly lowers a root's
/// recurrence time, so this only caps pathological inputs.
const MAX_ROUNDS: usize = 8;

// ELF machines (local, as in backedge_pre: passes do not depend on the
// backend crate modules).
const EM_386: u16 = 3;
const EM_AARCH64: u16 = 183;
const EM_RISCV: u16 = 243;

/// Allocatable general-purpose registers of the target, excluding the
/// stack/frame pointers and the reserved scratch registers.
fn gpr_budget() -> u32 {
    match crate::common::types::target_elf_machine() {
        EM_386 => 6,
        EM_AARCH64 | EM_RISCV => 26,
        _ => 13,
    }
}

/// GPRs a value of `ty` occupies (floating point: none).
fn gpr_weight(ty: Option<IrType>) -> u32 {
    let word = match crate::common::types::target_ptr_size() {
        0 => 8,
        w => w,
    };
    match ty {
        Some(t) if t.is_float() || t == IrType::Void => 0,
        Some(t) => t.size().div_ceil(word).max(1) as u32,
        None => 1,
    }
}

/// Availability of a value: `r` is `None` when the value is off every
/// loop-carried chain (free), `l` the local depth.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct Avail {
    r: Option<i32>,
    l: u32,
}

impl Avail {
    const FREE: Avail = Avail { r: None, l: 0 };

    fn after(self, other: Avail, lat: u32) -> Avail {
        Avail {
            r: self.r.max(other.r).map(|r| r + lat as i32),
            l: self.l.max(other.l) + lat,
        }
    }
}

fn is_reassociable(op: IrBinOp, ty: IrType) -> bool {
    matches!(op, IrBinOp::Add | IrBinOp::And | IrBinOp::Or | IrBinOp::Xor)
        && matches!(ty, IrType::I32 | IrType::U32 | IrType::I64 | IrType::U64)
}

fn is_int(ty: IrType) -> bool {
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
    )
}

/// Coarse, microarchitecture-neutral result latency of `inst`.
fn latency(inst: &Instruction) -> u32 {
    match inst {
        Instruction::BinOp { op, .. } => match op {
            IrBinOp::Mul => 3,
            IrBinOp::SDiv | IrBinOp::UDiv | IrBinOp::SRem | IrBinOp::URem => 20,
            _ => 1,
        },
        Instruction::Load { .. } => 5,
        Instruction::Copy { .. } | Instruction::Phi { .. } => 0,
        Instruction::Cast { from_ty, to_ty, .. }
            if is_int(*from_ty) && is_int(*to_ty) && to_ty.size() <= from_ty.size() =>
        {
            0
        }
        _ => 1,
    }
}

/// Header phis of `lp` that carry a value around the loop and are not
/// induction variables (`p' = p ± invariant`, `p' = gep p, invariant`).
fn recurrent_phis(
    func: &IrFunction,
    lp: &loop_analysis::NaturalLoop,
    def_block: &FxHashMap<u32, usize>,
    label_to_idx: &FxHashMap<crate::ir::reexports::BlockId, usize>,
) -> FxHashSet<u32> {
    let invariant = |op: &Operand| match op {
        Operand::Const(_) => true,
        Operand::Value(v) => def_block.get(&v.0).is_none_or(|b| !lp.body.contains(b)),
    };
    let defs: FxHashMap<u32, &Instruction> = lp
        .body
        .iter()
        .flat_map(|&b| func.blocks[b].instructions.iter())
        .filter_map(|i| i.dest().map(|d| (d.0, i)))
        .collect();
    let mut out = FxHashSet::default();
    for inst in &func.blocks[lp.header].instructions {
        let Instruction::Phi { dest, incoming, .. } = inst else {
            continue;
        };
        let mut carried = false;
        let mut is_iv = true;
        for (op, pred) in incoming {
            if !label_to_idx.get(pred).is_some_and(|p| lp.body.contains(p)) {
                continue;
            }
            carried = true;
            let step_of_self = match op {
                Operand::Value(v) => match defs.get(&v.0) {
                    Some(Instruction::BinOp {
                        op: IrBinOp::Add | IrBinOp::Sub,
                        lhs: Operand::Value(l),
                        rhs,
                        ..
                    }) if l.0 == dest.0 && invariant(rhs) => true,
                    Some(Instruction::BinOp {
                        op: IrBinOp::Add,
                        lhs,
                        rhs: Operand::Value(r),
                        ..
                    }) if r.0 == dest.0 && invariant(lhs) => true,
                    Some(Instruction::GetElementPtr { base, offset, .. }) => {
                        base.0 == dest.0 && invariant(offset)
                    }
                    _ => false,
                },
                Operand::Const(_) => false,
            };
            is_iv &= step_of_self;
        }
        if carried && !is_iv {
            out.insert(dest.0);
        }
    }
    out
}

/// Values of `lp` that depend on one of its recurrent phis (the only values
/// whose availability is measured against the recurrence).
fn tainted_values(
    func: &IrFunction,
    lp: &loop_analysis::NaturalLoop,
    seeds: &FxHashSet<u32>,
) -> FxHashSet<u32> {
    let mut tainted = seeds.clone();
    let mut blocks: Vec<usize> = lp.body.iter().copied().collect();
    blocks.sort_unstable();
    loop {
        let before = tainted.len();
        for &b in &blocks {
            for inst in &func.blocks[b].instructions {
                let Some(d) = inst.dest() else { continue };
                if tainted.contains(&d.0) {
                    continue;
                }
                let mut hit = false;
                inst.for_each_used_value(|v| hit |= tainted.contains(&v));
                if hit {
                    tainted.insert(d.0);
                }
            }
        }
        if tainted.len() == before {
            return tainted;
        }
    }
}

/// Rotation lag of each recurrent phi that is a pure copy of other
/// recurrent phis around the back edge(s): `lag(p) = 1 + min lag(q)` over
/// its in-loop incomings `q` (the latest source bounds availability), 0 for
/// a phi fed by a computed value.  Phis on a pure copy cycle keep 0.  Only
/// positive lags are returned.
fn rotation_lags(
    func: &IrFunction,
    lp: &loop_analysis::NaturalLoop,
    rec: &FxHashSet<u32>,
    label_to_idx: &FxHashMap<crate::ir::reexports::BlockId, usize>,
) -> FxHashMap<u32, i32> {
    // For each recurrent phi: its in-loop sources if they are all recurrent
    // phis, None otherwise.
    let mut srcs: FxHashMap<u32, Option<Vec<u32>>> = FxHashMap::default();
    for inst in &func.blocks[lp.header].instructions {
        let Instruction::Phi { dest, incoming, .. } = inst else {
            continue;
        };
        if !rec.contains(&dest.0) {
            continue;
        }
        let mut all: Option<Vec<u32>> = Some(Vec::new());
        for (op, pred) in incoming {
            if !label_to_idx.get(pred).is_some_and(|p| lp.body.contains(p)) {
                continue;
            }
            match (op, all.as_mut()) {
                (Operand::Value(v), Some(list)) if rec.contains(&v.0) && v.0 != dest.0 => {
                    list.push(v.0)
                }
                _ => all = None,
            }
        }
        srcs.insert(dest.0, all.filter(|l| !l.is_empty()));
    }
    // Forward propagation from the computed sources: a phi whose sources
    // all have a lag gets 1 + their minimum.  Monotone (each phi is
    // assigned once), so it ends after at most |phis| sweeps; phis left
    // unassigned sit on (or are fed only by) pure copy cycles: lag 0.
    let mut memo: FxHashMap<u32, i32> = srcs
        .iter()
        .filter(|(_, s)| s.is_none())
        .map(|(&p, _)| (p, 0))
        .collect();
    let mut keys: Vec<u32> = srcs.keys().copied().collect();
    keys.sort_unstable();
    loop {
        let mut progress = false;
        for &p in &keys {
            if memo.contains_key(&p) {
                continue;
            }
            let Some(Some(qs)) = srcs.get(&p) else {
                continue;
            };
            let lags: Option<Vec<i32>> = qs.iter().map(|q| memo.get(q).copied()).collect();
            if let Some(min) = lags.and_then(|l| l.into_iter().min()) {
                memo.insert(p, min + 1);
                progress = true;
            }
        }
        if !progress {
            break;
        }
    }
    memo.retain(|_, l| *l > 0);
    memo
}

/// Live-out sets of every block, restricted to the values in `track`
/// (exact SSA liveness: a phi operand is live out of its predecessor only).
fn live_out_restricted(
    func: &IrFunction,
    succs: &analysis::FlatAdj,
    label_to_idx: &FxHashMap<crate::ir::reexports::BlockId, usize>,
    track: &FxHashSet<u32>,
) -> Vec<FxHashSet<u32>> {
    let n = func.blocks.len();
    let mut upward: Vec<FxHashSet<u32>> = vec![FxHashSet::default(); n];
    let mut defs: Vec<FxHashSet<u32>> = vec![FxHashSet::default(); n];
    // phi_out[b]: tracked phi operands flowing out of b along its edges.
    let mut phi_out: Vec<FxHashSet<u32>> = vec![FxHashSet::default(); n];
    for (bi, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            if let Instruction::Phi { incoming, .. } = inst {
                for (op, pred) in incoming {
                    if let (Operand::Value(v), Some(&p)) = (op, label_to_idx.get(pred))
                        && track.contains(&v.0)
                    {
                        phi_out[p].insert(v.0);
                    }
                }
            } else {
                inst.for_each_used_value(|v| {
                    if track.contains(&v) && !defs[bi].contains(&v) {
                        upward[bi].insert(v);
                    }
                });
            }
            if let Some(d) = inst.dest()
                && track.contains(&d.0)
            {
                defs[bi].insert(d.0);
            }
        }
        block.terminator.for_each_used_value(|v| {
            if track.contains(&v) && !defs[bi].contains(&v) {
                upward[bi].insert(v);
            }
        });
    }
    let mut live_in: Vec<FxHashSet<u32>> = upward.clone();
    let mut live_out: Vec<FxHashSet<u32>> = vec![FxHashSet::default(); n];
    // Both families of sets only grow (unions of growing sets), so a size
    // change is exactly a content change.
    let mut changed = true;
    while changed {
        changed = false;
        for bi in (0..n).rev() {
            let mut out = phi_out[bi].clone();
            for &s in succs.row(bi) {
                out.extend(live_in[s as usize].iter().copied());
            }
            let mut inn = upward[bi].clone();
            inn.extend(out.iter().filter(|v| !defs[bi].contains(v)).copied());
            if out.len() != live_out[bi].len() {
                live_out[bi] = out;
                changed = true;
            }
            if inn.len() != live_in[bi].len() {
                live_in[bi] = inn;
                changed = true;
            }
        }
    }
    live_out
}

/// Per-loop facts shared by the loop's blocks.
struct LoopFacts {
    tainted: FxHashSet<u32>,
    lags: FxHashMap<u32, i32>,
    live_out: Vec<FxHashSet<u32>>,
}

/// Run the pass over one function to a fixpoint.  Returns the number of
/// trees rewritten.
pub(crate) fn run_function(func: &mut IrFunction) -> usize {
    let mut total = 0;
    for _ in 0..MAX_ROUNDS {
        let r = run_round(func);
        total += r;
        if r == 0 {
            break;
        }
    }
    total
}

fn run_round(func: &mut IrFunction) -> usize {
    let n = func.blocks.len();
    if n == 0 {
        return 0;
    }
    let label_to_idx = analysis::build_label_map(func);
    let (preds, succs) = analysis::build_cfg(func, &label_to_idx);
    let idom = analysis::compute_dominators(n, &preds, &succs);
    let loops = loop_analysis::find_natural_loops(n, &preds, &succs, &idom);
    if loops.is_empty() {
        return 0;
    }
    // Innermost loop of each block: the smallest body containing it.
    let mut innermost: Vec<Option<usize>> = vec![None; n];
    for (li, lp) in loops.iter().enumerate() {
        for &b in &lp.body {
            if innermost[b].is_none_or(|cur| loops[cur].body.len() > lp.body.len()) {
                innermost[b] = Some(li);
            }
        }
    }

    // Function-wide use counts (instructions and terminators) and def sites.
    let mut uses: FxHashMap<u32, u32> = FxHashMap::default();
    let mut def_block: FxHashMap<u32, usize> = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            inst.for_each_used_value(|v| *uses.entry(v).or_insert(0) += 1);
            if let Some(d) = inst.dest() {
                def_block.insert(d.0, bi);
            }
        }
        block
            .terminator
            .for_each_used_value(|v| *uses.entry(v).or_insert(0) += 1);
    }

    // Value types for the residency weights (a Copy takes its source's).
    let mut types: FxHashMap<u32, IrType> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some(d) = inst.dest() {
                if let Some(t) = inst.result_type() {
                    types.insert(d.0, t);
                } else if let Instruction::Copy {
                    src: Operand::Value(s),
                    ..
                } = inst
                    && let Some(&t) = types.get(&s.0)
                {
                    types.insert(d.0, t);
                }
            }
        }
    }
    let mut next_id = func.sound_next_value_id();
    let mut facts_cache: FxHashMap<usize, LoopFacts> = FxHashMap::default();
    let mut rewrites = 0usize;
    for bi in 0..n {
        let Some(li) = innermost[bi] else { continue };
        if !func.blocks[bi]
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::BinOp { op, ty, .. } if is_reassociable(*op, *ty)))
        {
            continue;
        }
        let facts = facts_cache.entry(li).or_insert_with(|| {
            let seeds = recurrent_phis(func, &loops[li], &def_block, &label_to_idx);
            let lags = rotation_lags(func, &loops[li], &seeds, &label_to_idx);
            let tainted = tainted_values(func, &loops[li], &seeds);
            let live_out = live_out_restricted(func, &succs, &label_to_idx, &tainted);
            LoopFacts {
                tainted,
                lags,
                live_out,
            }
        });
        let in_loop = |v: u32| {
            def_block
                .get(&v)
                .is_some_and(|b| loops[li].body.contains(b))
        };
        let weight = |v: u32| gpr_weight(types.get(&v).copied());
        rewrites += rewrite_block(func, bi, facts, &in_loop, &weight, &uses, &mut next_id);
    }
    if rewrites > 0 {
        func.next_value_id = next_id;
    }
    rewrites
}

/// One reassociation tree: its nodes (instruction indices, root last) and
/// its leaves in left-to-right order.
struct Tree {
    nodes: Vec<usize>,
    leaves: Vec<Operand>,
    op: IrBinOp,
    ty: IrType,
    root_dest: Value,
}

fn rewrite_block(
    func: &mut IrFunction,
    bi: usize,
    facts: &LoopFacts,
    in_loop: &dyn Fn(u32) -> bool,
    weight: &dyn Fn(u32) -> u32,
    uses: &FxHashMap<u32, u32>,
    next_id: &mut u32,
) -> usize {
    let tainted = &facts.tainted;
    let carried_start = |v: u32| Avail {
        r: Some(-facts.lags.get(&v).copied().unwrap_or(0)),
        l: 0,
    };
    let insts = &func.blocks[bi].instructions;
    // Local def index and availability of every value defined in the block.
    let mut local: FxHashMap<u32, usize> = FxHashMap::default();
    let mut avail: FxHashMap<u32, Avail> = FxHashMap::default();
    let operand_avail = |avail: &FxHashMap<u32, Avail>, v: u32| -> Avail {
        if let Some(a) = avail.get(&v) {
            return *a;
        }
        // Defined elsewhere: on the recurrence iff tainted inside the loop.
        if tainted.contains(&v) && in_loop(v) {
            carried_start(v)
        } else {
            Avail::FREE
        }
    };
    // `Not`s consumed only by `And`s: folded into `andn` by the backend.
    let mut and_uses: FxHashMap<u32, u32> = FxHashMap::default();
    for inst in insts {
        if let Instruction::BinOp {
            op: IrBinOp::And,
            lhs,
            rhs,
            ..
        } = inst
        {
            for o in [lhs, rhs] {
                if let Operand::Value(v) = o {
                    *and_uses.entry(v.0).or_insert(0) += 1;
                }
            }
        }
    }
    let folds_into_andn = |inst: &Instruction| match inst {
        Instruction::UnaryOp {
            dest,
            op: crate::ir::reexports::IrUnaryOp::Not,
            ..
        } => and_uses
            .get(&dest.0)
            .is_some_and(|n| uses.get(&dest.0) == Some(n)),
        _ => false,
    };
    for (ii, inst) in insts.iter().enumerate() {
        let Some(d) = inst.dest() else { continue };
        local.insert(d.0, ii);
        let a = if let Instruction::Phi { .. } = inst {
            if tainted.contains(&d.0) {
                carried_start(d.0)
            } else {
                Avail::FREE
            }
        } else {
            let mut a = Avail::FREE;
            inst.for_each_used_value(|v| a = a.max(operand_avail(&avail, v)));
            // Only tainted values sit on the recurrence; an untainted value
            // whose operand looked tainted-but-external stays free.
            let lat = if folds_into_andn(inst) {
                0
            } else {
                latency(inst)
            };
            Avail {
                r: if tainted.contains(&d.0) {
                    a.r.map(|r| r + lat as i32)
                } else {
                    None
                },
                l: a.l + lat,
            }
        };
        avail.insert(d.0, a);
    }

    // Tree membership: a node is interior iff its single use is a node of
    // the same operator and type in this block.
    let node_key = |inst: &Instruction| match inst {
        Instruction::BinOp { op, ty, .. } if is_reassociable(*op, *ty) => Some((*op, *ty)),
        _ => None,
    };
    let mut user_of: FxHashMap<u32, usize> = FxHashMap::default();
    for (ii, inst) in insts.iter().enumerate() {
        if let Instruction::BinOp { lhs, rhs, .. } = inst {
            for o in [lhs, rhs] {
                if let Operand::Value(v) = o {
                    user_of.insert(v.0, ii);
                }
            }
        }
    }
    let is_interior = |ii: usize| -> bool {
        let inst = &insts[ii];
        let (Some(key), Some(d)) = (node_key(inst), inst.dest()) else {
            return false;
        };
        uses.get(&d.0) == Some(&1)
            && user_of
                .get(&d.0)
                .is_some_and(|&u| u > ii && node_key(&insts[u]) == Some(key))
    };

    let mut trees: Vec<Tree> = Vec::new();
    for (ii, inst) in insts.iter().enumerate() {
        let Some((op, ty)) = node_key(inst) else {
            continue;
        };
        if is_interior(ii) {
            continue;
        }
        // Depth-first over operands, lhs before rhs, so leaves come out in
        // source (left-to-right) order: ties in the greedy keep that order.
        let Instruction::BinOp { lhs, rhs, .. } = inst else {
            unreachable!()
        };
        let mut nodes = vec![ii];
        let mut leaves = Vec::new();
        let mut stack = vec![rhs, lhs];
        let mut ok = true;
        while let Some(o) = stack.pop() {
            match o {
                Operand::Value(v) if local.get(&v.0).is_some_and(|&d| is_interior(d)) => {
                    let d = local[&v.0];
                    nodes.push(d);
                    let Instruction::BinOp { lhs, rhs, .. } = &insts[d] else {
                        unreachable!()
                    };
                    stack.push(rhs);
                    stack.push(lhs);
                }
                _ => leaves.push(o.clone()),
            }
            if leaves.len() > MAX_LEAVES {
                ok = false;
                break;
            }
        }
        if !ok || leaves.len() < 3 {
            continue;
        }
        if leaves
            .iter()
            .filter(|o| matches!(o, Operand::Const(_)))
            .count()
            > 1
        {
            continue; // constant folding's job; do not reassociate around it
        }
        nodes.sort_unstable();
        trees.push(Tree {
            nodes,
            leaves,
            op,
            ty,
            root_dest: inst.dest().unwrap(),
        });
    }
    if trees.is_empty() {
        return 0;
    }

    // Plan every profitable rewrite before touching the block.
    struct NewNode {
        anchor: isize,    // emit after this original index (-1: block start)
        root: usize,      // original root index: the new node's source span
        sunk: Vec<usize>, // leaf chains emitted just before the node
        inst: Instruction,
    }
    // A side-effect-free, non-trapping instruction whose result has exactly
    // one use may move down to just before that use.
    let sinkable = |ii: usize| -> bool {
        let inst = &insts[ii];
        let pure = match inst {
            Instruction::BinOp { op, .. } => !matches!(
                op,
                IrBinOp::SDiv | IrBinOp::UDiv | IrBinOp::SRem | IrBinOp::URem
            ),
            Instruction::UnaryOp { .. } | Instruction::Cast { .. } | Instruction::Copy { .. } => {
                true
            }
            _ => false,
        };
        pure && inst.dest().is_some_and(|d| uses.get(&d.0) == Some(&1))
    };
    let mut moved: FxHashSet<usize> = FxHashSet::default();
    let mut deleted: FxHashSet<usize> = FxHashSet::default();
    let mut new_nodes: Vec<NewNode> = Vec::new();
    let leaf_avail = |o: &Operand| match o {
        Operand::Const(_) => Avail::FREE,
        Operand::Value(v) => operand_avail(&avail, v.0),
    };
    let leaf_pos = |o: &Operand| -> isize {
        match o {
            Operand::Value(v) => local.get(&v.0).map_or(-1, |&p| p as isize),
            Operand::Const(_) => -1,
        }
    };
    // Recurrent register pressure after each instruction of the block
    // (computed on first need; the block is not modified until the end).
    let mut pressure: Option<Vec<u32>> = None;
    let recurrent_pressure = || -> Vec<u32> {
        let mut live: FxHashSet<u32> = facts.live_out[bi].clone();
        func.blocks[bi].terminator.for_each_used_value(|v| {
            if tainted.contains(&v) {
                live.insert(v);
            }
        });
        let mut out = vec![0u32; insts.len()];
        for ii in (0..insts.len()).rev() {
            out[ii] = live.iter().map(|&v| weight(v)).sum();
            if let Some(d) = insts[ii].dest() {
                live.remove(&d.0);
            }
            if !matches!(insts[ii], Instruction::Phi { .. }) {
                insts[ii].for_each_used_value(|v| {
                    if tainted.contains(&v) {
                        live.insert(v);
                    }
                });
            }
        }
        out
    };
    let budget = gpr_budget();
    // A single-use load defined in this block: the backend folds it into
    // the second operand of an ALU op.
    let foldable_load = |o: &Operand| match o {
        Operand::Value(v) => local.get(&v.0).is_some_and(|&d| {
            matches!(insts[d], Instruction::Load { .. }) && uses.get(&v.0) == Some(&1)
        }),
        Operand::Const(_) => false,
    };
    let mut rewrites = 0;
    for t in &trees {
        let root = *t.nodes.last().unwrap();
        let current = avail[&t.root_dest.0];
        // Greedy: (avail, seq) min-first; seq keeps ties deterministic and
        // biased to the original left-to-right order.
        let mut work: Vec<(Avail, usize, Operand, isize)> = t
            .leaves
            .iter()
            .enumerate()
            .map(|(i, o)| (leaf_avail(o), i, o.clone(), leaf_pos(o)))
            .collect();
        // Only trees on a loop-carried recurrence are worth reshaping: an
        // off-recurrence tree is overlapped across iterations by the
        // out-of-order core, so a shorter one gains nothing, while a
        // reshaped one can raise register pressure (measured: a 9-product
        // convolution sum balanced by local depth spilled).
        if work.iter().all(|w| w.0.r.is_none()) {
            continue;
        }
        let n_leaves = work.len();
        let floor = t.nodes[0] as isize - 1;
        let mut seq = work.len();
        let mut planned: Vec<NewNode> = Vec::new();
        let mut tree_moved: Vec<usize> = Vec::new();
        while work.len() > 1 {
            work.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
            // Free operands first, as a left fold in source order (keeps the
            // source's register pressure); then the Huffman merge by
            // availability, where the free partial sum (earliest of all)
            // joins the earliest carried operand.
            let mut frees: Vec<usize> =
                (0..work.len()).filter(|&k| work[k].0.r.is_none()).collect();
            frees.sort_by_key(|&k| work[k].1);
            let (k0, k1) = if frees.len() >= 2 {
                match frees.iter().position(|&k| work[k].1 >= n_leaves) {
                    Some(pi) => (frees[pi], frees[if pi == 0 { 1 } else { 0 }]),
                    None => (frees[0], frees[1]),
                }
            } else {
                (0, 1)
            };
            let (first, second) = (work[k0].clone(), work[k1].clone());
            work.remove(k0.max(k1));
            work.remove(k0.min(k1));
            let (a0, _, o0, p0) = first;
            let (a1, _, o1, p1) = second;
            let dest = if work.is_empty() {
                t.root_dest
            } else {
                Value(*next_id + planned.len() as u32)
            };
            let anchor = p0.max(p1).max(floor);
            // Sink the operands' single-use pure chains (original leaves
            // only: new nodes are already placed) to just before this node.
            let mut sunk = Vec::new();
            for o in [&o0, &o1] {
                let Operand::Value(v) = o else { continue };
                let Some(&d) = local.get(&v.0) else { continue };
                let mut stack = vec![d];
                while let Some(i) = stack.pop() {
                    if !sinkable(i)
                        || deleted.contains(&i)
                        || moved.contains(&i)
                        || tree_moved.contains(&i)
                        || t.nodes.contains(&i)
                    {
                        continue;
                    }
                    sunk.push(i);
                    tree_moved.push(i);
                    insts[i].for_each_used_value(|u| {
                        if let Some(&ud) = local.get(&u) {
                            stack.push(ud);
                        }
                    });
                }
            }
            sunk.sort_unstable();
            let (lhs, rhs) = if foldable_load(&o0) && !foldable_load(&o1) {
                (o1, o0)
            } else {
                (o0, o1)
            };
            planned.push(NewNode {
                anchor,
                root,
                sunk,
                inst: Instruction::BinOp {
                    dest,
                    op: t.op,
                    lhs,
                    rhs,
                    ty: t.ty,
                },
            });
            work.push((a0.after(a1, 1), seq, Operand::Value(dest), anchor));
            seq += 1;
        }
        // Accept only a strictly shorter recurrence: local depth alone is
        // not a reason to reshape (see above), so ties keep source shape.
        let new_root = work[0].0;
        if new_root.r >= current.r {
            continue;
        }
        // Register residency: skip when the recurrence cannot be register
        // resident anywhere in the tree's span.
        let p = pressure.get_or_insert_with(recurrent_pressure);
        if (t.nodes[0]..=root).all(|ii| p[ii] > budget) {
            continue;
        }
        debug_assert!(work[0].3 <= root as isize);
        *next_id += planned.len() as u32 - 1;
        deleted.extend(t.nodes.iter().copied());
        moved.extend(tree_moved);
        new_nodes.extend(planned);
        rewrites += 1;
    }
    if rewrites == 0 {
        return 0;
    }

    // Rebuild the block: original instructions minus the old tree nodes,
    // each followed by the new nodes anchored after it (creation order is
    // topological and anchors are monotone along operands).
    let block = &mut func.blocks[bi];
    let old_spans = std::mem::take(&mut block.source_spans);
    let mut old: Vec<Option<Instruction>> = std::mem::take(&mut block.instructions)
        .into_iter()
        .map(Some)
        .collect();
    let has_spans = old_spans.len() == old.len();
    // Each group entry: (sunk leaf chains with their original indices, root
    // index for the span, new node).  Sunk instructions leave their slots.
    type Group = Vec<(Vec<(usize, Instruction)>, usize, Instruction)>;
    let mut by_anchor: FxHashMap<isize, Group> = FxHashMap::default();
    for nn in new_nodes {
        let sunk = nn
            .sunk
            .iter()
            .map(|&i| (i, old[i].take().expect("an instruction is sunk once")))
            .collect();
        by_anchor
            .entry(nn.anchor)
            .or_default()
            .push((sunk, nn.root, nn.inst));
    }
    let mut insts = Vec::with_capacity(old.len() + 1);
    let mut spans = Vec::with_capacity(if has_spans { old.len() + 1 } else { 0 });
    let emit = |group: Option<Group>,
                insts: &mut Vec<Instruction>,
                spans: &mut Vec<crate::common::source::Span>| {
        for (sunk, root, i) in group.into_iter().flatten() {
            for (si, s) in sunk {
                if has_spans {
                    spans.push(old_spans[si]);
                }
                insts.push(s);
            }
            if has_spans {
                spans.push(old_spans[root]);
            }
            insts.push(i);
        }
    };
    emit(by_anchor.remove(&-1), &mut insts, &mut spans);
    for (ii, slot) in old.iter_mut().enumerate() {
        if let Some(inst) = slot.take() {
            if !deleted.contains(&ii) {
                insts.push(inst);
                if has_spans {
                    spans.push(old_spans[ii]);
                }
            }
        }
        emit(by_anchor.remove(&(ii as isize)), &mut insts, &mut spans);
    }
    debug_assert!(by_anchor.is_empty());
    block.instructions = insts;
    if has_spans {
        block.source_spans = spans;
    }
    rewrites
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, IrConst, Terminator};

    fn blk(id: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(id),
            instructions,
            terminator,
            source_spans: Vec::new(),
        }
    }

    fn bin(d: u32, op: IrBinOp, lhs: Operand, rhs: Operand, ty: IrType) -> Instruction {
        Instruction::BinOp {
            dest: Value(d),
            op,
            lhs,
            rhs,
            ty,
        }
    }

    fn v(id: u32) -> Operand {
        Operand::Value(Value(id))
    }

    /// The SHA-256 `t1` shape in a counted loop:
    ///
    /// ```text
    /// b1: i = phi [0, b0], [i', b2]        ; induction variable
    ///     e = phi [1, b0], [e', b2]        ; loop-carried value
    ///     h = phi [2, b0], [e,  b2]        ; carried, one round late
    /// b2: s  = rotr e, 6 ; s2 = s ^ e      ; Σ-like, 2 deep from e
    ///     w  = load K[i]                   ; IV-addressed: off the recurrence
    ///     t0 = h + s2 ; t1 = t0 + w        ; the source-order tree
    ///     e' = t1 ^ h ; i' = i + 1
    /// ```
    ///
    /// Source order puts `+ w` after `s2`: e→e' is rotr, xor, add, add, xor.
    /// Adding `w + h` first leaves one add after `s2`.
    fn sha_like(tree_in_loop: bool) -> IrFunction {
        let mut f = IrFunction::new("t".to_string(), IrType::U32, vec![], false);
        let tree = vec![
            bin(
                10,
                IrBinOp::RotateRight,
                v(2),
                Operand::Const(IrConst::I32(6)),
                IrType::U32,
            ),
            bin(11, IrBinOp::Xor, v(10), v(2), IrType::U32),
            bin(
                12,
                IrBinOp::Shl,
                v(1),
                Operand::Const(IrConst::I64(2)),
                IrType::I64,
            ),
            Instruction::GetElementPtr {
                dest: Value(13),
                base: Value(9),
                offset: v(12),
                ty: IrType::Ptr,
            },
            Instruction::Load {
                dest: Value(14),
                ptr: Value(13),
                ty: IrType::U32,
                seg_override: Default::default(),
                volatile: false,
            },
            bin(15, IrBinOp::Add, v(3), v(11), IrType::U32),
            bin(16, IrBinOp::Add, v(15), v(14), IrType::U32),
            bin(17, IrBinOp::Xor, v(16), v(3), IrType::U32),
            bin(
                18,
                IrBinOp::Add,
                v(1),
                Operand::Const(IrConst::I64(1)),
                IrType::I64,
            ),
        ];
        let phis = vec![
            Instruction::Phi {
                dest: Value(1),
                ty: IrType::I64,
                incoming: vec![
                    (Operand::Const(IrConst::I64(0)), BlockId(0)),
                    (v(18), BlockId(2)),
                ],
            },
            Instruction::Phi {
                dest: Value(2),
                ty: IrType::U32,
                incoming: vec![
                    (Operand::Const(IrConst::I32(1)), BlockId(0)),
                    (v(17), BlockId(2)),
                ],
            },
            Instruction::Phi {
                dest: Value(3),
                ty: IrType::U32,
                incoming: vec![
                    (Operand::Const(IrConst::I32(2)), BlockId(0)),
                    (v(2), BlockId(2)),
                ],
            },
            Instruction::Cmp {
                dest: Value(4),
                op: crate::ir::reexports::IrCmpOp::Slt,
                lhs: v(1),
                rhs: Operand::Const(IrConst::I64(64)),
                ty: IrType::I64,
            },
        ];
        let entry = vec![Instruction::GlobalAddr {
            dest: Value(9),
            name: "K".to_string(),
        }];
        if tree_in_loop {
            f.blocks = vec![
                blk(0, entry, Terminator::Branch(BlockId(1))),
                blk(
                    1,
                    phis,
                    Terminator::CondBranch {
                        cond: v(4),
                        true_label: BlockId(2),
                        false_label: BlockId(3),
                    },
                ),
                blk(2, tree, Terminator::Branch(BlockId(1))),
                blk(3, vec![], Terminator::Return(Some(v(2)))),
            ];
        } else {
            // Same instructions, straight-line: the phis become plain
            // copies of constants, and nothing repeats.
            let mut body = entry;
            body.push(Instruction::Copy {
                dest: Value(1),
                src: Operand::Const(IrConst::I64(0)),
            });
            body.push(Instruction::Copy {
                dest: Value(2),
                src: Operand::Const(IrConst::I32(1)),
            });
            body.push(Instruction::Copy {
                dest: Value(3),
                src: Operand::Const(IrConst::I32(2)),
            });
            body.extend(tree);
            f.blocks = vec![blk(0, body, Terminator::Return(Some(v(17))))];
        }
        f.next_value_id = 19;
        f.next_label = 4;
        f
    }

    fn def<'a>(f: &'a IrFunction, id: u32) -> &'a Instruction {
        f.blocks
            .iter()
            .flat_map(|b| b.instructions.iter())
            .find(|i| i.dest() == Some(Value(id)))
            .expect("value defined")
    }

    fn assert_valid(f: &IrFunction) {
        let mut out = Vec::new();
        crate::passes::verify::verify_function(f, "reassoc_lat test", &mut out);
        assert!(out.is_empty(), "IR verifier: {out:?}");
    }

    #[test]
    fn loop_carried_operand_is_combined_last() {
        let mut f = sha_like(true);
        assert_eq!(run_function(&mut f), 1);
        assert_valid(&f);
        // Root keeps its value id; its operands are (w + h) and s2.
        let Instruction::BinOp { op, lhs, rhs, .. } = def(&f, 16) else {
            panic!("root is not a BinOp")
        };
        assert_eq!(*op, IrBinOp::Add);
        assert_eq!(*rhs, v(11), "the deepest recurrence operand goes last");
        let Operand::Value(inner) = lhs else {
            panic!("inner node")
        };
        assert!(inner.0 >= 19, "inner node gets a fresh id");
        let Instruction::BinOp { lhs, rhs, .. } = def(&f, inner.0) else {
            panic!("inner node is not a BinOp")
        };
        assert_eq!(
            (lhs, rhs),
            (&v(3), &v(14)),
            "the free load joins h first, as the second operand (folds into add mem)"
        );
        // The superseded interior node is gone, the id cache is advanced.
        assert!(
            f.blocks
                .iter()
                .all(|b| b.instructions.iter().all(|i| i.dest() != Some(Value(15))))
        );
        assert_eq!(f.next_value_id, inner.0 + 1);
        // Σ's single-use chain is sunk to just before its new consumer, so
        // it is not live across the early part of the tree.
        let body = &f.blocks[2].instructions;
        let pos = |id: u32| {
            body.iter()
                .position(|i| i.dest() == Some(Value(id)))
                .unwrap()
        };
        assert_eq!(pos(11) + 1, pos(16), "s2 immediately precedes the root");
        assert_eq!(pos(10) + 1, pos(11), "its rotate moved with it");
        assert!(pos(14) < pos(inner.0), "the load stays where it was");
    }

    #[test]
    fn rewrite_is_idempotent() {
        let mut f = sha_like(true);
        assert_eq!(run_function(&mut f), 1);
        let once = format!("{:?}", f.blocks);
        assert_eq!(run_function(&mut f), 0, "an optimal tree is left alone");
        assert_eq!(format!("{:?}", f.blocks), once);
    }

    #[test]
    fn straight_line_code_is_not_touched() {
        let mut f = sha_like(false);
        let before = format!("{:?}", f.blocks);
        assert_eq!(run_function(&mut f), 0);
        assert_eq!(format!("{:?}", f.blocks), before);
    }

    #[test]
    fn multi_use_node_bounds_the_tree() {
        // `t0 = h + s2` also feeds the return value: it is a leaf of the
        // `t1` tree (2 leaves: nothing to reassociate) and its own root.
        let mut f = sha_like(true);
        f.blocks[3].terminator = Terminator::Return(Some(v(15)));
        let before = format!("{:?}", f.blocks);
        assert_eq!(run_function(&mut f), 0);
        assert_eq!(format!("{:?}", f.blocks), before);
    }

    #[test]
    fn induction_variables_are_not_recurrences() {
        let f = sha_like(true);
        let label_to_idx = analysis::build_label_map(&f);
        let (preds, succs) = analysis::build_cfg(&f, &label_to_idx);
        let idom = analysis::compute_dominators(f.blocks.len(), &preds, &succs);
        let loops = loop_analysis::find_natural_loops(f.blocks.len(), &preds, &succs, &idom);
        assert_eq!(loops.len(), 1);
        let mut def_block = FxHashMap::default();
        for (bi, b) in f.blocks.iter().enumerate() {
            for i in &b.instructions {
                if let Some(d) = i.dest() {
                    def_block.insert(d.0, bi);
                }
            }
        }
        let rec = recurrent_phis(&f, &loops[0], &def_block, &label_to_idx);
        assert!(!rec.contains(&1), "i' = i + 1 is an induction variable");
        assert!(rec.contains(&2) && rec.contains(&3), "e and h carry data");
        let tainted = tainted_values(&f, &loops[0], &rec);
        assert!(
            !tainted.contains(&14),
            "IV-addressed load is off the recurrence"
        );
        assert!(tainted.contains(&11) && tainted.contains(&16));
    }

    #[test]
    fn off_recurrence_tree_keeps_source_shape() {
        // In the loop, but fed only by the IV-addressed load and a
        // constant: ((w + w) + w) + 7.  Balancing it would shorten its
        // local depth (10 -> 9) yet no loop-carried chain runs through it,
        // so the out-of-order core overlaps it across iterations anyway,
        // and a reshaped tree only costs registers.
        let mut f = sha_like(true);
        let body = &mut f.blocks[2].instructions;
        body[5] = bin(15, IrBinOp::Add, v(14), v(14), IrType::U32);
        body[6] = bin(16, IrBinOp::Add, v(15), v(14), IrType::U32);
        body[7] = bin(
            17,
            IrBinOp::Add,
            v(16),
            Operand::Const(IrConst::I32(7)),
            IrType::U32,
        );
        let before = format!("{:?}", f.blocks);
        assert_eq!(run_function(&mut f), 0);
        assert_eq!(format!("{:?}", f.blocks), before);
    }

    /// An `n`-word state rotation (`w[k] = w[k-1]` for k >= 1, `w[0] = t`)
    /// with `t = ((((w[n-1] + w[0]) + w[1]) + ...) + w[n-2]) + K[i]`, the
    /// check_phi_acyclic_order.sh `rot()` shape.  Value ids: IV 40, IV' 41,
    /// cmp 42, K 43, index 44, gep 45, load 46, words 1..=n, tree 60.., and
    /// the last tree node (t) is returned by `rot_root`.  With `swap`, two
    /// extra carried phis 30/31 exchange values (a pure copy cycle).
    fn rot_fn(n: u32, swap: bool) -> IrFunction {
        assert!(n >= 3);
        let mut f = IrFunction::new("rot".to_string(), IrType::U32, vec![], false);
        let root = rot_root(n);
        let mut phis = vec![Instruction::Phi {
            dest: Value(40),
            ty: IrType::I64,
            incoming: vec![
                (Operand::Const(IrConst::I64(0)), BlockId(0)),
                (v(41), BlockId(2)),
            ],
        }];
        for k in 0..n {
            let back = if k == 0 { v(root) } else { v(k) }; // w[k] <- w[k-1]
            phis.push(Instruction::Phi {
                dest: Value(k + 1),
                ty: IrType::U32,
                incoming: vec![
                    (Operand::Const(IrConst::I32(k as i32)), BlockId(0)),
                    (back, BlockId(2)),
                ],
            });
        }
        if swap {
            for (d, other) in [(30, 31), (31, 30)] {
                phis.push(Instruction::Phi {
                    dest: Value(d),
                    ty: IrType::U32,
                    incoming: vec![
                        (Operand::Const(IrConst::I32(d as i32)), BlockId(0)),
                        (v(other), BlockId(2)),
                    ],
                });
            }
        }
        phis.push(Instruction::Cmp {
            dest: Value(42),
            op: crate::ir::reexports::IrCmpOp::Slt,
            lhs: v(40),
            rhs: Operand::Const(IrConst::I64(64)),
            ty: IrType::I64,
        });
        let mut body = vec![];
        // ((w[n-1] + w[0]) + w[1]) + ... + w[n-2]
        body.push(bin(60, IrBinOp::Add, v(n), v(1), IrType::U32));
        let mut last = 60;
        for k in 1..n - 1 {
            body.push(bin(last + 1, IrBinOp::Add, v(last), v(k + 1), IrType::U32));
            last += 1;
        }
        body.push(bin(
            44,
            IrBinOp::Shl,
            v(40),
            Operand::Const(IrConst::I64(2)),
            IrType::I64,
        ));
        body.push(Instruction::GetElementPtr {
            dest: Value(45),
            base: Value(43),
            offset: v(44),
            ty: IrType::Ptr,
        });
        body.push(Instruction::Load {
            dest: Value(46),
            ptr: Value(45),
            ty: IrType::U32,
            seg_override: Default::default(),
            volatile: false,
        });
        body.push(bin(root, IrBinOp::Add, v(last), v(46), IrType::U32));
        body.push(bin(
            41,
            IrBinOp::Add,
            v(40),
            Operand::Const(IrConst::I64(1)),
            IrType::I64,
        ));
        f.blocks = vec![
            blk(
                0,
                vec![Instruction::GlobalAddr {
                    dest: Value(43),
                    name: "K".to_string(),
                }],
                Terminator::Branch(BlockId(1)),
            ),
            blk(
                1,
                phis,
                Terminator::CondBranch {
                    cond: v(42),
                    true_label: BlockId(2),
                    false_label: BlockId(3),
                },
            ),
            blk(2, body, Terminator::Branch(BlockId(1))),
            blk(3, vec![], Terminator::Return(Some(v(1)))),
        ];
        f.next_value_id = root + 1;
        f.next_label = 4;
        f
    }

    fn rot_root(n: u32) -> u32 {
        60 + n - 1
    }

    fn loop_of(
        f: &IrFunction,
    ) -> (
        loop_analysis::NaturalLoop,
        FxHashMap<crate::ir::reexports::BlockId, usize>,
    ) {
        let label_to_idx = analysis::build_label_map(f);
        let (preds, succs) = analysis::build_cfg(f, &label_to_idx);
        let idom = analysis::compute_dominators(f.blocks.len(), &preds, &succs);
        let mut loops = loop_analysis::find_natural_loops(f.blocks.len(), &preds, &succs, &idom);
        assert_eq!(loops.len(), 1);
        (loops.remove(0), label_to_idx)
    }

    fn def_blocks(f: &IrFunction) -> FxHashMap<u32, usize> {
        let mut m = FxHashMap::default();
        for (bi, b) in f.blocks.iter().enumerate() {
            for i in &b.instructions {
                if let Some(d) = i.dest() {
                    m.insert(d.0, bi);
                }
            }
        }
        m
    }

    /// Sets the thread's target for the duration of a test.
    struct Target;
    impl Target {
        fn set(machine: u16, ptr: usize) -> Target {
            crate::common::types::set_target_elf_machine(machine);
            crate::common::types::set_target_ptr_size(ptr);
            Target
        }
    }
    impl Drop for Target {
        fn drop(&mut self) {
            crate::common::types::set_target_elf_machine(62);
            crate::common::types::set_target_ptr_size(8);
        }
    }

    #[test]
    fn rotation_lags_follow_copy_chains() {
        let f = rot_fn(8, true);
        let (lp, label_to_idx) = loop_of(&f);
        let rec = recurrent_phis(&f, &lp, &def_blocks(&f), &label_to_idx);
        assert!((1..=8).all(|k| rec.contains(&k)) && rec.contains(&30) && rec.contains(&31));
        let lags = rotation_lags(&f, &lp, &rec, &label_to_idx);
        for k in 1..8u32 {
            assert_eq!(lags.get(&(k + 1)), Some(&(k as i32)), "w[{k}] lags {k}");
        }
        assert!(!lags.contains_key(&1), "w[0] is computed: lag 0");
        assert!(
            !lags.contains_key(&30) && !lags.contains_key(&31),
            "a pure swap cycle has no computed source: lag 0"
        );
    }

    /// The PR #638 regression: with every carried phi at time 0 the pass
    /// reshaped rot() without shortening its recurrence and lost the load
    /// fold.  With lags, the computed word w[0] is combined last (one add
    /// after it), and the load is the second operand of its node.
    #[test]
    fn rotation_combines_the_computed_word_last() {
        let n = 4;
        let mut f = rot_fn(n, false);
        assert_eq!(run_function(&mut f), 1);
        assert_valid(&f);
        // Each level combines one leaf with the partial sum below it:
        // w[0] (lag 0) last, then w[1], w[2], and (w[3] + load) first.
        let split = |x: &Operand, leaf: Operand| -> Operand {
            let Operand::Value(x) = x else { panic!("node") };
            let Instruction::BinOp { lhs, rhs, .. } = def(&f, x.0) else {
                panic!("node is not a BinOp")
            };
            if *lhs == leaf {
                rhs.clone()
            } else {
                assert_eq!(*rhs, leaf, "leaf at this level");
                lhs.clone()
            }
        };
        let mut cur = split(&v(rot_root(n)), v(1));
        for leaf in [v(2), v(3)] {
            cur = split(&cur, leaf);
        }
        let Operand::Value(x) = cur else {
            panic!("first node")
        };
        let Instruction::BinOp { lhs, rhs, .. } = def(&f, x.0) else {
            panic!("first node")
        };
        assert_eq!(
            (lhs, rhs),
            (&v(4), &v(46)),
            "the load is the folded second operand"
        );
        assert_eq!(run_function(&mut f), 0, "fixpoint reached");
    }

    #[test]
    fn recurrent_liveness_is_exact() {
        let f = rot_fn(4, false);
        let (lp, label_to_idx) = loop_of(&f);
        let rec = recurrent_phis(&f, &lp, &def_blocks(&f), &label_to_idx);
        let tainted = tainted_values(&f, &lp, &rec);
        let (_, succs) = analysis::build_cfg(&f, &label_to_idx);
        let live = live_out_restricted(&f, &succs, &label_to_idx, &tainted);
        let mut body: Vec<u32> = live[2].iter().copied().collect();
        body.sort_unstable();
        // w[0..3] feed the next words' phis, t feeds w[0]'s; w[3] dies in
        // the tree and the tree's interior nodes are single-use temporaries.
        assert_eq!(body, vec![1, 2, 3, rot_root(4)]);
        let mut header: Vec<u32> = live[1].iter().copied().collect();
        header.sort_unstable();
        assert_eq!(header, vec![1, 2, 3, 4], "all words live into the body");
        assert!(live[3].is_empty(), "nothing is live out of the exit");
    }

    #[test]
    fn gpr_weights_follow_the_target_word() {
        {
            let _t = Target::set(EM_386, 4);
            assert_eq!(gpr_weight(Some(IrType::I64)), 2, "i686 pairs");
            assert_eq!(gpr_weight(Some(IrType::U32)), 1);
            assert_eq!(gpr_weight(Some(IrType::F64)), 0);
            assert_eq!(gpr_budget(), 6);
        }
        assert_eq!(gpr_weight(Some(IrType::I64)), 1);
        assert_eq!(gpr_weight(Some(IrType::I128)), 2);
        assert_eq!(gpr_weight(Some(IrType::U8)), 1);
        assert_eq!(gpr_budget(), 13);
    }

    /// Register residency: eight rotating words cannot stay in i686's six
    /// GPRs, so the recurrence runs through memory and the reshape is left
    /// out (measured 1.058x slower on SHA-256).  Four words fit, and x86-64
    /// holds all eight: both are rewritten.
    #[test]
    fn memory_resident_rotation_is_left_alone() {
        {
            let _t = Target::set(EM_386, 4);
            let mut f = rot_fn(8, false);
            let before = format!("{:?}", f.blocks);
            assert_eq!(run_function(&mut f), 0, "8 words / 6 GPRs: left alone");
            assert_eq!(format!("{:?}", f.blocks), before);
            let mut f = rot_fn(4, false);
            assert_eq!(run_function(&mut f), 1, "4 words fit i686");
            assert_valid(&f);
        }
        let mut f = rot_fn(8, false);
        assert_eq!(run_function(&mut f), 1, "8 words fit x86-64");
        assert_valid(&f);
    }

    #[test]
    fn availability_order_is_lexicographic() {
        let free_late = Avail { r: None, l: 9 };
        let carried = Avail { r: Some(0), l: 0 };
        assert!(
            free_late < carried,
            "off-recurrence beats any recurrence time"
        );
        assert_eq!(carried.after(free_late, 1), Avail { r: Some(1), l: 10 });
        assert_eq!(Avail::FREE.after(free_late, 1), Avail { r: None, l: 10 });
    }
}
