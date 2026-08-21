//! Loop-frame alias queries over the shared linear-form engine (SCEV-lite).
//!
//! The pointer analysis itself lives in `loop_memory_promote` (it was built
//! there and is shared verbatim).  This module adds the frame machinery the
//! block-local consumers need: a map from every block to its INNERMOST
//! containing natural loop, so a late pass can resolve a pointer to its
//! linear form `root + Σ coeff·iv + konst + march·t` *in the context of the
//! loop a block actually belongs to*, and a same-frame disjointness query
//! (`forms_disjoint`) for provably-non-overlapping access pairs.
//!
//! Derived from levkropp/lccc (Aug 19, 2026 commits), re-fitted to this
//! tree's engine (defs map, target-aware `byte_size`, checked arithmetic).

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::reexports::{IrFunction, Value};
use super::loop_analysis;
use super::loop_memory_promote as lmp;

pub(crate) use super::loop_memory_promote::LinForm;

/// Per-function loop context: value→def-block, innermost loop frames, and
/// the block→frame index.  Frame 0 is the smallest loop body; a block not in
/// any loop maps to `NO_FRAME`.
pub(crate) struct LoopFrames {
    pub(crate) def_block: FxHashMap<u32, usize>,
    /// (header, body) per loop, innermost (smallest) first.
    pub(crate) frames: Vec<(usize, FxHashSet<usize>)>,
    /// block index -> innermost frame index (`NO_FRAME` when not in a loop).
    pub(crate) block_frame: Vec<u32>,
}

pub(crate) const NO_FRAME: u32 = u32::MAX;

impl LoopFrames {
    pub(crate) fn build_with_cfg(
        func: &IrFunction,
        cfg: &crate::ir::analysis::CfgAnalysis,
    ) -> Self {
        let mut def_block: FxHashMap<u32, usize> = FxHashMap::default();
        for (bi, block) in func.blocks.iter().enumerate() {
            for inst in &block.instructions {
                if let Some(dest) = inst.dest() {
                    def_block.insert(dest.0, bi);
                }
            }
        }
        let loops = loop_analysis::merge_loops_by_header(loop_analysis::find_natural_loops(
            cfg.num_blocks, &cfg.preds, &cfg.succs, &cfg.idom,
        ));
        let mut frames: Vec<(usize, FxHashSet<usize>)> =
            loops.iter().map(|lp| (lp.header, lp.body.clone())).collect();
        // Innermost first: with ascending body size, the first frame claiming
        // a block is the smallest containing loop.
        frames.sort_by_key(|(_, body)| body.len());
        let mut block_frame = vec![NO_FRAME; func.blocks.len()];
        for (fi, (_, body)) in frames.iter().enumerate() {
            for &b in body.iter() {
                if block_frame[b] == NO_FRAME {
                    block_frame[b] = fi as u32;
                }
            }
        }
        LoopFrames { def_block, frames, block_frame }
    }
}

/// Resolve a pointer to its linear form under `frame` (`NO_FRAME` = outside
/// every loop; the empty body keeps symbolic terms opaque but valid).
/// `defs` is built once per pass invocation by the caller (see
/// `redundant_loads::run`) — the engine keys on instruction references.
pub(crate) fn resolve_in_frame(
    func: &IrFunction,
    defs: &FxHashMap<u32, &crate::ir::reexports::Instruction>,
    lf: &LoopFrames,
    frame: u32,
    v: Value,
) -> Option<lmp::LinForm> {
    static EMPTY: std::sync::OnceLock<FxHashSet<usize>> = std::sync::OnceLock::new();
    let (body_ref, header_idx) = if frame == NO_FRAME {
        (EMPTY.get_or_init(FxHashSet::default), usize::MAX)
    } else {
        let (h, b) = &lf.frames[frame as usize];
        (b, *h)
    };
    lmp::resolve_lin_form(func, defs, body_ref, &lf.def_block, header_idx, v, 32)
}

/// Same-frame disjointness of two resolved forms.
///
/// Soundness contract (unchanged from the loop_memory_promote engine):
/// - Different roots ⇒ MAY alias (false), never "disjoint": two distinct
///   root ids do not prove different objects when either root is opaque.
/// - Different symbolic parts ⇒ may alias.
/// - Marching terms are only comparable within the same loop frame; the
///   caller must pass `same_frame = true` only when both forms were resolved
///   under the same frame (the per-block consumer always does).
/// - The separation math assumes the loop parameter t ≥ 0 and monotone
///   march, which holds for natural loops counted forward.
pub(crate) fn forms_disjoint(
    load: &lmp::LinForm,
    load_sz: i64,
    store: &lmp::LinForm,
    store_sz: i64,
    same_frame: bool,
) -> bool {
    if load.root == 0 || load.root != store.root {
        return false;
    }
    if load.syms != store.syms {
        return false;
    }
    if !same_frame && (load.march != 0 || store.march != 0) {
        return false;
    }
    let Some(d) = store.konst.checked_sub(load.konst) else { return false; };
    let Some(dm) = store.march.checked_sub(load.march) else { return false; };
    if dm == 0 {
        let a = load.konst.checked_add(load_sz).is_some_and(|end| store.konst >= end);
        let b = store.konst.checked_add(store_sz).is_some_and(|end| load.konst >= end);
        return a || b;
    }
    if dm > 0 { d >= load_sz } else { d.checked_add(store_sz).is_some_and(|end| end <= 0) }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn form(konst: i64, march: i64) -> LinForm {
        LinForm { root: 1, syms: vec![], konst, march }
    }
    #[test]
    fn overflow_fails_closed() {
        assert!(!forms_disjoint(&form(i64::MIN, 0), 8, &form(i64::MAX, 0), 8, true));
    }
    #[test]
    fn separated_forms_prove() {
        assert!(forms_disjoint(&form(0, 0), 8, &form(8, 0), 4, true));
    }
}
