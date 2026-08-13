//! Profile-generation instrumentation: Knuth–Stevenson minimal edge counting
//! plus indirect-call value profiling.
//!
//! For every function we build the CFG, find a MAXIMUM spanning tree (hot
//! edges — loop backedges, cycle edges — stay on the tree and are never
//! instrumented), and count exactly the non-tree edges plus the virtual
//! entry edge. Flow conservation at profile-use time derives every block and
//! tree-edge count (GCC gcov / LLVM PGO do the same).
//!
//! Counters are emitted as a single `[lock] incq sym+off(%rip)` instruction
//! (Instruction::PgoCounterInc). Default update mode is `single` (plain
//! increment — the GCC/LLVM default); `-fprofile-update=atomic` selects the
//! `lock` prefix.
//!
//! v7 adds indirect-call VALUE profiling (LLVM `IPVK_IndirectCallTarget`):
//! each `CallIndirect` site gets a 72-byte global (4 target slots, 4 counts,
//! total) and a call to a per-TU runtime helper that records the top-4
//! callees seen at run time. At profile use the top target is promoted to a
//! guarded direct call when it dominates the site (>= 51%).
//!
//! Soundness: counters are placed so they never sit between a fused Cmp and
//! its branch/select consumer (`incq` clobbers flags), and instrumentation
//! runs post-optimization so no optimization pass ever sees a counter.
//!
//! Identity: profiles are keyed by `h0`, a fingerprint of the PRE-pass IR
//! (computed right after mem2reg, identical in generate and use builds).
//! `h1` is the post-pass fingerprint used to detect CFG drift from
//! profile-guided transforms; drift degrades edge-based layout gracefully
//! instead of dropping the whole function.
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::backend::Target;
use crate::ir::constants::IrConst;
use crate::ir::reexports::{
    BasicBlock, BlockId, CallInfo, GlobalInit, Instruction, IrFunction, IrGlobal, IrModule,
    Operand, Terminator, Value,
};
use crate::pgo::profile::{function_key, resolve_output_path, unit_hash};
const SEC: &str = ".lccc_pgo_cnts";
const SEC_VP: &str = ".lccc_pgo_vps";
type Rec = (
    String,
    String,
    Vec<(u32, u32, usize)>,
    usize,
    u64,
    u64,
    u32,
    u64,
);

/// Sentinel source label for the virtual entry edge (V -> entry). Cannot
/// collide with a real block label.
pub(crate) const VENTRY: u32 = u32::MAX - 1;
/// Sentinel destination label for the virtual EXIT node (ret block -> exit).
/// Modeling the exit closes the flow equations for RETURN-terminated blocks:
/// without it, leaf blocks (case targets, exits) derive count 0 and every
/// edge into them derives 0 (observed: switch-case edges all zero, so
/// profile-driven switch partitioning never fired).
pub(crate) const VEXIT: u32 = u32::MAX - 2;

/// One instrumented indirect-call site.
struct SiteRec {
    ordinal: usize,
    site: String,
    sig: String,
}

/// Stable textual signature of a call: `RET:nfixed:A0,A1,...` using the
/// Debug names of IrType (stable within a compiler version). Verified at
/// promotion time so a drifted site ordinal can never direct-call a callee
/// with a different signature.
fn site_signature(info: &CallInfo) -> String {
    let n = info.num_fixed_args.min(info.arg_types.len());
    let mut s = format!("{:?}:{}", info.return_type, n);
    for t in info.arg_types.iter().take(n) {
        s.push(':');
        s.push_str(&format!("{:?}", t));
    }
    s
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Edge {
    src: u32,
    dst: u32,
}

fn successors(t: &Terminator) -> Vec<u32> {
    match t {
        Terminator::Branch(x) => vec![x.0],
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => vec![true_label.0, false_label.0],
        Terminator::Switch { cases, default, .. } => {
            let mut v: Vec<u32> = cases.iter().map(|(_, b)| b.0).collect();
            v.push(default.0);
            v
        }
        Terminator::IndirectBranch {
            possible_targets, ..
        } => possible_targets.iter().map(|b| b.0).collect(),
        _ => vec![],
    }
}

/// Iterative dominator computation (simple dataflow) over the block graph.
/// Classic Cooper–Harvey–Kennedy worklist: dom[entry] = {entry}, all other
/// nodes start at the full set and shrink by intersecting predecessors' doms.
fn dominators(nodes: &[u32], edges: &[Edge], entry: u32) -> FxHashMap<u32, FxHashSet<u32>> {
    let mut preds: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    for e in edges {
        preds.entry(e.dst).or_default().push(e.src);
    }
    let all: FxHashSet<u32> = nodes.iter().copied().collect();
    let mut dom: FxHashMap<u32, FxHashSet<u32>> = FxHashMap::default();
    let mut entry_set = FxHashSet::default();
    entry_set.insert(entry);
    for &n in nodes {
        dom.insert(
            n,
            if n == entry {
                entry_set.clone()
            } else {
                all.clone()
            },
        );
    }
    let mut changed = true;
    while changed {
        changed = false;
        for &n in nodes {
            if n == entry {
                continue;
            }
            let ps = preds.get(&n).cloned().unwrap_or_default();
            if ps.is_empty() {
                continue;
            }
            let mut newd: FxHashSet<u32> = all.clone();
            for p in &ps {
                if let Some(d) = dom.get(p) {
                    newd = newd.intersection(d).copied().collect();
                }
            }
            newd.insert(n);
            if newd != *dom.get(&n).unwrap() {
                dom.insert(n, newd);
                changed = true;
            }
        }
    }
    dom
}

/// Tarjan SCC over the block graph.
fn scc_ids(nodes: &[u32], edges: &[Edge]) -> FxHashMap<u32, u32> {
    let idx: FxHashMap<u32, usize> = nodes.iter().enumerate().map(|(i, n)| (*n, i)).collect();
    let mut adj: FxHashMap<usize, Vec<usize>> = FxHashMap::default();
    for e in edges {
        if let (Some(&a), Some(&b)) = (idx.get(&e.src), idx.get(&e.dst)) {
            adj.entry(a).or_default().push(b);
        }
    }
    let n = nodes.len();
    let mut index = vec![usize::MAX; n];
    let mut low = vec![0usize; n];
    let mut on = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut comp = vec![usize::MAX; n];
    let mut next_index = 0usize;
    let mut next_comp = 0usize;
    fn dfs(
        v: usize,
        adj: &FxHashMap<usize, Vec<usize>>,
        index: &mut Vec<usize>,
        low: &mut Vec<usize>,
        on: &mut Vec<bool>,
        stack: &mut Vec<usize>,
        comp: &mut Vec<usize>,
        next_index: &mut usize,
        next_comp: &mut usize,
    ) {
        index[v] = *next_index;
        low[v] = *next_index;
        *next_index += 1;
        stack.push(v);
        on[v] = true;
        if let Some(succs) = adj.get(&v) {
            for &w in succs {
                if index[w] == usize::MAX {
                    dfs(w, adj, index, low, on, stack, comp, next_index, next_comp);
                    low[v] = low[v].min(low[w]);
                } else if on[w] {
                    low[v] = low[v].min(index[w]);
                }
            }
        }
        if low[v] == index[v] {
            loop {
                let w = stack.pop().unwrap();
                on[w] = false;
                comp[w] = *next_comp;
                if w == v {
                    break;
                }
            }
            *next_comp += 1;
        }
    }
    for v in 0..n {
        if index[v] == usize::MAX {
            dfs(
                v, &adj, &mut index, &mut low, &mut on, &mut stack, &mut comp, &mut next_index,
                &mut next_comp,
            );
        }
    }
    nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (*n, comp[i] as u32))
        .collect()
}

/// Choose the instrumented (non-tree) edges: maximum-weight directed spanning
/// ARBORESCENCE over the CFG (rooted at entry), with the virtual entry edge
/// forced instrumented.
fn choose_instrumented_edges(nodes: &[u32], edges: &[Edge], entry: u32) -> Vec<Edge> {
    let comp = scc_ids(nodes, edges);
    let dom = dominators(nodes, edges, entry);
    let weight = |e: &Edge| -> i64 {
        let backed = dom
            .get(&e.src)
            .map(|d| d.contains(&e.dst))
            .unwrap_or(false);
        let loopish = e.src == e.dst
            || comp
                .get(&e.src)
                .zip(comp.get(&e.dst))
                .map(|(a, b)| a == b)
                .unwrap_or(false);
        if backed {
            2000
        } else if loopish {
            1000
        } else {
            1
        }
    };
    // Build a MAX-WEIGHT DIRECTED SPANNING ARBORESCENCE
    // rooted at `entry` (Chu–Liu/Edmonds via BFS + greedy improvement),
    // over ALL nodes including the virtual EXIT. The old code used an
    // UNDIRECTED max spanning tree (Kruskal/DSU), which can orient a loop
    // backedge BACKWARDS: for a simple loop the backedge (latch->header,
    // weight 2000) was chosen as the tree edge, so the latch's only incoming
    // edge (header->latch) became non-tree/instrumented and the latch had NO
    // incoming tree edge. The flow-conservation solver (`derive_block_counts`)
    // then derived the latch's count from its outgoing edges = 0, silently
    // zeroing every loop-latch block count and the derived backedge count —
    // corrupting block layout, branch probability and profile-accurate loop
    // unrolling for every loop. A proper arborescence guarantees every
    // non-entry node (latch included) has exactly one incoming tree edge, so
    // the solver reconstructs exact counts.
    let tree = max_arborescence(nodes, edges, entry, &weight);
    let entry_edge = Edge {
        src: VENTRY,
        dst: entry,
    };
    let mut instr: Vec<Edge> = edges
        .iter()
        .copied()
        .filter(|e| !tree.contains(e))
        .collect();
    instr.push(entry_edge);
    instr.sort();
    instr
}

/// Chu–Liu/Edmonds maximum-weight directed spanning arborescence rooted at
/// `root` (which must be in `nodes`). Returns the set of TREE edges (edges NOT
/// instrumented). Weight is higher = more desirable to keep off the counter
/// path (hot backedges/cycle edges). The result is a valid directed tree: every
/// node except `root` has exactly one incoming tree edge and is reachable from
/// `root` via tree edges — exactly the invariant the flow-conservation solver
/// needs (see `choose_instrumented_edges`).
///
/// Implementation: start from a BFS arborescence (guaranteed correct), then
/// greedily replace each node's tree in-edge with a heavier in-edge whenever
/// that cannot create a cycle (v is not an ancestor of the new parent u).
/// Iterate to a fixed point. For the hierarchical backedge/loop/normal weight
/// scheme this yields the same tree as full Edmonds and is far simpler to keep
/// correct.
fn max_arborescence(
    nodes: &[u32],
    edges: &[Edge],
    root: u32,
    weight: &dyn Fn(&Edge) -> i64,
) -> FxHashSet<Edge> {
    use std::collections::VecDeque;
    let node_idx: FxHashMap<u32, usize> = nodes.iter().enumerate().map(|(i, n)| (*n, i)).collect();
    let n = nodes.len();
    let root_i = node_idx[&root];
    // Allowed edges: within the node set, not into root.
    let mut succs: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut in_edges: Vec<Vec<(usize, usize, i64)>> = vec![Vec::new(); n]; // dst -> (src,dst,w)
    for e in edges {
        let (Some(&a), Some(&b)) = (node_idx.get(&e.src), node_idx.get(&e.dst)) else {
            continue;
        };
        if b == root_i {
            continue; // no tree edge into the root
        }
        let w = weight(e);
        succs[a].push(b);
        in_edges[b].push((a, b, w));
    }
    // 1. BFS arborescence from root: guarantee a valid tree.
    let mut parent: Vec<usize> = vec![usize::MAX; n]; // parent[v] = u (edge u->v)
    {
        let mut q = VecDeque::new();
        q.push_back(root_i);
        while let Some(u) = q.pop_front() {
            for &b in &succs[u] {
                if parent[b] == usize::MAX {
                    parent[b] = u;
                    q.push_back(b);
                }
            }
        }
    }
    // Helper: is `anc` an ancestor of `node` in the current tree (walking up)?
    // Used to reject a swap that would create a cycle.
    fn is_ancestor(node: usize, anc: usize, parent: &[usize], n: usize) -> bool {
        let mut x = node;
        let mut steps = 0;
        while x != usize::MAX && steps <= n {
            if x == anc {
                return true;
            }
            x = parent[x];
            steps += 1;
        }
        false
    }
    // 2. Greedy improvement to fixed point.
    loop {
        let mut changed = false;
        for v in 0..n {
            if v == root_i {
                continue;
            }
            let cur = parent[v];
            if cur == usize::MAX {
                continue; // unreachable — leave as is (won't happen from BFS)
            }
            let mut cur_w = in_edges[v]
                .iter()
                .find(|(u, _, _)| *u == cur)
                .map(|(_, _, w)| *w)
                .unwrap_or(i64::MIN);
            for &(u, _, w) in &in_edges[v] {
                if u == cur || w <= cur_w {
                    continue;
                }
                // Replacing parent[v]=cur with parent[v]=u forms a cycle iff
                // u is already a DESCENDANT of v in the tree (a path v -> u
                // exists downward), because the new edge u->v would close it.
                // `is_ancestor(u, v)` returns true iff v is an ancestor of u.
                if is_ancestor(u, v, &parent, n) {
                    continue;
                }
                parent[v] = u;
                cur_w = w;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // Convert parent array to tree-edge set (for reachable nodes only).
    let mut tree = FxHashSet::default();
    for v in 0..n {
        if v == root_i || parent[v] == usize::MAX {
            continue;
        }
        tree.insert(Edge {
            src: nodes[parent[v]],
            dst: nodes[v],
        });
    }
    tree
}

pub fn instrument_module(
    m: &mut IrModule,
    dir: &str,
    unit: &str,
    update: Option<&str>,
    target: Target,
    pre_hashes: &FxHashMap<String, u64>,
    post_hashes: &FxHashMap<String, u64>,
) -> usize {
    if std::env::var_os("LCCC_PGO_NO_COUNTERS").is_some() {
        return 0;
    }
    if target != Target::X86_64 {
        eprintln!("lccc: PGO: instrumentation is x86-64 only; skipping");
        return 0;
    }
    let uid = unit_hash(unit);
    // Default: single-writer non-atomic increments (GCC/LLVM default).
    let atomic = update == Some("atomic")
        || std::env::var("LCCC_PGO_UPDATE")
            .map(|x| x == "atomic")
            .unwrap_or(false);
    let skip_funcs: Vec<String> = std::env::var("LCCC_PGO_SKIP_FUNC")
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let mut rec = Vec::new();
    let mut gs = Vec::new();
    let mut func_sites: FxHashMap<String, Vec<SiteRec>> = FxHashMap::default();
    let vp_recorder = format!("__lccc_pgo_vp_{:016x}", uid);
    // Block labels are MODULE-GLOBAL in this backend: the frontend assigns
    // them from a per-TU counter and the label-renumber pass compacts them
    // per TU (every `.LBB{id}` in the emitted assembly must be unique, or
    // branches bind to the wrong definition). Split blocks therefore need
    // labels above EVERY function's maximum in the module — a per-function
    // max_label would collide with another function's real block (observed:
    // pqdownheap's split `.LBB86` collided with build_tree's block 86, and
    // pqdownheap's `jl .LBB86` jumped into the middle of build_tree).
    let mut next_split_label: u32 = m
        .functions
        .iter()
        .flat_map(|f| f.blocks.iter().map(|b| b.label.0))
        .max()
        .map(|x| x + 1)
        .unwrap_or(0);
    for f in &mut m.functions {
        if f.is_declaration || f.blocks.is_empty() || f.name.starts_with("__lccc_pgo_dump_") {
            continue;
        }
        if skip_funcs.iter().any(|sf| f.name.contains(sf.as_str())) {
            continue;
        }
        // Identity: keyed by the PRE-pass fingerprint (stable across
        // gen/use even when profile-guided transforms change the CFG).
        let h0 = pre_hashes.get(&f.name).copied().unwrap_or(0);
        let h1 = post_hashes.get(&f.name).copied().unwrap_or(0);
        let cname = format!("__lccc_pgo_cnt_{:016x}_{:016x}", uid, h0);

        let nodes: Vec<u32> = f.blocks.iter().map(|b| b.label.0).collect();
        // Deduplicate CFG edges: a switch with two cases targeting the same
        // block (or a CondBranch with true == false) would otherwise emit two
        // counters sharing one slot via the (src,dst) slot map — the shared
        // slot double-counts and one counter slot is never written.
        let mut edge_set: FxHashSet<Edge> = FxHashSet::default();
        for b in &f.blocks {
            for s in successors(&b.terminator) {
                edge_set.insert(Edge {
                    src: b.label.0,
                    dst: s,
                });
            }
            // Virtual exit edge for RETURN-terminated blocks — closes
            // the flow equations at the exit so leaf counts derive correctly.
            if matches!(b.terminator, Terminator::Return(_)) {
                edge_set.insert(Edge {
                    src: b.label.0,
                    dst: VEXIT,
                });
            }
        }
        let edges: Vec<Edge> = edge_set.into_iter().collect();
        let entry = f.blocks.first().map(|b| b.label.0).unwrap_or(0);
        // The arborescence spans ALL nodes including the virtual EXIT so
        // it too gets a single incoming tree edge (instead of every
        // return->exit edge being instrumented) and the flow solver's
        // reconstruction stays exact.
        let mut tree_nodes = nodes.clone();
        tree_nodes.push(VEXIT);
        let instr_edges = choose_instrumented_edges(&tree_nodes, &edges, entry);
        let n_counters = instr_edges.len();
        let slot: FxHashMap<Edge, usize> = instr_edges
            .iter()
            .enumerate()
            .map(|(i, e)| (*e, i))
            .collect();

        // ── Critical edge splitting ──────────────────────────────────────
        let outdeg: FxHashMap<u32, usize> = {
            let mut m2 = FxHashMap::default();
            for e in &edges {
                *m2.entry(e.src).or_insert(0) += 1;
            }
            m2
        };
        let indeg: FxHashMap<u32, usize> = {
            let mut m2 = FxHashMap::default();
            for e in &edges {
                *m2.entry(e.dst).or_insert(0) += 1;
            }
            m2
        };
        // A block "ends with a fused Cmp pattern" when a trailing Cmp
        // (possibly followed by flag-neutral Copies) feeds the terminator's
        // CondBranch — the pending-cmp fusion skips setcc and the branch
        // consumes live flags. `incq` must never land between them.
        let ends_with_fused_cmp: FxHashSet<u32> = {
            let mut set = FxHashSet::default();
            for b in &f.blocks {
                let Terminator::CondBranch { cond, .. } = &b.terminator else {
                    continue;
                };
                let Operand::Value(condv) = cond else { continue };
                let mut cur = condv.0;
                let mut ok = false;
                for inst in b.instructions.iter().rev() {
                    match inst {
                        Instruction::Copy { dest, src: Operand::Value(v) } if dest.0 == cur => {
                            cur = v.0;
                        }
                        Instruction::Cmp { dest, .. } if dest.0 == cur => {
                            ok = true;
                            break;
                        }
                        _ => break,
                    }
                }
                if ok {
                    set.insert(b.label.0);
                }
            }
            set
        };
        let no_split = std::env::var("LCCC_PGO_NO_SPLIT").is_ok();
        let mut split_of: FxHashMap<Edge, u32> = FxHashMap::default();
        for e in &instr_edges {
            let critical = !no_split
                && e.src != VENTRY
                && e.src != e.dst
                && outdeg.get(&e.src).copied().unwrap_or(0) > 1
                && indeg.get(&e.dst).copied().unwrap_or(0) > 1;
            if critical {
                next_split_label += 1;
                split_of.insert(*e, next_split_label);
                f.blocks.push(BasicBlock {
                    label: BlockId(next_split_label),
                    instructions: vec![],
                    terminator: Terminator::Branch(BlockId(e.dst)),
                    source_spans: vec![],
                });
            }
        }
        // Rewire terminators through split blocks.
        for b in &mut f.blocks {
            let rewrite = |l: &mut BlockId| {
                let e = Edge {
                    src: b.label.0,
                    dst: l.0,
                };
                if let Some(&s) = split_of.get(&e) {
                    l.0 = s;
                }
            };
            match &mut b.terminator {
                Terminator::Branch(x) => rewrite(x),
                Terminator::CondBranch {
                    true_label,
                    false_label,
                    ..
                } => {
                    rewrite(true_label);
                    rewrite(false_label);
                }
                Terminator::Switch { cases, default, .. } => {
                    for (_, x) in cases.iter_mut() {
                        rewrite(x);
                    }
                    rewrite(default);
                }
                Terminator::IndirectBranch {
                    possible_targets, ..
                } => {
                    for x in possible_targets.iter_mut() {
                        rewrite(x);
                    }
                }
                _ => {}
            }
        }

        // ── Counter array ────────────────────────────────────────────────
        gs.push(IrGlobal {
            name: cname.clone(),
            ty: IrType::I64,
            size: n_counters * 8,
            align: 8,
            init: GlobalInit::Zero,
            is_static: true,
            is_extern: false,
            is_common: false,
            section: if std::env::var("LCCC_PGO_BSS_ARRAYS").is_ok() { None } else { Some(SEC.into()) },
            is_weak: false,
            visibility: Some("hidden".into()),
            has_explicit_align: true,
            is_const: false,
            is_used: true,
            is_thread_local: false,
        });

        // ── Insert increments ────────────────────────────────────────────
        // Placement: 1-succ source (and not fused-cmp-terminated) -> append
        // at source end; else destination start (single-pred) or a split
        // block. The virtual entry counter goes at the entry block start.
        let mut per_block: FxHashMap<u32, Vec<usize>> = FxHashMap::default();
        let no_insert = std::env::var("LCCC_PGO_NO_INSERT").is_ok();
        let entry_only = std::env::var("LCCC_PGO_ENTRY_ONLY").is_ok();
        if no_insert {
            // Debug: emit arrays + dump helpers but ZERO counter instructions.
            f.next_label = next_split_label + 1;
        } else {
        for (i, e) in instr_edges.iter().enumerate() {
            if entry_only && e.src != VENTRY {
                continue;
            }
            let place_in = if e.src == VENTRY {
                entry
            } else if std::env::var("LCCC_PGO_START_ONLY").is_ok() {
                // Debug: place ALL counters at the destination start.
                if let Some(&s) = split_of.get(e) { s } else { e.dst }
            } else if outdeg.get(&e.src).copied().unwrap_or(0) == 1
                && !ends_with_fused_cmp.contains(&e.src)
            {
                e.src
            } else if let Some(&s) = split_of.get(e) {
                s
            } else if indeg.get(&e.dst).copied().unwrap_or(0) == 1 {
                e.dst
            } else {
                // create a split block for this edge
                next_split_label += 1;
                let s = next_split_label;
                split_of.insert(*e, s);
                for b in &mut f.blocks {
                    if b.label.0 != e.src {
                        continue;
                    }
                    let mut rw = |l: &mut BlockId| {
                        if l.0 == e.dst {
                            l.0 = s;
                        }
                    };
                    match &mut b.terminator {
                        Terminator::Branch(x) => rw(x),
                        Terminator::CondBranch {
                            true_label,
                            false_label,
                            ..
                        } => {
                            rw(true_label);
                            rw(false_label);
                        }
                        Terminator::Switch { cases, default, .. } => {
                            for (_, x) in cases.iter_mut() {
                                rw(x);
                            }
                            rw(default);
                        }
                        _ => {}
                    }
                }
                f.blocks.push(BasicBlock {
                    label: BlockId(s),
                    instructions: vec![],
                    terminator: Terminator::Branch(BlockId(e.dst)),
                    source_spans: vec![],
                });
                s
            };
            per_block.entry(place_in).or_default().push(i);
        }
        } // !no_insert
        f.next_label = next_split_label + 1;
        for b in &mut f.blocks {
            let Some(slots) = per_block.remove(&b.label.0) else {
                continue;
            };
            let mut v = Vec::new();
            for slot_idx in slots {
                v.push(Instruction::PgoCounterInc {
                    name: cname.clone(),
                    offset: slot_idx as i64 * 8,
                    atomic,
                });
            }
            let pos = b
                .instructions
                .iter()
                .take_while(|i| matches!(i, Instruction::Phi { .. }))
                .count();
            for (k, x) in v.into_iter().enumerate() {
                b.instructions.insert(pos + k, x);
                if !b.source_spans.is_empty() {
                    b.source_spans
                        .insert(pos + k, crate::common::source::Span::dummy());
                }
            }
        }

        // Edge list (without the virtual entry edge), tagged with slot.
        let mut edge_list: Vec<(u32, u32, usize)> = Vec::new();
        let mut entry_slot = 0usize;
        for (i, e) in instr_edges.iter().enumerate() {
            if e.src == VENTRY {
                entry_slot = i;
            } else {
                edge_list.push((e.src, e.dst, i));
            }
        }
        let _ = slot;

        // ── v7 indirect-call value profiling ─────────────────────────────
        // Each CallIndirect site gets a 72-byte site global and a call to the
        // per-TU recorder immediately before the call: record(site, fp).
        // The recorder keeps the top-4 callee addresses (evicting the least
        // frequent slot). Site ordinals count CallIndirect instructions in
        // block order — stable across phi elimination, so the use-side
        // promotion pass (which runs pre-phi) sees the same ordinals.
        let mut site_recs: Vec<SiteRec> = Vec::new();
        if std::env::var("LCCC_PGO_NO_VALUE_PROF").is_err() {
            let mut ordinal = 0usize;
            let mut next_val = f.next_value_id;
            for b in &mut f.blocks {
                let mut k = 0usize;
                while k < b.instructions.len() {
                    let ind = matches!(b.instructions[k], Instruction::CallIndirect { .. });
                    if ind {
                        let (fp, sig) = match &b.instructions[k] {
                            Instruction::CallIndirect { func_ptr, info } => match func_ptr {
                                Operand::Value(v) => (
                                    *v,
                                    site_signature(info),
                                ),
                                _ => {
                                    ordinal += 1;
                                    k += 1;
                                    continue;
                                }
                            },
                            _ => unreachable!(),
                        };
                        let site =
                            format!("__lccc_pgo_vp_{:016x}_{:016x}_{}", uid, h0, ordinal);
                        let sa = Value(next_val);
                        next_val += 1;
                        let g = IrGlobal {
                            name: site.clone(),
                            ty: IrType::I64,
                            size: 72,
                            align: 8,
                            init: GlobalInit::Zero,
                            is_static: true,
                            is_extern: false,
                            is_common: false,
                            section: Some(SEC_VP.into()),
                            is_weak: false,
                            visibility: Some("hidden".into()),
                            has_explicit_align: true,
                            is_const: false,
                            is_used: true,
                            is_thread_local: false,
                        };
                        if !m.globals.iter().any(|x| x.name == g.name) {
                            m.globals.push(g);
                        }
                        b.instructions.insert(
                            k,
                            Instruction::GlobalAddr {
                                dest: sa,
                                name: site.clone(),
                            },
                        );
                        b.instructions.insert(
                            k + 1,
                            Instruction::Call {
                                func: vp_recorder.clone(),
                                info: ci(
                                    None,
                                    vec![Operand::Value(sa), Operand::Value(fp)],
                                    vec![IrType::Ptr, IrType::I64],
                                    IrType::I32,
                                    false,
                                ),
                            },
                        );
                        if !b.source_spans.is_empty() {
                            b.source_spans
                                .insert(k, crate::common::source::Span::dummy());
                            b.source_spans
                                .insert(k + 1, crate::common::source::Span::dummy());
                        }
                        k += 2;
                        site_recs.push(SiteRec {
                            ordinal,
                            site,
                            sig,
                        });
                        ordinal += 1;
                    }
                    k += 1;
                }
            }
            f.next_value_id = next_val;
        }

        rec.push((
            function_key(unit, &f.name),
            cname,
            edge_list,
            entry_slot,
            h0,
            h1,
            entry,
            uid,
        ));
        if !site_recs.is_empty() {
            func_sites.insert(function_key(unit, &f.name), site_recs);
        }
    }
    for g in gs {
        if std::env::var("LCCC_PGO_NO_ARRAYS").is_ok() {
            break;
        }
        if !m.globals.iter().any(|x| x.name == g.name) {
            m.globals.push(g)
        }
    }
    if rec.is_empty() && func_sites.is_empty() {
        return 0;
    }
    let path = resolve_output_path(dir, unit);
    let helper = format!("__lccc_pgo_dump_{:016x}_{}", uid, std::process::id());
    if std::env::var("LCCC_PGO_NO_DUMP").is_err() {
        emit_value_prof_helpers(m, uid, &vp_recorder);
        let lookup_name = format!("__lccc_pgo_lookup_{:016x}", uid);
        dump_helper(m, &path, &rec, &helper, &func_sites, &lookup_name);
    }
    eprintln!(
        "lccc: PGO: instrumented {} functions ({} value-profile sites) into {}",
        rec.len(),
        func_sites.values().map(|v| v.len()).sum::<usize>(),
        path.display()
    );
    rec.len()
}

fn dump_helper(
    m: &mut IrModule,
    path: &std::path::Path,
    rec: &[Rec],
    name: &str,
    func_sites: &FxHashMap<String, Vec<SiteRec>>,
    lookup_name: &str,
) {
    let tag = format!("__lccc_pgo_path_{}", m.functions.len());
    let mode = format!("__lccc_pgo_mode_{}", m.functions.len());
    let hdr = format!("__lccc_pgo_hdr_{}", m.functions.len());
    let pair = format!("__lccc_pgo_pair_{}", m.functions.len());
    let fpair = format!("__lccc_pgo_fpair_{}", m.functions.len());
    let vp_fmt = format!("__lccc_pgo_vp_fmt_{}", m.functions.len());
    m.string_literals
        .push((tag.clone(), path.display().to_string()));
    m.string_literals.push((mode.clone(), "a".into()));
    // Header is written as ONE line by two <=2-vararg calls (the backend's
    // variadic codegen drops register args 3+ — observed: 4-vararg fprintf
    // wrote uid=0). fmt1 ends with a space so fmt2 continues the line.
    m.string_literals
        .push((hdr.clone(), "lccc-pgo-v1 %llu %llu ".into()));
    let hdr2 = format!("__lccc_pgo_hdr2_{}", m.functions.len());
    m.string_literals
        .push((hdr2.clone(), "%u %llu\nfunc %s\n".into()));
    m.string_literals.push((pair.clone(), "e %u %u %llu\n".into()));
    m.string_literals.push((fpair.clone(), "f %llu\n".into()));
    m.string_literals
        .push((vp_fmt.clone(), "v %u %llu %s\n".into()));
    let vp_fmt2 = format!("__lccc_pgo_vp_fmt2_{}", m.functions.len());
    m.string_literals
        .push((vp_fmt2.clone(), "%s %llu %llu\n".into()));
    let mut names = Vec::new();
    for (r, _, _, _, _, _, _, _) in rec {
        let x = format!("__lccc_pgo_name_{}_{}", sanitize(r), m.functions.len());
        m.string_literals.push((x.clone(), r.clone()));
        names.push(x)
    }
    let mut next = 6000;
    let file = Value(next);
    next += 1;
    let pp = Value(next);
    next += 1;
    let mm = Value(next);
    next += 1;
    let fd = Value(next);
    next += 1;
    // Entry: resolve the output path — LCCC_PROFILE_FILE or (LLVM
    // convention) LLVM_PROFILE_FILE override the compile-time path at
    // RUNTIME, so training runs can redirect/merge profiles without a
    // rebuild (LLVM_PROFILE_FILE / GCC GCOV_PREFIX behaviour). The value is
    // used verbatim as the output file path.
    let skip_l = BlockId(910099);
    let main_l = BlockId(910010);
    let pathv = Value(next);
    next += 1;
    let e1 = Value(next);
    next += 1;
    let e1z = Value(next);
    next += 1;
    let e2 = Value(next);
    next += 1;
    let e2z = Value(next);
    next += 1;
    let is_null = Value(next);
    next += 1;
    let env1_sym = format!("__lccc_pgo_env1_{}", m.functions.len());
    let env2_sym = format!("__lccc_pgo_env2_{}", m.functions.len());
    m.string_literals
        .push((env1_sym.clone(), "LCCC_PROFILE_FILE".into()));
    m.string_literals
        .push((env2_sym.clone(), "LLVM_PROFILE_FILE".into()));
    let mut b0: Vec<Instruction> = Vec::new();
    let e1p = Value(next);
    next += 1;
    b0.push(Instruction::GlobalAddr {
        dest: e1p,
        name: env1_sym.clone(),
    });
    b0.push(Instruction::Call {
        func: "getenv".into(),
        info: ci(
            Some(e1),
            vec![Operand::Value(e1p)],
            vec![IrType::Ptr],
            IrType::Ptr,
            false,
        ),
    });
    b0.push(Instruction::Cmp {
        dest: e1z,
        op: crate::ir::ops::IrCmpOp::Ne,
        lhs: Operand::Value(e1),
        rhs: Operand::Const(IrConst::I64(0)),
        ty: IrType::I64,
    });
    let mut b1: Vec<Instruction> = Vec::new();
    let e2p = Value(next);
    next += 1;
    b1.push(Instruction::GlobalAddr {
        dest: e2p,
        name: env2_sym.clone(),
    });
    b1.push(Instruction::Call {
        func: "getenv".into(),
        info: ci(
            Some(e2),
            vec![Operand::Value(e2p)],
            vec![IrType::Ptr],
            IrType::Ptr,
            false,
        ),
    });
    b1.push(Instruction::Cmp {
        dest: e2z,
        op: crate::ir::ops::IrCmpOp::Ne,
        lhs: Operand::Value(e2),
        rhs: Operand::Const(IrConst::I64(0)),
        ty: IrType::I64,
    });
    let use_env = BlockId(910002);
    let use_env2 = BlockId(910003);
    let use_tag = BlockId(910004);
    let open_l = BlockId(910005);
    let mut b2: Vec<Instruction> = Vec::new();
    b2.push(Instruction::Copy {
        dest: pathv,
        src: Operand::Value(e1),
    });
    let mut b3: Vec<Instruction> = Vec::new();
    b3.push(Instruction::Copy {
        dest: pathv,
        src: Operand::Value(e2),
    });
    let mut b4: Vec<Instruction> = Vec::new();
    b4.push(Instruction::GlobalAddr {
        dest: pathv,
        name: tag,
    });
    let mut b5: Vec<Instruction> = Vec::new();
    b5.push(Instruction::GlobalAddr {
        dest: mm,
        name: mode,
    });
    b5.push(Instruction::Call {
        func: "fopen".into(),
        info: ci(
            Some(file),
            vec![Operand::Value(pathv), Operand::Value(mm)],
            vec![IrType::Ptr, IrType::Ptr],
            IrType::Ptr,
            false,
        ),
    });
    b5.push(Instruction::Cmp {
        dest: is_null,
        op: crate::ir::ops::IrCmpOp::Eq,
        lhs: Operand::Value(file),
        rhs: Operand::Const(IrConst::I64(0)),
        ty: IrType::I64,
    });
    let mk = |label: u32, is: Vec<Instruction>, term: Terminator| BasicBlock {
        label: BlockId(label),
        instructions: is,
        terminator: term,
        source_spans: vec![],
    };
    let entry_block = mk(
        910000,
        b0,
        Terminator::CondBranch {
            cond: Operand::Value(e1z),
            true_label: use_env,
            false_label: BlockId(910001),
        },
    );
    let env2_block = mk(
        910001,
        b1,
        Terminator::CondBranch {
            cond: Operand::Value(e2z),
            true_label: use_env2,
            false_label: use_tag,
        },
    );
    let use_env_block = mk(910002, b2, Terminator::Branch(open_l));
    let use_env2_block = mk(910003, b3, Terminator::Branch(open_l));
    let use_tag_block = mk(910004, b4, Terminator::Branch(open_l));
    let open_block = mk(
        910005,
        b5,
        Terminator::CondBranch {
            cond: Operand::Value(is_null),
            true_label: skip_l,
            false_label: main_l,
        },
    );
    let skip_block = BasicBlock {
        label: skip_l,
        instructions: vec![],
        terminator: Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
        source_spans: vec![],
    };

    // Main dump block.
    let mut is: Vec<Instruction> = Vec::new();
    is.push(Instruction::Call {
        func: "fileno".into(),
        info: ci(
            Some(fd),
            vec![Operand::Value(file)],
            vec![IrType::Ptr],
            IrType::I32,
            false,
        ),
    });
    is.push(Instruction::Call {
        func: "flock".into(),
        info: ci(
            None,
            vec![Operand::Value(fd), Operand::Const(IrConst::I32(2))],
            vec![IrType::I32, IrType::I32],
            IrType::I32,
            false,
        ),
    });
    for (i, (fkey, cnt, edges, entry_slot, h0, h1, entry, uid)) in rec.iter().enumerate() {
        let np = Value(next);
        next += 1;
        let hp = Value(next);
        next += 1;
        let bp = Value(next);
        next += 1;
        is.push(Instruction::GlobalAddr {
            dest: np,
            name: names[i].clone(),
        });
        is.push(Instruction::GlobalAddr {
            dest: hp,
            name: hdr.clone(),
        });
        is.push(Instruction::Call {
            func: "fprintf".into(),
            info: ci(
                None,
                vec![
                    Operand::Value(file),
                    Operand::Value(hp),
                    Operand::Const(IrConst::I64(*h0 as i64)),
                    Operand::Const(IrConst::I64(*h1 as i64)),
                ],
                vec![IrType::Ptr, IrType::Ptr, IrType::I64, IrType::I64],
                IrType::I32,
                true,
            ),
        });
        let hp2 = Value(next);
        next += 1;
        is.push(Instruction::GlobalAddr {
            dest: hp2,
            name: hdr2.clone(),
        });
        is.push(Instruction::Call {
            func: "fprintf".into(),
            info: ci(
                None,
                vec![
                    Operand::Value(file),
                    Operand::Value(hp2),
                    Operand::Const(IrConst::I32(*entry as i32)),
                    Operand::Const(IrConst::I64(*uid as i64)),
                    Operand::Value(np),
                ],
                vec![
                    IrType::Ptr,
                    IrType::Ptr,
                    IrType::I32,
                    IrType::I64,
                    IrType::Ptr,
                ],
                IrType::I32,
                true,
            ),
        });
        is.push(Instruction::GlobalAddr {
            dest: bp,
            name: cnt.clone(),
        });
        // Function entry count (virtual entry edge's slot).
        let fcnt = Value(next);
        next += 1;
        let fp_fmt = Value(next);
        next += 1;
        let fg = Value(next);
        next += 1;
        is.push(Instruction::GetElementPtr {
            dest: fg,
            base: bp,
            offset: Operand::Const(IrConst::I64((*entry_slot * 8) as i64)),
            ty: IrType::I64,
        });
        is.push(Instruction::Load {
            dest: fcnt,
            ptr: fg,
            ty: IrType::I64,
            seg_override: crate::common::types::AddressSpace::Default,
        });
        is.push(Instruction::GlobalAddr {
            dest: fp_fmt,
            name: fpair.clone(),
        });
        is.push(Instruction::Call {
            func: "fprintf".into(),
            info: ci(
                None,
                vec![
                    Operand::Value(file),
                    Operand::Value(fp_fmt),
                    Operand::Value(fcnt),
                ],
                vec![IrType::Ptr, IrType::Ptr, IrType::I64],
                IrType::I32,
                true,
            ),
        });
        for (src, dst, slot) in edges.iter() {
            let lp = Value(next);
            next += 1;
            let fmt = Value(next);
            next += 1;
            is.push(Instruction::GetElementPtr {
                dest: lp,
                base: bp,
                offset: Operand::Const(IrConst::I64((*slot * 8) as i64)),
                ty: IrType::I64,
            });
            is.push(Instruction::Load {
                dest: fmt,
                ptr: lp,
                ty: IrType::I64,
                seg_override: crate::common::types::AddressSpace::Default,
            });
            let fp2 = Value(next);
            next += 1;
            is.push(Instruction::GlobalAddr {
                dest: fp2,
                name: pair.clone(),
            });
            is.push(Instruction::Call {
                func: "fprintf".into(),
                info: ci(
                    None,
                    vec![
                        Operand::Value(file),
                        Operand::Value(fp2),
                        Operand::Const(IrConst::I32(*src as i32)),
                        Operand::Const(IrConst::I32(*dst as i32)),
                        Operand::Value(fmt),
                    ],
                    vec![IrType::Ptr, IrType::Ptr, IrType::I32, IrType::I32, IrType::I64],
                    IrType::I32,
                    true,
                ),
            })
        }
        // Indirect-call value-profile lines. Four slots per site; empty
        // slots carry count 0 (parser skips them; lookup(0) -> "?").
        if let Some(sites) = func_sites.get(fkey) {
            for sr in sites {
                let sp = Value(next);
                next += 1;
                let totp = Value(next);
                next += 1;
                let tot = Value(next);
                next += 1;
                is.push(Instruction::GlobalAddr {
                    dest: sp,
                    name: sr.site.clone(),
                });
                is.push(Instruction::GetElementPtr {
                    dest: totp,
                    base: sp,
                    offset: Operand::Const(IrConst::I64(64)),
                    ty: IrType::I64,
                });
                is.push(Instruction::Load {
                    dest: tot,
                    ptr: totp,
                    ty: IrType::I64,
                    seg_override: crate::common::types::AddressSpace::Default,
                });
                let sig_sym = format!(
                    "__lccc_pgo_sig_{}_{}",
                    sanitize(fkey),
                    m.functions.len()
                );
                m.string_literals.push((sig_sym.clone(), sr.sig.clone()));
                for slot in 0..4usize {
                    let ap = Value(next);
                    next += 1;
                    let av = Value(next);
                    next += 1;
                    let cp = Value(next);
                    next += 1;
                    let cv = Value(next);
                    next += 1;
                    let nm = Value(next);
                    next += 1;
                    let nm2 = Value(next);
                    next += 1;
                    let fl = Value(next);
                    next += 1;
                    let fl2 = Value(next);
                    next += 1;
                    let sp2 = Value(next);
                    next += 1;
                    is.push(Instruction::GetElementPtr {
                        dest: ap,
                        base: sp,
                        offset: Operand::Const(IrConst::I64((slot * 8) as i64)),
                        ty: IrType::I64,
                    });
                    is.push(Instruction::Load {
                        dest: av,
                        ptr: ap,
                        ty: IrType::I64,
                        seg_override: crate::common::types::AddressSpace::Default,
                    });
                    is.push(Instruction::GetElementPtr {
                        dest: cp,
                        base: sp,
                        offset: Operand::Const(IrConst::I64((32 + slot * 8) as i64)),
                        ty: IrType::I64,
                    });
                    is.push(Instruction::Load {
                        dest: cv,
                        ptr: cp,
                        ty: IrType::I64,
                        seg_override: crate::common::types::AddressSpace::Default,
                    });
                    is.push(Instruction::Call {
                        func: lookup_name.into(),
                        info: ci(
                            Some(nm),
                            vec![Operand::Value(av)],
                            vec![IrType::I64],
                            IrType::Ptr,
                            false,
                        ),
                    });
                    // name = ep + 8, flags = ep + 16 (ep is the lookup result)
                    let ep2 = nm;
                    is.push(Instruction::GetElementPtr {
                        dest: nm2,
                        base: ep2,
                        offset: Operand::Const(IrConst::I64(8)),
                        ty: IrType::I64,
                    });
                    is.push(Instruction::Load {
                        dest: nm2,
                        ptr: nm2,
                        ty: IrType::I64,
                        seg_override: crate::common::types::AddressSpace::Default,
                    });
                    is.push(Instruction::GetElementPtr {
                        dest: fl2,
                        base: ep2,
                        offset: Operand::Const(IrConst::I64(16)),
                        ty: IrType::I64,
                    });
                    is.push(Instruction::Load {
                        dest: fl,
                        ptr: fl2,
                        ty: IrType::I64,
                        seg_override: crate::common::types::AddressSpace::Default,
                    });
                    is.push(Instruction::GlobalAddr {
                        dest: sp2,
                        name: sig_sym.clone(),
                    });
                    let vpf = Value(next);
                    next += 1;
                    is.push(Instruction::GlobalAddr {
                        dest: vpf,
                        name: vp_fmt.clone(),
                    });
                    is.push(Instruction::Call {
                        func: "fprintf".into(),
                        info: ci(
                            None,
                            vec![
                                Operand::Value(file),
                                Operand::Value(vpf),
                                Operand::Const(IrConst::I32(sr.ordinal as i32)),
                                Operand::Value(tot),
                                Operand::Value(sp2),
                            ],
                            vec![
                                IrType::Ptr,
                                IrType::Ptr,
                                IrType::I32,
                                IrType::I64,
                                IrType::Ptr,
                            ],
                            IrType::I32,
                            true,
                        ),
                    });
                    let vpf2 = Value(next);
                    next += 1;
                    is.push(Instruction::GlobalAddr {
                        dest: vpf2,
                        name: vp_fmt2.clone(),
                    });
                    is.push(Instruction::Call {
                        func: "fprintf".into(),
                        info: ci(
                            None,
                            vec![
                                Operand::Value(file),
                                Operand::Value(vpf2),
                                Operand::Value(nm2),
                                Operand::Value(cv),
                                Operand::Value(fl),
                            ],
                            vec![
                                IrType::Ptr,
                                IrType::Ptr,
                                IrType::Ptr,
                                IrType::I64,
                                IrType::I64,
                            ],
                            IrType::I32,
                            true,
                        ),
                    });
                }
            }
        }
    }
    is.push(Instruction::Call {
        func: "flock".into(),
        info: ci(
            None,
            vec![Operand::Value(fd), Operand::Const(IrConst::I32(8))],
            vec![IrType::I32, IrType::I32],
            IrType::I32,
            false,
        ),
    });
    is.push(Instruction::Call {
        func: "fclose".into(),
        info: ci(
            None,
            vec![Operand::Value(file)],
            vec![IrType::Ptr],
            IrType::I32,
            false,
        ),
    });
    let f = IrFunction {
        name: name.into(),
        return_type: IrType::I32,
        params: vec![],
        blocks: vec![
            entry_block,
            env2_block,
            use_env_block,
            use_env2_block,
            use_tag_block,
            open_block,
            BasicBlock {
                label: main_l,
                instructions: is,
                terminator: Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
                source_spans: vec![],
            },
            skip_block,
        ],
        is_variadic: false,
        is_declaration: false,
        is_static: true,
        is_inline: false,
        is_always_inline: false,
        is_noinline: true,
        next_value_id: next,
        next_label: 910011,
        section: None,
        visibility: Some("hidden".into()),
        is_weak: false,
        is_used: true,
        has_inlined_calls: false,
        param_alloca_values: vec![],
        uses_sret: false,
        is_fastcall: false,
        is_naked: false,
        global_init_label_blocks: vec![],
        ret_eightbyte_classes: vec![],
        ret_is_f128_sse: false,
        is_gnu_inline_def: false,
    };
    m.functions.push(f);
    m.destructors.push(name.into());
    for (n, t, v) in [
        ("getenv", IrType::Ptr, false),
        ("fopen", IrType::Ptr, true),
        ("fileno", IrType::I32, false),
        ("flock", IrType::I32, false),
        ("fprintf", IrType::I32, true),
        ("fclose", IrType::I32, false),
    ] {
        if !m.functions.iter().any(|f| f.name == n) {
            m.functions.push(IrFunction {
                name: n.into(),
                return_type: t,
                params: vec![],
                blocks: vec![],
                is_variadic: v,
                is_declaration: true,
                is_static: false,
                is_inline: false,
                is_always_inline: false,
                is_noinline: false,
                next_value_id: 0,
                next_label: 0,
                section: None,
                visibility: None,
                is_weak: false,
                is_used: false,
                has_inlined_calls: false,
                param_alloca_values: vec![],
                uses_sret: false,
                is_fastcall: false,
                is_naked: false,
                global_init_label_blocks: vec![],
                ret_eightbyte_classes: vec![],
                ret_is_f128_sse: false,
                is_gnu_inline_def: false,
            })
        }
    }
}
fn ci(d: Option<Value>, a: Vec<Operand>, t: Vec<IrType>, r: IrType, var: bool) -> CallInfo {
    let n = a.len();
    CallInfo {
        dest: d,
        args: a,
        arg_types: t,
        return_type: r,
        is_variadic: var,
        num_fixed_args: if var { 2 } else { n },
        struct_arg_sizes: vec![None; n],
        struct_arg_aligns: vec![None; n],
        struct_arg_classes: vec![Vec::new(); n],
        struct_arg_riscv_float_classes: vec![None; n],
        struct_arg_is_f128_sse: Vec::new(),
        is_sret: false,
        is_fastcall: false,
        ret_eightbyte_classes: vec![],
        ret_is_f128_sse: false,
    }
}
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}


/// Build a tiny standalone helper function (IR-built runtime for PGO value
/// profiling). Params follow the frontend's exact lowering pattern — Alloca +
/// ParamRef + Store in the entry block — because the backend's prologue
/// (param pre-store analysis) expects parameter values to be stored to their
/// allocas; IR-built functions that skipped the allocas produced wrong code
/// (observed: reg_add stored an uninitialized register as the table pointer,
/// corrupting the registry and crashing at startup). Body uses of the raw
/// ParamRef values are rewritten to loads from the allocas.
fn push_helper_fn(
    m: &mut IrModule,
    name: &str,
    params: Vec<IrType>,
    mut blocks: Vec<BasicBlock>,
    next_value_id: u32,
    label_base: u32,
) {
    let ps = params
        .iter()
        .map(|&ty| crate::ir::reexports::IrParam {
            ty,
            struct_size: None,
            struct_align: None,
            struct_eightbyte_classes: vec![],
            is_f128_sse: false,
            riscv_float_class: None,
        })
        .collect();
    // Frontend param lowering: allocas + ParamRef + Store in the entry block.
    let mut next = next_value_id;
    let mut entry_is: Vec<Instruction> = Vec::new();
    let mut param_allocas: Vec<Value> = Vec::new();
    let mut param_loads: Vec<Value> = Vec::new();
    for (i, &ty) in params.iter().enumerate() {
        let a = Value(next);
        next += 1;
        let l = Value(next);
        next += 1;
        entry_is.push(Instruction::Alloca {
            dest: a,
            ty,
            size: ty.size(),
            align: 0,
            volatile: false,
            semantic_volatile: false,
        });
        entry_is.push(Instruction::ParamRef {
            dest: Value(i as u32),
            param_idx: i,
            ty,
        });
        entry_is.push(Instruction::Store {
            val: Operand::Value(Value(i as u32)),
            ptr: a,
            ty,
            seg_override: crate::common::types::AddressSpace::Default,
        });
        entry_is.push(Instruction::Load {
            dest: l,
            ptr: a,
            ty,
            seg_override: crate::common::types::AddressSpace::Default,
        });
        param_allocas.push(a);
        param_loads.push(l);
    }
    if !param_loads.is_empty() {
        // Rewrite body references to the raw ParamRef values (Value(0..n))
        // to the alloca loads. The helpers use a small instruction set, so a
        // targeted match is sufficient and safe.
        let mut remap_op = |op: &mut Operand| {
            if let Operand::Value(v) = op {
                if let Some(&l) = param_loads.get(v.0 as usize) {
                    *op = Operand::Value(l);
                }
            }
        };
        let mut remap_dest = |d: &mut Value| {
            if let Some(&l) = param_loads.get(d.0 as usize) {
                *d = l;
            }
        };
        for b in blocks.iter_mut() {
            for inst in b.instructions.iter_mut() {
                match inst {
                    Instruction::Store {
                        val, ptr, ty, seg_override,
                    } => {
                        let _ = ty;
                        let _ = seg_override;
                        remap_op(val);
                        remap_dest(ptr);
                    }
                    Instruction::Load { dest, ptr, .. } => {
                        remap_dest(dest);
                        remap_dest(ptr);
                    }
                    Instruction::GetElementPtr { dest, base, offset, .. } => {
                        remap_dest(dest);
                        remap_dest(base);
                        remap_op(offset);
                    }
                    Instruction::BinOp {
                        dest, lhs, rhs, ..
                    } => {
                        remap_dest(dest);
                        remap_op(lhs);
                        remap_op(rhs);
                    }
                    Instruction::Cmp { dest, lhs, rhs, .. } => {
                        remap_dest(dest);
                        remap_op(lhs);
                        remap_op(rhs);
                    }
                    Instruction::Select {
                        dest,
                        cond,
                        true_val,
                        false_val,
                        ..
                    } => {
                        remap_dest(dest);
                        remap_op(cond);
                        remap_op(true_val);
                        remap_op(false_val);
                    }
                    Instruction::Copy { dest, src } => {
                        remap_dest(dest);
                        remap_op(src);
                    }
                    Instruction::GlobalAddr { dest, .. } => remap_dest(dest),
                    Instruction::Call { info, .. } => {
                        if let Some(d) = info.dest.as_mut() {
                            remap_dest(d);
                        }
                        for a in info.args.iter_mut() {
                            remap_op(a);
                        }
                    }
                    Instruction::CallIndirect { func_ptr, info } => {
                        remap_op(func_ptr);
                        if let Some(d) = info.dest.as_mut() {
                            remap_dest(d);
                        }
                        for a in info.args.iter_mut() {
                            remap_op(a);
                        }
                    }
                    Instruction::Phi { dest, incoming, .. } => {
                        remap_dest(dest);
                        for (op, _) in incoming.iter_mut() {
                            remap_op(op);
                        }
                    }
                    _ => {}
                }
            }
            match &mut b.terminator {
                Terminator::CondBranch { cond, .. } => remap_op(cond),
                Terminator::Return(Some(v)) => remap_op(v),
                _ => {}
            }
        }
        // Prepend the param setup to the entry block.
        let entry = &mut blocks[0];
        let mut is = entry_is;
        is.append(&mut entry.instructions);
        entry.instructions = is;
    }
    // Block labels are TU-global in the emitted assembly: every `.LBB{id}`
    // must be unique across ALL functions (observed: a helper reusing label
    // 18 made work()'s loop-exit `jge` bind to a block INSIDE the vp
    // recorder — cross-function jump, immediate SIGSEGV). Helpers get a
    // module-global label base.
    for b in blocks.iter_mut() {
        b.label.0 += label_base;
        let add = |l: &mut BlockId| l.0 += label_base;
        match &mut b.terminator {
            Terminator::Branch(x) => add(x),
            Terminator::CondBranch {
                true_label,
                false_label,
                ..
            } => {
                add(true_label);
                add(false_label);
            }
            Terminator::Switch { cases, default, .. } => {
                for (_, x) in cases.iter_mut() {
                    add(x);
                }
                add(default);
            }
            Terminator::IndirectBranch {
                possible_targets, ..
            } => {
                for x in possible_targets.iter_mut() {
                    add(x);
                }
            }
            _ => {}
        }
    }
    let next_label = blocks
        .iter()
        .map(|b| b.label.0)
        .max()
        .map(|x| x + 1)
        .unwrap_or(label_base + 1);
    m.functions.push(IrFunction {
        name: name.into(),
        return_type: IrType::I32,
        params: ps,
        blocks,
        is_variadic: false,
        is_declaration: false,
        is_static: true,
        is_inline: false,
        is_always_inline: false,
        is_noinline: true,
        next_value_id,
        next_label,
        section: None,
        visibility: Some("hidden".into()),
        is_weak: false,
        is_used: true,
        has_inlined_calls: false,
        param_alloca_values: param_allocas,
        uses_sret: false,
        is_fastcall: false,
        is_naked: false,
        global_init_label_blocks: vec![],
        ret_eightbyte_classes: vec![],
        ret_is_f128_sse: false,
        is_gnu_inline_def: false,
    });
}

struct VpBuilder {
    next: u32,
    is: Vec<Instruction>,
}
impl VpBuilder {
    fn new(start: u32) -> Self {
        VpBuilder {
            next: start,
            is: Vec::new(),
        }
    }
    fn v(&mut self) -> Value {
        let x = Value(self.next);
        self.next += 1;
        x
    }
    fn push(&mut self, i: Instruction) {
        self.is.push(i);
    }
    fn gep(&mut self, base: Value, off: i64) -> Value {
        let d = self.v();
        self.push(Instruction::GetElementPtr {
            dest: d,
            base,
            offset: Operand::Const(IrConst::I64(off)),
            ty: IrType::I64,
        });
        d
    }
    fn load(&mut self, ptr: Value) -> Value {
        let d = self.v();
        self.push(Instruction::Load {
            dest: d,
            ptr,
            ty: IrType::I64,
            seg_override: crate::common::types::AddressSpace::Default,
        });
        d
    }
    fn store(&mut self, ptr: Value, val: Operand) {
        self.push(Instruction::Store {
            val,
            ptr,
            ty: IrType::I64,
            seg_override: crate::common::types::AddressSpace::Default,
        });
    }
    fn add(&mut self, a: Operand, b: Operand) -> Value {
        let d = self.v();
        self.push(Instruction::BinOp {
            dest: d,
            op: crate::ir::reexports::IrBinOp::Add,
            lhs: a,
            rhs: b,
            ty: IrType::I64,
        });
        d
    }
    fn mul(&mut self, a: Operand, b: Operand) -> Value {
        let d = self.v();
        self.push(Instruction::BinOp {
            dest: d,
            op: crate::ir::reexports::IrBinOp::Mul,
            lhs: a,
            rhs: b,
            ty: IrType::I64,
        });
        d
    }
    fn cmp(&mut self, op: crate::ir::ops::IrCmpOp, a: Operand, b: Operand) -> Value {
        let d = self.v();
        self.push(Instruction::Cmp {
            dest: d,
            op,
            lhs: a,
            rhs: b,
            ty: IrType::I64,
        });
        d
    }
    fn copy(&mut self, dest: Value, src: Operand) {
        self.push(Instruction::Copy { dest, src });
    }
    fn sel(&mut self, cond: Operand, t: Operand, f: Operand) -> Value {
        let d = self.v();
        self.push(Instruction::Select {
            dest: d,
            cond,
            true_val: t,
            false_val: f,
            ty: IrType::I64,
        });
        d
    }
}

/// Per-TU runtime for indirect-call value profiling:
///   * `__lccc_pgo_registry`  — cross-TU linked-list head (common symbol,
///     merged by the linker so any TU's dump can resolve any TU's targets).
///   * `__lccc_pgo_pool_<uid>` — one node {next, table, count}.
///   * `__lccc_pgo_an_<uid>`   — {addr, name} data pairs for every defined
///     function in this TU.
///   * `__lccc_pgo_reg_add_<uid>(table, count)` + a constructor that
///     registers the table (runs at startup via .init_array).
///   * `__lccc_pgo_vp_<uid>(site, fp)` — recorder: linear-scan 4 slots,
///     insert into an empty slot or evict the least frequent slot.
///   * `__lccc_pgo_lookup_<uid>(addr)` — resolve a recorded target address
///     to its function-name string via the registry.
fn emit_value_prof_helpers(m: &mut IrModule, uid: u64, vp_recorder: &str) {
    let registry = "__lccc_pgo_registry".to_string();
    let pool = format!("__lccc_pgo_pool_{:016x}", uid);
    let table = format!("__lccc_pgo_an_{:016x}", uid);
    let reg_add = format!("__lccc_pgo_reg_add_{:016x}", uid);
    let ctor = format!("__lccc_pgo_ctor_{:016x}", uid);
    let lookup = format!("__lccc_pgo_lookup_{:016x}", uid);
    let none = format!("__lccc_pgo_none_{:016x}", uid);
    {
        let q = format!("__lccc_pgo_q_{:016x}", uid);
        m.string_literals.push((q.clone(), "?".into()));
        m.globals.push(IrGlobal {
            name: none.clone(),
            ty: IrType::I64,
            size: 24,
            align: 8,
            init: GlobalInit::Compound(vec![
                GlobalInit::Scalar(IrConst::I64(0)),
                GlobalInit::GlobalAddr(q),
                GlobalInit::Scalar(IrConst::I64(0)),
            ]),
            is_static: true,
            is_extern: false,
            is_common: false,
            section: None,
            is_weak: false,
            visibility: Some("hidden".into()),
            has_explicit_align: true,
            is_const: true,
            is_used: true,
            is_thread_local: false,
        });
    }

    // Module-global label base for helper functions (see push_helper_fn).
    let mut label_base: u32 = m
        .functions
        .iter()
        .flat_map(|f| f.blocks.iter().map(|b| b.label.0))
        .max()
        .map(|x| x + 1)
        .unwrap_or(0);

    // Registry head: common symbol, merged across TUs.
    if !m.globals.iter().any(|g| g.name == registry) {
        m.globals.push(IrGlobal {
            name: registry.clone(),
            ty: IrType::I64,
            size: 8,
            align: 8,
            init: GlobalInit::Zero,
            is_static: false,
            is_extern: false,
            is_common: true,
            section: None,
            is_weak: false,
            visibility: None,
            has_explicit_align: true,
            is_const: false,
            is_used: true,
            is_thread_local: false,
        });
    }
    // Pool node.
    m.globals.push(IrGlobal {
        name: pool.clone(),
        ty: IrType::I64,
        size: 24,
        align: 8,
        init: GlobalInit::Zero,
        is_static: true,
        is_extern: false,
        is_common: false,
        section: None,
        is_weak: false,
        visibility: Some("hidden".into()),
        has_explicit_align: true,
        is_const: false,
        is_used: true,
        is_thread_local: false,
    });
    // addr2name table: {addr, name} pairs for every defined function.
    let mut elems: Vec<GlobalInit> = Vec::new();
    let mut n_entries = 0u64;
    for f in &m.functions {
        if f.is_declaration || f.name.starts_with("__lccc_pgo_") {
            continue;
        }
        let s = format!("__lccc_pgo_nm_{:016x}_{}", uid, n_entries);
        m.string_literals.push((s.clone(), f.name.clone()));
        elems.push(GlobalInit::GlobalAddr(f.name.clone()));
        elems.push(GlobalInit::GlobalAddr(s));
        // flags: bit0 = static (a static function's pointer can escape via a
        // global variable, e.g. zlib-ng functable.force_init_stub — but the
        // emitted symbol is LOCAL, so cross-TU direct calls cannot link).
        let flags: i64 = if f.is_static { 1 } else { 0 };
        elems.push(GlobalInit::Scalar(IrConst::I64(flags)));
        n_entries += 1;
    }
    if n_entries > 0 {
        m.globals.push(IrGlobal {
            name: table.clone(),
            ty: IrType::I64,
            size: (n_entries * 24) as usize,
            align: 8,
            init: GlobalInit::Compound(elems),
            is_static: true,
            is_extern: false,
            is_common: false,
            section: None,
            is_weak: false,
            visibility: Some("hidden".into()),
            has_explicit_align: true,
            is_const: true,
            is_used: true,
            is_thread_local: false,
        });
    }

    // reg_add(table, count): append {table,count} node to the registry.
    {
        let mut b = VpBuilder::new(2);
        b.push(Instruction::ParamRef {
            dest: Value(0),
            param_idx: 0,
            ty: IrType::Ptr,
        });
        b.push(Instruction::ParamRef {
            dest: Value(1),
            param_idx: 1,
            ty: IrType::I64,
        });
        let headp = b.v();
        b.push(Instruction::GlobalAddr {
            dest: headp,
            name: registry.clone(),
        });
        let old = b.load(headp);
        let poolv = b.v();
        b.push(Instruction::GlobalAddr {
            dest: poolv,
            name: pool.clone(),
        });
        let tmp = b.gep(poolv, 0);
            b.store(tmp, Operand::Value(old));
        let tmp = b.gep(poolv, 8);
            b.store(tmp, Operand::Value(Value(0)));
        let tmp = b.gep(poolv, 16);
            b.store(tmp, Operand::Value(Value(1)));
        b.store(headp, Operand::Value(poolv));
        label_base += 1;
        push_helper_fn(
            m,
            &reg_add,
            vec![IrType::Ptr, IrType::I64],
            vec![BasicBlock {
                label: BlockId(0),
                instructions: b.is,
                terminator: Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
                source_spans: vec![],
            }],
            b.next,
            label_base,
        );
        label_base += 1;
    }
    // Constructor: register the table (only if there are entries).
    if n_entries > 0 {
        let mut b = VpBuilder::new(1);
        let t = b.v();
        b.push(Instruction::GlobalAddr {
            dest: t,
            name: table.clone(),
        });
        b.push(Instruction::Call {
            func: reg_add.clone(),
            info: ci(
                None,
                vec![
                    Operand::Value(t),
                    Operand::Const(IrConst::I64(n_entries as i64)),
                ],
                vec![IrType::Ptr, IrType::I64],
                IrType::I32,
                false,
            ),
        });
        push_helper_fn(
            m,
            &ctor,
            vec![],
            vec![BasicBlock {
                label: BlockId(0),
                instructions: b.is,
                terminator: Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
                source_spans: vec![],
            }],
            b.next,
            label_base,
        );
        label_base += 1;
        m.constructors.push(ctor);
    }

    let mk_cb = |cond: Value, tt: BlockId, ff: BlockId| -> Terminator {
        Terminator::CondBranch {
            cond: Operand::Value(cond),
            true_label: tt,
            false_label: ff,
        }
    };

    // Recorder: vp(site, fp). Site layout: t[0..4] @ 0, c[0..4] @ 32, total @ 64.
    {
        // Labels
        let (l0, l1, l2, l3, l4) = (BlockId(0), BlockId(1), BlockId(2), BlockId(3), BlockId(4));
        let (li0, li1, li2, li3) = (BlockId(5), BlockId(6), BlockId(7), BlockId(8));
        let (le0, le1, le2, le3) = (BlockId(9), BlockId(10), BlockId(11), BlockId(12));
        let (ls0, ls1, ls2, ls3) = (BlockId(13), BlockId(14), BlockId(15), BlockId(16));
        let (lev, lq1, lq2) = (BlockId(17), BlockId(18), BlockId(19));

        // entry: t0..t3 loads + match compares; terminator branches to l1..l4
        let mut e = VpBuilder::new(2);
        e.push(Instruction::ParamRef {
            dest: Value(0),
            param_idx: 0,
            ty: IrType::Ptr,
        });
        e.push(Instruction::ParamRef {
            dest: Value(1),
            param_idx: 1,
            ty: IrType::I64,
        });
        let site = Value(0);
        let fp = Value(1);
        let mut t: Vec<Value> = Vec::new();
        let mut eqs: Vec<Value> = Vec::new();
        for i in 0..4 {
            let tmp = e.gep(site, (i * 8) as i64);
        let tv = e.load(tmp);;
            t.push(tv);
            let q = e.cmp(crate::ir::ops::IrCmpOp::Eq, Operand::Value(tv), Operand::Value(fp));
            eqs.push(q);
        }
        // empty-slot zero compares (reuse t values)
        let z0 = e.cmp(crate::ir::ops::IrCmpOp::Eq, Operand::Value(t[0]), Operand::Const(IrConst::I64(0)));
        let z1 = e.cmp(crate::ir::ops::IrCmpOp::Eq, Operand::Value(t[1]), Operand::Const(IrConst::I64(0)));
        let z2 = e.cmp(crate::ir::ops::IrCmpOp::Eq, Operand::Value(t[2]), Operand::Const(IrConst::I64(0)));
        let z3 = e.cmp(crate::ir::ops::IrCmpOp::Eq, Operand::Value(t[3]), Operand::Const(IrConst::I64(0)));
        // load counts for the eviction min
        let mut c: Vec<Value> = Vec::new();
        for i in 0..4 {
            let tmp = e.gep(site, (32 + i * 8) as i64);
            c.push(e.load(tmp));
        }
        let u01 = e.cmp(crate::ir::ops::IrCmpOp::Ule, Operand::Value(c[0]), Operand::Value(c[1]));
        let m01 = e.sel(Operand::Value(u01), Operand::Value(c[0]), Operand::Value(c[1]));
        let u012 = e.cmp(crate::ir::ops::IrCmpOp::Ule, Operand::Value(m01), Operand::Value(c[2]));
        let m012 = e.sel(Operand::Value(u012), Operand::Value(m01), Operand::Value(c[2]));
        let u0123 = e.cmp(crate::ir::ops::IrCmpOp::Ule, Operand::Value(m012), Operand::Value(c[3]));
        let min = e.sel(Operand::Value(u0123), Operand::Value(m012), Operand::Value(c[3]));
        let q0 = e.cmp(crate::ir::ops::IrCmpOp::Eq, Operand::Value(min), Operand::Value(c[0]));
        let q1 = e.cmp(crate::ir::ops::IrCmpOp::Eq, Operand::Value(min), Operand::Value(c[1]));
        let q2 = e.cmp(crate::ir::ops::IrCmpOp::Eq, Operand::Value(min), Operand::Value(c[2]));
        let entry = BasicBlock {
            label: l0,
            instructions: e.is,
            terminator: Terminator::CondBranch {
                cond: Operand::Value(eqs[0]),
                true_label: li0,
                false_label: l1,
            },
            source_spans: vec![],
        };
        let mut mk_inc = |slot: usize, label: BlockId, next: u32| -> (BasicBlock, u32) {
            let mut b = VpBuilder::new(next);
            let cp = b.gep(site, (32 + slot * 8) as i64);
            let cv = b.load(cp);
            let cn = b.add(Operand::Value(cv), Operand::Const(IrConst::I64(1)));
            b.store(cp, Operand::Value(cn));
            let tp = b.gep(site, 64);
            let tv = b.load(tp);
            let tn = b.add(Operand::Value(tv), Operand::Const(IrConst::I64(1)));
            b.store(tp, Operand::Value(tn));
            (
                BasicBlock {
                    label,
                    instructions: b.is,
                    terminator: Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
                    source_spans: vec![],
                },
                b.next,
            )
        };
        let (blk_li0, n1) = mk_inc(0, li0, e.next);
        let (blk_li1, n2) = mk_inc(1, li1, n1);
        let (blk_li2, n3) = mk_inc(2, li2, n2);
        let (blk_li3, n4) = mk_inc(3, li3, n3);
        let mut mk_set = |slot: usize, label: BlockId, next: u32| -> (BasicBlock, u32) {
            let mut b = VpBuilder::new(next);
            let tmp = b.gep(site, (slot * 8) as i64);
            b.store(tmp, Operand::Value(fp));
            let tmp = b.gep(site, (32 + slot * 8) as i64);
            b.store(tmp, Operand::Const(IrConst::I64(1)));
            let tp = b.gep(site, 64);
            let tv = b.load(tp);
            let tn = b.add(Operand::Value(tv), Operand::Const(IrConst::I64(1)));
            b.store(tp, Operand::Value(tn));
            (
                BasicBlock {
                    label,
                    instructions: b.is,
                    terminator: Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
                    source_spans: vec![],
                },
                b.next,
            )
        };
        let (blk_ls0, n5) = mk_set(0, ls0, n4);
        let (blk_ls1, n6) = mk_set(1, ls1, n5);
        let (blk_ls2, n7) = mk_set(2, ls2, n6);
        let (blk_ls3, n8) = mk_set(3, ls3, n7);
        let blocks = vec![
            entry,
            BasicBlock {
                label: l1,
                instructions: vec![],
                terminator: mk_cb(eqs[1], li1, l2),
                source_spans: vec![],
            },
            BasicBlock {
                label: l2,
                instructions: vec![],
                terminator: mk_cb(eqs[2], li2, l3),
                source_spans: vec![],
            },
            BasicBlock {
                label: l3,
                instructions: vec![],
                terminator: mk_cb(eqs[3], li3, l4),
                source_spans: vec![],
            },
            BasicBlock {
                label: l4,
                instructions: vec![],
                terminator: mk_cb(z0, ls0, le0),
                source_spans: vec![],
            },
            blk_li0,
            blk_li1,
            blk_li2,
            blk_li3,
            BasicBlock {
                label: le0,
                instructions: vec![],
                terminator: mk_cb(z1, ls1, le1),
                source_spans: vec![],
            },
            BasicBlock {
                label: le1,
                instructions: vec![],
                terminator: mk_cb(z2, ls2, le2),
                source_spans: vec![],
            },
            BasicBlock {
                label: le2,
                instructions: vec![],
                terminator: mk_cb(z3, ls3, le3),
                source_spans: vec![],
            },
            BasicBlock {
                label: le3,
                instructions: vec![],
                terminator: Terminator::Branch(lev),
                source_spans: vec![],
            },
            blk_ls0,
            blk_ls1,
            blk_ls2,
            blk_ls3,
            BasicBlock {
                label: lev,
                instructions: vec![],
                terminator: mk_cb(q0, ls0, lq1),
                source_spans: vec![],
            },
            BasicBlock {
                label: lq1,
                instructions: vec![],
                terminator: mk_cb(q1, ls1, lq2),
                source_spans: vec![],
            },
            BasicBlock {
                label: lq2,
                instructions: vec![],
                terminator: mk_cb(q2, ls2, ls3),
                source_spans: vec![],
            },
        ];
        push_helper_fn(m, vp_recorder, vec![IrType::Ptr, IrType::I64], blocks, n8, label_base);
        label_base += 20;
    }

    // Lookup: addr -> name string (or 0). Walks the registry linked list and
    // each table linearly. Loop-carried values use FIXED copy destinations
    // (post-SSA style, like phi-eliminated code) so the loop actually
    // advances — a fresh value per iteration never updates the comparison.
    {
        let (l0, lcond, lbody, lpc, lpb, lpn, lnn, ldone, lfound, lret) = (
            BlockId(0), BlockId(1), BlockId(2), BlockId(3), BlockId(4),
            BlockId(5), BlockId(6), BlockId(7), BlockId(8), BlockId(9),
        );
        // Shared loop-carried value ids (allocated once, redefined by copies).
        let mut e = VpBuilder::new(1);
        e.push(Instruction::ParamRef {
            dest: Value(0),
            param_idx: 0,
            ty: IrType::I64,
        });
        let addr = Value(0);
        let headp = e.v();
        e.push(Instruction::GlobalAddr {
            dest: headp,
            name: registry.clone(),
        });
        let head = e.load(headp);
        let icur = e.v();
        let node0 = e.v();
        e.copy(node0, Operand::Value(head));
        let entry = BasicBlock {
            label: l0,
            instructions: e.is,
            terminator: Terminator::Branch(lcond),
            source_spans: vec![],
        };
        // lcond: if node0 != 0 -> body else done (node0 redefined in lnn)
        let mut c = VpBuilder::new(e.next);
        let nz = c.cmp(crate::ir::ops::IrCmpOp::Ne, Operand::Value(node0), Operand::Const(IrConst::I64(0)));
        let blk_cond = BasicBlock {
            label: lcond,
            instructions: c.is,
            terminator: mk_cb(nz, lbody, ldone),
            source_spans: vec![],
        };
        // lbody: load next/table/count; icur = 0
        let mut b = VpBuilder::new(c.next);
        let tmp0 = b.gep(node0, 0);
        let nn = b.load(tmp0);
        let tmp1 = b.gep(node0, 8);
        let table_v = b.load(tmp1);
        let tmp2 = b.gep(node0, 16);
        let count = b.load(tmp2);
        b.copy(icur, Operand::Const(IrConst::I64(0)));
        let blk_body = BasicBlock {
            label: lbody,
            instructions: b.is,
            terminator: Terminator::Branch(lpc),
            source_spans: vec![],
        };
        // lpc: icur < count -> pb else nn
        let mut p = VpBuilder::new(b.next);
        let ic = p.cmp(crate::ir::ops::IrCmpOp::Ult, Operand::Value(icur), Operand::Value(count));
        let blk_pc = BasicBlock {
            label: lpc,
            instructions: p.is,
            terminator: mk_cb(ic, lpb, lnn),
            source_spans: vec![],
        };
        // lpb: ep = table + icur*16; a = load ep; a == addr -> found else pn
        let mut q = VpBuilder::new(p.next);
        let off = q.mul(Operand::Value(icur), Operand::Const(IrConst::I64(24)));
        let ep = q.add(Operand::Value(table_v), Operand::Value(off));
        let av = q.load(ep);
        let ae = q.cmp(crate::ir::ops::IrCmpOp::Eq, Operand::Value(av), Operand::Value(addr));
        let blk_pb = BasicBlock {
            label: lpb,
            instructions: q.is,
            terminator: mk_cb(ae, lfound, lpn),
            source_spans: vec![],
        };
        // lpn: icur += 1 (redefine the SAME value id)
        let mut r = VpBuilder::new(q.next);
        let i1 = r.add(Operand::Value(icur), Operand::Const(IrConst::I64(1)));
        r.copy(icur, Operand::Value(i1));
        let blk_pn = BasicBlock {
            label: lpn,
            instructions: r.is,
            terminator: Terminator::Branch(lpc),
            source_spans: vec![],
        };
        // lnn: node0 = next (redefine the loop-carried value id)
        let mut s = VpBuilder::new(r.next);
        s.copy(node0, Operand::Value(nn));
        let blk_nn = BasicBlock {
            label: lnn,
            instructions: s.is,
            terminator: Terminator::Branch(lcond),
            source_spans: vec![],
        };
        // ldone: retv = &dummy (an empty slot must still yield a valid entry
        // pointer — the dump GEPs +8/+16 off the result; NULL would crash).
        let mut u = VpBuilder::new(s.next);
        let retv = u.v();
        u.push(Instruction::GlobalAddr {
            dest: retv,
            name: none.clone(),
        });
        let blk_done = BasicBlock {
            label: ldone,
            instructions: u.is,
            terminator: Terminator::Branch(lret),
            source_spans: vec![],
        };
        // lfound: retv = ep (the matched {addr,name,flags} entry base); the
        // dump derives name = ep+8 and flags = ep+16.
        let mut w = VpBuilder::new(u.next);
        w.copy(retv, Operand::Value(ep));
        let blk_found = BasicBlock {
            label: lfound,
            instructions: w.is,
            terminator: Terminator::Branch(lret),
            source_spans: vec![],
        };
        // lret: return the multi-def retv (NO redefinition — a copy here
        // would override the value set by ldone/lfound; the backend treats
        // multi-def values like phi-eliminated copies).
        let mut x = VpBuilder::new(w.next);
        let blk_ret = BasicBlock {
            label: lret,
            instructions: x.is,
            terminator: Terminator::Return(Some(Operand::Value(retv))),
            source_spans: vec![],
        };
        push_helper_fn(
            m,
            &lookup,
            vec![IrType::I64],
            vec![
                entry, blk_cond, blk_body, blk_pc, blk_pb, blk_pn, blk_nn, blk_done,
                blk_found, blk_ret,
            ],
            x.next,
            label_base,
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loop_backedge_is_instrumented_not_forward_edge() {
        // Simple loop: 0(entry)->1(header)->2(latch), 2->1 backedge,
        // 1->3(exit) return, 3->VEXIT. Build the CFG edge list.
        let nodes = vec![0u32, 1, 2, 3];
        let edges = vec![
            Edge { src: 0, dst: 1 },
            Edge { src: 1, dst: 2 },
            Edge { src: 2, dst: 1 }, // backedge
            Edge { src: 1, dst: 3 },
            Edge { src: 3, dst: VEXIT },
        ];
        // Include VEXIT in the node set (as the pipeline does).
        let mut tree_nodes = nodes.clone();
        tree_nodes.push(VEXIT);
        let instr = choose_instrumented_edges(&tree_nodes, &edges, 0);
        let has = |s: u32, d: u32| instr.iter().any(|e| e.src == s && e.dst == d);
        // The forward edge into the latch (1->2) is a TREE edge: it must NOT
        // be instrumented. The backedge (2->1) must be instrumented.
        assert!(!has(1, 2), "forward edge 1->2 must be a tree edge, not instrumented");
        assert!(has(2, 1), "backedge 2->1 must be instrumented (off the arborescence)");
        // Every node except entry must have exactly one incoming TREE edge.
        let tree: FxHashSet<Edge> = edges
            .iter()
            .copied()
            .filter(|e| !instr.iter().any(|i| i.src == e.src && i.dst == e.dst))
            .collect();
        for &v in &[1u32, 2, 3, VEXIT] {
            let n = tree.iter().filter(|e| e.dst == v).count();
            assert_eq!(n, 1, "node {} must have exactly one incoming tree edge", v);
        }
        // Reachability from entry via tree edges must cover all nodes.
        let mut reachable = FxHashSet::default();
        let mut stack = vec![0u32];
        while let Some(u) = stack.pop() {
            if !reachable.insert(u) {
                continue;
            }
            for e in &tree {
                if e.src == u {
                    stack.push(e.dst);
                }
            }
        }
        for &v in &[1u32, 2, 3, VEXIT] {
            assert!(reachable.contains(&v), "node {} must be tree-reachable from entry", v);
        }
    }
}
