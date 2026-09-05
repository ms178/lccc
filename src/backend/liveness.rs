//! Liveness analysis for IR values.
//!
//! Two live-range views (both use the same program-point numbering:
//! +1 per instruction, +1 per terminator):
//!
//! * [`LivenessResult::intervals`] — **fat** `[earliest def, last preserve]`
//!   per value. Loop-carried values cover the whole layout span of the loop.
//!   Part 1 `iv_map` / phi-coalesce / loop-pin depend on one interval per
//!   value (last-write-wins on a segmented vec is a miscompile).
//! * [`LivenessResult::segments`] — **hole-aware** pieces. A value defined
//!   in B0, used in B2, dead in B1 (diamond) does *not* cover B1. Linear
//!   scan on `segments` is the cheap 80% of LLVM's split: a call on the
//!   dead arm no longer forces a callee-saved GPR.
//!
//! Pipeline:
//! 1. Sequential program points.
//! 2. Per-block gen/kill bitsets (dense value ids). Implicit uses (GEP-fold,
//!    F128 source pointer, phi-incoming Copies) are inserted into gen *before*
//!    dataflow.
//! 3. Backward dataflow to a true fixpoint (worklist, seeded so exits pop
//!    first). No iteration cap — a cap is an under-approx → miscompile.
//!    `live_in[B]  = gen[B] ∪ (live_out[B] − kill[B])`
//!    `live_out[B] = ⋃ live_in[S]`  for successors `S` of `B`
//! 4. Fat intervals from live-through blocks + setjmp extension.
//! 5. Segments from raw def/use + live_in/live_out (holes preserved).
//!
//! Safe degradation is **over**-approximation (more spills), never under.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::reexports::{Instruction, IrBinOp, IrConst, IrFunction, Operand, Terminator, Value};
use std::sync::OnceLock;

/// `[start, end]` in program-point numbering.
/// `start` = defining point (or block entry if live-in), `end` = last point
/// the value must be preserved. Closed on both ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveInterval {
    pub start: u32,
    pub end: u32,
    pub value_id: u32,
}

/// Result of liveness analysis.
///
/// Extra fields beyond `intervals` / `call_points` / `block_loop_depth` are
/// additive: Part 1 that only reads those three stays correct. Switch the
/// linear scan to [`Self::segments`] to take the hole-aware win.
pub struct LivenessResult {
    /// Fat `[def, last_use]` — **exactly one** entry per value that has a def.
    pub intervals: Vec<LiveInterval>,
    /// Hole-aware pieces, sorted by `(start, value_id)`. Multiple per value.
    pub segments: Vec<LiveInterval>,
    /// Sorted program points that clobber the caller-saved set.
    pub call_points: Vec<u32>,
    /// Loop nesting depth per block. Length is always `func.blocks.len()`.
    pub block_loop_depth: Vec<u32>,
    /// Inclusive start point of each block. Length = `func.blocks.len()`.
    pub block_starts: Vec<u32>,
    /// Inclusive terminator point of each block. Length = `func.blocks.len()`.
    pub block_ends: Vec<u32>,
    /// One past the last assigned program point.
    pub num_points: u32,
    /// Values whose liveness was extended past their direct uses because they
    /// are GEP bases folded into Load/Store addressing. Each folded access
    /// reads the base register at the access point, so these values are used
    /// far more often than the raw operand walk records; the allocator must
    /// not rank them by that under-count (see regalloc's priority boost).
    pub gep_base_values: FxHashSet<u32>,
    /// Exact program points at which a folded Load/Store re-reads a value's
    /// register with NO IR operand (the GEP chain was absorbed into the
    /// addressing). Map of value id → the access points. This is the exact
    /// hidden-read table the phi-coalesce destructive-update veto consults:
    /// the block-granular segments cannot distinguish a same-block latch
    /// phi's live-in/live-out whole-block cover from a real read inside the
    /// update window (every tight-loop accumulator web was vetoed).
    pub folded_read_points: FxHashMap<u32, Vec<u32>>,
    id_to_dense: FxHashMap<u32, usize>,
    live_in: Vec<BitSet>,
    live_out: Vec<BitSet>,
}

impl LivenessResult {
    pub fn is_live_in(&self, block_idx: usize, value_id: u32) -> bool {
        let Some(set) = self.live_in.get(block_idx) else {
            return false;
        };
        let Some(&d) = self.id_to_dense.get(&value_id) else {
            return false;
        };
        set.contains(d)
    }

    pub fn is_live_out(&self, block_idx: usize, value_id: u32) -> bool {
        let Some(set) = self.live_out.get(block_idx) else {
            return false;
        };
        let Some(&d) = self.id_to_dense.get(&value_id) else {
            return false;
        };
        set.contains(d)
    }

    /// Block containing `point`, or `None` if out of range.
    pub fn block_index_at(&self, point: u32) -> Option<usize> {
        if self.block_starts.is_empty() {
            return None;
        }
        let i = self.block_starts.partition_point(|&s| s <= point);
        if i == 0 {
            return None;
        }
        let b = i - 1;
        if point <= self.block_ends[b] {
            Some(b)
        } else {
            None
        }
    }

    /// Point query against hole-aware segments (closed `[start, end]`).
    pub fn is_live_at(&self, value_id: u32, point: u32) -> bool {
        self.segments
            .iter()
            .any(|iv| iv.value_id == value_id && iv.start <= point && point <= iv.end)
    }

    /// True iff some **segment** strictly contains a call (`start < cp < end`).
    /// Matches Part 1 `spans_any_call`. Fat intervals over-approx this.
    pub fn live_across_any_call(&self, value_id: u32) -> bool {
        for iv in &self.segments {
            if iv.value_id != value_id {
                continue;
            }
            let idx = self.call_points.partition_point(|&cp| cp <= iv.start);
            if idx < self.call_points.len() && self.call_points[idx] < iv.end {
                return true;
            }
        }
        false
    }

    pub fn fat_interval(&self, value_id: u32) -> Option<(u32, u32)> {
        self.intervals
            .iter()
            .find(|iv| iv.value_id == value_id)
            .map(|iv| (iv.start, iv.end))
    }
}

// ── Compact bitset for dataflow ──────────────────────────────────────────────

/// Bitset over a dense `[0..num_bits)` index space, stored as `u64` words.
#[derive(Clone, Debug)]
struct BitSet {
    words: Vec<u64>,
}

impl BitSet {
    fn new(num_bits: usize) -> Self {
        Self {
            words: vec![0u64; num_bits.div_ceil(64)],
        }
    }

    #[inline(always)]
    fn insert(&mut self, idx: usize) {
        debug_assert!(
            idx / 64 < self.words.len(),
            "BitSet::insert index {idx} out of range ({} words)",
            self.words.len()
        );
        let word = idx / 64;
        let bit = idx % 64;
        self.words[word] |= 1u64 << bit;
    }

    #[inline(always)]
    fn contains(&self, idx: usize) -> bool {
        let word = idx / 64;
        if word >= self.words.len() {
            return false;
        }
        let bit = idx % 64;
        (self.words[word] >> bit) & 1 != 0
    }

    /// `self |= other`. Returns whether `self` changed.
    fn union_with(&mut self, other: &BitSet) -> bool {
        debug_assert_eq!(self.words.len(), other.words.len());
        let mut changed = false;
        for (w, o) in self.words.iter_mut().zip(other.words.iter()) {
            let old = *w;
            *w |= *o;
            changed |= *w != old;
        }
        changed
    }

    /// `self = gen ∪ (out − kill)`. Returns whether `self` changed.
    fn assign_gen_union_out_minus_kill(
        &mut self,
        gen: &BitSet,
        out: &BitSet,
        kill: &BitSet,
    ) -> bool {
        debug_assert_eq!(self.words.len(), gen.words.len());
        debug_assert_eq!(self.words.len(), out.words.len());
        debug_assert_eq!(self.words.len(), kill.words.len());
        let mut changed = false;
        for i in 0..self.words.len() {
            let new_val = gen.words[i] | (out.words[i] & !kill.words[i]);
            if new_val != self.words[i] {
                self.words[i] = new_val;
                changed = true;
            }
        }
        changed
    }

    fn bits_eq(&self, other: &BitSet) -> bool {
        self.words == other.words
    }

    fn copy_from(&mut self, other: &BitSet) {
        debug_assert_eq!(self.words.len(), other.words.len());
        self.words.copy_from_slice(&other.words);
    }

    fn for_each_set_bit(&self, mut f: impl FnMut(usize)) {
        for (word_idx, &word) in self.words.iter().enumerate() {
            if word == 0 {
                continue;
            }
            let base = word_idx * 64;
            let mut w = word;
            while w != 0 {
                let tz = w.trailing_zeros() as usize;
                f(base + tz);
                w &= w - 1;
            }
        }
    }

    fn clear(&mut self) {
        for w in &mut self.words {
            *w = 0;
        }
    }
}

/// One value's def/use footprint inside one block.
#[derive(Clone, Copy, Debug)]
struct BlockTouch {
    block: u32,
    /// First program point in `block` that defines or reads the value.
    first: u32,
    /// Last program point in `block` that defines or reads the value.
    last: u32,
    /// `first` is a definition (the value is born there, not live-in).
    first_is_def: bool,
}

/// Per-value, per-block def/use footprints — the local evidence
/// [`build_segments`] needs in addition to the block-level `live_in` /
/// `live_out` bits.
///
/// The global `def_points[d]` (FIRST def) and `last_use_points[d]` (LAST
/// use) are enough for the fat `[def, last_use]` interval, but a hole-aware
/// segment must be produced for EVERY block in which the value is defined
/// or read, including blocks where it is neither live-in nor live-out.
/// Phi-eliminated loop-carried values are the canonical shape: the latch
/// re-defines the value (`Copy v_phi ← v_next`) and reads it (the rotated
/// loop condition) with the value dead at both block boundaries. Deriving
/// segments from live-in/live-out bits alone left that block as a hole, and
/// the segment scan handed the value's register to a temporary between the
/// latch copy and the condition (preboot ZSTD `HUF_decompress4X2` on the
/// non-BMI2 path: `endSignal` was clobbered by the `op4 < oend` compare, the
/// 4-stream Huffman loop exited early, and the kernel decompressor reported
/// "ZSTD-compressed data is corrupt").
///
/// Every writer of `last_use_points` / `def_points` records the same event
/// here, so `touch(d, b)` is exactly the set of points the fat interval was
/// built from, split by block.
struct BlockTouches {
    per_value: Vec<Vec<BlockTouch>>,
}

impl BlockTouches {
    fn new(num_values: usize) -> Self {
        Self {
            per_value: vec![Vec::new(); num_values],
        }
    }

    /// Record that `dense` is defined (`is_def`) or read at `point` inside
    /// `block`. The main walk visits blocks in layout order with increasing
    /// points, so the common case is an update of the last entry; the
    /// post-walk extensions (folded reads, phi-incoming copies, F128 source
    /// pointers) may hit any earlier block and fall back to a search.
    fn add(&mut self, dense: usize, block: usize, point: u32, is_def: bool) {
        let block = block as u32;
        let list = &mut self.per_value[dense];
        let slot = match list.last_mut() {
            Some(t) if t.block == block => Some(t),
            _ => list.iter_mut().rev().find(|t| t.block == block),
        };
        match slot {
            Some(t) => {
                if point < t.first || (point == t.first && is_def) {
                    t.first = point;
                    t.first_is_def = is_def;
                }
                if point > t.last {
                    t.last = point;
                }
            }
            None => list.push(BlockTouch {
                block,
                first: point,
                last: point,
                first_is_def: is_def,
            }),
        }
    }

    fn get(&self, dense: usize, block: usize) -> Option<BlockTouch> {
        let block = block as u32;
        self.per_value[dense]
            .iter()
            .find(|t| t.block == block)
            .copied()
    }

    /// `block -> dense values touched in it` (for the segment builder).
    fn by_block(&self, num_blocks: usize) -> Vec<Vec<usize>> {
        let mut out: Vec<Vec<usize>> = vec![Vec::new(); num_blocks];
        for (dense, list) in self.per_value.iter().enumerate() {
            for t in list {
                out[t.block as usize].push(dense);
            }
        }
        out
    }
}

/// Block index containing program point `point` (`block_start_points` is
/// sorted ascending and every point belongs to exactly one block).
fn block_of_point(block_start_points: &[u32], point: u32) -> usize {
    block_start_points
        .partition_point(|&s| s <= point)
        .saturating_sub(1)
}

/// Intermediate state from Phase 1 (program points + gen/kill).
struct ProgramPointState {
    block_start_points: Vec<u32>,
    block_end_points: Vec<u32>,
    def_points: Vec<u32>,
    last_use_points: Vec<u32>,
    block_gen: Vec<BitSet>,
    block_kill: Vec<BitSet>,
    block_id_to_idx: FxHashMap<u32, usize>,
    setjmp_points: Vec<u32>,
    f128_loads: Vec<(u32, u32)>,
    /// SSA / phi-elim copy edges `dest → [src, …]`.
    copy_src: FxHashMap<u32, Vec<u32>>,
    call_points: Vec<u32>,
    num_points: u32,
    /// Per-value, per-block def/use footprints (see [`BlockTouches`]).
    touches: BlockTouches,
}

/// Compute live intervals for all non-alloca values in a function.
///
/// Always walks the IR (even when every dest is an alloca): vecreg builds
/// synthetic alloca intervals and *must* see real `call_points`.
pub fn compute_live_intervals(func: &IrFunction) -> LivenessResult {
    let num_blocks = func.blocks.len();
    if num_blocks == 0 {
        return LivenessResult {
            intervals: Vec::new(),
            segments: Vec::new(),
            call_points: Vec::new(),
            block_loop_depth: Vec::new(),
            block_starts: Vec::new(),
            block_ends: Vec::new(),
            num_points: 0,
            gep_base_values: FxHashSet::default(),
            folded_read_points: FxHashMap::default(),
            id_to_dense: FxHashMap::default(),
            live_in: Vec::new(),
            live_out: Vec::new(),
        };
    }

    let (alloca_set, value_ids, id_to_dense) = collect_values_and_allocas(func);
    let num_values = value_ids.len();

    let mut ps = assign_program_points(func, num_blocks, num_values, &alloca_set, &id_to_dense);

    let mut folded_read_points: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    let gep_base_values = extend_gep_base_liveness(
        func,
        &alloca_set,
        &id_to_dense,
        &ps.copy_src,
        &ps.def_points,
        &ps.block_start_points,
        &ps.block_end_points,
        &mut ps.last_use_points,
        &mut ps.block_gen,
        &mut folded_read_points,
        &mut ps.touches,
    );

    apply_f128_source_gen(
        func,
        &ps.f128_loads,
        &id_to_dense,
        &ps.block_start_points,
        &mut ps.last_use_points,
        &mut ps.block_gen,
        &mut ps.touches,
    );

    let successors = build_successor_lists(func, num_blocks, &ps.block_id_to_idx);
    let predecessors = invert_cfg(&successors, num_blocks);
    let (back_edges, postorder) = analyze_forward_cfg(&successors, num_blocks);
    let block_loop_depth = compute_loop_depth(&predecessors, &back_edges, num_blocks);

    let (live_in, live_out) = run_backward_dataflow(
        num_blocks,
        num_values,
        &successors,
        &predecessors,
        &postorder,
        &ps.block_gen,
        &ps.block_kill,
    );

    // Raw def/use *before* fat live-through pull — segments need holes.
    let raw_def = ps.def_points.clone();
    let raw_last = ps.last_use_points.clone();

    extend_intervals_from_liveness(
        num_blocks,
        &live_in,
        &live_out,
        &ps.block_start_points,
        &ps.block_end_points,
        &mut ps.def_points,
        &mut ps.last_use_points,
    );

    resync_f128_last_use(&ps.f128_loads, &id_to_dense, &mut ps.last_use_points);

    extend_intervals_across_setjmp(
        &ps.setjmp_points,
        ps.num_points,
        &ps.def_points,
        &mut ps.last_use_points,
    );

    let intervals = build_intervals(&value_ids, &ps.def_points, &ps.last_use_points);

    let mut segments = build_segments(
        num_blocks,
        &value_ids,
        &raw_def,
        &raw_last,
        &ps.touches,
        &live_in,
        &live_out,
        &ps.block_start_points,
        &ps.block_end_points,
    );
    extend_segments_across_setjmp(&ps.setjmp_points, ps.num_points, &mut segments);
    resync_f128_segments(&ps.f128_loads, &mut segments);

    if let Some(target) = debug_live_target().filter(|_| debug_live_func_matches(&func.name)) {
        if let Some(&dense) = id_to_dense.get(&target) {
            eprintln!(
                "[LIVE] fn={} v{} def={} last_use={}",
                func.name, target, ps.def_points[dense], ps.last_use_points[dense]
            );
            for (bi, b) in func.blocks.iter().enumerate() {
                let li = live_in[bi].contains(dense);
                let lo = live_out[bi].contains(dense);
                if li || lo {
                    eprintln!(
                        "[LIVE]   block {} (label {}) live_in={} live_out={} pts=[{},{}]",
                        bi, b.label.0, li, lo, ps.block_start_points[bi], ps.block_end_points[bi]
                    );
                }
            }
            let segs: Vec<(u32, u32)> = segments
                .iter()
                .filter(|iv| iv.value_id == target)
                .map(|iv| (iv.start, iv.end))
                .collect();
            eprintln!("[LIVE]   call_points={:?}", ps.call_points);
            eprintln!("[LIVE]   segments={segs:?}");
        }
    }

    LivenessResult {
        intervals,
        segments,
        call_points: ps.call_points,
        block_loop_depth,
        block_starts: ps.block_start_points,
        block_ends: ps.block_end_points,
        num_points: ps.num_points,
        gep_base_values,
        folded_read_points,
        id_to_dense,
        live_in,
        live_out,
    }
}

/// `CCC_DEBUG_LIVE_FUNC=<name>` restricts the `CCC_DEBUG_LIVE` dump to one
/// function (value ids are per-function, so a TU-wide dump is mostly noise).
fn debug_live_func_matches(name: &str) -> bool {
    static F: OnceLock<Option<String>> = OnceLock::new();
    F.get_or_init(|| std::env::var("CCC_DEBUG_LIVE_FUNC").ok())
        .as_deref()
        .is_none_or(|f| f == name)
}

fn debug_live_target() -> Option<u32> {
    static T: OnceLock<Option<u32>> = OnceLock::new();
    *T.get_or_init(|| {
        std::env::var("CCC_DEBUG_LIVE")
            .ok()
            .and_then(|s| s.parse().ok())
    })
}

/// `CCC_DEBUG_LIVE_TRACE=1` (with `CCC_DEBUG_LIVE=<v>`): print each raw
/// def/use event that touches the traced value, as it is recorded.
fn debug_live_trace_enabled() -> bool {
    static T: OnceLock<bool> = OnceLock::new();
    *T.get_or_init(|| std::env::var_os("CCC_DEBUG_LIVE_TRACE").is_some())
}

/// Single IR walk: alloca set + dense remap of every non-alloca value.
fn collect_values_and_allocas(
    func: &IrFunction,
) -> (FxHashSet<u32>, Vec<u32>, FxHashMap<u32, usize>) {
    let mut alloca_set: FxHashSet<u32> = FxHashSet::default();
    let mut value_ids: Vec<u32> = Vec::new();
    let mut seen: FxHashSet<u32> = FxHashSet::default();
    let hint = func.next_value_id as usize;
    value_ids.reserve(hint);
    seen.reserve(hint);

    let mut add = |id: u32, seen: &mut FxHashSet<u32>, value_ids: &mut Vec<u32>| {
        if seen.insert(id) {
            value_ids.push(id);
        }
    };

    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca { dest, .. } = inst {
                alloca_set.insert(dest.0);
            }
            if let Some(dest) = inst.dest() {
                add(dest.0, &mut seen, &mut value_ids);
            }
            for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    add(v.0, &mut seen, &mut value_ids);
                }
            });
            for_each_value_use_in_instruction(inst, |v| {
                add(v.0, &mut seen, &mut value_ids);
            });
        }
        for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                add(v.0, &mut seen, &mut value_ids);
            }
        });
    }

    value_ids.retain(|id| !alloca_set.contains(id));

    let mut id_to_dense: FxHashMap<u32, usize> = FxHashMap::default();
    id_to_dense.reserve(value_ids.len());
    for (dense_idx, &vid) in value_ids.iter().enumerate() {
        id_to_dense.insert(vid, dense_idx);
    }

    (alloca_set, value_ids, id_to_dense)
}

/// Phase 1: program points, gen/kill, def/use, call points, setjmp, copies, F128 loads.
fn assign_program_points(
    func: &IrFunction,
    num_blocks: usize,
    num_values: usize,
    alloca_set: &FxHashSet<u32>,
    id_to_dense: &FxHashMap<u32, usize>,
) -> ProgramPointState {
    let mut point: u32 = 0;
    let mut block_start_points: Vec<u32> = Vec::with_capacity(num_blocks);
    let mut block_end_points: Vec<u32> = Vec::with_capacity(num_blocks);
    let mut def_points: Vec<u32> = vec![u32::MAX; num_values];
    let mut last_use_points: Vec<u32> = vec![u32::MAX; num_values];
    let mut block_gen: Vec<BitSet> = Vec::with_capacity(num_blocks);
    let mut block_kill: Vec<BitSet> = Vec::with_capacity(num_blocks);
    let mut block_id_to_idx: FxHashMap<u32, usize> = FxHashMap::default();
    block_id_to_idx.reserve(num_blocks);
    let mut setjmp_points: Vec<u32> = Vec::new();
    let mut f128_loads: Vec<(u32, u32)> = Vec::new();
    let mut copy_src: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    let mut call_points: Vec<u32> = Vec::new();
    let mut touches = BlockTouches::new(num_values);

    for (block_idx, block) in func.blocks.iter().enumerate() {
        block_id_to_idx.insert(block.label.0, block_idx);
        block_start_points.push(point);
        let mut gen = BitSet::new(num_values);
        let mut kill = BitSet::new(num_values);

        for inst in &block.instructions {
            if is_returns_twice_call(inst) || matches!(inst, Instruction::NonlocalGotoSave { .. }) {
                setjmp_points.push(point);
            }
            if instruction_is_call_point(inst) {
                call_points.push(point);
            }
            if let Instruction::Copy {
                dest,
                src: Operand::Value(src),
            } = inst
            {
                let srcs = copy_src.entry(dest.0).or_default();
                if !srcs.contains(&src.0) {
                    srcs.push(src.0);
                }
            }
            if let Instruction::Load { dest, ptr, ty, .. } = inst {
                if *ty == IrType::F128 && !alloca_set.contains(&ptr.0) {
                    f128_loads.push((ptr.0, dest.0));
                }
            }

            if let Some(t) = debug_live_target()
                .filter(|_| debug_live_trace_enabled() && debug_live_func_matches(&func.name))
            {
                let mut hit = false;
                for_each_operand_in_instruction(inst, |op| {
                    if let Operand::Value(v) = op {
                        hit |= v.0 == t;
                    }
                });
                for_each_value_use_in_instruction(inst, |v| hit |= v.0 == t);
                if hit || inst.dest().is_some_and(|d| d.0 == t) {
                    eprintln!("[LIVE-TRACE] pt={} block={} {:?}", point, block_idx, inst);
                }
            }
            record_instruction_uses_dense(
                inst,
                point,
                block_idx,
                alloca_set,
                id_to_dense,
                &mut last_use_points,
                &mut touches,
            );

            // Kill *promoted* InlineAsm outputs (first def here) before gen
            // so they are not upward-exposed. Outputs already defined
            // (pointers passed through for `"=a"(*ptr)`) are uses of the
            // pointer, not defs — killing them truncated the pointer and
            // let the slot packer reuse it mid-loop.
            if let Instruction::InlineAsm { outputs, .. } = inst {
                for (_, out_val, _) in outputs {
                    if !alloca_set.contains(&out_val.0) {
                        if let Some(&dense) = id_to_dense.get(&out_val.0) {
                            if def_points[dense] == u32::MAX {
                                def_points[dense] = point;
                                kill.insert(dense);
                                touches.add(dense, block_idx, point, true);
                            }
                        }
                    }
                }
            }

            collect_instruction_gen_dense(inst, alloca_set, id_to_dense, &kill, &mut gen);

            if let Some(dest) = inst.dest() {
                if !alloca_set.contains(&dest.0) {
                    if let Some(&dense) = id_to_dense.get(&dest.0) {
                        if def_points[dense] == u32::MAX {
                            def_points[dense] = point;
                        }
                        kill.insert(dense);
                        touches.add(dense, block_idx, point, true);
                    }
                }
            }

            point = point.saturating_add(1);
        }

        record_terminator_uses_dense(
            &block.terminator,
            point,
            block_idx,
            alloca_set,
            id_to_dense,
            &mut last_use_points,
            &mut touches,
        );
        collect_terminator_gen_dense(&block.terminator, alloca_set, id_to_dense, &kill, &mut gen);
        block_end_points.push(point);
        point = point.saturating_add(1);

        block_gen.push(gen);
        block_kill.push(kill);
    }

    // Phi incoming: after phi elim each `(V, pred)` becomes a Copy at the
    // *end* of `pred`. V must be live there. On a back-edge `pred` is later
    // in layout than the Phi, so the Phi's own use point is *earlier* and
    // does not cover the Copy — sqlite3RunParser `n`/r13.
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Phi { incoming, .. } = inst {
                for (op, pred_label) in incoming {
                    if let Operand::Value(v) = op {
                        if alloca_set.contains(&v.0) {
                            continue;
                        }
                        if let Some(&dense) = id_to_dense.get(&v.0) {
                            if let Some(&pred_idx) = block_id_to_idx.get(&pred_label.0) {
                                let pred_end = block_end_points[pred_idx];
                                let entry = &mut last_use_points[dense];
                                if *entry == u32::MAX || pred_end > *entry {
                                    *entry = pred_end;
                                }
                                block_gen[pred_idx].insert(dense);
                                touches.add(dense, pred_idx, pred_end, false);
                            }
                        }
                    }
                }
            }
        }
    }

    ProgramPointState {
        block_start_points,
        block_end_points,
        def_points,
        last_use_points,
        block_gen,
        block_kill,
        block_id_to_idx,
        setjmp_points,
        f128_loads,
        copy_src,
        call_points,
        num_points: point,
        touches,
    }
}

/// Instructions that clobber the caller-saved set at the assembly level.
fn instruction_is_call_point(inst: &Instruction) -> bool {
    let is_32bit = crate::common::types::target_is_32bit();
    match inst {
        Instruction::Call { .. } | Instruction::CallIndirect { .. } => true,
        Instruction::InlineAsm {
            outputs,
            inputs,
            clobbers,
            ..
        } => {
            // `asm volatile("" ::: "memory")` is a compiler barrier, not a
            // register clobber. A GP/XMM clobber list without operands
            // (`syscall ::: rcx,r11`) *is* a call point.
            !outputs.is_empty()
                || !inputs.is_empty()
                || clobbers.iter().any(|c| clobber_is_allocatable_reg(c))
        }
        Instruction::Memcpy { .. }
        | Instruction::VaArg { .. }
        | Instruction::VaStart { .. }
        | Instruction::VaCopy { .. }
        | Instruction::VaArgStruct { .. } => true,
        Instruction::BinOp { op, ty, .. }
            if matches!(
                op,
                IrBinOp::SDiv | IrBinOp::UDiv | IrBinOp::SRem | IrBinOp::URem
            ) && (matches!(ty, IrType::I128 | IrType::U128)
                || (is_32bit && matches!(ty, IrType::I64 | IrType::U64))) =>
        {
            true
        }
        Instruction::BinOp { ty, .. } if *ty == IrType::F128 => true,
        Instruction::UnaryOp { ty, .. } if *ty == IrType::F128 => true,
        Instruction::Cmp { ty, .. } if *ty == IrType::F128 => true,
        Instruction::Cast { from_ty, to_ty, .. } => {
            let i128_fp = (matches!(from_ty, IrType::I128 | IrType::U128) && to_ty.is_float())
                || (from_ty.is_float() && matches!(to_ty, IrType::I128 | IrType::U128));
            let f128 = *from_ty == IrType::F128 || *to_ty == IrType::F128;
            // i686: I64 ↔ FP is `__floatdidf` / `__fixdfdi`.
            let i64_fp_32 = is_32bit
                && ((matches!(from_ty, IrType::I64 | IrType::U64) && to_ty.is_float())
                    || (from_ty.is_float() && matches!(to_ty, IrType::I64 | IrType::U64)));
            i128_fp || f128 || i64_fp_32
        }
        _ => false,
    }
}

/// True if an asm clobber names an allocatable register (GPR / XMM / …),
/// as opposed to a token like `"memory"` / `"cc"`.
fn clobber_is_allocatable_reg(c: &str) -> bool {
    let c = c.trim().trim_start_matches('%');
    if c.is_empty() {
        return false;
    }
    let eq_ignore_ascii = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    if eq_ignore_ascii(c, "memory")
        || eq_ignore_ascii(c, "cc")
        || eq_ignore_ascii(c, "flags")
        || eq_ignore_ascii(c, "fpsr")
        || eq_ignore_ascii(c, "dirflag")
        || eq_ignore_ascii(c, "uninitialized")
    {
        return false;
    }
    true
}

// ── GEP-fold implicit uses ───────────────────────────────────────────────────

enum GepOffset {
    Const(i64),
    Index(u32),
}

/// Hard bound on the number of address links one folded access may walk up
/// (`GEP(GEP(GEP(p,+a),+b),+c)` ...). The backend composes const chains into a
/// single `disp(%base)` and can carry at most one variable index, so real
/// chains are short; the bound only exists to keep the walk finite if the IR
/// ever contains a cycle.
const MAX_GEP_CHAIN_LINKS: usize = 64;

/// Keep GEP/Add bases (and variable indices) live at the Load/Store that
/// folds them into an addressing mode. Mirrors `build_gep_fold_map`.
fn extend_gep_base_liveness(
    func: &IrFunction,
    alloca_set: &FxHashSet<u32>,
    id_to_dense: &FxHashMap<u32, usize>,
    copy_src: &FxHashMap<u32, Vec<u32>>,
    def_points: &[u32],
    block_start_points: &[u32],
    block_end_points: &[u32],
    last_use_points: &mut [u32],
    block_gen: &mut [BitSet],
    folded_read_points: &mut FxHashMap<u32, Vec<u32>>,
    touches: &mut BlockTouches,
) -> FxHashSet<u32> {
    let mut gep_info: FxHashMap<u32, (u32, GepOffset)> = FxHashMap::default();

    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::GetElementPtr {
                    dest,
                    base,
                    offset: Operand::Const(c),
                    ..
                } => {
                    if alloca_set.contains(&base.0) {
                        continue;
                    }
                    let offset_val = match c {
                        IrConst::I64(n) => *n,
                        IrConst::I32(n) => *n as i64,
                        IrConst::I16(n) => *n as i64,
                        IrConst::I8(n) => *n as i64,
                        _ => continue,
                    };
                    if offset_val >= i32::MIN as i64 && offset_val <= i32::MAX as i64 {
                        gep_info.insert(dest.0, (base.0, GepOffset::Const(offset_val)));
                    }
                }
                Instruction::GetElementPtr {
                    dest,
                    base,
                    offset: Operand::Value(off),
                    ..
                } => {
                    if alloca_set.contains(&base.0) {
                        continue;
                    }
                    gep_info.insert(dest.0, (base.0, GepOffset::Index(off.0)));
                }
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::Add,
                    lhs: Operand::Value(base),
                    rhs: Operand::Const(c),
                    ty,
                }
                | Instruction::BinOp {
                    dest,
                    op: IrBinOp::Add,
                    lhs: Operand::Const(c),
                    rhs: Operand::Value(base),
                    ty,
                } if !ty.is_float() && !ty.is_long_double() => {
                    if alloca_set.contains(&base.0) {
                        continue;
                    }
                    if let Some(offset) = c.to_i64() {
                        let offset = if offset >= i32::MIN as i64 && offset <= i32::MAX as i64 {
                            Some(offset)
                        } else if offset > i32::MAX as i64 && offset <= u32::MAX as i64 {
                            Some(offset as i32 as i64)
                        } else {
                            None
                        };
                        if let Some(offset) = offset {
                            gep_info.insert(dest.0, (base.0, GepOffset::Const(offset)));
                        }
                    }
                }
                _ => {}
            }
        }
    }

    if gep_info.is_empty() {
        return FxHashSet::default();
    }

    // ── Which GEP dests can the backend actually absorb? ───────────────────
    //
    // A GEP dest is foldable only if the backend can absorb it into the
    // addressing mode of *every* consumer. Two use kinds are absorbable:
    //
    //   1. the `ptr` of a foldable (non-i128) Load/Store — the access reads
    //      `disp(%base)` (or `disp(%base,%idx,scale)`) directly;
    //   2. the `base` / variable `offset` of ANOTHER foldable GEP — a const
    //      chain `GEP(GEP(p, +32), +16)` is composed by the backend into the
    //      single displacement `48(%p)` (`compose_const_gep_folds`), so the
    //      register the access reads is the ROOT `p`, not the intermediate.
    //
    // Anything else (call argument, stored value, Cmp operand, terminator
    // operand, i128 access, ...) materialises the address and is a hard
    // non-fold use.  Kind (2) is only absorbable while its consumer stays
    // foldable, so foldability is a fixed point over the address-link graph.
    //
    // The pre-session-30 code treated kind (2) as non-foldable and extended
    // only the IMMEDIATE base of a folded access.  For a chain of depth >= 2
    // that left the root's last use at the first GEP — before an inlined
    // callee body that (legally, per IR liveness) recycled the root's
    // register while the folded accesses still read it: sqlite3.50
    // `sqlite3FindIndex` inlines `sqlite3HashFind`/`findElementWithHash`, the
    // `strHash` loop takes the register holding `pSchema`, and
    // `pSchema->idxHash.ht` then dereferences the key pointer (SIGSEGV at
    // -O1; -O2 hides it because the allocation differs).
    let mut hard_bad: FxHashSet<u32> = FxHashSet::default();
    // address-link value -> GEP dests that consume it as `base`/`offset`.
    let mut link_users: FxHashMap<u32, Vec<u32>> = FxHashMap::default();

    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Load { ptr, ty, .. } => {
                    if matches!(ty, IrType::I128 | IrType::U128) && gep_info.contains_key(&ptr.0) {
                        hard_bad.insert(ptr.0);
                    }
                }
                Instruction::Store { val, ptr, ty, .. } => {
                    if let Operand::Value(v) = val {
                        if gep_info.contains_key(&v.0) {
                            hard_bad.insert(v.0);
                        }
                    }
                    if matches!(ty, IrType::I128 | IrType::U128) && gep_info.contains_key(&ptr.0) {
                        hard_bad.insert(ptr.0);
                    }
                }
                Instruction::GetElementPtr {
                    dest, base, offset, ..
                } => {
                    if gep_info.contains_key(&base.0) {
                        link_users.entry(base.0).or_default().push(dest.0);
                    }
                    if let Operand::Value(off) = offset {
                        if gep_info.contains_key(&off.0) {
                            link_users.entry(off.0).or_default().push(dest.0);
                        }
                    }
                }
                _ => {
                    for_each_operand_in_instruction(inst, |op| {
                        if let Operand::Value(v) = op {
                            if gep_info.contains_key(&v.0) {
                                hard_bad.insert(v.0);
                            }
                        }
                    });
                    for_each_value_use_in_instruction(inst, |v| {
                        if gep_info.contains_key(&v.0) {
                            hard_bad.insert(v.0);
                        }
                    });
                }
            }
        }
        for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                if gep_info.contains_key(&v.0) {
                    hard_bad.insert(v.0);
                }
            }
        });
    }

    // Foldability fixed point. Dropping a dest re-exposes the bases it
    // consumed as ordinary (non-fold) uses, so iterate until stable.
    let mut foldable: FxHashSet<u32> = gep_info.keys().copied().collect();
    loop {
        let mut dropped: Vec<u32> = Vec::new();
        for &d in &foldable {
            let materialised = hard_bad.contains(&d)
                || link_users
                    .get(&d)
                    .is_some_and(|users| users.iter().any(|u| !foldable.contains(u)));
            if materialised {
                dropped.push(d);
            }
        }
        if dropped.is_empty() {
            break;
        }
        for d in dropped {
            foldable.remove(&d);
        }
    }
    gep_info.retain(|dest, _| foldable.contains(dest));
    if gep_info.is_empty() {
        return FxHashSet::default();
    }

    // Collect the folded base/index values: every folded Load/Store reads them
    // at its own program point, so their effective use count is far higher
    // than the raw operand walk records. The allocator uses this set to rank
    // them fairly (otherwise a hot-loop base with one recorded use loses its
    // register and every folded access reloads it).
    let mut folded_bases: FxHashSet<u32> = FxHashSet::default();
    // Debug A/B: `CCC_GEP_CHAIN_LIVENESS_DEPTH=1` restores the pre-session-30
    // immediate-base-only extension.
    let max_links: usize = match std::env::var("CCC_GEP_CHAIN_LIVENESS_DEPTH").ok() {
        Some(v) => v.parse().unwrap_or(MAX_GEP_CHAIN_LINKS),
        None => MAX_GEP_CHAIN_LINKS,
    };

    let mut block_point: u32 = 0;
    for (bi, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            let ptr_id = match inst {
                Instruction::Load { ptr, .. } | Instruction::Store { ptr, .. } => Some(ptr.0),
                _ => None,
            };
            if let Some(ptr_id) = ptr_id {
                if gep_info.contains_key(&ptr_id) {
                    // Walk the WHOLE composed chain: `GEP(GEP(p,+32),+16)` is
                    // emitted as `48(%p)`, so `p` — not only the immediate
                    // base — is read at this access. Stop before a link the
                    // backend cannot fold into one addressing mode (a second
                    // variable index, or a displacement that leaves the i32
                    // disp range): that link is materialised into its own
                    // register, whose def/use the raw walk already records.
                    let mut cur = ptr_id;
                    let mut disp: i64 = 0;
                    let mut indices: usize = 0;
                    let mut hops: usize = 0;
                    while hops < max_links {
                        let Some(&(base_id, ref offset)) = gep_info.get(&cur) else {
                            break;
                        };
                        if let GepOffset::Index(idx_id) = *offset {
                            if indices > 0 {
                                break;
                            }
                            indices += 1;
                            extend_use_following_copies(
                                idx_id,
                                block_point,
                                bi,
                                alloca_set,
                                id_to_dense,
                                def_points,
                                &(block_start_points[bi], block_end_points[bi]),
                                copy_src,
                                last_use_points,
                                block_gen,
                                folded_read_points,
                                touches,
                            );
                            folded_bases.insert(idx_id);
                        } else if let GepOffset::Const(off) = *offset {
                            match disp.checked_add(off) {
                                Some(d) if (i32::MIN as i64..=i32::MAX as i64).contains(&d) => {
                                    disp = d;
                                }
                                _ => break,
                            }
                        }
                        extend_use_following_copies(
                            base_id,
                            block_point,
                            bi,
                            alloca_set,
                            id_to_dense,
                            def_points,
                            &(block_start_points[bi], block_end_points[bi]),
                            copy_src,
                            last_use_points,
                            block_gen,
                            folded_read_points,
                            touches,
                        );
                        folded_bases.insert(base_id);
                        hops += 1;
                        if gep_info.contains_key(&base_id) {
                            cur = base_id;
                        } else {
                            break;
                        }
                    }
                }
            }
            block_point = block_point.saturating_add(1);
        }
        block_point = block_point.saturating_add(1);
    }
    folded_bases
}

/// Extend `start_id` and every Copy-chain source (all phi-elim preds) to `point`.
///
/// The `block_gen` insertion — which keeps the value live at the consuming
/// block's ENTRY so liveness propagates through intermediate blocks — is
/// applied ONLY when the value's def is OUTSIDE the consuming block
/// (cross-block fold). A value defined in the same block is NOT upward-
/// exposed: inserting it into gen anyway inflated its fat interval to the
/// whole block span (single-block functions: the whole function), which
/// masqueraded as register pressure and blocked homes for values whose real
/// ranges are a few instructions (vsprintf number()'s `num % base` digit
/// index measured [0..22] instead of [12..15]).
#[allow(clippy::too_many_arguments)]
fn extend_use_following_copies(
    start_id: u32,
    point: u32,
    block_idx: usize,
    alloca_set: &FxHashSet<u32>,
    id_to_dense: &FxHashMap<u32, usize>,
    def_points: &[u32],
    block_range: &(u32, u32),
    copy_src: &FxHashMap<u32, Vec<u32>>,
    last_use_points: &mut [u32],
    block_gen: &mut [BitSet],
    read_points: &mut FxHashMap<u32, Vec<u32>>,
    touches: &mut BlockTouches,
) {
    let mut stack = vec![start_id];
    let mut seen: FxHashSet<u32> = FxHashSet::default();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        if alloca_set.contains(&id) {
            continue;
        }
        if let Some(&dense) = id_to_dense.get(&id) {
            let entry = &mut last_use_points[dense];
            if *entry == u32::MAX || point > *entry {
                *entry = point;
            }
            touches.add(dense, block_idx, point, false);
            // The folded access re-reads this value's register at `point`
            // with no IR operand — record the exact point for the
            // phi-coalesce destructive-update veto (phi_live_in_window).
            read_points.entry(id).or_default().push(point);
            // Only values whose def is NOT inside the consuming block are
            // live at its entry. SSA dominance guarantees a same-block def
            // precedes the use, so the fat interval already covers it.
            let def_in_block = def_points[dense] != u32::MAX
                && def_points[dense] >= block_range.0
                && def_points[dense] <= block_range.1;
            if !def_in_block {
                block_gen[block_idx].insert(dense);
            }
        }
        if let Some(srcs) = copy_src.get(&id) {
            for &src in srcs {
                if src != id {
                    stack.push(src);
                }
            }
        }
    }
}

// ── F128 source-pointer implicit uses ────────────────────────────────────────

/// Codegen reloads the F128 *source pointer* at the dest's last use
/// (`emit_f128_operand_to_a0_a1`). The pointer must stay live that long.
fn apply_f128_source_gen(
    func: &IrFunction,
    f128_loads: &[(u32, u32)],
    id_to_dense: &FxHashMap<u32, usize>,
    block_start_points: &[u32],
    last_use_points: &mut [u32],
    block_gen: &mut [BitSet],
    touches: &mut BlockTouches,
) {
    if f128_loads.is_empty() {
        return;
    }

    for &(ptr_id, dest_id) in f128_loads {
        let (Some(&dd), Some(&pd)) = (id_to_dense.get(&dest_id), id_to_dense.get(&ptr_id)) else {
            continue;
        };
        let dest_last = last_use_points[dd];
        if dest_last != u32::MAX {
            let ptr_entry = &mut last_use_points[pd];
            if *ptr_entry == u32::MAX || dest_last > *ptr_entry {
                *ptr_entry = dest_last;
            }
            touches.add(pd, block_of_point(block_start_points, dest_last), dest_last, false);
        }
    }

    let mut dest_to_ptr: FxHashMap<u32, u32> = FxHashMap::default();
    dest_to_ptr.reserve(f128_loads.len());
    for &(ptr_id, dest_id) in f128_loads {
        dest_to_ptr.insert(dest_id, ptr_id);
    }

    for (bi, block) in func.blocks.iter().enumerate() {
        let mut mark = |vid: u32| {
            if let Some(&ptr_id) = dest_to_ptr.get(&vid) {
                if let Some(&pd) = id_to_dense.get(&ptr_id) {
                    block_gen[bi].insert(pd);
                }
            }
        };
        for inst in &block.instructions {
            for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    mark(v.0);
                }
            });
            for_each_value_use_in_instruction(inst, |v| mark(v.0));
        }
        for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                mark(v.0);
            }
        });
    }
}

fn resync_f128_last_use(
    f128_loads: &[(u32, u32)],
    id_to_dense: &FxHashMap<u32, usize>,
    last_use_points: &mut [u32],
) {
    for &(ptr_id, dest_id) in f128_loads {
        let (Some(&dd), Some(&pd)) = (id_to_dense.get(&dest_id), id_to_dense.get(&ptr_id)) else {
            continue;
        };
        let dest_last = last_use_points[dd];
        if dest_last == u32::MAX {
            continue;
        }
        let ptr_entry = &mut last_use_points[pd];
        if *ptr_entry == u32::MAX || dest_last > *ptr_entry {
            *ptr_entry = dest_last;
        }
    }
}

fn resync_f128_segments(f128_loads: &[(u32, u32)], segments: &mut Vec<LiveInterval>) {
    if f128_loads.is_empty() {
        return;
    }
    for &(ptr_id, dest_id) in f128_loads {
        let dest_end = segments
            .iter()
            .filter(|iv| iv.value_id == dest_id)
            .map(|iv| iv.end)
            .max();
        let Some(dest_end) = dest_end else {
            continue;
        };
        let mut best: Option<usize> = None;
        for (i, iv) in segments.iter().enumerate() {
            if iv.value_id == ptr_id {
                best = Some(match best {
                    Some(j) if segments[j].end >= iv.end => j,
                    _ => i,
                });
            }
        }
        if let Some(i) = best {
            if segments[i].end < dest_end {
                segments[i].end = dest_end;
            }
        } else {
            segments.push(LiveInterval {
                value_id: ptr_id,
                start: dest_end,
                end: dest_end,
            });
        }
    }
}

// ── Use / gen recorders ──────────────────────────────────────────────────────

fn record_instruction_uses_dense(
    inst: &Instruction,
    point: u32,
    block_idx: usize,
    alloca_set: &FxHashSet<u32>,
    id_to_dense: &FxHashMap<u32, usize>,
    last_use: &mut [u32],
    touches: &mut BlockTouches,
) {
    let mut record = |vid: u32| {
        if alloca_set.contains(&vid) {
            return;
        }
        if let Some(&dense) = id_to_dense.get(&vid) {
            let entry = &mut last_use[dense];
            if *entry == u32::MAX || point > *entry {
                *entry = point;
            }
            touches.add(dense, block_idx, point, false);
        }
    };
    for_each_operand_in_instruction(inst, |op| {
        if let Operand::Value(v) = op {
            record(v.0);
        }
    });
    for_each_value_use_in_instruction(inst, |v| record(v.0));
}

fn record_terminator_uses_dense(
    term: &Terminator,
    point: u32,
    block_idx: usize,
    alloca_set: &FxHashSet<u32>,
    id_to_dense: &FxHashMap<u32, usize>,
    last_use: &mut [u32],
    touches: &mut BlockTouches,
) {
    for_each_operand_in_terminator(term, |op| {
        if let Operand::Value(v) = op {
            if alloca_set.contains(&v.0) {
                return;
            }
            if let Some(&dense) = id_to_dense.get(&v.0) {
                let entry = &mut last_use[dense];
                if *entry == u32::MAX || point > *entry {
                    *entry = point;
                }
                touches.add(dense, block_idx, point, false);
            }
        }
    });
}

fn collect_instruction_gen_dense(
    inst: &Instruction,
    alloca_set: &FxHashSet<u32>,
    id_to_dense: &FxHashMap<u32, usize>,
    kill: &BitSet,
    gen: &mut BitSet,
) {
    let mut add_use = |vid: u32| {
        if alloca_set.contains(&vid) {
            return;
        }
        if let Some(&dense) = id_to_dense.get(&vid) {
            if !kill.contains(dense) {
                gen.insert(dense);
            }
        }
    };
    for_each_operand_in_instruction(inst, |op| {
        if let Operand::Value(v) = op {
            add_use(v.0);
        }
    });
    for_each_value_use_in_instruction(inst, |v| add_use(v.0));
}

fn collect_terminator_gen_dense(
    term: &Terminator,
    alloca_set: &FxHashSet<u32>,
    id_to_dense: &FxHashMap<u32, usize>,
    kill: &BitSet,
    gen: &mut BitSet,
) {
    for_each_operand_in_terminator(term, |op| {
        if let Operand::Value(v) = op {
            if alloca_set.contains(&v.0) {
                return;
            }
            if let Some(&dense) = id_to_dense.get(&v.0) {
                if !kill.contains(dense) {
                    gen.insert(dense);
                }
            }
        }
    });
}

// ── CFG ──────────────────────────────────────────────────────────────────────

fn build_successor_lists(
    func: &IrFunction,
    num_blocks: usize,
    block_id_to_idx: &FxHashMap<u32, usize>,
) -> Vec<Vec<usize>> {
    let mut successors: Vec<Vec<usize>> = vec![Vec::new(); num_blocks];
    for (idx, block) in func.blocks.iter().enumerate() {
        for_each_terminator_target(&block.terminator, |target_id| {
            if let Some(&target_idx) = block_id_to_idx.get(&target_id) {
                push_unique(&mut successors[idx], target_idx);
            }
        });
        for inst in &block.instructions {
            if let Instruction::InlineAsm { goto_labels, .. } = inst {
                for (_, label) in goto_labels {
                    if let Some(&target_idx) = block_id_to_idx.get(&label.0) {
                        push_unique(&mut successors[idx], target_idx);
                    }
                }
            }
        }
    }
    successors
}

fn push_unique(vec: &mut Vec<usize>, x: usize) {
    if !vec.contains(&x) {
        vec.push(x);
    }
}

fn invert_cfg(successors: &[Vec<usize>], num_blocks: usize) -> Vec<Vec<usize>> {
    let mut predecessors: Vec<Vec<usize>> = vec![Vec::new(); num_blocks];
    for (src, succs) in successors.iter().enumerate() {
        for &dst in succs {
            if dst < num_blocks {
                predecessors[dst].push(src);
            }
        }
    }
    predecessors
}

fn for_each_terminator_target(term: &Terminator, mut f: impl FnMut(u32)) {
    match term {
        Terminator::Branch(target) => f(target.0),
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => {
            f(true_label.0);
            f(false_label.0);
        }
        Terminator::IndirectBranch {
            possible_targets, ..
        } => {
            for t in possible_targets {
                f(t.0);
            }
        }
        Terminator::Switch { cases, default, .. } => {
            f(default.0);
            for (_, label) in cases {
                f(label.0);
            }
        }
        _ => {}
    }
}

/// One iterative DFS from every unvisited root: back-edges + postorder.
fn analyze_forward_cfg(
    successors: &[Vec<usize>],
    num_blocks: usize,
) -> (Vec<(usize, usize)>, Vec<usize>) {
    let mut state = vec![0u8; num_blocks];
    let mut postorder = Vec::with_capacity(num_blocks);
    let mut back_edges: Vec<(usize, usize)> = Vec::new();
    let mut next_succ = vec![0usize; num_blocks];
    for start in 0..num_blocks {
        if state[start] != 0 {
            continue;
        }
        state[start] = 1;
        let mut stack = vec![start];
        while let Some(&block) = stack.last() {
            let i = next_succ[block];
            if i < successors[block].len() {
                next_succ[block] = i + 1;
                let nxt = successors[block][i];
                if nxt >= num_blocks {
                    continue;
                }
                match state[nxt] {
                    0 => {
                        state[nxt] = 1;
                        stack.push(nxt);
                    }
                    1 => back_edges.push((block, nxt)),
                    _ => {}
                }
            } else {
                state[block] = 2;
                postorder.push(block);
                stack.pop();
            }
        }
    }
    (back_edges, postorder)
}

fn find_back_edges(successors: &[Vec<usize>], num_blocks: usize) -> Vec<(usize, usize)> {
    analyze_forward_cfg(successors, num_blocks).0
}

// ── Dataflow ─────────────────────────────────────────────────────────────────

/// Worklist backward liveness. Seeded so exits pop first.
///
/// Each `live_in` bit flips at most once (monotonic ∪), so each block is
/// dequeued at most `O(num_values)` times. No timeout.
fn run_backward_dataflow(
    num_blocks: usize,
    num_values: usize,
    successors: &[Vec<usize>],
    predecessors: &[Vec<usize>],
    postorder: &[usize],
    block_gen: &[BitSet],
    block_kill: &[BitSet],
) -> (Vec<BitSet>, Vec<BitSet>) {
    let mut live_in: Vec<BitSet> = (0..num_blocks).map(|_| BitSet::new(num_values)).collect();
    let mut live_out: Vec<BitSet> = (0..num_blocks).map(|_| BitSet::new(num_values)).collect();
    let mut tmp_out = BitSet::new(num_values);

    // postorder = exits first; rev puts exits at vec-end; pop = exits first.
    let mut worklist: Vec<usize> = postorder.iter().copied().rev().collect();
    let mut in_queue = vec![false; num_blocks];
    for &b in &worklist {
        in_queue[b] = true;
    }
    for b in 0..num_blocks {
        if !in_queue[b] {
            in_queue[b] = true;
            worklist.push(b);
        }
    }

    while let Some(idx) = worklist.pop() {
        in_queue[idx] = false;

        tmp_out.clear();
        for &succ in &successors[idx] {
            tmp_out.union_with(&live_in[succ]);
        }
        if !tmp_out.bits_eq(&live_out[idx]) {
            live_out[idx].copy_from(&tmp_out);
        }

        if live_in[idx].assign_gen_union_out_minus_kill(
            &block_gen[idx],
            &live_out[idx],
            &block_kill[idx],
        ) {
            for &pred in &predecessors[idx] {
                if !in_queue[pred] {
                    in_queue[pred] = true;
                    worklist.push(pred);
                }
            }
        }
    }

    (live_in, live_out)
}

/// Fat-interval live-through pull.
///
/// Live-*in* to B covers `[B.start, B.end]` (may pull `def` earlier — needed
/// for loop-carried values whose def is later in layout).
/// Live-*out* of B only extends `last_use` to `B.end`. Pulling `def` back to
/// `B.start` for a value *defined in B* invented a live range before the def.
fn extend_intervals_from_liveness(
    num_blocks: usize,
    live_in: &[BitSet],
    live_out: &[BitSet],
    block_start_points: &[u32],
    block_end_points: &[u32],
    def_points: &mut [u32],
    last_use_points: &mut [u32],
) {
    for idx in 0..num_blocks {
        let start = block_start_points[idx];
        let end = block_end_points[idx];

        live_in[idx].for_each_set_bit(|dense_idx| {
            let def_entry = &mut def_points[dense_idx];
            if *def_entry == u32::MAX || start < *def_entry {
                *def_entry = start;
            }
            let entry = &mut last_use_points[dense_idx];
            if *entry == u32::MAX || end > *entry {
                *entry = end;
            }
        });

        live_out[idx].for_each_set_bit(|dense_idx| {
            if def_points[dense_idx] == u32::MAX {
                def_points[dense_idx] = start;
            }
            let entry = &mut last_use_points[dense_idx];
            if *entry == u32::MAX || end > *entry {
                *entry = end;
            }
        });
    }
}

/// Hole-aware segments from per-block def/use footprints + block
/// live_in/live_out.
///
/// A block where the value is neither live-in nor live-out and has no local
/// def/use is a **hole**. Adjacent covered blocks merge. Never
/// under-approximates:
///
/// * a block the value is live-in to is covered from its start, a block it
///   is live-out of is covered to its end;
/// * a block with a local footprint is covered from its first local def
///   (or the block start when the first local event is a read — an
///   upward-exposed read is live-in anyway, so this only over-approximates)
///   to its last local def/use;
/// * on top of that the historical envelope derived from the global
///   `raw_def` / `raw_last` points is kept as a floor in every block the
///   value is live-in to or live-out of, so no coverage this builder
///   produced before the per-block footprints existed is ever removed
///   (a `live_in && !live_out` block stays covered to its end where the
///   last local read would do; tightening that is a separate, measurable
///   change — see engineering notes).
///
/// The per-block footprints are what make multi-def (phi-eliminated) values
/// correct: without them a latch block that re-defines and reads the value
/// while it is dead at both boundaries was a hole (see [`BlockTouches`]).
#[allow(clippy::too_many_arguments)]
fn build_segments(
    num_blocks: usize,
    value_ids: &[u32],
    raw_def: &[u32],
    raw_last: &[u32],
    touches: &BlockTouches,
    live_in: &[BitSet],
    live_out: &[BitSet],
    block_starts: &[u32],
    block_ends: &[u32],
) -> Vec<LiveInterval> {
    let nvals = value_ids.len();
    if nvals == 0 || num_blocks == 0 {
        return Vec::new();
    }

    let mut cover: Vec<Vec<(u32, u32)>> = vec![Vec::new(); nvals];
    let mut seen = vec![false; nvals];
    let mut touched: Vec<usize> = Vec::new();
    let touched_by_block = touches.by_block(num_blocks);

    for b in 0..num_blocks {
        let bs = block_starts[b];
        let be = block_ends[b];

        let mut consider = |dense: usize| {
            if seen[dense] {
                return;
            }
            seen[dense] = true;
            touched.push(dense);

            let li = live_in[b].contains(dense);
            let lo = live_out[b].contains(dense);
            let touch = touches.get(dense, b);

            // Historical envelope from the global first-def / last-use
            // points (kept as a floor, see the doc comment). Blocks that
            // are neither live-in nor live-out were never considered by
            // the boundary-only builder, so they carry no envelope: their
            // coverage is exactly the local footprint.
            let def = raw_def[dense];
            let last = raw_last[dense];
            let def_in = def != u32::MAX && def >= bs && def <= be;
            let last_in = last != u32::MAX && last >= bs && last <= be;
            let envelope = if li || lo {
                let s = if li {
                    bs
                } else if def_in {
                    def
                } else {
                    bs
                };
                let e = if lo {
                    be
                } else if last_in {
                    last
                } else if def_in {
                    def
                } else {
                    be
                };
                Some((s, e))
            } else {
                None
            };

            // Local footprint: born at the first local def (or live-in),
            // dead after the last local event (or live-out).
            let local = touch.map(|t| {
                let s = if li {
                    bs
                } else if t.first_is_def {
                    t.first
                } else {
                    bs
                };
                let e = if lo { be } else { t.last };
                (s, e)
            });

            let piece = match (envelope, local) {
                (Some((es, ee)), Some((ls, le))) => Some((es.min(ls), ee.max(le))),
                (Some(p), None) | (None, Some(p)) => Some(p),
                (None, None) => None,
            };
            if let Some((s, e)) = piece {
                if e >= s {
                    cover[dense].push((s, e));
                }
            }
        };

        live_in[b].for_each_set_bit(&mut consider);
        live_out[b].for_each_set_bit(&mut consider);
        for &dense in &touched_by_block[b] {
            consider(dense);
        }

        for d in touched.drain(..) {
            seen[d] = false;
        }
    }

    let mut result: Vec<LiveInterval> = Vec::with_capacity(nvals);
    for (d, &vid) in value_ids.iter().enumerate() {
        let segs = &mut cover[d];
        if segs.is_empty() {
            continue;
        }
        segs.sort_unstable_by_key(|&(s, _)| s);
        let mut cs = segs[0].0;
        let mut ce = segs[0].1;
        for &(s, e) in segs.iter().skip(1) {
            if s <= ce.saturating_add(1) {
                ce = ce.max(e);
            } else {
                result.push(LiveInterval {
                    start: cs,
                    end: ce,
                    value_id: vid,
                });
                cs = s;
                ce = e;
            }
        }
        result.push(LiveInterval {
            start: cs,
            end: ce,
            value_id: vid,
        });
    }
    result.sort_unstable_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then_with(|| a.value_id.cmp(&b.value_id))
    });
    result
}

/// Any value whose interval contains a setjmp point is live across a
/// returns-twice edge: longjmp restores the frame, so the slot/reg must
/// not be reused for the rest of the function.
fn extend_intervals_across_setjmp(
    setjmp_points: &[u32],
    num_points: u32,
    def_points: &[u32],
    last_use_points: &mut [u32],
) {
    if setjmp_points.is_empty() {
        return;
    }
    let func_end = num_points.saturating_sub(1);
    for &p in setjmp_points {
        for (dense_idx, &start) in def_points.iter().enumerate() {
            if start == u32::MAX || start > p {
                continue;
            }
            let end = last_use_points[dense_idx];
            if end == u32::MAX || end < p {
                continue;
            }
            if func_end > last_use_points[dense_idx] {
                last_use_points[dense_idx] = func_end;
            }
        }
    }
}

fn extend_segments_across_setjmp(
    setjmp_points: &[u32],
    num_points: u32,
    segments: &mut Vec<LiveInterval>,
) {
    if setjmp_points.is_empty() {
        return;
    }
    let func_end = num_points.saturating_sub(1);
    let mut extend: FxHashSet<u32> = FxHashSet::default();
    for iv in segments.iter() {
        for &p in setjmp_points {
            if iv.start <= p && p <= iv.end {
                extend.insert(iv.value_id);
                break;
            }
        }
    }
    if extend.is_empty() {
        return;
    }
    let mut last_idx: FxHashMap<u32, usize> = FxHashMap::default();
    for (i, iv) in segments.iter().enumerate() {
        if extend.contains(&iv.value_id) {
            last_idx.insert(iv.value_id, i);
        }
    }
    for (_, i) in last_idx {
        if segments[i].end < func_end {
            segments[i].end = func_end;
        }
    }
}

fn build_intervals(
    value_ids: &[u32],
    def_points: &[u32],
    last_use_points: &[u32],
) -> Vec<LiveInterval> {
    let mut intervals: Vec<LiveInterval> = Vec::with_capacity(value_ids.len());
    for (dense_idx, &vid) in value_ids.iter().enumerate() {
        let start = def_points[dense_idx];
        if start == u32::MAX {
            continue;
        }
        let end = last_use_points[dense_idx];
        let end = if end == u32::MAX {
            start
        } else {
            end.max(start)
        };
        intervals.push(LiveInterval {
            start,
            end,
            value_id: vid,
        });
    }
    intervals.sort_unstable_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then_with(|| a.value_id.cmp(&b.value_id))
    });
    intervals
}

pub(crate) fn is_returns_twice_call(inst: &Instruction) -> bool {
    if let Instruction::Call { func, .. } = inst {
        matches!(
            func.as_str(),
            "setjmp" | "_setjmp" | "sigsetjmp" | "__sigsetjmp" | "vfork"
        )
    } else {
        false
    }
}

// ── Canonical operand visitors (shared with live_range.rs) ───────────────────

/// Iterate over all `Operand` references in an instruction.
/// Single source of truth — liveness, use-counting, GEP-fold verification.
pub(crate) fn for_each_operand_in_instruction(inst: &Instruction, mut f: impl FnMut(&Operand)) {
    match inst {
        Instruction::Alloca { .. } | Instruction::PgoCounterInc { .. } => {}
        Instruction::DynAlloca { size, .. } => f(size),
        Instruction::Store { val, .. } => f(val),
        Instruction::Load { .. } => {}
        Instruction::BinOp { lhs, rhs, .. } => {
            f(lhs);
            f(rhs);
        }
        Instruction::UnaryOp { src, .. } => f(src),
        Instruction::Cmp { lhs, rhs, .. } => {
            f(lhs);
            f(rhs);
        }
        Instruction::Call { info, .. } => {
            for a in &info.args {
                f(a);
            }
        }
        Instruction::CallIndirect { func_ptr, info } => {
            f(func_ptr);
            for a in &info.args {
                f(a);
            }
        }
        Instruction::GetElementPtr { offset, .. } => f(offset),
        Instruction::Cast { src, .. } => f(src),
        Instruction::Copy { src, .. } => f(src),
        Instruction::GlobalAddr { .. } => {}
        Instruction::Memcpy { .. } => {}
        Instruction::VaArg { .. } => {}
        Instruction::VaStart { .. } => {}
        Instruction::VaEnd { .. } => {}
        Instruction::VaCopy { .. } => {}
        Instruction::VaArgStruct { .. } => {}
        Instruction::AtomicRmw { ptr, val, .. } => {
            f(ptr);
            f(val);
        }
        Instruction::AtomicInc { ptr, .. } => f(ptr),
        Instruction::AtomicCmpxchg {
            ptr,
            expected,
            desired,
            ..
        } => {
            f(ptr);
            f(expected);
            f(desired);
        }
        Instruction::AtomicLoad { ptr, .. } => f(ptr),
        Instruction::AtomicStore { ptr, val, .. } => {
            f(ptr);
            f(val);
        }
        Instruction::Fence { .. } => {}
        Instruction::Phi { incoming, .. } => {
            for (op, _) in incoming {
                f(op);
            }
        }
        Instruction::LabelAddr { .. } => {}
        Instruction::GetReturnF64Second { .. } => {}
        Instruction::SetReturnF64Second { src } => f(src),
        Instruction::GetReturnF32Second { .. } => {}
        Instruction::SetReturnF32Second { src } => f(src),
        Instruction::GetReturnF128Second { .. } => {}
        Instruction::SetReturnF128Second { src } => f(src),
        Instruction::InlineAsm { inputs, .. } => {
            for (_, op, _) in inputs {
                f(op);
            }
        }
        Instruction::Intrinsic { args, .. } => {
            for a in args {
                f(a);
            }
        }
        Instruction::Select {
            cond,
            true_val,
            false_val,
            ..
        } => {
            f(cond);
            f(true_val);
            f(false_val);
        }
        Instruction::StackSave { .. } => {}
        Instruction::StackRestore { .. } => {}
        Instruction::ParamRef { .. } => {}
        Instruction::GetStaticChain { .. } => {}
        Instruction::SetStaticChain { src } => f(src),
        Instruction::InitTrampoline { chain, .. } => f(chain),
        Instruction::NonlocalGotoSave { .. } => {}
        Instruction::NonlocalGoto { chain, .. } => f(chain),
    }
}

/// Iterate over bare `Value` uses (pointers, GEP bases, memcpy endpoints, …).
pub(crate) fn for_each_value_use_in_instruction(inst: &Instruction, mut f: impl FnMut(&Value)) {
    match inst {
        Instruction::Store { ptr, .. } => f(ptr),
        Instruction::Load { ptr, .. } => f(ptr),
        Instruction::GetElementPtr { base, .. } => f(base),
        Instruction::Memcpy { dest, src, .. } => {
            f(dest);
            f(src);
        }
        Instruction::VaArg { va_list_ptr, .. } => f(va_list_ptr),
        Instruction::VaStart { va_list_ptr } => f(va_list_ptr),
        Instruction::VaEnd { va_list_ptr } => f(va_list_ptr),
        Instruction::VaCopy { dest_ptr, src_ptr } => {
            f(dest_ptr);
            f(src_ptr);
        }
        Instruction::VaArgStruct {
            dest_ptr,
            va_list_ptr,
            ..
        } => {
            f(dest_ptr);
            f(va_list_ptr);
        }
        Instruction::InlineAsm { outputs, .. } => {
            for (_, v, _) in outputs {
                f(v);
            }
        }
        Instruction::Intrinsic {
            dest_ptr: Some(dp), ..
        } => {
            f(dp);
        }
        Instruction::StackRestore { ptr } => f(ptr),
        Instruction::InitTrampoline { buffer, .. } => f(buffer),
        Instruction::NonlocalGotoSave { frame, .. } => f(frame),
        _ => {}
    }
}

/// Iterate over `Operand`s in a terminator.
///
/// Phi incoming values are **not** terminator operands — they live on
/// `Instruction::Phi` and are visited by [`for_each_operand_in_instruction`].
pub(crate) fn for_each_operand_in_terminator(term: &Terminator, mut f: impl FnMut(&Operand)) {
    match term {
        Terminator::Return(Some(op)) => f(op),
        Terminator::CondBranch { cond, .. } => f(cond),
        Terminator::IndirectBranch { target, .. } => f(target),
        Terminator::Switch { val, .. } => f(val),
        _ => {}
    }
}

// ── Loop nesting depth ───────────────────────────────────────────────────────

/// Natural-loop nesting depth per block, used as `10^depth` in RA priority.
///
/// A back-edge `tail → header` defines a natural loop: `header` plus every
/// block that reaches `tail` without going through `header`. **All back-edges
/// that share a header are one loop** — incrementing once per back-edge
/// double-counted two-latch loops.
fn compute_loop_depth(
    predecessors: &[Vec<usize>],
    back_edges: &[(usize, usize)],
    num_blocks: usize,
) -> Vec<u32> {
    if num_blocks == 0 {
        return Vec::new();
    }
    let mut depth = vec![0u32; num_blocks];
    if back_edges.is_empty() {
        return depth;
    }

    let mut tails_of: FxHashMap<usize, Vec<usize>> = FxHashMap::default();
    for &(tail, header) in back_edges {
        if header < num_blocks && tail < num_blocks {
            tails_of.entry(header).or_default().push(tail);
        }
    }

    for (&header, tails) in &tails_of {
        let mut visited = vec![false; num_blocks];
        visited[header] = true;
        depth[header] = depth[header].saturating_add(1);

        let mut worklist: Vec<usize> = Vec::new();
        for &tail in tails {
            if tail == header || visited[tail] {
                continue;
            }
            visited[tail] = true;
            depth[tail] = depth[tail].saturating_add(1);
            worklist.push(tail);
        }
        while let Some(b) = worklist.pop() {
            for &pred in &predecessors[b] {
                if pred < num_blocks && !visited[pred] {
                    visited[pred] = true;
                    depth[pred] = depth[pred].saturating_add(1);
                    worklist.push(pred);
                }
            }
        }
    }
    depth
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::IrType;
    use crate::ir::reexports::{BasicBlock, BlockId, IrBinOp};

    fn empty_call_info(dest: Option<Value>) -> crate::ir::reexports::CallInfo {
        crate::ir::reexports::CallInfo {
            dest,
            args: Vec::new(),
            arg_types: Vec::new(),
            return_type: IrType::I32,
            is_variadic: false,
            num_fixed_args: 0,
            struct_arg_sizes: Vec::new(),
            struct_arg_aligns: Vec::new(),
            struct_arg_classes: Vec::new(),
            struct_arg_riscv_float_classes: Vec::new(),
            struct_arg_is_f128_sse: Vec::new(),
            ret_is_f128_sse: false,
            is_sret: false,
            is_fastcall: false,
            is_pure: false,
            is_const: false,
            ret_eightbyte_classes: Vec::new(),
        }
    }

    #[test]
    fn test_inline_asm_with_operands_is_call_point() {
        let mut func = IrFunction::new("test".to_string(), IrType::I32, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::BinOp {
                    dest: Value(0),
                    op: IrBinOp::Add,
                    lhs: Operand::Const(IrConst::I32(1)),
                    rhs: Operand::Const(IrConst::I32(2)),
                    ty: IrType::I32,
                },
                Instruction::InlineAsm {
                    template: "nop".to_string(),
                    outputs: vec![("=r".to_string(), Value(1), Some("out".to_string()))],
                    inputs: vec![],
                    clobbers: vec![],
                    operand_types: vec![IrType::I32],
                    goto_labels: vec![],
                    input_symbols: vec![],
                    seg_overrides: vec![],
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(0)))),
            source_spans: Vec::new(),
        });
        func.next_value_id = 2;

        let result = compute_live_intervals(&func);
        assert!(
            !result.call_points.is_empty(),
            "InlineAsm with register operands should be a call point"
        );
    }

    #[test]
    fn test_empty_inline_asm_barrier_not_call_point() {
        let mut func = IrFunction::new("test".to_string(), IrType::I32, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::BinOp {
                    dest: Value(0),
                    op: IrBinOp::Add,
                    lhs: Operand::Const(IrConst::I32(1)),
                    rhs: Operand::Const(IrConst::I32(2)),
                    ty: IrType::I32,
                },
                Instruction::InlineAsm {
                    template: String::new(),
                    outputs: vec![],
                    inputs: vec![],
                    clobbers: vec!["memory".to_string()],
                    operand_types: vec![],
                    goto_labels: vec![],
                    input_symbols: vec![],
                    seg_overrides: vec![],
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(0)))),
            source_spans: Vec::new(),
        });
        func.next_value_id = 1;

        let result = compute_live_intervals(&func);
        assert!(
            result.call_points.is_empty(),
            "Empty inline asm barriers should NOT be call points"
        );
    }

    #[test]
    fn test_inline_asm_gpr_clobber_is_call_point() {
        let mut func = IrFunction::new("test".to_string(), IrType::I32, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::BinOp {
                    dest: Value(0),
                    op: IrBinOp::Add,
                    lhs: Operand::Const(IrConst::I32(1)),
                    rhs: Operand::Const(IrConst::I32(2)),
                    ty: IrType::I32,
                },
                Instruction::InlineAsm {
                    template: "syscall".to_string(),
                    outputs: vec![],
                    inputs: vec![],
                    clobbers: vec!["rcx".to_string(), "r11".to_string(), "memory".to_string()],
                    operand_types: vec![],
                    goto_labels: vec![],
                    input_symbols: vec![],
                    seg_overrides: vec![],
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(0)))),
            source_spans: Vec::new(),
        });
        func.next_value_id = 1;

        let result = compute_live_intervals(&func);
        assert!(
            !result.call_points.is_empty(),
            "InlineAsm that clobbers GPRs must be a call point"
        );
    }

    #[test]
    fn binop_address_fold_extends_base_through_load() {
        let mut func = IrFunction::new("binop_gep_liveness".to_string(), IrType::U8, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::ParamRef {
                    dest: Value(0),
                    param_idx: 0,
                    ty: IrType::Ptr,
                },
                Instruction::BinOp {
                    dest: Value(1),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I64(4)),
                    ty: IrType::Ptr,
                },
                Instruction::Load {
                    volatile: false,
                    dest: Value(2),
                    ptr: Value(1),
                    ty: IrType::U8,
                    seg_override: crate::common::types::AddressSpace::Default,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(2)))),
            source_spans: Vec::new(),
        });
        func.next_value_id = 3;

        let result = compute_live_intervals(&func);
        let base = result.intervals.iter().find(|iv| iv.value_id == 0).unwrap();
        assert!(base.end >= 2, "base ended before folded Load: {base:?}");
    }

    #[test]
    fn no_values_still_reports_per_block_loop_depth() {
        let mut func = IrFunction::new("empty".to_string(), IrType::Void, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        func.next_value_id = 0;
        let result = compute_live_intervals(&func);
        assert!(result.intervals.is_empty());
        assert_eq!(result.block_loop_depth.len(), 1);
        assert_eq!(result.block_loop_depth[0], 0);
        assert_eq!(result.block_starts.len(), 1);
        assert_eq!(result.block_ends.len(), 1);
    }

    #[test]
    fn loop_depth_single_back_edge() {
        let succs = vec![vec![1], vec![2], vec![1, 3], vec![]];
        let preds = invert_cfg(&succs, 4);
        let back = find_back_edges(&succs, 4);
        let d = compute_loop_depth(&preds, &back, 4);
        assert_eq!(d[0], 0);
        assert_eq!(d[1], 1);
        assert_eq!(d[2], 1);
        assert_eq!(d[3], 0);
    }

    #[test]
    fn loop_depth_two_latches_not_double_counted() {
        let succs = vec![vec![1, 2], vec![0], vec![0]];
        let preds = invert_cfg(&succs, 3);
        let back = find_back_edges(&succs, 3);
        let d = compute_loop_depth(&preds, &back, 3);
        assert_eq!(d[0], 1, "header double-counted: {d:?}");
        assert_eq!(d[1], 1, "{d:?}");
        assert_eq!(d[2], 1, "{d:?}");
    }

    #[test]
    fn loop_depth_nested() {
        let succs = vec![vec![1], vec![2, 3], vec![1], vec![0]];
        let preds = invert_cfg(&succs, 4);
        let back = find_back_edges(&succs, 4);
        let d = compute_loop_depth(&preds, &back, 4);
        assert_eq!(d[0], 1, "{d:?}");
        assert!(d[1] >= 2, "inner header should be nested: {d:?}");
        assert!(d[2] >= 2, "inner latch should be nested: {d:?}");
        assert_eq!(d[3], 1, "{d:?}");
    }

    #[test]
    fn bitset_transfer_function() {
        let mut gen = BitSet::new(80);
        let mut kill = BitSet::new(80);
        let mut out = BitSet::new(80);
        let mut live_in = BitSet::new(80);
        gen.insert(1);
        gen.insert(70);
        kill.insert(2);
        out.insert(2);
        out.insert(3);
        assert!(live_in.assign_gen_union_out_minus_kill(&gen, &out, &kill));
        assert!(live_in.contains(1));
        assert!(live_in.contains(70));
        assert!(!live_in.contains(2), "killed");
        assert!(live_in.contains(3));
        assert!(!live_in.assign_gen_union_out_minus_kill(&gen, &out, &kill));
    }

    #[test]
    fn dataflow_propagates_through_long_chain() {
        let n = 80;
        let mut succs = vec![Vec::new(); n];
        for i in 0..n - 1 {
            succs[i].push(i + 1);
        }
        let preds = invert_cfg(&succs, n);
        let (_, post) = analyze_forward_cfg(&succs, n);

        let mut gen: Vec<BitSet> = (0..n).map(|_| BitSet::new(1)).collect();
        let kill: Vec<BitSet> = (0..n).map(|_| BitSet::new(1)).collect();
        gen[n - 1].insert(0);
        let mut kill0 = BitSet::new(1);
        kill0.insert(0);
        let mut kill = kill;
        kill[0] = kill0;

        let (live_in, live_out) = run_backward_dataflow(n, 1, &succs, &preds, &post, &gen, &kill);
        assert!(!live_in[0].contains(0), "def kills upward exposure");
        for i in 1..n {
            assert!(live_in[i].contains(0), "live_in[{i}] missing");
        }
        for i in 0..n - 1 {
            assert!(live_out[i].contains(0), "live_out[{i}] missing");
        }
    }

    #[test]
    fn diamond_hole_does_not_cover_dead_arm() {
        // B0: v0 = 1; condbr c, B2, B1
        // B1: call(); br B3          ← v0 is dead here
        // B2: v1 = v0 + 1; br B3
        // B3: ret v1
        //
        // Fat interval covers B1 (layout hole). Segments must not.
        let mut func = IrFunction::new("diamond".to_string(), IrType::I32, vec![], false);
        func.blocks = vec![
            BasicBlock {
                label: BlockId(0),
                instructions: vec![
                    Instruction::Copy {
                        dest: Value(0),
                        src: Operand::Const(IrConst::I32(1)),
                    },
                    Instruction::Copy {
                        dest: Value(3),
                        src: Operand::Const(IrConst::I32(1)),
                    },
                ],
                terminator: Terminator::CondBranch {
                    cond: Operand::Value(Value(3)),
                    true_label: BlockId(2),
                    false_label: BlockId(1),
                },
                source_spans: Vec::new(),
            },
            BasicBlock {
                label: BlockId(1),
                instructions: vec![Instruction::Call {
                    func: "puts".to_string(),
                    info: empty_call_info(None),
                }],
                terminator: Terminator::Branch(BlockId(3)),
                source_spans: Vec::new(),
            },
            BasicBlock {
                label: BlockId(2),
                instructions: vec![Instruction::BinOp {
                    dest: Value(1),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                }],
                terminator: Terminator::Branch(BlockId(3)),
                source_spans: Vec::new(),
            },
            BasicBlock {
                label: BlockId(3),
                instructions: vec![],
                terminator: Terminator::Return(Some(Operand::Value(Value(1)))),
                source_spans: Vec::new(),
            },
        ];
        func.next_value_id = 4;

        let result = compute_live_intervals(&func);
        assert_eq!(result.block_starts.len(), 4);
        assert!(!result.call_points.is_empty(), "B1 call must be recorded");

        let b1_lo = result.block_starts[1];
        let b1_hi = result.block_ends[1];
        let covers_b1 = result
            .segments
            .iter()
            .any(|iv| iv.value_id == 0 && iv.start <= b1_hi && b1_lo <= iv.end);
        assert!(
            !covers_b1,
            "v0 segment must hole the dead call arm: segs={:?}",
            result
                .segments
                .iter()
                .filter(|iv| iv.value_id == 0)
                .collect::<Vec<_>>()
        );
        assert!(
            !result.live_across_any_call(0),
            "v0 must be caller-saved-eligible"
        );

        // Fat interval still spans the layout hole (Part 1 iv_map contract).
        let fat = result.fat_interval(0).unwrap();
        assert!(
            fat.0 <= b1_lo && fat.1 >= b1_lo,
            "fat interval must remain conservative: {fat:?}"
        );
    }

    /// A phi-eliminated loop-carried value that is re-defined AND read in
    /// the latch block while being live-in to and live-out of nothing there
    /// (the rotated `while ((op < oend) & endSignal)` shape of zstd's
    /// HUF_decompress4X2). The segment builder must cover [copy, read] in
    /// that block; a hole there let the register allocator reuse the
    /// value's register between the copy and the compare (preboot ZSTD
    /// "compressed data is corrupt" on non-BMI2 CPUs).
    #[test]
    fn latch_block_redef_and_read_is_covered() {
        // block 0: v0 = 1; br 1
        // block 1: v1 = v0 & v9; condbr v1 -> 2 / 3        (header)
        // block 2: v2 = call(); v0 = v2 (copy); v3 = v0 & v9; condbr v3 -> 2 / 3
        // block 3: ret v9
        let mut func = IrFunction::new("latch".to_string(), IrType::I32, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Copy {
                    dest: Value(9),
                    src: Operand::Const(IrConst::I32(1)),
                },
                Instruction::Copy {
                    dest: Value(0),
                    src: Operand::Const(IrConst::I32(1)),
                },
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![Instruction::BinOp {
                dest: Value(1),
                op: IrBinOp::And,
                lhs: Operand::Value(Value(0)),
                rhs: Operand::Value(Value(9)),
                ty: IrType::I32,
            }],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(1)),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![
                Instruction::Call {
                    func: "reload".to_string(),
                    info: empty_call_info(Some(Value(2))),
                },
                Instruction::Copy {
                    dest: Value(0),
                    src: Operand::Value(Value(2)),
                },
                Instruction::BinOp {
                    dest: Value(3),
                    op: IrBinOp::And,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Value(Value(9)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(3)),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![],
            terminator: Terminator::Return(Some(Operand::Value(Value(9)))),
            source_spans: Vec::new(),
        });
        func.next_value_id = 10;

        let result = compute_live_intervals(&func);
        let b2_lo = result.block_starts[2];
        // Points in block 2: call = b2_lo, copy = b2_lo+1, and = b2_lo+2.
        let copy_pt = b2_lo + 1;
        let read_pt = b2_lo + 2;
        let segs: Vec<(u32, u32)> = result
            .segments
            .iter()
            .filter(|iv| iv.value_id == 0)
            .map(|iv| (iv.start, iv.end))
            .collect();
        assert!(
            segs.iter().any(|&(s, e)| s <= copy_pt && read_pt <= e),
            "v0 must be covered from the latch copy to the rotated read: segs={segs:?}"
        );
        // And the hole before the copy is real: v0 is dead across the call.
        assert!(
            !segs.iter().any(|&(s, e)| s <= b2_lo && b2_lo <= e),
            "v0 must not be live at the latch call: segs={segs:?}"
        );
    }

    #[test]
    fn one_fat_interval_per_value() {
        let mut func = IrFunction::new("uniq".to_string(), IrType::I32, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::Copy {
                dest: Value(0),
                src: Operand::Const(IrConst::I32(1)),
            }],
            terminator: Terminator::Return(Some(Operand::Value(Value(0)))),
            source_spans: Vec::new(),
        });
        func.next_value_id = 1;
        let result = compute_live_intervals(&func);
        let n = result
            .intervals
            .iter()
            .filter(|iv| iv.value_id == 0)
            .count();
        assert_eq!(n, 1, "{:?}", result.intervals);
    }
}
