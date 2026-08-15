//! Shared loop analysis utilities for optimization passes.
//!
//! Provides high-performance natural loop detection, loop hierarchy construction,
//! preheader identification, latch/exit analysis, and deterministic loop traversals
//! used by LICM, induction variable strength reduction, and loop transformations.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::analysis;

/// High-performance dense bitset optimized for basic block index operations.
///
/// Executes bitwise set operations in 1 CPU cycle using native `u64` words
/// (`BT`, `BTS`, `BTR`, `POPCNT`, `TZCNT` on x86-64).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct BlockBitSet {
    words: Vec<u64>,
    num_bits: usize,
}

impl BlockBitSet {
    /// Create a new bitset capable of holding `num_bits` basic block indices.
    #[inline]
    pub fn new(num_bits: usize) -> Self {
        let num_words = (num_bits + 63) / 64;
        Self {
            words: vec![0u64; num_words],
            num_bits,
        }
    }

    /// Insert a block index. Returns `true` if the block was newly inserted.
    #[inline(always)]
    pub fn insert(&mut self, block: usize) -> bool {
        if block >= self.num_bits {
            return false;
        }
        let word_idx = block / 64;
        let bit_mask = 1u64 << (block % 64);
        let was_present = (self.words[word_idx] & bit_mask) != 0;
        self.words[word_idx] |= bit_mask;
        !was_present
    }

    /// Returns `true` if the bitset contains the given block index.
    #[inline(always)]
    pub fn contains(&self, block: usize) -> bool {
        if block >= self.num_bits {
            return false;
        }
        (self.words[block / 64] & (1u64 << (block % 64))) != 0
    }

    /// Remove a block index. Returns `true` if the block was present.
    #[inline(always)]
    pub fn remove(&mut self, block: usize) -> bool {
        if block >= self.num_bits {
            return false;
        }
        let word_idx = block / 64;
        let bit_mask = 1u64 << (block % 64);
        let was_present = (self.words[word_idx] & bit_mask) != 0;
        self.words[word_idx] &= !bit_mask;
        was_present
    }

    /// In-place union with another bitset.
    #[inline]
    pub fn union_with(&mut self, other: &Self) {
        let min_words = self.words.len().min(other.words.len());
        for i in 0..min_words {
            self.words[i] |= other.words[i];
        }
    }

    /// Returns `true` if `self` is a subset of `other`.
    #[inline]
    pub fn is_subset(&self, other: &Self) -> bool {
        for (i, &w) in self.words.iter().enumerate() {
            let other_w = other.words.get(i).copied().unwrap_or(0);
            if (w & !other_w) != 0 {
                return false;
            }
        }
        true
    }

    /// Returns the number of set bits (blocks) using hardware `POPCNT`.
    #[inline]
    pub fn count(&self) -> usize {
        self.words.iter().map(|w| w.count_ones() as usize).sum()
    }

    /// Iterate over all set block indices in ascending order using hardware `TZCNT`.
    #[inline]
    pub fn iter(&self) -> BlockBitSetIter<'_> {
        BlockBitSetIter {
            bitset: self,
            word_idx: 0,
            current_word: self.words.first().copied().unwrap_or(0),
        }
    }

    /// Convert this bitset into an `FxHashSet<usize>`.
    pub fn to_hash_set(&self) -> FxHashSet<usize> {
        let mut set = FxHashSet::with_capacity_and_hasher(self.count(), Default::default());
        for block in self.iter() {
            set.insert(block);
        }
        set
    }
}

/// Iterator over set block indices in a `BlockBitSet`.
pub struct BlockBitSetIter<'a> {
    bitset: &'a BlockBitSet,
    word_idx: usize,
    current_word: u64,
}

impl<'a> Iterator for BlockBitSetIter<'a> {
    type Item = usize;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        while self.current_word == 0 {
            self.word_idx += 1;
            if self.word_idx >= self.bitset.words.len() {
                return None;
            }
            self.current_word = self.bitset.words[self.word_idx];
        }

        let bit = self.current_word.trailing_zeros() as usize;
        self.current_word &= self.current_word - 1; // Clear lowest set bit (BLSR)
        Some(self.word_idx * 64 + bit)
    }
}

/// High-performance dominance query engine.
///
/// Precomputes dominator tree discovery/finish timestamps (`tin` and `tout`)
/// in a single linear-time $O(V)$ pass, enabling arbitrary dominance queries
/// in **$O(1)$ time** (2 integer comparisons) instead of $O(\text{depth})$ `idom` chain walks.
#[derive(Clone, Debug)]
pub struct DominanceChecker {
    tin: Vec<u32>,
    tout: Vec<u32>,
}

impl DominanceChecker {
    /// Build a dominance checker from CFG block count and immediate dominators.
    ///
    /// Handles multiple disconnected components and arbitrary root markers (`idom[u] == u`
    /// or `idom[u] == usize::MAX`).
    pub fn new(num_blocks: usize, idom: &[usize]) -> Self {
        if num_blocks == 0 {
            return Self {
                tin: Vec::new(),
                tout: Vec::new(),
            };
        }

        // 1. Count children to build a contiguous adjacency array (0 per-node allocation)
        let mut child_counts = vec![0u32; num_blocks];
        let mut is_root = vec![false; num_blocks];

        for u in 0..num_blocks {
            if u < idom.len() {
                let p = idom[u];
                if p == u || p == usize::MAX || p >= num_blocks {
                    is_root[u] = true;
                } else {
                    child_counts[p] += 1;
                }
            } else {
                is_root[u] = true;
            }
        }

        // 2. Compute prefix sum offsets
        let mut offsets = vec![0u32; num_blocks + 1];
        for i in 0..num_blocks {
            offsets[i + 1] = offsets[i] + child_counts[i];
        }

        let total_edges = offsets[num_blocks] as usize;
        let mut child_storage = vec![0u32; total_edges];
        let mut current_offset = offsets.clone();

        for u in 0..num_blocks {
            if u < idom.len() {
                let p = idom[u];
                if !(p == u || p == usize::MAX || p >= num_blocks) {
                    let pos = current_offset[p] as usize;
                    child_storage[pos] = u as u32;
                    current_offset[p] += 1;
                }
            }
        }

        // 3. Non-recursive DFS timestamping
        let mut tin = vec![0u32; num_blocks];
        let mut tout = vec![0u32; num_blocks];
        let mut timer = 0u32;
        let mut stack = Vec::with_capacity(num_blocks);

        for root in 0..num_blocks {
            if is_root[root] && tin[root] == 0 {
                timer += 1;
                tin[root] = timer;
                let start = offsets[root] as usize;
                let end = offsets[root + 1] as usize;
                stack.push((root, start, end));

                while let Some(&mut (u, ref mut idx, end_idx)) = stack.last_mut() {
                    if *idx < end_idx {
                        let child = child_storage[*idx] as usize;
                        *idx += 1;
                        if tin[child] == 0 {
                            timer += 1;
                            tin[child] = timer;
                            let c_start = offsets[child] as usize;
                            let c_end = offsets[child + 1] as usize;
                            stack.push((child, c_start, c_end));
                        }
                    } else {
                        timer += 1;
                        tout[u] = timer;
                        stack.pop();
                    }
                }
            }
        }

        Self { tin, tout }
    }

    /// Returns `true` if block `a` dominates block `b` (reflexive: `a` dominates `a`).
    #[inline(always)]
    pub fn dominates(&self, a: usize, b: usize) -> bool {
        if a < self.tin.len() && b < self.tin.len() {
            self.tin[a] <= self.tin[b] && self.tout[a] >= self.tout[b]
        } else {
            false
        }
    }

    /// Returns `true` if block `a` strictly dominates block `b` (`a` dominates `b` and `a != b`).
    #[inline(always)]
    pub fn strictly_dominates(&self, a: usize, b: usize) -> bool {
        a != b && self.dominates(a, b)
    }
}

/// A natural loop identified by its header block and the set of blocks in the loop body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NaturalLoop {
    /// The header block index - the single entry target of back edges.
    pub header: usize,
    /// All block indices that form the loop body (includes the header).
    pub body: FxHashSet<usize>,
}

impl NaturalLoop {
    /// Returns `true` if the loop body contains the given block.
    #[inline(always)]
    pub fn contains(&self, block: usize) -> bool {
        self.body.contains(&block)
    }

    /// Number of basic blocks in the loop body.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.body.len()
    }

    /// Returns `true` if the loop body is empty (should never happen for valid loops).
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.body.is_empty()
    }

    /// Returns `true` if this is a single-block self-loop (`header -> header`).
    #[inline(always)]
    pub fn is_self_loop(&self) -> bool {
        self.body.len() == 1 && self.body.contains(&self.header)
    }

    /// Get all latch blocks (blocks in the loop with a back edge targeting `header`).
    pub fn latches(&self, preds: &analysis::FlatAdj) -> Vec<usize> {
        preds
            .row(self.header)
            .iter()
            .map(|&p| p as usize)
            .filter(|&p| self.body.contains(&p))
            .collect()
    }

    /// Get the single latch block if the loop has exactly one back edge, or `None` otherwise.
    pub fn single_latch(&self, preds: &analysis::FlatAdj) -> Option<usize> {
        let latches = self.latches(preds);
        if latches.len() == 1 {
            Some(latches[0])
        } else {
            None
        }
    }

    /// Find all exiting blocks (blocks inside the loop that have at least one successor outside).
    pub fn exiting_blocks(&self, succs: &analysis::FlatAdj) -> Vec<usize> {
        let mut exiting = Vec::new();
        for &block in &self.body {
            for &succ in succs.row(block) {
                if !self.body.contains(&(succ as usize)) {
                    exiting.push(block);
                    break;
                }
            }
        }
        exiting.sort_unstable();
        exiting
    }

    /// Find all exit target blocks (blocks outside the loop that are successors of loop blocks).
    pub fn exit_blocks(&self, succs: &analysis::FlatAdj) -> Vec<usize> {
        let mut exits = FxHashSet::default();
        for &block in &self.body {
            for &succ in succs.row(block) {
                let s = succ as usize;
                if !self.body.contains(&s) {
                    exits.insert(s);
                }
            }
        }
        let mut result: Vec<usize> = exits.into_iter().collect();
        result.sort_unstable();
        result
    }

    /// Find all exit edges `(from, to)` where `from` is inside the loop and `to` is outside.
    pub fn exit_edges(&self, succs: &analysis::FlatAdj) -> Vec<(usize, usize)> {
        let mut edges = Vec::new();
        for &block in &self.body {
            for &succ in succs.row(block) {
                let s = succ as usize;
                if !self.body.contains(&s) {
                    edges.push((block, s));
                }
            }
        }
        edges.sort_unstable();
        edges
    }

    /// Find the preheader block of this loop if one exists.
    #[inline(always)]
    pub fn find_preheader(&self, preds: &analysis::FlatAdj) -> Option<usize> {
        find_preheader(self.header, &self.body, preds)
    }

    /// Find the dedicated preheader block of this loop if one exists.
    #[inline(always)]
    pub fn find_dedicated_preheader(
        &self,
        preds: &analysis::FlatAdj,
        succs: &analysis::FlatAdj,
    ) -> Option<usize> {
        find_dedicated_preheader(self.header, &self.body, preds, succs)
    }

    /// Returns `true` if `self` is a proper subloop of `other`.
    #[inline(always)]
    pub fn is_subloop_of(&self, other: &NaturalLoop) -> bool {
        self.body.len() < other.body.len() && self.body.is_subset(&other.body)
    }

    /// Compute a topological / Reverse Post-Order (RPO) ordering of basic blocks in the loop.
    ///
    /// Essential for LICM and SSA dataflow analysis: ensures dominators within the loop
    /// are visited before dominated blocks.
    pub fn blocks_in_rpo(&self, succs: &analysis::FlatAdj) -> Vec<usize> {
        let mut visited = FxHashSet::default();
        let mut post_order = Vec::with_capacity(self.body.len());
        let mut stack: Vec<(usize, usize)> = Vec::with_capacity(self.body.len());

        visited.insert(self.header);
        stack.push((self.header, 0));

        while let Some(&mut (u, ref mut edge_idx)) = stack.last_mut() {
            let row = succs.row(u);
            let mut pushed = false;

            while *edge_idx < row.len() {
                let v = row[*edge_idx] as usize;
                *edge_idx += 1;

                if self.body.contains(&v) && visited.insert(v) {
                    stack.push((v, 0));
                    pushed = true;
                    break;
                }
            }

            if !pushed {
                post_order.push(u);
                stack.pop();
            }
        }

        // Include any remaining blocks defensively (in sorted order for determinism)
        if post_order.len() < self.body.len() {
            let mut remaining: Vec<usize> = self
                .body
                .iter()
                .copied()
                .filter(|b| !visited.contains(b))
                .collect();
            remaining.sort_unstable();
            for b in remaining {
                post_order.push(b);
            }
        }

        post_order.reverse();
        post_order
    }

    /// Check if this loop is in Canonical Simplified Loop Form:
    /// 1. Has a dedicated preheader (`find_dedicated_preheader` is `Some`).
    /// 2. Has a single latch block (`single_latch` is `Some`).
    /// 3. Has dedicated exit blocks (all predecessors of each exit block are inside the loop).
    pub fn is_canonical(&self, preds: &analysis::FlatAdj, succs: &analysis::FlatAdj) -> bool {
        if self.find_dedicated_preheader(preds, succs).is_none() {
            return false;
        }
        if self.single_latch(preds).is_none() {
            return false;
        }
        for exit in self.exit_blocks(succs) {
            for &p in preds.row(exit) {
                if !self.body.contains(&(p as usize)) {
                    return false;
                }
            }
        }
        true
    }
}

/// Find all natural loops in the CFG.
///
/// A natural loop is defined by a back edge (tail -> header) where the header
/// dominates the tail. The loop body is the set of blocks that can reach the
/// tail without going through the header.
pub fn find_natural_loops(
    num_blocks: usize,
    preds: &analysis::FlatAdj,
    succs: &analysis::FlatAdj,
    idom: &[usize],
) -> Vec<NaturalLoop> {
    if num_blocks == 0 {
        return Vec::new();
    }

    let dom = DominanceChecker::new(num_blocks, idom);
    let mut loops = Vec::new();

    // Find back edges: an edge (tail -> header) where header dominates tail in O(1)
    for tail in 0..num_blocks {
        for &header in succs.row(tail) {
            let header = header as usize;
            if header < num_blocks && dom.dominates(header, tail) {
                let body = compute_loop_body(header, tail, preds);
                loops.push(NaturalLoop { header, body });
            }
        }
    }

    loops
}

/// Find and merge natural loops in a single optimized pass.
///
/// Groups all back edges targeting the same header and computes the unified loop body
/// in a single multi-source backward traversal, eliminating duplicate BFS/DFS work.
pub fn find_merged_natural_loops(
    num_blocks: usize,
    preds: &analysis::FlatAdj,
    succs: &analysis::FlatAdj,
    idom: &[usize],
) -> Vec<NaturalLoop> {
    if num_blocks == 0 {
        return Vec::new();
    }

    let dom = DominanceChecker::new(num_blocks, idom);
    let mut header_to_tails: FxHashMap<usize, Vec<usize>> = FxHashMap::default();

    for tail in 0..num_blocks {
        for &header in succs.row(tail) {
            let header = header as usize;
            if header < num_blocks && dom.dominates(header, tail) {
                header_to_tails.entry(header).or_default().push(tail);
            }
        }
    }

    let mut headers: Vec<usize> = header_to_tails.keys().copied().collect();
    headers.sort_unstable();

    let mut result = Vec::with_capacity(headers.len());
    for header in headers {
        let tails = &header_to_tails[&header];
        let body = compute_loop_body_multi_bitset(header, tails, preds, num_blocks).to_hash_set();
        result.push(NaturalLoop { header, body });
    }

    result
}

/// Merge natural loops that share the same header block.
///
/// Ensures deterministic output order (sorted by header index) for bitwise reproducible builds.
pub fn merge_loops_by_header(loops: Vec<NaturalLoop>) -> Vec<NaturalLoop> {
    if loops.len() <= 1 {
        return loops;
    }

    // Fast-path: check if headers are already unique to avoid hash map overhead
    let mut unique_headers = true;
    let mut seen = FxHashSet::default();
    for nl in &loops {
        if !seen.insert(nl.header) {
            unique_headers = false;
            break;
        }
    }

    if unique_headers {
        let mut sorted = loops;
        sorted.sort_by_key(|l| l.header);
        return sorted;
    }

    let mut header_map: FxHashMap<usize, FxHashSet<usize>> = FxHashMap::default();
    for nl in loops {
        header_map
            .entry(nl.header)
            .or_default()
            .extend(nl.body);
    }

    let mut headers: Vec<usize> = header_map.keys().copied().collect();
    headers.sort_unstable();

    headers
        .into_iter()
        .map(|header| {
            let body = header_map.remove(&header).unwrap();
            NaturalLoop { header, body }
        })
        .collect()
}

/// Compute the body of a natural loop given a back edge (tail -> header).
pub fn compute_loop_body(
    header: usize,
    tail: usize,
    preds: &analysis::FlatAdj,
) -> FxHashSet<usize> {
    let mut body = FxHashSet::default();
    body.insert(header);

    if header == tail {
        return body;
    }

    let mut worklist = Vec::with_capacity(8);
    body.insert(tail);
    worklist.push(tail);

    while let Some(block) = worklist.pop() {
        for &pred in preds.row(block) {
            let pred = pred as usize;
            if body.insert(pred) {
                worklist.push(pred);
            }
        }
    }

    body
}

/// Compute the unified body of a natural loop with multiple back edges targeting `header`.
pub fn compute_loop_body_multi(
    header: usize,
    tails: &[usize],
    preds: &analysis::FlatAdj,
) -> FxHashSet<usize> {
    let mut body = FxHashSet::default();
    body.insert(header);

    let mut worklist = Vec::with_capacity(tails.len() * 2);
    for &tail in tails {
        if body.insert(tail) {
            worklist.push(tail);
        }
    }

    while let Some(block) = worklist.pop() {
        for &pred in preds.row(block) {
            let pred = pred as usize;
            if body.insert(pred) {
                worklist.push(pred);
            }
        }
    }

    body
}

/// Compute the body of a natural loop using an allocation-efficient `BlockBitSet`.
pub fn compute_loop_body_multi_bitset(
    header: usize,
    tails: &[usize],
    preds: &analysis::FlatAdj,
    num_blocks: usize,
) -> BlockBitSet {
    let mut bitset = BlockBitSet::new(num_blocks);
    bitset.insert(header);

    let mut worklist = Vec::with_capacity(tails.len() * 2);
    for &tail in tails {
        if bitset.insert(tail) {
            worklist.push(tail);
        }
    }

    while let Some(block) = worklist.pop() {
        for &pred in preds.row(block) {
            let pred = pred as usize;
            if bitset.insert(pred) {
                worklist.push(pred);
            }
        }
    }

    bitset
}

/// Find a suitable preheader block for a loop.
///
/// The preheader must be the single predecessor of the header that is not
/// part of the loop body. Returns `None` if no unique preheader exists.
///
/// Performs **0 heap allocations** and early-exits on finding a second external predecessor.
#[inline]
pub fn find_preheader(
    header: usize,
    loop_body: &FxHashSet<usize>,
    preds: &analysis::FlatAdj,
) -> Option<usize> {
    let mut unique_preheader = None;
    for &pred in preds.row(header) {
        let p = pred as usize;
        if !loop_body.contains(&p) {
            if unique_preheader.is_some() {
                return None;
            }
            unique_preheader = Some(p);
        }
    }
    unique_preheader
}

/// Find a dedicated preheader block for a loop.
///
/// A dedicated preheader is a unique preheader that has the loop header as its
/// ONLY successor. This guarantees that entering the preheader will always
/// immediately transfer control to the loop header without branching elsewhere.
#[inline]
pub fn find_dedicated_preheader(
    header: usize,
    loop_body: &FxHashSet<usize>,
    preds: &analysis::FlatAdj,
    succs: &analysis::FlatAdj,
) -> Option<usize> {
    let preheader = find_preheader(header, loop_body, preds)?;
    let row = succs.row(preheader);
    if row.len() == 1 && row[0] as usize == header {
        Some(preheader)
    } else {
        None
    }
}

/// Full loop hierarchy and nesting analysis for a CFG.
///
/// Constructs a tree/forest of loops where outer loops are parents of inner loops.
/// Provides loop nesting depths, block-to-loop mappings, and topological traversals
/// (innermost-first for LICM, outermost-first for unrolling).
#[derive(Clone, Debug)]
pub struct LoopNest {
    /// All merged natural loops in the function.
    pub loops: Vec<NaturalLoop>,
    /// Parent loop index for each loop in `loops` (`None` for top-level loops).
    pub parent: Vec<Option<usize>>,
    /// Child loop indices for each loop in `loops`.
    pub children: Vec<Vec<usize>>,
    /// Index of the innermost loop containing each block, or `None` if the block is not in any loop.
    pub block_to_loop: Vec<Option<usize>>,
    /// Nesting depth for each block (0 = outside any loop, 1 = in top-level loop, 2 = nested once, etc.).
    pub block_depth: Vec<usize>,
}

impl LoopNest {
    /// Build loop hierarchy analysis from CFG block count and a list of merged natural loops.
    pub fn analyze(num_blocks: usize, loops: &[NaturalLoop]) -> Self {
        let num_loops = loops.len();
        let mut parent = vec![None; num_loops];
        let mut children = vec![Vec::new(); num_loops];

        // 1. Determine parent-child relations (smallest containing loop is immediate parent)
        for b_idx in 0..num_loops {
            let b_body = &loops[b_idx].body;
            let mut smallest_parent = None;
            let mut smallest_size = usize::MAX;

            for a_idx in 0..num_loops {
                if a_idx == b_idx {
                    continue;
                }
                let a_body = &loops[a_idx].body;
                if a_body.len() > b_body.len() && b_body.is_subset(a_body) {
                    if a_body.len() < smallest_size {
                        smallest_size = a_body.len();
                        smallest_parent = Some(a_idx);
                    }
                }
            }

            if let Some(p) = smallest_parent {
                parent[b_idx] = Some(p);
                children[p].push(b_idx);
            }
        }

        // 2. Assign each block to its innermost containing loop
        let mut block_to_loop = vec![None; num_blocks];
        for (l_idx, l) in loops.iter().enumerate() {
            for &blk in &l.body {
                if blk < num_blocks {
                    let curr = block_to_loop[blk];
                    if curr.is_none() || loops[l_idx].body.len() < loops[curr.unwrap()].body.len() {
                        block_to_loop[blk] = Some(l_idx);
                    }
                }
            }
        }

        // 3. Compute nesting depth for every block
        let mut block_depth = vec![0usize; num_blocks];
        for blk in 0..num_blocks {
            let mut curr = block_to_loop[blk];
            let mut depth = 0;
            while let Some(l_idx) = curr {
                depth += 1;
                curr = parent[l_idx];
            }
            block_depth[blk] = depth;
        }

        Self {
            loops: loops.to_vec(),
            parent,
            children,
            block_to_loop,
            block_depth,
        }
    }

    /// Returns loop indices in **innermost-first (post-order)** traversal order.
    ///
    /// This is the required processing order for LICM and strength reduction.
    pub fn innermost_first(&self) -> Vec<usize> {
        let mut visited = vec![false; self.loops.len()];
        let mut order = Vec::with_capacity(self.loops.len());

        for root in 0..self.loops.len() {
            if self.parent[root].is_none() && !visited[root] {
                self.dfs_post_order(root, &mut visited, &mut order);
            }
        }

        order
    }

    /// Returns loop indices in **outermost-first (pre-order)** traversal order.
    pub fn outermost_first(&self) -> Vec<usize> {
        let mut visited = vec![false; self.loops.len()];
        let mut order = Vec::with_capacity(self.loops.len());

        for root in 0..self.loops.len() {
            if self.parent[root].is_none() && !visited[root] {
                self.dfs_pre_order(root, &mut visited, &mut order);
            }
        }

        order
    }

    fn dfs_post_order(&self, u: usize, visited: &mut [bool], order: &mut Vec<usize>) {
        visited[u] = true;
        for &child in &self.children[u] {
            if !visited[child] {
                self.dfs_post_order(child, visited, order);
            }
        }
        order.push(u);
    }

    fn dfs_pre_order(&self, u: usize, visited: &mut [bool], order: &mut Vec<usize>) {
        visited[u] = true;
        order.push(u);
        for &child in &self.children[u] {
            if !visited[child] {
                self.dfs_pre_order(child, visited, order);
            }
        }
    }

    /// Get the innermost loop containing a basic block.
    #[inline(always)]
    pub fn loop_for_block(&self, block: usize) -> Option<&NaturalLoop> {
        self.block_to_loop
            .get(block)
            .and_then(|&opt| opt)
            .map(|idx| &self.loops[idx])
    }

    /// Get loop nesting depth for a basic block (0 = outside loops).
    #[inline(always)]
    pub fn loop_depth(&self, block: usize) -> usize {
        self.block_depth.get(block).copied().unwrap_or(0)
    }

    /// Check if a loop is an innermost loop (has no child subloops).
    #[inline(always)]
    pub fn is_innermost(&self, loop_idx: usize) -> bool {
        self.children.get(loop_idx).map_or(true, |c| c.is_empty())
    }

    /// Check if a loop is an outermost loop (has no parent loop).
    #[inline(always)]
    pub fn is_outermost(&self, loop_idx: usize) -> bool {
        self.parent.get(loop_idx).map_or(true, |p| p.is_none())
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Test helper for building mock FlatAdj structures.
    struct MockAdj {
        rows: Vec<Vec<u32>>,
    }

    impl MockAdj {
        fn new(adj: Vec<Vec<u32>>) -> Self {
            Self { rows: adj }
        }

        fn as_flat(&self) -> analysis::FlatAdj {
            let num_rows = self.rows.len();
            let mut row_offsets = Vec::with_capacity(num_rows + 1);
            let mut storage = Vec::new();

            for row in &self.rows {
                row_offsets.push(storage.len() as u32);
                storage.extend_from_slice(row);
            }
            row_offsets.push(storage.len() as u32);

            analysis::FlatAdj {
                num_rows,
                row_offsets,
                storage,
            }
        }
    }

    #[test]
    fn test_block_bitset_comprehensive() {
        let mut bs1 = BlockBitSet::new(130);
        assert_eq!(bs1.count(), 0);
        assert!(!bs1.contains(0));
        assert!(!bs1.contains(63));
        assert!(!bs1.contains(64));
        assert!(!bs1.contains(129));

        assert!(bs1.insert(0));
        assert!(bs1.insert(63));
        assert!(bs1.insert(64));
        assert!(bs1.insert(129));
        assert!(!bs1.insert(0)); // Duplicate
        assert_eq!(bs1.count(), 4);

        assert!(bs1.contains(0));
        assert!(bs1.contains(63));
        assert!(bs1.contains(64));
        assert!(bs1.contains(129));
        assert!(!bs1.contains(1));

        let elements: Vec<usize> = bs1.iter().collect();
        assert_eq!(elements, vec![0, 63, 64, 129]);

        let mut bs2 = BlockBitSet::new(130);
        bs2.insert(64);
        assert!(bs2.is_subset(&bs1));
        assert!(!bs1.is_subset(&bs2));

        assert!(bs1.remove(64));
        assert!(!bs1.contains(64));
        assert_eq!(bs1.count(), 3);
    }

    #[test]
    fn test_dominance_checker_basic_and_strict() {
        // CFG: 0 -> 1 -> 2 -> 3
        let idom = vec![0, 0, 1, 2];
        let dom = DominanceChecker::new(4, &idom);

        assert!(dom.dominates(0, 0));
        assert!(dom.dominates(0, 1));
        assert!(dom.dominates(0, 2));
        assert!(dom.dominates(0, 3));
        assert!(dom.dominates(1, 2));
        assert!(dom.dominates(1, 3));
        assert!(!dom.dominates(2, 1));
        assert!(!dom.dominates(3, 0));

        assert!(dom.strictly_dominates(0, 1));
        assert!(!dom.strictly_dominates(0, 0));
        assert!(!dom.dominates(99, 1)); // Out of bounds safety
    }

    #[test]
    fn test_self_loop() {
        // CFG: 0 -> 1 -> 1 (self loop), 1 -> 2
        let succs = MockAdj::new(vec![vec![1], vec![1, 2], vec![]]).as_flat();
        let preds = MockAdj::new(vec![vec![], vec![0, 1], vec![1]]).as_flat();
        let idom = vec![0, 0, 1];

        let loops = find_natural_loops(3, &preds, &succs, &idom);
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].header, 1);
        assert!(loops[0].is_self_loop());
        assert_eq!(loops[0].body, [1].into_iter().collect());
        assert_eq!(loops[0].single_latch(&preds), Some(1));
    }

    #[test]
    fn test_single_loop_and_preheader() {
        // 0 -> 1 -> 2 -> 1 (backedge), 2 -> 3 (exit)
        let succs = MockAdj::new(vec![vec![1], vec![2], vec![1, 3], vec![]]).as_flat();
        let preds = MockAdj::new(vec![vec![], vec![0, 2], vec![1], vec![2]]).as_flat();
        let idom = vec![0, 0, 1, 2];

        let loops = find_natural_loops(4, &preds, &succs, &idom);
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].header, 1);
        assert_eq!(loops[0].body, [1, 2].into_iter().collect());

        assert_eq!(find_preheader(1, &loops[0].body, &preds), Some(0));
        assert_eq!(
            find_dedicated_preheader(1, &loops[0].body, &preds, &succs),
            Some(0)
        );

        assert_eq!(loops[0].latches(&preds), vec![2]);
        assert_eq!(loops[0].single_latch(&preds), Some(2));
        assert_eq!(loops[0].exiting_blocks(&succs), vec![2]);
        assert_eq!(loops[0].exit_blocks(&succs), vec![3]);
        assert_eq!(loops[0].exit_edges(&succs), vec![(2, 3)]);
        assert!(loops[0].is_canonical(&preds, &succs));
    }

    #[test]
    fn test_multi_latch_loop_merging() {
        // 0 -> 1, 1 -> 2, 1 -> 3, 2 -> 1 (backedge 1), 3 -> 1 (backedge 2), 1 -> 4
        let succs = MockAdj::new(vec![vec![1], vec![2, 3, 4], vec![1], vec![1], vec![]]).as_flat();
        let preds = MockAdj::new(vec![vec![], vec![0, 2, 3], vec![1], vec![1], vec![1]]).as_flat();
        let idom = vec![0, 0, 1, 1, 1];

        let unmerged = find_natural_loops(5, &preds, &succs, &idom);
        assert_eq!(unmerged.len(), 2);

        let merged = merge_loops_by_header(unmerged);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].header, 1);
        assert_eq!(merged[0].body, [1, 2, 3].into_iter().collect());

        let single_pass = find_merged_natural_loops(5, &preds, &succs, &idom);
        assert_eq!(single_pass, merged);
    }

    #[test]
    fn test_nested_and_sibling_loops_hierarchy() {
        // CFG:
        // 0 -> 1 -> 2 (Outer A header) -> 3 (Inner A header) -> 4 -> 3 (Inner A backedge)
        // 4 -> 2 (Outer A backedge), 2 -> 5 (Exit A) -> 6 (Loop B header) -> 7 -> 6 (Loop B backedge), 7 -> 8 (Exit B)
        let l_outer_a = NaturalLoop {
            header: 2,
            body: [2, 3, 4].into_iter().collect(),
        };
        let l_inner_a = NaturalLoop {
            header: 3,
            body: [3, 4].into_iter().collect(),
        };
        let l_loop_b = NaturalLoop {
            header: 6,
            body: [6, 7].into_iter().collect(),
        };

        let loops = vec![l_outer_a, l_inner_a, l_loop_b];
        let nest = LoopNest::analyze(9, &loops);

        assert_eq!(nest.parent, vec![None, Some(0), None]);
        assert_eq!(nest.children[0], vec![1]);
        assert!(nest.children[1].is_empty());
        assert!(nest.children[2].is_empty());

        assert_eq!(nest.block_depth, vec![0, 0, 1, 2, 2, 0, 1, 1, 0]);
        assert_eq!(nest.innermost_first(), vec![1, 0, 2]);
        assert_eq!(nest.outermost_first(), vec![0, 1, 2]);

        assert!(nest.is_innermost(1));
        assert!(nest.is_innermost(2));
        assert!(!nest.is_innermost(0));

        assert!(nest.is_outermost(0));
        assert!(nest.is_outermost(2));
        assert!(!nest.is_outermost(1));
    }

    #[test]
    fn test_loop_rpo_traversal() {
        // Loop with internal diamond:
        // 0 -> 1 -> (2, 3) -> 4 -> 1 (backedge), 4 -> 5 (exit)
        let succs = MockAdj::new(vec![
            vec![1],
            vec![2, 3],
            vec![4],
            vec![4],
            vec![1, 5],
            vec![],
        ])
        .as_flat();

        let l = NaturalLoop {
            header: 1,
            body: [1, 2, 3, 4].into_iter().collect(),
        };

        let rpo = l.blocks_in_rpo(&succs);
        assert_eq!(rpo[0], 1); // Header must be first
        assert_eq!(rpo[3], 4); // Latch must be last
        assert!(rpo.contains(&2));
        assert!(rpo.contains(&3));
    }

    #[test]
    fn test_empty_and_unreachable_cfg() {
        let empty_preds = MockAdj::new(vec![]).as_flat();
        let empty_succs = MockAdj::new(vec![]).as_flat();
        let loops = find_natural_loops(0, &empty_preds, &empty_succs, &[]);
        assert!(loops.is_empty());

        let nest = LoopNest::analyze(0, &[]);
        assert!(nest.loops.is_empty());
        assert!(nest.innermost_first().is_empty());
    }
}
