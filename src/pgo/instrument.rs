//! PGO v4 generation instrumentation: Knuth–Stevenson minimal edge counting.
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
//! Soundness: counters are placed so they never sit between a fused Cmp and
//! its branch/select consumer (`incq` clobbers flags), and instrumentation
//! runs post-optimization so no optimization pass ever sees a counter.
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::backend::Target;
use crate::ir::constants::IrConst;
use crate::ir::reexports::{
    BasicBlock, BlockId, CallInfo, GlobalInit, Instruction, IrFunction, IrGlobal, IrModule,
    Operand, Terminator, Value,
};
use crate::pgo::profile::{cfg_fingerprint, function_key, resolve_output_path, unit_hash};
const SEC: &str = ".lccc_pgo_cnts";
type Rec = (String, String, Vec<(u32, u32, usize)>, usize, u64, u32, u64);

/// Sentinel source label for the virtual entry edge (V -> entry). Cannot
/// collide with a real block label.
pub(crate) const VENTRY: u32 = u32::MAX - 1;

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

struct Dsu {
    parent: Vec<usize>,
}
impl Dsu {
    fn new(n: usize) -> Self {
        Dsu {
            parent: (0..n).collect(),
        }
    }
    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]];
            x = self.parent[x];
        }
        x
    }
    fn union(&mut self, a: usize, b: usize) -> bool {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra == rb {
            return false;
        }
        self.parent[ra] = rb;
        true
    }
}

/// Choose the instrumented (non-tree) edges: maximum spanning tree over the
/// CFG (Kruskal), with the virtual entry edge forced instrumented.
fn choose_instrumented_edges(nodes: &[u32], edges: &[Edge], entry: u32) -> Vec<Edge> {
    let comp = scc_ids(nodes, edges);
    let dom = dominators(nodes, edges, entry);
    let weight = |e: &Edge| -> (i64, u32, u32) {
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
        let w = if backed {
            2000
        } else if loopish {
            1000
        } else {
            1
        };
        (w, e.src, e.dst)
    };
    let node_index: FxHashMap<u32, usize> =
        nodes.iter().enumerate().map(|(i, n)| (*n, i)).collect();
    let mut ordered: Vec<usize> = (0..edges.len()).collect();
    ordered.sort_by(|&a, &b| weight(&edges[a]).cmp(&weight(&edges[b])).reverse());
    let mut dsu = Dsu::new(nodes.len());
    let mut tree = FxHashSet::default();
    for &ei in &ordered {
        let e = edges[ei];
        let (Some(&a), Some(&b)) = (node_index.get(&e.src), node_index.get(&e.dst)) else {
            continue;
        };
        if dsu.union(a, b) {
            tree.insert(e);
        }
    }
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

pub fn instrument_module(
    m: &mut IrModule,
    dir: &str,
    unit: &str,
    update: Option<&str>,
    target: Target,
) -> usize {
    if std::env::var_os("LCCC_PGO_NO_COUNTERS").is_some() {
        return 0;
    }
    if target != Target::X86_64 {
        eprintln!("lccc: PGO v4: instrumentation is x86-64 only; skipping");
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
        let h = cfg_fingerprint(&f.name, unit, f);
        let cname = format!("__lccc_pgo_cnt_{:016x}_{:016x}", uid, h);

        let nodes: Vec<u32> = f.blocks.iter().map(|b| b.label.0).collect();
        let mut edges: Vec<Edge> = Vec::new();
        for b in &f.blocks {
            for s in successors(&b.terminator) {
                edges.push(Edge {
                    src: b.label.0,
                    dst: s,
                });
            }
        }
        let entry = f.blocks.first().map(|b| b.label.0).unwrap_or(0);
        let instr_edges = choose_instrumented_edges(&nodes, &edges, entry);
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
        rec.push((
            function_key(unit, &f.name),
            cname,
            edge_list,
            entry_slot,
            h,
            entry,
            uid,
        ));
    }
    for g in gs {
        if std::env::var("LCCC_PGO_NO_ARRAYS").is_ok() {
            break;
        }
        if !m.globals.iter().any(|x| x.name == g.name) {
            m.globals.push(g)
        }
    }
    if rec.is_empty() {
        return 0;
    }
    let path = resolve_output_path(dir, unit);
    let helper = format!("__lccc_pgo_dump_{:016x}_{}", uid, std::process::id());
    if std::env::var("LCCC_PGO_NO_DUMP").is_err() {
        dump_helper(m, &path, &rec, &helper);
    }
    eprintln!(
        "lccc: PGO v4: instrumented {} functions into {}",
        rec.len(),
        path.display()
    );
    rec.len()
}

fn dump_helper(m: &mut IrModule, path: &std::path::Path, rec: &[Rec], name: &str) {
    let tag = format!("__lccc_pgo_path_{}", m.functions.len());
    let mode = format!("__lccc_pgo_mode_{}", m.functions.len());
    let hdr = format!("__lccc_pgo_hdr_{}", m.functions.len());
    let pair = format!("__lccc_pgo_pair_{}", m.functions.len());
    let fpair = format!("__lccc_pgo_fpair_{}", m.functions.len());
    m.string_literals
        .push((tag.clone(), path.display().to_string()));
    m.string_literals.push((mode.clone(), "a".into()));
    m.string_literals
        .push((hdr.clone(), "lccc-pgo-v4 %llu %u %llu\nfunc %s\n".into()));
    m.string_literals.push((pair.clone(), "e %u %u %llu\n".into()));
    m.string_literals.push((fpair.clone(), "f %llu\n".into()));
    let mut names = Vec::new();
    for (r, _, _, _, _, _, _) in rec {
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
    // Entry block: fopen + null check.
    let skip_l = BlockId(910099);
    let main_l = BlockId(910001);
    let is_null = Value(next);
    next += 1;
    let mut entry_is: Vec<Instruction> = Vec::new();
    entry_is.push(Instruction::GlobalAddr {
        dest: pp,
        name: tag,
    });
    entry_is.push(Instruction::GlobalAddr {
        dest: mm,
        name: mode,
    });
    entry_is.push(Instruction::Call {
        func: "fopen".into(),
        info: ci(
            Some(file),
            vec![Operand::Value(pp), Operand::Value(mm)],
            vec![IrType::Ptr, IrType::Ptr],
            IrType::Ptr,
            false,
        ),
    });
    entry_is.push(Instruction::Cmp {
        dest: is_null,
        op: crate::ir::ops::IrCmpOp::Eq,
        lhs: Operand::Value(file),
        rhs: Operand::Const(IrConst::I64(0)),
        ty: IrType::I64,
    });
    let entry_block = BasicBlock {
        label: BlockId(910000),
        instructions: entry_is,
        terminator: Terminator::CondBranch {
            cond: Operand::Value(is_null),
            true_label: skip_l,
            false_label: main_l,
        },
        source_spans: vec![],
    };
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
    for (i, (_, cnt, edges, entry_slot, h, entry, uid)) in rec.iter().enumerate() {
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
                    Operand::Const(IrConst::I64(*h as i64)),
                    Operand::Const(IrConst::I32(*entry as i32)),
                    Operand::Const(IrConst::I64(*uid as i64)),
                    Operand::Value(np),
                ],
                vec![
                    IrType::Ptr,
                    IrType::Ptr,
                    IrType::I64,
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
        next_label: 910002,
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
