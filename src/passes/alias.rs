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

use super::loop_analysis;
use super::loop_memory_promote as lmp;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::instruction::Operand;
use crate::ir::intrinsics::IntrinsicOp;
use crate::ir::reexports::{IrFunction, Value};

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
    /// Values with MORE THAN ONE defining instruction (post-phi-coalescing
    /// copy webs). `resolve_lin_form` stops at these as symbol leaves; see
    /// its COPY-WEB LEAF arm for the soundness contract.
    pub(crate) multi_def: FxHashSet<u32>,
}

pub(crate) const NO_FRAME: u32 = u32::MAX;

impl LoopFrames {
    pub(crate) fn build_with_cfg(
        func: &IrFunction,
        cfg: &crate::ir::analysis::CfgAnalysis,
    ) -> Self {
        let mut def_block: FxHashMap<u32, usize> = FxHashMap::default();
        let mut multi_def: FxHashSet<u32> = FxHashSet::default();
        for (bi, block) in func.blocks.iter().enumerate() {
            for inst in &block.instructions {
                if let Some(dest) = inst.dest() {
                    if def_block.insert(dest.0, bi).is_some() {
                        multi_def.insert(dest.0);
                    }
                }
            }
        }
        let loops = loop_analysis::merge_loops_by_header(loop_analysis::find_natural_loops(
            cfg.num_blocks,
            &cfg.preds,
            &cfg.succs,
            &cfg.idom,
        ));
        let mut frames: Vec<(usize, FxHashSet<usize>)> = loops
            .iter()
            .map(|lp| (lp.header, lp.body.clone()))
            .collect();
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
        LoopFrames {
            def_block,
            frames,
            block_frame,
            multi_def,
        }
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
    static EMPTY: std::sync::LazyLock<FxHashSet<usize>> =
        std::sync::LazyLock::new(FxHashSet::default);
    let (body_ref, header_idx) = if frame == NO_FRAME {
        (&*EMPTY, usize::MAX)
    } else {
        let (h, b) = &lf.frames[frame as usize];
        (b, *h)
    };
    lmp::resolve_lin_form(
        func,
        defs,
        body_ref,
        &lf.def_block,
        &lf.multi_def,
        header_idx,
        v,
        32,
    )
}

/// Byte width and address operands of a register-dest vector access
// (`VecLoad*` / `VecStore*` families), for the alias engine.
// The families the emitter addresses as `base + index + disp`: loads carry
// `[base, index, disp]` in `args[0..3]`, stores carry
// `[src, base, index, disp]` (the `vec_mem_operand` / `emit_vec_store_addr`
// convention). The memory-dest `Load*`/`Loadu*`/`Store*` families round-trip
// through `dest_ptr` memory and are NOT modeled — callers fail closed on
// them. Widths verified against the emitters: 128-bit lanes (16 B),
// 256-bit lanes (32 B), the half-wide pair forms (8 B), and the widening
// load (8 B read).
pub(crate) fn vec_intrinsic_access<'a>(
    op: &IntrinsicOp,
    args: &'a [Operand],
) -> Option<(bool, i64, &'a Operand, &'a Operand, i64)> {
    let (is_store, bytes) = match op {
        IntrinsicOp::VecStoreI32x4
        | IntrinsicOp::VecStoreF32x4
        | IntrinsicOp::VecStoreF64x2
        | IntrinsicOp::VecStoreI64x2
        | IntrinsicOp::VecStoreI16x8
        | IntrinsicOp::VecStoreI8x16 => (true, 16),
        IntrinsicOp::VecStoreI32x8
        | IntrinsicOp::VecStoreF32x8
        | IntrinsicOp::VecStoreF64x4
        | IntrinsicOp::VecStoreI64x4
        | IntrinsicOp::VecStoreI8x32
        | IntrinsicOp::VecStoreI16x16 => (true, 32),
        IntrinsicOp::VecStoreI32x4Pair => (true, 8),
        IntrinsicOp::VecLoadF64x4
        | IntrinsicOp::VecLoadI32x8
        | IntrinsicOp::VecLoadF32x8
        | IntrinsicOp::VecLoadI8x32
        | IntrinsicOp::VecLoadI64x4
        | IntrinsicOp::VecLoadI16x16 => (false, 32),
        IntrinsicOp::VecLoadF64x2
        | IntrinsicOp::VecLoadI32x4
        | IntrinsicOp::VecLoadF32x4
        | IntrinsicOp::VecLoadI64x2
        | IntrinsicOp::VecLoadI16x8
        | IntrinsicOp::VecLoadI8x16 => (false, 16),
        IntrinsicOp::VecLoadWidenI32ToI64x2 | IntrinsicOp::VecLoadI32x4Pair => (false, 8),
        _ => return None,
    };
    let (base, off) = if is_store {
        (args.get(1)?, args.get(2)?)
    } else {
        (args.first()?, args.get(1)?)
    };
    // The trailing displacement argument must be a constant when present —
    // the emitter's `vec_disp_arg` contract. A value operand there is not a
    // shape this engine models: fail closed.
    let disp = match args.get(if is_store { 3 } else { 2 }) {
        Some(Operand::Const(c)) => c.to_i64()?,
        None => 0,
        Some(Operand::Value(_)) => return None,
    };
    Some((is_store, bytes, base, off, disp))
}

/// Resolve the `(base, index, disp)` vector-access address to a linear
/// form under `frame` — the same engine the GEP walker uses, composed by
/// hand for the intrinsic operand triple (merge_forms rejects two roots,
/// exactly like a GEP whose base and offset both carry opaque roots).
pub(crate) fn resolve_vec_addr_in_frame(
    func: &IrFunction,
    defs: &FxHashMap<u32, &crate::ir::reexports::Instruction>,
    lf: &LoopFrames,
    frame: u32,
    base: &Operand,
    index: &Operand,
    disp: i64,
) -> Option<lmp::LinForm> {
    let mut f = match base {
        Operand::Value(v) => resolve_in_frame(func, defs, lf, frame, *v)?,
        Operand::Const(c) => lmp::LinForm {
            root: 0,
            syms: vec![],
            konst: c.to_i64()?,
            march: 0,
        },
    };
    match index {
        Operand::Value(v) => {
            let g = resolve_in_frame(func, defs, lf, frame, *v)?;
            f = lmp::merge_forms(f, g)?;
        }
        Operand::Const(c) => {
            f.konst = f.konst.checked_add(c.to_i64()?)?;
        }
    }
    f.konst = f.konst.checked_add(disp)?;
    Some(f)
}

/// Same-frame disjointness of two resolved forms.
///
/// Soundness contract (from the loop_memory_promote engine, extended):
/// - Different roots ⇒ MAY alias (false), never "disjoint": two distinct
///   root ids do not prove different objects when either root is opaque.
/// - IDENTICAL symbolic parts: exact interval/march arithmetic (below).
/// - DIFFERENT symbolic parts: the stride-period rule
///   ([`stride_period_disjoint`]) — provable only when every symbolic
///   coefficient (and march) shares a stride divisor and the constant
///   residues are field-disjoint within one period.
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
    if !same_frame && (load.march != 0 || store.march != 0) {
        return false;
    }
    if load.syms == store.syms {
        let Some(d) = store.konst.checked_sub(load.konst) else {
            return false;
        };
        let Some(dm) = store.march.checked_sub(load.march) else {
            return false;
        };
        if dm == 0 {
            let a = load
                .konst
                .checked_add(load_sz)
                .is_some_and(|end| store.konst >= end);
            let b = store
                .konst
                .checked_add(store_sz)
                .is_some_and(|end| load.konst >= end);
            return a || b;
        }
        if dm > 0 {
            d >= load_sz
        } else {
            d.checked_add(store_sz).is_some_and(|end| end <= 0)
        }
    } else {
        stride_period_disjoint(load, load_sz, store, store_sz)
    }
}

/// STRIDE-PERIOD FIELD DISJOINTNESS — the cross-element rule.
///
/// Two accesses to the SAME root object whose symbolic terms are all
/// integer multiples of a common stride `s` (an array of 56-byte structs
/// indexed `base + 56*i + c`) reduce, modulo `s`, to their constant
/// residues: `addr ≡ c (mod s)`. When both residues with their access
/// widths fit inside one period (`c + w ≤ s`) and the residue intervals are
/// disjoint, the accesses cannot overlap for ANY combination of the symbolic
/// values — same element (the residues differ) or different elements (whole
/// periods apart, and each access sits inside its own period).
///
/// This is the array-of-struct field rule: `a[i].x` never aliases
/// `a[j].vx` when `x` and `vx` are disjoint fields of one element, no
/// matter how `i` and `j` relate. The engine previously required identical
/// symbolic parts, so every cross-element pair was "may alias" and SLP'd
/// loops re-loaded invariant fields each iteration (nbody: `bodies[j].mass`
/// loaded twice per pair iteration — once for the packed vx/vy update,
/// once for the scalar vz tail).
///
/// Soundness:
/// * Every nonzero symbolic coefficient AND march of both forms must be
///   divisible by `s = gcd(all of them)`; each address is then
///   `root + s·(integer) + c` at every point in time, and the residue
///   argument is exact. Forms only ever arise from GEP byte-offset chains
///   (integer arithmetic); a Cast in the chain does not disturb
///   divisibility (the runtime term stays `coeff × integer`).
/// * A march divisible by `s` keeps the residue constant over `t` — the
///   rule holds for any store-time/load-time pair, so LICM's hoisted
///   (preheader) load versus any in-loop store is covered.
/// * Symbol LEAVES of copy webs are position-dependent; block-local
///   consumers must invalidate on the web's redefinition (the callers in
///   this tree do), and cross-iteration consumers only see leaves that are
///   loop-invariant by dominance.
/// * Overflow, period straddling (`c + w > s`), or no common divisor:
///   false (may-alias), never a wrong disjoint.
fn stride_period_disjoint(
    load: &lmp::LinForm,
    load_sz: i64,
    store: &lmp::LinForm,
    store_sz: i64,
) -> bool {
    let mut s: u64 = 0;
    for &(_, c) in load.syms.iter().chain(store.syms.iter()) {
        if c != 0 {
            s = gcd_u64(s, c.unsigned_abs());
        }
    }
    if load.march != 0 {
        s = gcd_u64(s, load.march.unsigned_abs());
    }
    if store.march != 0 {
        s = gcd_u64(s, store.march.unsigned_abs());
    }
    if s == 0 || s > i64::MAX as u64 {
        return false;
    }
    let s = s as i64;
    let cl = load.konst.rem_euclid(s);
    let cs = store.konst.rem_euclid(s);
    let (Some(le), Some(se)) = (cl.checked_add(load_sz), cs.checked_add(store_sz)) else {
        return false;
    };
    le <= s && se <= s && (le <= cs || se <= cl)
}

fn gcd_u64(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    fn form(konst: i64, march: i64) -> LinForm {
        LinForm {
            root: 1,
            syms: vec![],
            konst,
            march,
        }
    }
    #[test]
    fn overflow_fails_closed() {
        assert!(!forms_disjoint(
            &form(i64::MIN, 0),
            8,
            &form(i64::MAX, 0),
            8,
            true
        ));
    }
    #[test]
    fn separated_forms_prove() {
        assert!(forms_disjoint(&form(0, 0), 8, &form(8, 0), 4, true));
    }

    // ── stride-period field disjointness ──────────────────────────────
    fn sym_form(sym: u32, coeff: i64, konst: i64) -> LinForm {
        LinForm {
            root: 1,
            syms: vec![(sym, coeff)],
            konst,
            march: 0,
        }
    }
    #[test]
    fn stride_rule_nbody_shape() {
        // bodies[i].x/y [0,16) vs bodies[j].vx/vy [24,40), stride 56:
        // the nbody pair-loop case (different copy-web symbols).
        assert!(forms_disjoint(
            &sym_form(1, 56, 0),
            16,
            &sym_form(2, 56, 24),
            16,
            true
        ));
        // bodies[j].mass [48,56) vs bodies[i].vz [40,48): adjacent but
        // disjoint fields.
        assert!(forms_disjoint(
            &sym_form(2, 56, 48),
            8,
            &sym_form(1, 56, 40),
            8,
            true
        ));
        // Same residues on different symbols still MAY alias (i and j are
        // unrelated values): [24,40) vs [24,40) overlaps.
        assert!(!forms_disjoint(
            &sym_form(1, 56, 24),
            16,
            &sym_form(2, 56, 24),
            16,
            true
        ));
    }
    #[test]
    fn stride_rule_mixed_strides() {
        // A 2-body-stride walker (coeff 112) vs an element index (56):
        // gcd 56, store residue 24 — disjoint from [0,16).
        assert!(forms_disjoint(
            &sym_form(1, 56, 0),
            16,
            &sym_form(2, 112, 24),
            16,
            true
        ));
        // Store residue 0 vs load [48,56): overlap at residue 0? No —
        // [0,8) vs [48,56) are disjoint residues.
        assert!(forms_disjoint(
            &sym_form(1, 56, 48),
            8,
            &sym_form(2, 112, 0),
            8,
            true
        ));
    }
    #[test]
    fn stride_rule_fixed_address_vs_indexed() {
        // A constant store (no syms) at root+1000 vs an indexed load with
        // residue [48,56): 1000 mod 56 = 48 — residues coincide, overlap.
        assert!(!forms_disjoint(
            &sym_form(1, 56, 48),
            8,
            &form(1000, 0),
            8,
            true
        ));
        // Offset 992 (= 56·17 + 40): residue [40,48), disjoint from [48,56).
        assert!(forms_disjoint(
            &sym_form(1, 56, 48),
            8,
            &form(992, 0),
            8,
            true
        ));
    }
    #[test]
    fn stride_rule_straddling_fails_closed() {
        // A 16-byte access whose residue interval exceeds the period:
        // residue 48 + 16 > 56 — cannot sit inside one period, refuse.
        assert!(!forms_disjoint(
            &sym_form(1, 56, 48),
            16,
            &sym_form(2, 56, 24),
            16,
            true
        ));
        // Odd strides with no common divisor structure: 24 vs 56 → gcd 8,
        // residues collapse into overlapping [0,8) windows — refuse.
        assert!(!forms_disjoint(
            &sym_form(1, 24, 0),
            8,
            &sym_form(2, 56, 24),
            8,
            true
        ));
    }
    #[test]
    fn stride_rule_march_divisible_by_stride() {
        // The load is a loop-invariant field; the store's pointer marches a
        // full element per iteration (march 56, divisible by the stride):
        // the store's residue stays [24,40) at every t — provable.
        let mut store = sym_form(2, 56, 24);
        store.march = 56;
        assert!(forms_disjoint(&sym_form(1, 56, 0), 16, &store, 16, true));
        // A march NOT divisible by the stride moves the residue over time:
        // refuse.
        let mut store2 = sym_form(2, 56, 24);
        store2.march = 8;
        assert!(!forms_disjoint(&sym_form(1, 56, 0), 16, &store2, 16, true));
    }
}
