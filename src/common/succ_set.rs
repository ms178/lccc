//! The "successor edges of one CFG block, deduplicated, in insertion order"
//! container, shared by every pipeline stage that builds a CFG.
//!
//! A block's terminator names its successors, and a well-formed terminator
//! names each target at most once.  Real input is not always well-formed --
//! `switch (x) { case 0: case 1: ... }` lowers to a `Switch` whose case list
//! repeats targets, and `CondBranch { true_label == false_label }` is legal --
//! so every CFG builder dedups.  They all used to write
//!
//! ```ignore
//! if !succs[from].contains(&to) { succs[from].push(to); }
//! ```
//!
//! which is O(degree^2) per block.  That is fine for ordinary code and
//! pathological for the two shapes machine-generated C loves: a single
//! `switch` with N arms that all share one body reaches N targets from one
//! block (one `dispatch` block, degree N), and an N-arm switch whose arms
//! fall through into the next arm in IR-name order is *itself* the wide
//! block.  Measured on a 25,000-arm switch, `Vec::contains` in
//! `mem2reg::phi_eliminate::successor_count` alone was 8.2% of the entire
//! compile, and three sibling builders added 4.1% each (callgrind).
//!
//! [`SuccSet`] keeps the O(1) membership test in a per-block stamp vector and
//! pays nothing per edge beyond the push itself: `insert` marks
//! `stamp[to] = generation`, and a repeat is recognised without touching the
//! edge list.  The generation counter restarts only when the stamp vector has
//! been fully consumed (2^32 inserts), so clearing it never costs a memset in
//! practice -- a detail that matters because the builders run per pass, per
//! function, on every compile.
//!
//! Alternatives considered and rejected:
//!
//! * A sorted `Vec` + `binary_search`: O(degree log degree) per block, no
//!   allocation, but successors are consumed in terminator order by
//!   `named_by_terminator` bookkeeping and by layout heuristics, so sorting
//!   would be a semantic change.  Rejected.
//! * A `FxHashSet<usize>` per block plus a `Vec`: one allocation per block
//!   and a hash on the hot path, for a container that is usually 1-2
//!   elements.  Slower than the stamp for real code.
//! * A per-function `vec![false; n]` cleared per block: the memset is
//!   O(n) per block, i.e. O(n^2) per function, which is the very quadratic
//!   this module removes (it is exactly what a "visited" array reused
//!   without draining would cost).
//! * A per-function bitset: 1 bit per block per *sequence*, so N sequences
//!   cost O(N^2/8) time to clear even though only O(N) bits are ever set.
//!   The stamp vector wins by making clear free.

use crate::common::fx_hash::FxHashMap;

/// Deduplicating, insertion-ordered accumulator for one block's successors.
#[derive(Debug, Default)]
pub(crate) struct SuccSet {
    /// Generation marker per target index. `stamp[to] == generation` means
    /// `to` is already an edge of the current block.
    stamp: Vec<u32>,
    generation: u32,
    edges: usize,
}

impl SuccSet {
    /// A fresh accumulator.  Allocates nothing until the first `begin`.
    pub(crate) fn new() -> Self {
        Self {
            stamp: Vec::new(),
            generation: 0,
            edges: 0,
        }
    }

    /// Number of distinct edges recorded since the last `begin`.
    pub(crate) fn len(&self) -> usize {
        self.edges
    }

    /// Start collecting the edges of one block.
    ///
    /// Must be called once per block.  Retired generations are reused by
    /// bumping the counter; the vector is only re-zeroed when the counter
    /// would wrap, which takes 2^32 inserts into the widest target index.
    pub(crate) fn begin(&mut self, num_targets: usize) {
        if self.stamp.len() < num_targets {
            self.stamp.resize(num_targets, 0);
        }
        self.edges = 0;
        self.generation = match self.generation.checked_add(1) {
            Some(g) => g,
            None => {
                // 2^32 blocks in one sequence: unreachable in practice, but
                // wrapping would silently drop edges, so pay the memset and
                // start over rather than wrap.
                self.stamp.fill(0);
                1
            }
        };
    }

    /// Record edge `to` if this block has not recorded it yet.
    ///
    /// Returns `true` when the edge was new.
    ///
    /// `to` is a target block index and is therefore always
    /// `< num_targets` for a consistent caller.  A caller that declares too
    /// few targets gets its stamp GROWN rather than its edge dropped: a
    /// silently missing CFG edge is a miscompile, while a larger stamp is
    /// only a little memory.  The sole case that returns `false` for an
    /// in-range-looking index is one that cannot be represented as a length,
    /// which no block index can be.
    #[inline]
    pub(crate) fn insert(&mut self, to: usize) -> bool {
        if to >= self.stamp.len() {
            match to.checked_add(1) {
                Some(needed) => self.stamp.resize(needed, 0),
                None => return false,
            }
        }
        if self.stamp[to] == self.generation {
            return false;
        }
        self.stamp[to] = self.generation;
        self.edges += 1;
        true
    }
}

/// Build one CFG successor list per block, in one pass, deduplicated.
///
/// `label_to_idx` maps a block label to its index; this is the single shape
/// every CFG builder in the pipeline needs (mem2reg, both back ends, block
/// layout and copy coalescing all build it before walking terminators), so
/// the loop lives here instead of being written out four times with four
/// chances to reintroduce `Vec::contains`.
///
/// `successors(from, add)` is called once per block and must call `add(label)`
/// for every target label of that block's terminator (and its asm-goto
/// labels), in the order the pipeline documents.  The edges keep exactly that
/// order.  `add` takes only the label and is created per block, so there is no
/// way for a caller to attribute an edge to the wrong block.
///
/// `add` returns `true` when the label became a NEW edge of this block: a
/// label that does not resolve (dangling) and a repeat of an edge the block
/// already has both return `false`.  Block layout needs that distinction to
/// count how many successors the terminator itself contributed, and getting
/// it here keeps the dedup knowledge in one place.
pub(crate) fn build_edges<F>(
    num_blocks: usize,
    label_to_idx: &FxHashMap<u32, usize>,
    mut successors: F,
) -> Vec<Vec<usize>>
where
    F: FnMut(usize, &mut dyn FnMut(u32) -> bool),
{
    let mut edges: Vec<Vec<usize>> = vec![Vec::new(); num_blocks];
    let mut set = SuccSet::new();
    for (from, out) in edges.iter_mut().enumerate() {
        set.begin(num_blocks);
        successors(from, &mut |label| match label_to_idx.get(&label) {
            Some(&to) if set.insert(to) => {
                out.push(to);
                true
            }
            _ => false,
        });
    }
    edges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_reports_each_edge_once_and_marks_new_first() {
        let mut s = SuccSet::new();
        s.begin(8);
        assert!(s.insert(3));
        assert!(!s.insert(3), "repeat must be rejected");
        assert!(s.insert(5));
        assert!(s.insert(1));
        assert!(!s.insert(1));
        assert!(!s.insert(5));
        // A block that never inserts anything leaves the stamp untouched.
        s.begin(8);
        assert!(s.insert(3), "a new generation forgets the previous block");
    }

    #[test]
    fn len_counts_distinct_edges_since_begin() {
        let mut s = SuccSet::new();
        s.begin(8);
        assert_eq!(s.len(), 0);
        assert!(s.insert(2));
        assert!(s.insert(6));
        assert!(!s.insert(2));
        assert_eq!(s.len(), 2);
        s.begin(8);
        assert_eq!(s.len(), 0, "begin starts a new block");
        assert!(s.insert(6));
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn begin_clears_only_the_previous_block() {
        let mut s = SuccSet::new();
        for block in 0..1000u32 {
            s.begin(4);
            assert!(s.insert(block as usize % 4));
            assert!(!s.insert(block as usize % 4));
            for other in 0..4 {
                if other != block as usize % 4 {
                    assert!(s.insert(other), "block {block} target {other}");
                }
            }
        }
    }

    #[test]
    fn insert_beyond_the_declared_range_grows_instead_of_dropping_the_edge() {
        // A caller that under-declares must not lose a CFG edge: dropping one
        // is a miscompile, growing the stamp is only memory.
        let mut s = SuccSet::new();
        s.begin(2);
        assert!(s.insert(1));
        assert!(s.insert(9), "an under-declared target must still be recorded");
        assert!(!s.insert(9));
        assert!(s.insert(0), "growth must not disturb target 0");
        assert!(s.insert(4), "the grown region is usable too");
        assert!(!s.insert(4));
        // An index that cannot be a length is the only rejected one; it must
        // not panic and must not disturb anything already recorded.
        assert!(!s.insert(usize::MAX));
        assert!(!s.insert(1));
    }

    #[test]
    fn generation_wrap_rezeroes_instead_of_dropping_edges() {
        let mut s = SuccSet::new();
        s.begin(4);
        assert!(s.insert(2));
        // Force the counter to the last value before wrap and consume it.
        s.generation = u32::MAX;
        s.begin(4); // `checked_add` fails -> memset + generation 1
        assert_eq!(s.generation, 1);
        assert!(s.insert(2), "wrap must not remember stale marks");
        assert!(!s.insert(2));
    }

    #[test]
    fn wide_center_is_linear_in_edges() {
        // A dispatch block naming n targets: the shape that was quadratic.
        let n = 100_000usize;
        let mut s = SuccSet::new();
        s.begin(n);
        let mut out = Vec::new();
        for to in (0..n).rev() {
            if s.insert(to) {
                out.push(to);
            }
        }
        assert_eq!(out.len(), n);
        // A second pass over the same block adds nothing.
        for to in 0..n {
            assert!(!s.insert(to));
        }
    }

    #[test]
    fn build_edges_preserves_insertion_order_and_dedups() {
        let mut map = FxHashMap::default();
        for (idx, label) in [10u32, 20, 30, 40].into_iter().enumerate() {
            map.insert(label, idx);
        }
        // Block 0 names 30, 10, 30, 99 (dangling), 20 -- order must survive,
        // and no other block may acquire an edge from it.
        let edges = build_edges(4, &map, |from, add| {
            if from == 0 {
                assert!(add(30), "first edge");
                assert!(add(10), "second edge");
                assert!(!add(30), "a repeated edge reports false");
                assert!(!add(99), "a dangling label reports false");
                assert!(add(20), "third edge");
            }
        });
        assert_eq!(edges, vec![vec![2, 0, 1], vec![], vec![], vec![]]);
    }

    #[test]
    fn build_edges_gives_every_block_its_own_generation() {
        let mut map = FxHashMap::default();
        for (idx, label) in [1u32, 2, 3].into_iter().enumerate() {
            map.insert(label, idx);
        }
        // Every block names every target: without per-block generations only
        // block 0 would get edges.
        let edges = build_edges(3, &map, |_from, add| {
            for label in [1u32, 2, 3] {
                assert!(add(label));
            }
        });
        for block in &edges {
            assert_eq!(block, &vec![0, 1, 2]);
        }
    }
}
