//! Shared CFG and dominator tree analysis utilities.
//!
//! These functions compute control flow graph (CFG) information and dominator
//! trees (Semi-NCA, see [`compute_dominators`]). They are used by mem2reg
//! for SSA construction and by optimization passes (e.g., GVN) that need
//! dominator information.
//!
//! Performance: The CFG is stored as a flat CSR (Compressed Sparse Row)
//! adjacency list (`FlatAdj`) instead of `Vec<Vec<usize>>`. This reduces
//! n+1 heap allocations to 2 per build_cfg call and improves cache locality,
//! which is critical since build_cfg is called per-function by GVN, LICM,
//! if_convert, and mem2reg.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::reexports::{BlockId, Instruction, IrFunction, Terminator};

// ── Flat adjacency list (CSR format) ──────────────────────────────────────────

/// A flat adjacency list using Compressed Sparse Row (CSR) format.
///
/// Stores `n` variable-length rows in two flat arrays:
/// - `offsets[i]..offsets[i+1]` is the range of indices into `data` for row i
/// - `data[offsets[i]..offsets[i+1]]` contains the neighbors of node i
///
/// This uses exactly 2 heap allocations regardless of the number of rows,
/// compared to n+1 for `Vec<Vec<usize>>`. The flat layout also provides
/// better cache locality when iterating over adjacency lists.
pub struct FlatAdj {
    /// offsets[i] is the start index in `data` for row i.
    /// offsets[n] is the total number of entries (= data.len()).
    /// Length: n + 1
    offsets: Vec<u32>,
    /// Flat storage of all adjacency entries.
    data: Vec<u32>,
}

impl FlatAdj {
    /// Get the adjacency list (neighbors) of node `i` as a slice.
    #[inline]
    pub fn row(&self, i: usize) -> &[u32] {
        let start = self.offsets[i] as usize;
        let end = self.offsets[i + 1] as usize;
        &self.data[start..end]
    }

    /// Get the number of neighbors of node `i`.
    #[inline]
    pub fn len(&self, i: usize) -> usize {
        (self.offsets[i + 1] - self.offsets[i]) as usize
    }

    /// Build a FlatAdj from `Vec<Vec<usize>>` for tests.
    #[cfg(test)]
    pub fn from_vecs_usize(vecs: &[Vec<usize>]) -> Self {
        let n = vecs.len();
        let mut offsets = Vec::with_capacity(n + 1);
        let total: usize = vecs.iter().map(|v| v.len()).sum();
        let mut data = Vec::with_capacity(total);

        let mut offset = 0u32;
        for v in vecs {
            offsets.push(offset);
            for &val in v {
                data.push(val as u32);
            }
            offset += v.len() as u32;
        }
        offsets.push(offset);

        FlatAdj { offsets, data }
    }

    /// Build a FlatAdj from a Vec<Vec<u32>> (used in the construction phase).
    fn from_vecs(vecs: Vec<Vec<u32>>) -> Self {
        let n = vecs.len();
        let mut offsets = Vec::with_capacity(n + 1);
        let total: usize = vecs.iter().map(|v| v.len()).sum();
        let mut data = Vec::with_capacity(total);

        let mut offset = 0u32;
        for v in &vecs {
            offsets.push(offset);
            data.extend_from_slice(v);
            offset += v.len() as u32;
        }
        offsets.push(offset);

        FlatAdj { offsets, data }
    }
}

// ── Label map ─────────────────────────────────────────────────────────────────

/// Build a map from block label to block index.
pub fn build_label_map(func: &IrFunction) -> FxHashMap<BlockId, usize> {
    func.blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.label, i))
        .collect()
}

// ── CFG construction ──────────────────────────────────────────────────────────

/// Build predecessor and successor lists from the function's CFG.
/// Returns (preds, succs) as flat adjacency lists (CSR format).
///
/// Uses only 4 heap allocations total (2 per FlatAdj) instead of 2*n+2 for
/// the old `Vec<Vec<usize>>` representation.
pub fn build_cfg(
    func: &IrFunction,
    label_to_idx: &FxHashMap<BlockId, usize>,
) -> (FlatAdj, FlatAdj) {
    let n = func.blocks.len();
    // Build using temporary Vec<Vec<u32>> then flatten to CSR.
    // The inner Vecs are tiny (usually 1-4 entries) so this is fast.
    let mut preds: Vec<Vec<u32>> = vec![Vec::new(); n];
    let mut succs: Vec<Vec<u32>> = vec![Vec::new(); n];

    for (i, block) in func.blocks.iter().enumerate() {
        let i32 = i as u32;
        match &block.terminator {
            Terminator::Branch(label) => {
                if let Some(&target) = label_to_idx.get(label) {
                    succs[i].push(target as u32);
                    preds[target].push(i32);
                }
            }
            Terminator::CondBranch {
                true_label,
                false_label,
                ..
            } => {
                if let Some(&t) = label_to_idx.get(true_label) {
                    succs[i].push(t as u32);
                    preds[t].push(i32);
                }
                if let Some(&f) = label_to_idx.get(false_label) {
                    let f32v = f as u32;
                    if !succs[i].contains(&f32v) {
                        succs[i].push(f32v);
                    }
                    preds[f].push(i32);
                }
            }
            Terminator::IndirectBranch {
                possible_targets, ..
            } => {
                for label in possible_targets {
                    if let Some(&t) = label_to_idx.get(label) {
                        let t32 = t as u32;
                        if !succs[i].contains(&t32) {
                            succs[i].push(t32);
                        }
                        preds[t].push(i32);
                    }
                }
            }
            Terminator::Switch { cases, default, .. } => {
                if let Some(&d) = label_to_idx.get(default) {
                    succs[i].push(d as u32);
                    preds[d].push(i32);
                }
                for (_, label) in cases {
                    if let Some(&t) = label_to_idx.get(label) {
                        let t32 = t as u32;
                        if !succs[i].contains(&t32) {
                            succs[i].push(t32);
                        }
                        preds[t].push(i32);
                    }
                }
            }
            Terminator::Return(_) | Terminator::Unreachable => {}
        }
        // InlineAsm goto_labels are implicit control flow edges.
        for inst in &block.instructions {
            if let Instruction::InlineAsm { goto_labels, .. } = inst {
                for (_, label) in goto_labels {
                    if let Some(&t) = label_to_idx.get(label) {
                        let t32 = t as u32;
                        if !succs[i].contains(&t32) {
                            succs[i].push(t32);
                        }
                        preds[t].push(i32);
                    }
                }
            }
        }
    }

    (FlatAdj::from_vecs(preds), FlatAdj::from_vecs(succs))
}

// ── Reverse postorder ─────────────────────────────────────────────────────────

/// Compute reverse postorder traversal of the CFG.
pub fn compute_reverse_postorder(num_blocks: usize, succs: &FlatAdj) -> Vec<usize> {
    let mut visited = vec![false; num_blocks];
    let mut postorder = Vec::with_capacity(num_blocks);

    fn dfs(node: usize, succs: &FlatAdj, visited: &mut Vec<bool>, postorder: &mut Vec<usize>) {
        visited[node] = true;
        for &succ in succs.row(node) {
            let s = succ as usize;
            if !visited[s] {
                dfs(s, succs, visited, postorder);
            }
        }
        postorder.push(node);
    }

    if num_blocks > 0 {
        dfs(0, succs, &mut visited, &mut postorder);
    }

    postorder.reverse();
    postorder
}

// ── Dominator computation ─────────────────────────────────────────────────────

/// Compute immediate dominators.
/// Returns idom[i] = immediate dominator of block i (idom[0] = 0 for entry).
/// Uses usize::MAX as sentinel for undefined/unreachable blocks.
///
/// Algorithm: Semi-NCA (Lengauer-Tarjan semidominators with path
/// compression, then the nearest-common-ancestor pass of Georgiadis et al.;
/// the variant LLVM's `GenericDomTreeConstruction` uses).  O(E log V)
/// independent of CFG shape.  The iterative Cooper-Harvey-Kennedy scheme
/// this replaces is fast on typical CFGs but walks the dominator chain once
/// per predecessor of a join: a join with P predecessors at depth D costs
/// O(P * D) per sweep, and a long compare-and-branch chain whose exits all
/// meet at one label (gcc.c-torture/compile 20001226-1: 8192 branches, two
/// joins of 4096 predecessors each) made every dominator build quadratic.
/// Immediate dominators are unique, so the result is identical; the CHK
/// version is kept as the test oracle.
pub fn compute_dominators(num_blocks: usize, preds: &FlatAdj, succs: &FlatAdj) -> Vec<usize> {
    const UNDEF: usize = usize::MAX;
    const NONE: u32 = u32::MAX;
    let mut idom = vec![UNDEF; num_blocks];
    if num_blocks == 0 {
        return idom;
    }

    // Depth-first preorder from the entry (block 0), iteratively: CFG depth
    // reaches the tens of thousands on generated code.
    let mut pre = vec![NONE; num_blocks]; // block -> preorder number
    let mut vertex: Vec<usize> = Vec::with_capacity(num_blocks); // number -> block
    let mut parent: Vec<u32> = Vec::with_capacity(num_blocks); // number -> parent number
    let mut stack: Vec<(usize, usize)> = vec![(0, 0)]; // (block, next succ index)
    pre[0] = 0;
    vertex.push(0);
    parent.push(0);
    while let Some(&mut (b, ref mut next)) = stack.last_mut() {
        let row = succs.row(b);
        if let Some(&s) = row.get(*next) {
            *next += 1;
            let s = s as usize;
            if pre[s] == NONE {
                pre[s] = vertex.len() as u32;
                parent.push(pre[b]);
                vertex.push(s);
                stack.push((s, 0));
            }
        } else {
            stack.pop();
        }
    }
    let n = vertex.len();

    // Semidominators in reverse preorder.  `ancestor`/`label` form the
    // link-eval forest over already-processed vertices; `eval` compresses
    // paths iteratively (the forest can be as deep as the CFG).
    let mut semi: Vec<u32> = (0..n as u32).collect();
    let mut label: Vec<u32> = (0..n as u32).collect();
    let mut ancestor: Vec<u32> = vec![NONE; n];
    let mut path: Vec<u32> = Vec::new();
    for w in (1..n).rev() {
        for &p in preds.row(vertex[w]) {
            let v = pre[p as usize];
            if v == NONE {
                continue; // unreachable predecessor
            }
            let u = if ancestor[v as usize] == NONE {
                v
            } else {
                // Compress the path from `v` to its forest root.
                path.clear();
                let mut x = v;
                while ancestor[ancestor[x as usize] as usize] != NONE {
                    path.push(x);
                    x = ancestor[x as usize];
                }
                for &x in path.iter().rev() {
                    let a = ancestor[x as usize] as usize;
                    if semi[label[a] as usize] < semi[label[x as usize] as usize] {
                        label[x as usize] = label[a];
                    }
                    ancestor[x as usize] = ancestor[a];
                }
                label[v as usize]
            };
            if semi[u as usize] < semi[w] {
                semi[w] = semi[u as usize];
            }
        }
        ancestor[w] = parent[w];
    }

    // NCA pass: the idom of `w` is the nearest ancestor of `parent[w]` in
    // the (already final) dominator tree whose number is at most semi[w].
    let mut idom_num: Vec<u32> = parent;
    for w in 1..n {
        let mut d = idom_num[w];
        while d > semi[w] {
            d = idom_num[d as usize];
        }
        idom_num[w] = d;
    }
    for (w, &b) in vertex.iter().enumerate() {
        idom[b] = vertex[idom_num[w] as usize];
    }
    idom
}

/// Cooper-Harvey-Kennedy iterative dominators: the former implementation,
/// kept as an independent oracle for [`compute_dominators`].
#[cfg(test)]
fn compute_dominators_chk(num_blocks: usize, preds: &FlatAdj, succs: &FlatAdj) -> Vec<usize> {
    fn intersect(
        mut finger1: usize,
        mut finger2: usize,
        idom: &[usize],
        rpo_number: &[usize],
    ) -> usize {
        while finger1 != finger2 {
            while rpo_number[finger1] > rpo_number[finger2] {
                finger1 = idom[finger1];
            }
            while rpo_number[finger2] > rpo_number[finger1] {
                finger2 = idom[finger2];
            }
        }
        finger1
    }
    const UNDEF: usize = usize::MAX;

    let rpo = compute_reverse_postorder(num_blocks, succs);
    let mut rpo_number = vec![UNDEF; num_blocks];
    for (order, &block) in rpo.iter().enumerate() {
        rpo_number[block] = order;
    }

    let mut idom = vec![UNDEF; num_blocks];
    if rpo.is_empty() {
        return idom;
    }
    idom[rpo[0]] = rpo[0]; // Entry dominates itself

    let mut changed = true;
    while changed {
        changed = false;
        for &b in rpo.iter().skip(1) {
            let mut new_idom = UNDEF;
            for &p in preds.row(b) {
                let p = p as usize;
                if idom[p] != UNDEF {
                    new_idom = p;
                    break;
                }
            }
            if new_idom == UNDEF {
                continue;
            }
            for &p in preds.row(b) {
                let p = p as usize;
                if p != new_idom && idom[p] != UNDEF {
                    new_idom = intersect(new_idom, p, &idom, &rpo_number);
                }
            }
            if idom[b] != new_idom {
                idom[b] = new_idom;
                changed = true;
            }
        }
    }
    idom
}

// ── Dominance frontiers ───────────────────────────────────────────────────────

/// Compute dominance frontiers for each block.
/// DF(b) = set of blocks where b's dominance ends (join points).
pub fn compute_dominance_frontiers(
    num_blocks: usize,
    preds: &FlatAdj,
    idom: &[usize],
) -> Vec<FxHashSet<usize>> {
    let mut df = vec![FxHashSet::default(); num_blocks];

    for b in 0..num_blocks {
        if preds.len(b) < 2 {
            continue;
        }
        for &p in preds.row(b) {
            let mut runner = p as usize;
            while runner != idom[b] && runner != usize::MAX {
                df[runner].insert(b);
                if runner == idom[runner] {
                    break;
                }
                runner = idom[runner];
            }
        }
    }

    df
}

// ── Dominator tree ────────────────────────────────────────────────────────────

/// Build dominator tree children lists from idom array.
/// children[b] lists block indices whose immediate dominator is b.
pub fn build_dom_tree_children(num_blocks: usize, idom: &[usize]) -> Vec<Vec<usize>> {
    let mut children = vec![Vec::new(); num_blocks];
    for b in 1..num_blocks {
        if idom[b] != usize::MAX && idom[b] != b {
            children[idom[b]].push(b);
        }
    }
    children
}

// ── Cached analysis bundle ──────────────────────────────────────────────────

/// Pre-computed CFG analysis results shared across multiple passes within
/// a single pipeline iteration.
///
/// GVN, LICM, and IVSR all need the same CFG, dominator, and loop analysis.
/// Since GVN does not modify the CFG (it only replaces instruction operands),
/// these results remain valid across all three passes. Computing them once
/// and sharing avoids redundant `build_cfg` + `compute_dominators` +
/// `find_natural_loops` calls per function per iteration.
pub struct CfgAnalysis {
    pub preds: FlatAdj,
    pub succs: FlatAdj,
    pub idom: Vec<usize>,
    pub dom_children: Vec<Vec<usize>>,
    pub num_blocks: usize,
}

impl CfgAnalysis {
    /// Build a complete CFG analysis bundle for a function.
    pub fn build(func: &IrFunction) -> Self {
        let num_blocks = func.blocks.len();
        let label_to_idx = build_label_map(func);
        let (preds, succs) = build_cfg(func, &label_to_idx);
        let idom = compute_dominators(num_blocks, &preds, &succs);
        let dom_children = build_dom_tree_children(num_blocks, &idom);
        CfgAnalysis {
            preds,
            succs,
            idom,
            dom_children,
            num_blocks,
        }
    }
}

#[cfg(test)]
mod dominator_tests {
    use super::*;

    fn preds_of(succs: &[Vec<usize>]) -> Vec<Vec<usize>> {
        let mut preds = vec![Vec::new(); succs.len()];
        for (b, row) in succs.iter().enumerate() {
            for &s in row {
                preds[s].push(b);
            }
        }
        preds
    }

    fn both(succs: &[Vec<usize>]) -> (Vec<usize>, Vec<usize>) {
        let n = succs.len();
        let s = FlatAdj::from_vecs_usize(succs);
        let p = FlatAdj::from_vecs_usize(&preds_of(succs));
        (
            compute_dominators(n, &p, &s),
            compute_dominators_chk(n, &p, &s),
        )
    }

    #[test]
    fn textbook_shapes() {
        const U: usize = usize::MAX;
        // Diamond with an unreachable block 4 (which branches into the join).
        let (snca, chk) = both(&[vec![1, 2], vec![3], vec![3], vec![], vec![3]]);
        assert_eq!(snca, vec![0, 0, 0, 0, U]);
        assert_eq!(snca, chk);
        // Irreducible loop entered at both 1 and 2.
        let (snca, chk) = both(&[vec![1, 2], vec![2], vec![1, 3], vec![]]);
        assert_eq!(snca, vec![0, 0, 0, 2]);
        assert_eq!(snca, chk);
        // Self loop on the entry, duplicate edges.
        let (snca, chk) = both(&[vec![0, 1, 1], vec![]]);
        assert_eq!(snca, vec![0, 0]);
        assert_eq!(snca, chk);
        // Single block, and the empty CFG.
        assert_eq!(both(&[vec![]]).0, vec![0]);
        assert_eq!(both(&[]).0, Vec::<usize>::new());
    }

    /// The 20001226-1 shape: a chain of N two-way branches whose early
    /// exits all meet at two join blocks.  Also a depth test for the
    /// iterative DFS and path compression.
    #[test]
    fn long_branch_chain_with_shared_exits() {
        let n = 20_000;
        let (gt, lt, ret) = (n, n + 1, n + 2);
        let mut succs = vec![Vec::new(); n + 3];
        for (i, row) in succs.iter_mut().enumerate().take(n) {
            row.push(if i % 2 == 0 { gt } else { lt });
            row.push(if i + 1 < n { i + 1 } else { ret });
        }
        let s = FlatAdj::from_vecs_usize(&succs);
        let p = FlatAdj::from_vecs_usize(&preds_of(&succs));
        let idom = compute_dominators(n + 3, &p, &s);
        for (i, &d) in idom.iter().enumerate().take(n).skip(1) {
            assert_eq!(d, i - 1);
        }
        assert_eq!(idom[gt], 0);
        assert_eq!(idom[lt], 1);
        assert_eq!(idom[ret], n - 1);
    }

    /// Differential test against the CHK oracle on pseudo-random CFGs
    /// (reducible and irreducible, with unreachable blocks, self loops and
    /// duplicate edges).
    #[test]
    fn matches_cooper_harvey_kennedy_on_random_cfgs() {
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut rnd = |m: usize| -> usize {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % m as u64) as usize
        };
        for _ in 0..2000 {
            let n = 1 + rnd(40);
            let mut succs = vec![Vec::new(); n];
            for (b, row) in succs.iter_mut().enumerate() {
                let k = rnd(4);
                for _ in 0..k {
                    // Bias toward forward edges so most blocks are reachable.
                    let t = if rnd(3) == 0 {
                        rnd(n)
                    } else {
                        (b + 1 + rnd(3)).min(n - 1)
                    };
                    row.push(t);
                }
            }
            let (snca, chk) = both(&succs);
            assert_eq!(snca, chk, "succs = {succs:?}");
        }
    }
}
