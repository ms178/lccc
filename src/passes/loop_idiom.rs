//! Loop-idiom recognition: byte-copy loops become a `memcpy` libcall.
//!
//! Real-world C copies bytes in loops everywhere (compressors, codecs, string
//! routines, network buffers, kernels). A byte-at-a-time loop costs ~7-9
//! instructions per byte; glibc `memcpy` moves 32+ bytes per instruction with
//! SIMD. The vectorizer only takes canonical single-level indexed loops —
//! lz4's literal copy (`*op++ = anchor[i]`, mixed pointer-bump/indexed form,
//! nested, outer-reused IV) stays scalar at 7 insns/byte and accounts for a
//! large share of lz4's ~10x gap to GCC (which emits `call memcpy@PLT`).
//!
//! This pass matches the counted single-load/single-store byte-copy loop in
//! its guard-at-top form and replaces the whole loop with one
//! `Call{memcpy}` in the preheader, computing the loop's exit values
//! directly (`dst_final = dst_init + n`, `i_final = n`). The dead loop is
//! removed immediately via `eliminate_unreachable_blocks`.
//!
//! Legality (every guard fails closed — bail keeps the original loop):
//!
//! - Shape: 2–3 block natural loop, single latch, header preds exactly
//!   `{preheader, latch}`, single exit edge from the header, no other
//!   entries/exits/branches/calls/memory ops in the loop. Non-header blocks
//!   contain exactly one `U8` load + one `U8` store plus `Copy`/`Cast`/`GEP`/
//!   `Add`-1 plumbing. No volatile, no atomics, no inline asm, no non-header
//!   phis.
//! - IV: one integer phi, init const 0, step const `+1`, bound
//!   loop-invariant and dominating the preheader, header test exactly
//!   `Ult(iv, bound)`.
//! - Pointers: load/store addresses are `GEP(base, iv)` (indexed) or a
//!   header pointer-phi (bump form); the load reads the pre-bump value.
//!   Bases/inits are loop-invariant and dominate the preheader.
//! - Coverage: the loaded value's only use is the store (through `Copy`s);
//!   bumps' only uses are their phis. Nothing else escapes the loop.
//! - Overlap: the two object roots must differ and both name uniquely
//!   identified objects (distinct globals, distinct static allocas, or
//!   global↔alloca) — the same proof `loop_memory_promote::disjoint`
//!   uses. Same-object or parameter-rooted copies bail (a `dst > src`
//!   overlap makes the forward loop read smeared bytes, which is neither
//!   `memcpy` nor `memmove` semantics).
//! - Exits: every exit phi's loop-edge incoming is the IV (rewritten to the
//!   bound), a pointer-phi (rewritten to `GEP(init, bound)`), or a
//!   loop-invariant (kept). Anything else bails.
//!
//! The 0-trip case is exact: `memcpy(d, s, 0)` performs no access, matching
//! the skipped loop (glibc defines this; LLVM/GCC perform the same
//! transform). `memcpy` needs no declaration: like the frontend's
//! `emit_dynamic_memcpy` (VLA path), the backend lowers the plain call and
//! the default link resolves libc.
//!
//! Placement: main loop, iter 0, immediately before `vectorize`, so idiom
//! loops become one call instead of versioned-vectorize + runtime check +
//! scalar remainder; everything unmatched still flows to the vectorizer.
//! A `memcpy` call cannot block outer-loop vectorization that the nested
//! counted loop it replaced would have allowed (nested loops never
//! vectorize here), and later passes treat the call conservatively.
//! Enabled at -O2+ including -Os/-Oz (a call is smaller than a loop).
//!
//! Kill-switch: `CCC_NO_LOOP_IDIOM` set (any value) disables the pass.
//! Opt-in during bring-up: `CCC_LOOP_IDIOM=1` (`true`/`yes`/`on` also
//! accepted); default-off until the regression suite + corpus A/B + fuzz
//! validate the guards (loop transforms earned this caution: loop_rotate's
//! v16 default-enable shipped 16 miscompiles).
//! Debug: `CCC_DEBUG_LOOP_IDIOM=1` logs matches, rewrites, and bail reasons.
//!
//! Phase 2 (not yet implemented): byte-compare loops
//! (`while (p < end && *p == *q)`) expand to a word-at-a-time loop with the
//! original byte loop as the tail. That needs a same-object + ordering
//! range proof (word loads must not over-read the shorter side) and is a
//! separate matcher + legality argument.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::analysis::{self, CfgAnalysis};
use crate::ir::reexports::{
    BlockId, Instruction, IrBinOp, IrCmpOp, IrConst, IrFunction, Operand, Terminator, Value,
};
use crate::passes::loop_analysis::{DominanceChecker, find_natural_loops, merge_loops_by_header};

/// Per-function entry point for the dirty-tracking pipeline.
pub(crate) fn run_function(func: &mut IrFunction) -> usize {
    recognize_idioms(func)
}

/// Maximum loops rewritten per function per fixpoint run. Each rewrite
/// rebuilds the CFG, bounding quadratic worst cases in deep nests.
#[allow(dead_code)] // M2: used by the rewrite fixpoint.
const MAX_REWRITES_PER_FUNC: usize = 256;

/// Follow `Copy` chains (cycle-guarded). Most plumbing between the load and
/// the store, and around IV/bound operands, is `Copy` nodes awaiting
/// copy-prop; resolving through them keeps the matcher tight without
/// depending on pass order.
fn resolve_copy(defs: &FxHashMap<u32, &Instruction>, mut v: Value) -> Value {
    for _ in 0..32 {
        match defs.get(&v.0) {
            Some(Instruction::Copy { src, .. }) => match src {
                Operand::Value(inner) => v = *inner,
                Operand::Const(_) => break,
            },
            _ => break,
        }
    }
    v
}

/// Follow `Copy` and integer `Cast` chains (cycle-guarded). GEP offsets
/// commonly pass through a `U32 -> I64` widening cast.
fn resolve_copy_cast(defs: &FxHashMap<u32, &Instruction>, mut v: Value) -> Value {
    for _ in 0..32 {
        match defs.get(&v.0) {
            Some(Instruction::Copy { src, .. }) => match src {
                Operand::Value(inner) => v = *inner,
                Operand::Const(_) => break,
            },
            Some(Instruction::Cast { src, .. }) => match src {
                Operand::Value(inner) => v = *inner,
                Operand::Const(_) => break,
            },
            _ => break,
        }
    }
    v
}

fn const_int_value(c: IrConst) -> Option<i64> {
    c.to_i64()
}

fn operand_const_int(op: &Operand) -> Option<i64> {
    match op {
        Operand::Const(c) => const_int_value(*c),
        Operand::Value(_) => None,
    }
}

fn is_int_type(ty: IrType) -> bool {
    matches!(
        ty,
        IrType::I8
            | IrType::I16
            | IrType::I32
            | IrType::I64
            | IrType::I128
            | IrType::U8
            | IrType::U16
            | IrType::U32
            | IrType::U64
            | IrType::U128
    )
}

fn is_byte_type(ty: IrType) -> bool {
    matches!(ty, IrType::I8 | IrType::U8)
}

/// Object identity for overlap reasoning: `Global(name)` for globals,
/// `Alloca(id)` for static stack slots, `Param(idx)` for function
/// parameters (distinct params are assumed non-overlapping for the
/// memcpy idiom; if they may alias we use memmove), `Other` for everything
/// else (dynamic allocas, computed pointers, unknowns).
///
/// The walk sees through `Phi` (preheader-edge init only — the latch edge
/// marches within the same object), `Copy`, integer `Cast`, `GEP` (base),
/// and `Add`/`Sub` (the side that resolves; an integer offset contributes
/// no root, and two rooted sides is nonsense → `Other`). Offsets are
/// deliberately ignored: cross-object disjointness needs roots only, and
/// same-object copies bail regardless of offsets (v1).
#[derive(Clone, PartialEq, Eq, Debug)]
enum ObjectRoot {
    Global(String),
    Alloca(u32),
    Param(u32),
    Other,
}

fn object_root(
    defs: &FxHashMap<u32, &Instruction>,
    label_to_idx: &FxHashMap<BlockId, usize>,
    preheader_label: BlockId,
    v: Value,
) -> ObjectRoot {
    object_root_inner(defs, label_to_idx, preheader_label, v, 0)
}

fn object_root_inner(
    defs: &FxHashMap<u32, &Instruction>,
    label_to_idx: &FxHashMap<BlockId, usize>,
    preheader_label: BlockId,
    v: Value,
    depth: usize,
) -> ObjectRoot {
    if depth > 32 {
        return ObjectRoot::Other;
    }
    let Some(inst) = defs.get(&v.0) else {
        return ObjectRoot::Other;
    };
    match inst {
        Instruction::GlobalAddr { name, .. } => ObjectRoot::Global(name.clone()),
        Instruction::Alloca { dest, .. } => ObjectRoot::Alloca(dest.0),
        Instruction::ParamRef { param_idx, .. } => ObjectRoot::Param(*param_idx as u32),
        Instruction::Copy { src, .. } | Instruction::Cast { src, .. } => match src {
            Operand::Value(inner) => {
                object_root_inner(defs, label_to_idx, preheader_label, *inner, depth + 1)
            }
            Operand::Const(_) => ObjectRoot::Other,
        },
        Instruction::GetElementPtr { base, .. } => {
            object_root_inner(defs, label_to_idx, preheader_label, *base, depth + 1)
        }
        Instruction::Phi { incoming, .. } => {
            for (op, label) in incoming {
                if *label == preheader_label {
                    if let Operand::Value(inner) = op {
                        return object_root_inner(
                            defs,
                            label_to_idx,
                            preheader_label,
                            *inner,
                            depth + 1,
                        );
                    }
                    return ObjectRoot::Other;
                }
            }
            ObjectRoot::Other
        }
        Instruction::BinOp {
            op: IrBinOp::Add | IrBinOp::Sub,
            lhs,
            rhs,
            ..
        } => {
            let l = match lhs {
                Operand::Value(inner) => {
                    object_root_inner(defs, label_to_idx, preheader_label, *inner, depth + 1)
                }
                Operand::Const(_) => ObjectRoot::Other,
            };
            let r = match rhs {
                Operand::Value(inner) => {
                    object_root_inner(defs, label_to_idx, preheader_label, *inner, depth + 1)
                }
                Operand::Const(_) => ObjectRoot::Other,
            };
            match (l, r) {
                (ObjectRoot::Other, ObjectRoot::Other) => ObjectRoot::Other,
                (root, ObjectRoot::Other) | (ObjectRoot::Other, root) => root,
                // Two rooted sides (ptr + ptr) is nonsense; fail closed.
                _ => ObjectRoot::Other,
            }
        }
        _ => {
            let _ = label_to_idx;
            ObjectRoot::Other
        }
    }
}

fn is_unique_root(root: &ObjectRoot) -> bool {
    matches!(
        root,
        ObjectRoot::Global(_) | ObjectRoot::Alloca(_) | ObjectRoot::Param(_)
    )
}

/// Returns true if two roots are provably distinct objects (no overlap).
fn roots_distinct(a: &ObjectRoot, b: &ObjectRoot) -> bool {
    match (a, b) {
        (ObjectRoot::Global(ga), ObjectRoot::Global(gb)) => ga != gb,
        (ObjectRoot::Alloca(aa), ObjectRoot::Alloca(ab)) => aa != ab,
        (ObjectRoot::Param(pa), ObjectRoot::Param(pb)) => pa != pb,
        (ObjectRoot::Global(_), ObjectRoot::Alloca(_))
        | (ObjectRoot::Alloca(_), ObjectRoot::Global(_))
        | (ObjectRoot::Global(_), ObjectRoot::Param(_))
        | (ObjectRoot::Param(_), ObjectRoot::Global(_))
        | (ObjectRoot::Alloca(_), ObjectRoot::Param(_))
        | (ObjectRoot::Param(_), ObjectRoot::Alloca(_)) => true,
        _ => false,
    }
}

/// A matched byte-copy loop with everything the rewrite needs.
#[allow(dead_code)]
struct CopyLoop {
    header: usize,
    preheader: usize,
    latch: usize,
    exit: usize,
    /// Integer IV phi in the header.
    iv_phi: Value,
    /// Loop-invariant trip count operand (`Ult(iv, bound)`).
    bound: Operand,
    /// Comparison type of the `Ult` (bound and IV element type).
    ult_ty: IrType,
    /// Header bound-load dest, if the bound is computed by one (M2
    /// clones it into the preheader).
    bound_load: Option<Value>,
    /// `true` if the store address is a bump-phi, `false` if `GEP(init, iv)`.
    store_is_bump: bool,
    /// Store base: bump-phi dest, or GEP base value.
    store_base: Value,
    /// Store init pointer (bump-phi init, or GEP base itself).
    store_init: Value,
    /// Load side, same layout.
    load_is_bump: bool,
    load_base: Value,
    load_init: Value,
    /// Use memmove instead of memcpy (param roots may alias).
    use_memmove: bool,
}

fn debug_enabled() -> bool {
    static FLAG: std::sync::LazyLock<bool> =
        std::sync::LazyLock::new(|| std::env::var_os("CCC_DEBUG_LOOP_IDIOM").is_some());
    *FLAG
}

macro_rules! dlog {
    ($($arg:tt)*) => {
        if debug_enabled() {
            eprintln!("[IDIOM] {}", format!($($arg)*));
        }
    };
}

/// True when `name` is set to a truthy value (`1`/`true`/`yes`/`on`).
fn env_flag_truthy(name: &str) -> bool {
    match std::env::var(name) {
        Ok(v) => {
            let t = v.trim();
            t == "1"
                || t.eq_ignore_ascii_case("true")
                || t.eq_ignore_ascii_case("yes")
                || t.eq_ignore_ascii_case("on")
        }
        Err(_) => false,
    }
}

pub(crate) fn recognize_idioms(func: &mut IrFunction) -> usize {
    if std::env::var("CCC_NO_LOOP_IDIOM").is_ok() {
        return 0;
    }
    // Default-on after P0 validation: single-block Ptr-IV (header==latch)
    // and 2-3 block forms are proven safe; opt-out via CCC_NO_LOOP_IDIOM.
    // The old CCC_LOOP_IDIOM=1 opt-in knob is retained as a no-op for
    // compatibility but no longer gates the pass.
    if func.blocks.len() < 3 {
        return 0;
    }
    // M2: match + rewrite fixpoint. One rewrite per rescan (block
    // indices shift when the dead loop is removed), innermost-first,
    // bounded (same discipline as loop_rotate).
    let mut total = 0;
    for _ in 0..MAX_REWRITES_PER_FUNC {
        let cfg = CfgAnalysis::build(func);
        let raw = find_natural_loops(cfg.num_blocks, &cfg.preds, &cfg.succs, &cfg.idom);
        if raw.is_empty() {
            break;
        }
        let loops = merge_loops_by_header(raw);
        let mut sorted: Vec<_> = loops.iter().collect();
        sorted.sort_by_key(|lp| lp.body.len());
        let mut progressed = false;
        for lp in sorted {
            if let Some(matched) = try_match_copy_loop(func, lp, &cfg) {
                dlog!(
                    "{} loop@{}: MATCH copy (header={} pre={} latch={} exit={} bound={:?} store_bump={} load_bump={})",
                    func.name,
                    lp.header,
                    matched.header,
                    matched.preheader,
                    matched.latch,
                    matched.exit,
                    matched.bound,
                    matched.store_is_bump,
                    matched.load_is_bump,
                );
                if rewrite_copy_loop(func, &matched, &lp.body) {
                    total += 1;
                    progressed = true;
                    break;
                }
            }
        }
        if !progressed {
            break;
        }
    }
    total
}

/// Block index of the definition of `v`, or `None` for arguments/unknowns.
fn def_block(
    def_blocks: &FxHashMap<u32, usize>,
    label_to_idx: &FxHashMap<BlockId, usize>,
    op: &Operand,
) -> Option<usize> {
    let _ = label_to_idx;
    match op {
        Operand::Const(_) => None,
        Operand::Value(v) => def_blocks.get(&v.0).copied(),
    }
}

#[allow(clippy::too_many_arguments)]
fn try_match_copy_loop(
    func: &IrFunction,
    lp: &crate::passes::loop_analysis::NaturalLoop,
    cfg: &CfgAnalysis,
) -> Option<CopyLoop> {
    let name = func.name.as_str();
    // Shape: 1–3 blocks (header + body/latch). Single-block self-loops
    // (header==latch) are now supported — they appear when the frontend
    // does not split the latch, e.g. `for (p=d,s=s; i<n; ++i) *p++=*s++;`.
    // Larger bodies bail (v1 tightness).
    if lp.body.is_empty() || lp.body.len() > 3 {
        dlog!(
            "{name} loop@{}: bail (body size {})",
            lp.header,
            lp.body.len()
        );
        return None;
    }
    let header = lp.header;
    let label_to_idx = analysis::build_label_map(func);
    let header_label = func.blocks[header].label;

    // Header must end in a conditional branch with one successor in the
    // loop (body entry) and one outside (the single exit).
    let (cond_val, in_succ, exit_idx) = match &func.blocks[header].terminator {
        Terminator::CondBranch {
            cond,
            true_label,
            false_label,
        } => {
            let ti = *label_to_idx.get(true_label)?;
            let fi = *label_to_idx.get(false_label)?;
            let t_in = lp.body.contains(&ti);
            let f_in = lp.body.contains(&fi);
            if t_in == f_in {
                dlog!("{name} loop@{header}: bail (header succs not split)");
                return None;
            }
            let cond_val = match cond {
                Operand::Value(v) => *v,
                Operand::Const(_) => {
                    dlog!("{name} loop@{header}: bail (const header cond)");
                    return None;
                }
            };
            if t_in {
                (cond_val, ti, fi)
            } else {
                (cond_val, fi, ti)
            }
        }
        _ => {
            dlog!("{name} loop@{header}: bail (header not CondBranch)");
            return None;
        }
    };

    // Header preds must be exactly {preheader, latch}.
    let preds: Vec<usize> = cfg.preds.row(header).iter().map(|&p| p as usize).collect();
    if preds.len() != 2 {
        dlog!("{name} loop@{header}: bail (header preds {})", preds.len());
        return None;
    }
    let latch = lp.single_latch(&cfg.preds)?;
    let preheader = if preds[0] == latch {
        preds[1]
    } else {
        preds[0]
    };
    if preheader == latch || lp.body.contains(&preheader) {
        dlog!("{name} loop@{header}: bail (preheader inside loop)");
        return None;
    }
    let preheader_label = func.blocks[preheader].label;
    let latch_label = func.blocks[latch].label;

    // The M2 rewrite retargets the preheader to the exit, so the preheader
    // must transfer control unconditionally to the header (single succ).
    match &func.blocks[preheader].terminator {
        Terminator::Branch(target) if *label_to_idx.get(target)? == header => {}
        _ => {
            dlog!("{name} loop@{header}: bail (preheader not Branch header)");
            return None;
        }
    }
    // exit == preheader would make the retarget a self-loop (the original
    // is infinite or unreachable in that case); bail defensively.
    if exit_idx == preheader {
        dlog!("{name} loop@{header}: bail (exit is preheader)");
        return None;
    }

    // No other entries: every non-header block's preds are inside the loop.
    // No other exits: every non-header block's succs are inside the loop,
    // and non-header terminators are unconditional branches.
    for &b in lp.body.iter() {
        if b == header {
            continue;
        }
        for &p in cfg.preds.row(b).iter() {
            if !lp.body.contains(&(p as usize)) {
                dlog!("{name} loop@{header}: bail (outside entry to {b})");
                return None;
            }
        }
        match &func.blocks[b].terminator {
            Terminator::Branch(target) => {
                let ti = *label_to_idx.get(target)?;
                if !lp.body.contains(&ti) {
                    dlog!("{name} loop@{header}: bail (block {b} exits loop)");
                    return None;
                }
            }
            _ => {
                dlog!("{name} loop@{header}: bail (block {b} not Branch)");
                return None;
            }
        }
    }

    // Definition maps.
    let mut defs: FxHashMap<u32, &Instruction> = FxHashMap::default();
    let mut def_blocks: FxHashMap<u32, usize> = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            if let Some(d) = inst.dest() {
                defs.insert(d.0, inst);
                def_blocks.insert(d.0, bi);
            }
        }
    }
    // Use counts over instructions, phis, and terminators (function-wide;
    // the loop values must not escape).
    let mut uses: FxHashMap<u32, usize> = FxHashMap::default();
    let count_op = |op: &Operand, uses: &mut FxHashMap<u32, usize>| {
        if let Operand::Value(v) = op {
            *uses.entry(v.0).or_insert(0) += 1;
        }
    };
    for block in &func.blocks {
        for inst in &block.instructions {
            inst.for_each_used_value(|v| {
                *uses.entry(v).or_insert(0) += 1;
            });
        }
        match &block.terminator {
            Terminator::CondBranch { cond, .. } => count_op(cond, &mut uses),
            Terminator::Branch(_) => {}
            Terminator::Return(ret) => {
                if let Some(op) = ret {
                    count_op(op, &mut uses);
                }
            }
            _ => {}
        }
    }

    // Body scan: exactly one U8 load + one U8 store, plumbing otherwise.
    // For multi-block loops the copy lives in non-header blocks; for
    // single-block self-loops (header==latch) the copy lives in the header
    // itself (Ptr-IV form: `*dst++ = *src++`). We support both.
    let is_self_loop = lp.body.len() == 1 || latch == header;
    let mut load_inst: Option<(usize, Value, Value)> = None; // (block, dest, ptr)
    let mut store_inst: Option<(usize, Operand, Value)> = None; // (block, val, ptr)

    if !is_self_loop {
        for &b in lp.body.iter() {
            if b == header {
                continue;
            }
            for inst in &func.blocks[b].instructions {
                match inst {
                    Instruction::Copy { .. }
                    | Instruction::Cast { .. }
                    | Instruction::GetElementPtr { .. } => {}
                    Instruction::BinOp { .. } => {}
                    Instruction::GlobalAddr { .. }
                    | Instruction::ParamRef { .. }
                    | Instruction::LabelAddr { .. } => {}
                    Instruction::Load {
                        dest,
                        ptr,
                        ty,
                        seg_override,
                        volatile,
                    } => {
                        if !is_byte_type(*ty)
                            || *seg_override != crate::common::types::AddressSpace::Default
                            || *volatile
                        {
                            dlog!("{name} loop@{header}: bail (non-U8/volatile load)");
                            return None;
                        }
                        if load_inst.is_some() {
                            dlog!("{name} loop@{header}: bail (second load)");
                            return None;
                        }
                        load_inst = Some((b, *dest, *ptr));
                    }
                    Instruction::Store {
                        val,
                        ptr,
                        ty,
                        seg_override,
                        volatile,
                    } => {
                        if !is_byte_type(*ty)
                            || *seg_override != crate::common::types::AddressSpace::Default
                            || *volatile
                        {
                            dlog!("{name} loop@{header}: bail (non-U8/volatile store)");
                            return None;
                        }
                        if store_inst.is_some() {
                            dlog!("{name} loop@{header}: bail (second store)");
                            return None;
                        }
                        store_inst = Some((b, val.clone(), *ptr));
                    }
                    _ => {
                        dlog!("{name} loop@{header}: bail (body inst {inst:?})");
                        return None;
                    }
                }
            }
        }
    }

    // Header scan: phis + pure plumbing, plus bound loads. For self-loops
    // we also accept the single U8 load/store that constitute the copy.
    struct PhiInfo {
        dest: Value,
        ty: IrType,
        init: Operand,
        latch: Operand,
    }
    let mut phis: Vec<PhiInfo> = Vec::new();
    let mut header_loads: Vec<Value> = Vec::new();
    for inst in &func.blocks[header].instructions {
        match inst {
            Instruction::Phi { dest, ty, incoming } => {
                if incoming.len() != 2 {
                    dlog!("{name} loop@{header}: bail (phi arity {})", incoming.len());
                    return None;
                }
                let mut init: Option<Operand> = None;
                let mut latch_op: Option<Operand> = None;
                for (op, label) in incoming {
                    if *label == preheader_label {
                        init = Some(op.clone());
                    } else if *label == latch_label {
                        latch_op = Some(op.clone());
                    } else {
                        dlog!("{name} loop@{header}: bail (phi from outside)");
                        return None;
                    }
                }
                phis.push(PhiInfo {
                    dest: *dest,
                    ty: *ty,
                    init: init?,
                    latch: latch_op?,
                });
            }
            Instruction::Copy { .. }
            | Instruction::Cast { .. }
            | Instruction::BinOp { .. }
            | Instruction::Cmp { .. }
            | Instruction::GetElementPtr { .. }
            | Instruction::Select { .. }
            | Instruction::GlobalAddr { .. }
            | Instruction::ParamRef { .. }
            | Instruction::LabelAddr { .. } => {}
            Instruction::Load {
                dest,
                ptr,
                ty,
                seg_override,
                volatile,
            } => {
                if *volatile || *seg_override != crate::common::types::AddressSpace::Default {
                    dlog!("{name} loop@{header}: bail (header load volatile/seg)");
                    return None;
                }
                if is_self_loop && is_byte_type(*ty) {
                    if load_inst.is_some() {
                        dlog!("{name} loop@{header}: bail (second load self-loop)");
                        return None;
                    }
                    load_inst = Some((header, *dest, *ptr));
                } else {
                    if !is_int_type(*ty) {
                        dlog!("{name} loop@{header}: bail (header load form)");
                        return None;
                    }
                    header_loads.push(*dest);
                }
            }
            Instruction::Store {
                val,
                ptr,
                ty,
                seg_override,
                volatile,
            } => {
                if is_self_loop
                    && is_byte_type(*ty)
                    && *seg_override == crate::common::types::AddressSpace::Default
                    && !*volatile
                {
                    if store_inst.is_some() {
                        dlog!("{name} loop@{header}: bail (second store self-loop)");
                        return None;
                    }
                    store_inst = Some((header, val.clone(), *ptr));
                } else {
                    dlog!("{name} loop@{header}: bail (header inst {inst:?})");
                    return None;
                }
            }
            _ => {
                dlog!("{name} loop@{header}: bail (header inst {inst:?})");
                return None;
            }
        }
    }

    let Some((_, load_dest, load_ptr)) = load_inst else {
        dlog!("{name} loop@{header}: bail (missing load)");
        return None;
    };
    let Some((_, store_val, store_ptr)) = store_inst else {
        dlog!("{name} loop@{header}: bail (missing store)");
        return None;
    };

    // Header test: exactly `Ult(iv, bound)` where iv is one of the phis.
    let cond_def = defs.get(&cond_val.0)?;
    let (iv_phi, bound, ult_ty) = match cond_def {
        Instruction::Cmp {
            op: IrCmpOp::Ult,
            lhs,
            rhs,
            ty,
            ..
        } => {
            let lhs_v = match lhs {
                Operand::Value(v) => resolve_copy(&defs, *v),
                Operand::Const(_) => {
                    dlog!("{name} loop@{header}: bail (Ult lhs const)");
                    return None;
                }
            };
            (lhs_v, rhs.clone(), *ty)
        }
        _ => {
            dlog!("{name} loop@{header}: bail (test not Ult)");
            return None;
        }
    };
    // The bound must be const, defined outside the loop, or a header
    // bound-load (validated later: static-address + disjoint-root
    // checks prove it hoistable + loop-invariant).
    match &bound {
        Operand::Const(_) => {}
        Operand::Value(v) => match def_block(&def_blocks, &label_to_idx, &bound) {
            Some(db) if !lp.body.contains(&db) => {}
            Some(db) if db == header && header_loads.contains(&resolve_copy(&defs, *v)) => {}
            _ => {
                dlog!("{name} loop@{header}: bail (bound in loop/arg)");
                return None;
            }
        },
    }

    // IV phi: init const 0, latch is `iv + 1` (either operand order),
    // plus-form check happens after the bump-pointer scan.
    let mut iv_init_ok = false;
    let mut iv_latch_add: Option<Value> = None; // the `iv+1` BinOp dest
    // Bump-pointer phis: dest -> (init Value, bump BinOp dest).
    let mut bump_phis: FxHashMap<u32, (Value, Value)> = FxHashMap::default();
    let mut int_phis = 0usize;
    for phi in &phis {
        if phi.dest == iv_phi {
            if !is_int_type(phi.ty) {
                dlog!("{name} loop@{header}: bail (IV not int)");
                return None;
            }
            // 128-bit trip counts cannot be proven to fit `size_t`
            // (truncating `len` would copy less than the loop); bail.
            if matches!(phi.ty, IrType::I128 | IrType::U128) {
                dlog!("{name} loop@{header}: bail (IV 128-bit)");
                return None;
            }
            match &phi.init {
                Operand::Const(c) if const_int_value(*c) == Some(0) => iv_init_ok = true,
                _ => {
                    dlog!("{name} loop@{header}: bail (IV init not 0)");
                    return None;
                }
            }
            let latch_v = match &phi.latch {
                Operand::Value(v) => resolve_copy(&defs, *v),
                Operand::Const(_) => {
                    dlog!("{name} loop@{header}: bail (IV latch const)");
                    return None;
                }
            };
            match defs.get(&latch_v.0) {
                Some(Instruction::BinOp {
                    dest,
                    op: IrBinOp::Add,
                    lhs,
                    rhs,
                    ..
                }) => {
                    let l = match lhs {
                        Operand::Value(v) => resolve_copy(&defs, *v),
                        Operand::Const(_) => {
                            dlog!("{name} loop@{header}: bail (IV add lhs const)");
                            return None;
                        }
                    };
                    let r_is_one = operand_const_int(rhs) == Some(1);
                    let l_is_one = operand_const_int(lhs) == Some(1);
                    let r = match rhs {
                        Operand::Value(v) => resolve_copy(&defs, *v),
                        Operand::Const(_) => Value(u32::MAX),
                    };
                    if (l == phi.dest && r_is_one) || (l_is_one && r == phi.dest) {
                        iv_latch_add = Some(*dest);
                    } else {
                        dlog!("{name} loop@{header}: bail (IV step not +1)");
                        return None;
                    }
                }
                _ => {
                    dlog!("{name} loop@{header}: bail (IV latch not Add)");
                    return None;
                }
            }
            int_phis += 1;
        } else if phi.ty == IrType::Ptr {
            // Candidate bump-pointer phi: latch must be `phi + 1`.
            let init_v = match &phi.init {
                Operand::Value(v) => *v,
                Operand::Const(_) => {
                    dlog!("{name} loop@{header}: bail (ptr phi const init)");
                    return None;
                }
            };
            let latch_v = match &phi.latch {
                Operand::Value(v) => resolve_copy(&defs, *v),
                Operand::Const(_) => {
                    dlog!("{name} loop@{header}: bail (ptr phi const latch)");
                    return None;
                }
            };
            match defs.get(&latch_v.0) {
                Some(Instruction::BinOp {
                    dest,
                    op: IrBinOp::Add,
                    lhs,
                    rhs,
                    ..
                }) => {
                    let l = match lhs {
                        Operand::Value(v) => resolve_copy(&defs, *v),
                        Operand::Const(_) => {
                            dlog!("{name} loop@{header}: bail (bump lhs const)");
                            return None;
                        }
                    };
                    let r_is_one = operand_const_int(rhs) == Some(1);
                    let l_is_one = operand_const_int(lhs) == Some(1);
                    let r = match rhs {
                        Operand::Value(v) => resolve_copy(&defs, *v),
                        Operand::Const(_) => Value(u32::MAX),
                    };
                    if (l == phi.dest && r_is_one) || (l_is_one && r == phi.dest) {
                        bump_phis.insert(phi.dest.0, (init_v, *dest));
                    } else {
                        dlog!("{name} loop@{header}: bail (bump step not +1)");
                        return None;
                    }
                }
                _ => {
                    dlog!("{name} loop@{header}: bail (ptr phi latch not Add)");
                    return None;
                }
            }
        } else if is_int_type(phi.ty) {
            int_phis += 1;
        } else {
            dlog!("{name} loop@{header}: bail (phi ty {ty:?})", ty = phi.ty);
            return None;
        }
    }
    if !iv_init_ok || iv_latch_add.is_none() {
        dlog!("{name} loop@{header}: bail (no IV phi)");
        return None;
    }
    // Only the IV may be an integer phi: a second integer phi that
    // latches anything other than a covered form is live state the
    // rewrite cannot reproduce.
    if int_phis != 1 {
        dlog!("{name} loop@{header}: bail ({int_phis} int phis)");
        return None;
    }

    let dom = DominanceChecker::new(func.blocks.len(), &cfg.idom);

    // Pointer-form analysis: each side is `GEP(base, iv)` or a bump phi.
    // Returns (is_bump, base_or_phi, init_ptr).
    let classify_addr = |ptr: Value, side: &str| -> Option<(bool, Value, Value)> {
        let r = resolve_copy_cast(&defs, ptr);
        if let Some(bump) = bump_phis.get(&r.0) {
            return Some((true, r, bump.0));
        }
        match defs.get(&r.0) {
            Some(Instruction::GetElementPtr { base, offset, .. }) => {
                let off = match offset {
                    Operand::Value(v) => resolve_copy_cast(&defs, *v),
                    Operand::Const(_) => {
                        dlog!("{name} loop@{header}: bail ({side} GEP const off)");
                        return None;
                    }
                };
                if off != iv_phi {
                    dlog!("{name} loop@{header}: bail ({side} GEP off not IV)");
                    return None;
                }
                Some((false, *base, *base))
            }
            _ => {
                dlog!("{name} loop@{header}: bail ({side} addr form)");
                None
            }
        }
    };
    let (store_is_bump, store_base, store_init) = classify_addr(store_ptr, "store")?;
    let (load_is_bump, load_base, load_init) = classify_addr(load_ptr, "load")?;

    // Base/init invariance: const or defined outside the loop, and the
    // definition must dominate the preheader (the rewrite sinks the base
    // computation's *uses* into the preheader call — the defs themselves
    // stay put, but the call needs them available in the preheader).
    let mut check_invariant = |v: Value, what: &str| -> Option<()> {
        // Operand-free pure leaves are rematerializable anywhere (M2
        // clones them into the preheader); their position is irrelevant.
        if let Some(def) = defs.get(&v.0) {
            match def {
                Instruction::GlobalAddr { .. }
                | Instruction::ParamRef { .. }
                | Instruction::LabelAddr { .. } => return Some(()),
                _ => {}
            }
        }
        match def_blocks.get(&v.0) {
            Some(&db) => {
                if lp.body.contains(&db) {
                    dlog!("{name} loop@{header}: bail ({what} defined in loop)");
                    return None;
                }
                if !dom.dominates(db, preheader) {
                    dlog!("{name} loop@{header}: bail ({what} not dom preheader)");
                    return None;
                }
            }
            None => {
                dlog!("{name} loop@{header}: bail ({what} is arg/unknown)");
                return None;
            }
        }
        Some(())
    };
    // For bump sides the "base" is the header phi itself (loop-defined
    // by design); only the init must be invariant. For indexed sides the
    // GEP base must be invariant (init == base there).
    if !store_is_bump {
        check_invariant(store_base, "store base")?;
    }
    check_invariant(store_init, "store init")?;
    if !load_is_bump {
        check_invariant(load_base, "load base")?;
    }
    check_invariant(load_init, "load init")?;
    // The bound likewise (consts are trivially invariant; args are
    // available everywhere so they pass; header bound-loads are
    // validated later and hoisted by the M2 rewrite).
    match &bound {
        Operand::Const(_) => {}
        Operand::Value(v) => {
            if header_loads.contains(&resolve_copy(&defs, *v)) {
                // Deferred to the bound-load validation below.
            } else {
                match def_blocks.get(&v.0) {
                    Some(&db) => {
                        if lp.body.contains(&db) || !dom.dominates(db, preheader) {
                            dlog!("{name} loop@{header}: bail (bound not avail in pre)");
                            return None;
                        }
                    }
                    None => {} // function argument: available in the preheader.
                }
            }
        }
    }

    // Coverage: store value resolves (through Copies) exactly to the load.
    let stored = match &store_val {
        Operand::Value(v) => resolve_copy(&defs, *v),
        Operand::Const(_) => {
            dlog!("{name} loop@{header}: bail (store val const)");
            return None;
        }
    };
    if stored != load_dest {
        dlog!("{name} loop@{header}: bail (store val not load)");
        return None;
    }

    // Single-use: the load feeds only the store (through a single-threaded
    // Copy chain); each bump feeds only its phi (through Copies). Any
    // second use is live state the rewrite cannot reproduce.
    let mut check_single_threaded = |mut v: Value, end: Value, what: &str| -> Option<()> {
        for _ in 0..33 {
            if v == end {
                if uses.get(&v.0).copied().unwrap_or(0) != 1 {
                    dlog!("{name} loop@{header}: bail ({what} end uses)");
                    return None;
                }
                return Some(());
            }
            match defs.get(&v.0) {
                Some(Instruction::Copy { dest, src }) => {
                    if uses.get(&dest.0).copied().unwrap_or(0) != 1 {
                        dlog!("{name} loop@{header}: bail ({what} chain uses)");
                        return None;
                    }
                    match src {
                        Operand::Value(inner) => v = *inner,
                        Operand::Const(_) => {
                            dlog!("{name} loop@{header}: bail ({what} chain const)");
                            return None;
                        }
                    }
                }
                _ => {
                    dlog!("{name} loop@{header}: bail ({what} chain form)");
                    return None;
                }
            }
        }
        dlog!("{name} loop@{header}: bail ({what} chain too long)");
        None
    };
    // Store operand -> ... -> load dest (store operand itself is the use).
    match &store_val {
        Operand::Value(v) => check_single_threaded(*v, load_dest, "load")?,
        Operand::Const(_) => return None,
    }
    // IV bump -> phi latch edge.
    let Some(iv_bump) = iv_latch_add else {
        dlog!("{name} loop@{header}: bail (IV phi not found)");
        return None;
    };
    match phis.iter().find(|p| p.dest == iv_phi).map(|p| &p.latch) {
        Some(Operand::Value(v)) => check_single_threaded(*v, iv_bump, "IV bump")?,
        _ => return None,
    }
    for phi in &phis {
        if phi.ty == IrType::Ptr {
            let bump = bump_phis.get(&phi.dest.0)?.1;
            match &phi.latch {
                Operand::Value(v) => check_single_threaded(*v, bump, "ptr bump")?,
                Operand::Const(_) => return None,
            }
        }
    }

    // Overlap: same-root copies bail. Distinct roots (including Param and
    // Other) are allowed — Global/Alloca distinct uses memcpy, Param/Other
    // uses memmove for safety (may alias). This unlocks LZ4 literal copies
    // where anchor/op derive from params through outer phis. For Other
    // roots we must compare the actual init SSA values, not just the enum
    // (Other==Other would otherwise bail distinct pointers).
    let store_root = object_root(&defs, &label_to_idx, preheader_label, store_init);
    let load_root = object_root(&defs, &label_to_idx, preheader_label, load_init);
    // Same SSA init → same object → bail.
    if store_init == load_init {
        dlog!("{name} loop@{header}: bail (same init {store_init:?})");
        return None;
    }
    if store_root == load_root && !matches!(store_root, ObjectRoot::Other) {
        dlog!("{name} loop@{header}: bail (same root {store_root:?})");
        return None;
    }
    // For Other vs Other with different inits, allow with memmove.
    let use_memmove = !matches!(
        (&store_root, &load_root),
        (ObjectRoot::Global(_), ObjectRoot::Alloca(_))
            | (ObjectRoot::Alloca(_), ObjectRoot::Global(_))
            | (ObjectRoot::Global(_), ObjectRoot::Global(_))
            | (ObjectRoot::Alloca(_), ObjectRoot::Alloca(_))
            | (ObjectRoot::Param(_), ObjectRoot::Param(_))
    ) || matches!(store_root, ObjectRoot::Param(_) | ObjectRoot::Other)
        || matches!(load_root, ObjectRoot::Param(_) | ObjectRoot::Other);

    // Bound loads: every header load must be THE bound — resolving
    // through copies to the `Ult` rhs — read through a bare
    // GlobalAddr/Alloca pointer. Bare static addresses are
    // unconditionally dereferenceable (globals always mapped, stack
    // slots always live), so M2 may hoist the load above the loop even
    // when the trip count is 0; and its object differs from both copy
    // roots, so the copy's writes cannot change the loaded bound
    // (loop-invariant). Anything else bails: a stray header load might
    // fault (dropping it would remove a trap) or vary per iteration.
    let bound_val = match &bound {
        Operand::Value(v) => Some(resolve_copy(&defs, *v)),
        Operand::Const(_) => None,
    };
    for hl in &header_loads {
        match bound_val {
            Some(bv) if *hl == bv => {}
            _ => {
                dlog!("{name} loop@{header}: bail (stray header load)");
                return None;
            }
        }
        let ptr = match defs.get(&hl.0) {
            Some(Instruction::Load { ptr, .. }) => *ptr,
            _ => {
                dlog!("{name} loop@{header}: bail (header load form)");
                return None;
            }
        };
        let addr = resolve_copy_cast(&defs, ptr);
        let bound_root = match defs.get(&addr.0) {
            Some(Instruction::GlobalAddr { name, .. }) => ObjectRoot::Global(name.clone()),
            Some(Instruction::Alloca { dest, .. }) => ObjectRoot::Alloca(dest.0),
            _ => {
                dlog!("{name} loop@{header}: bail (bound load not static addr)");
                return None;
            }
        };
        if bound_root == store_root || bound_root == load_root {
            dlog!("{name} loop@{header}: bail (bound load aliases copy)");
            return None;
        }
    }

    // Escape check: scan every outside block. Loop-edge phi incomings must
    // be the IV, a bump phi, or loop-invariant (M2 rewrites the first two
    // to bound/`GEP(init, bound)`, keeps the third). Every other operand
    // defined inside the loop is escaping state -> bail.
    let loop_defined =
        |v: Value| -> bool { matches!(def_blocks.get(&v.0), Some(db) if lp.body.contains(db)) };
    let invariant_outside = |v: Value| -> bool {
        match def_blocks.get(&v.0) {
            None => true, // const-impossible here (Value); args are invariant.
            Some(db) => !lp.body.contains(db),
        }
    };
    for (bi, block) in func.blocks.iter().enumerate() {
        if lp.body.contains(&bi) {
            continue;
        }
        for inst in &block.instructions {
            if let Instruction::Phi { incoming, .. } = inst {
                for (op, label) in incoming {
                    let from = match label_to_idx.get(label) {
                        Some(f) => *f,
                        None => {
                            dlog!("{name} loop@{header}: bail (phi bad label)");
                            return None;
                        }
                    };
                    if lp.body.contains(&from) {
                        match op {
                            Operand::Const(_) => {}
                            Operand::Value(v) => {
                                let r = resolve_copy(&defs, *v);
                                if r == iv_phi
                                    || bump_phis.contains_key(&r.0)
                                    || invariant_outside(r)
                                {
                                    continue;
                                }
                                dlog!("{name} loop@{header}: bail (exit phi val v{})", v.0);
                                return None;
                            }
                        }
                    } else if let Operand::Value(v) = op {
                        if loop_defined(*v) {
                            dlog!("{name} loop@{header}: bail (outside-edge phi esc)");
                            return None;
                        }
                    }
                }
                continue;
            }
            // Direct uses of the IV / bump phis in blocks dominated by
            // the exit are rewritable (M2 substitutes bound /
            // `GEP(init, bound)`): the single-predecessor exit needs no
            // phi, so post-loop uses commonly reference the header phis
            // directly. Domination by the exit proves the loop ran.
            let mut escaped = None;
            let mut direct_ok = true;
            inst.for_each_used_value(|v| {
                if loop_defined(Value(v)) {
                    let r = resolve_copy(&defs, Value(v));
                    // With a const bound the IV rewrites to a const, which
                    // only fits `Operand` slots; a `Value`-slot use would
                    // be unrewritable. That check lives in the M2 pre-scan
                    // (pre-scan 2), which uses the canonical exhaustive
                    // `Value`-slot walker and aborts pre-mutation — the
                    // matcher cannot distinguish slot kinds without a
                    // fragile hand-rolled walk, so it allows the use here.
                    if (r == iv_phi || bump_phis.contains_key(&r.0)) && dom.dominates(exit_idx, bi)
                    {
                        return;
                    }
                    if direct_ok {
                        escaped = Some(v);
                        direct_ok = false;
                    }
                }
            });
            if let Some(vn) = escaped {
                dlog!("{name} loop@{header}: bail (escape v{vn} in block {bi})");
                return None;
            }
        }
        // Terminator operands (Return value): same direct-use rule —
        // the returned IV / bump phi (post-loop value) is rewritable iff
        // the return block is dominated by the exit.
        if let Terminator::Return(Some(op)) = &block.terminator {
            if let Operand::Value(v) = op {
                if loop_defined(*v) {
                    let r = resolve_copy(&defs, *v);
                    if (r == iv_phi || bump_phis.contains_key(&r.0)) && dom.dominates(exit_idx, bi)
                    {
                        continue;
                    }
                    dlog!("{name} loop@{header}: bail (return escapes)");
                    return None;
                }
            }
        }
    }

    Some(CopyLoop {
        header,
        preheader,
        latch,
        exit: exit_idx,
        iv_phi,
        bound,
        ult_ty,
        bound_load: bound_val.filter(|bv| header_loads.contains(bv)),
        store_is_bump,
        store_base,
        store_init,
        load_is_bump,
        load_base,
        load_init,
        use_memmove,
    })
}

// NOTE: definition/operand walking uses the canonical
// `Instruction::dest()` and `Instruction::for_each_used_value()` so the
// matcher stays exhaustive as the IR grows (a hand-rolled walker would
// silently miss new variants and undercount uses -- a miscompile vector).

// ---------------------------------------------------------------------------
// M2: rewrite — replace the matched loop with a preheader `memcpy` call.
// ---------------------------------------------------------------------------

/// Mint a fresh SSA value id.
fn alloc_value(func: &mut IrFunction) -> Value {
    let v = Value(func.next_value_id);
    func.next_value_id += 1;
    v
}

/// Rewrite a matched copy loop into a preheader `memcpy` call.
///
/// Returns `true` iff the loop was rewritten. All abort checks run
/// before the first mutation, so `false` leaves the loop intact:
///
/// - bound wider than `size_t` (defensive; the matcher already bails
///   128-bit IVs, this covers 64-bit IVs on 32-bit targets),
/// - exit phi already holding a preheader-edge incoming (proven
///   impossible by the single-succ preheader check; re-verified),
/// - exit phi with duplicate header-edge incomings (invalid SSA),
/// - const bound with an IV use in a `Value`-typed slot (consts only
///   fit `Operand` slots; the matcher already bails the reachable
///   cases, this closes the proof against future IR growth).
fn rewrite_copy_loop(func: &mut IrFunction, m: &CopyLoop, body: &FxHashSet<usize>) -> bool {
    let fname = func.name.clone();
    let size_ty = crate::common::types::target_int_ir_type();
    if m.ult_ty.size() > size_ty.size() {
        dlog!(
            "{fname} loop@{}: rewrite abort (bound wider than size_t)",
            m.header
        );
        return false;
    }
    if !matches!(size_ty, IrType::I32 | IrType::I64) {
        dlog!("{fname} loop@{}: rewrite abort (size_t form)", m.header);
        return false;
    }
    // A const bound must be an in-range integer const (float consts can
    // appear as `Ult` operands after bit-twiddling folds).
    if let Operand::Const(c) = &m.bound {
        if c.to_i64().is_none() {
            dlog!(
                "{fname} loop@{}: rewrite abort (non-int const bound)",
                m.header
            );
            return false;
        }
    }
    let header_label = func.blocks[m.header].label;
    let preheader_label = func.blocks[m.preheader].label;
    let exit_label = func.blocks[m.exit].label;

    // Pre-scan 1: exit phis must have no preheader-edge incoming yet and
    // at most one header-edge incoming each.
    for inst in &func.blocks[m.exit].instructions {
        if let Instruction::Phi { incoming, .. } = inst {
            let mut header_entries = 0u32;
            for (_, label) in incoming {
                if *label == preheader_label {
                    dlog!(
                        "{fname} loop@{}: rewrite abort (exit phi has pre edge)",
                        m.header
                    );
                    return false;
                }
                if *label == header_label {
                    header_entries += 1;
                }
            }
            if header_entries > 1 {
                dlog!(
                    "{fname} loop@{}: rewrite abort (dup header entries)",
                    m.header
                );
                return false;
            }
        }
    }

    // Pre-scan 2: with a const bound the IV rewrites to a const, which
    // only fits `Operand` slots. Uses the canonical walker so the check
    // stays exhaustive as the IR grows.
    if matches!(&m.bound, Operand::Const(_)) {
        for (bi, block) in func.blocks.iter_mut().enumerate() {
            if body.contains(&bi) {
                continue;
            }
            for inst in &mut block.instructions {
                let mut bad = false;
                inst.for_each_value_use_mut(|v| {
                    if *v == m.iv_phi {
                        bad = true;
                    }
                });
                if bad {
                    dlog!(
                        "{fname} loop@{}: rewrite abort (const IV in Value slot)",
                        m.header
                    );
                    return false;
                }
            }
        }
    }

    // Definition sites + dominance for preheader-availability.
    let mut def_blocks: FxHashMap<u32, usize> = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            if let Some(d) = inst.dest() {
                def_blocks.insert(d.0, bi);
            }
        }
    }
    // NOTE: CfgAnalysis::build dominates here; DominanceChecker needs it.
    let (num_blocks, idom) = (func.blocks.len(), {
        let cfg = CfgAnalysis::build(func);
        cfg.idom
    });
    let dom = DominanceChecker::new(num_blocks, &idom);

    // Classify pointer operands: reusable as-is (outside + dominating
    // the preheader), needing a preheader clone (operand-free pure
    // leaves), or abort (unreachable: the matcher proved every operand
    // is one of the first two).
    enum Mat {
        Reuse(Value),
        CloneLeaf(Instruction),
    }
    let mut classify = |v: Value| -> Option<Mat> {
        let mut reusable = false;
        if let Some(&db) = def_blocks.get(&v.0) {
            if !body.contains(&db) && dom.dominates(db, m.preheader) {
                reusable = true;
            }
        } else {
            // Function argument: available in the preheader. (Pointer
            // operands are never args in a v1 match — param roots bail —
            // but treat them uniformly.)
            reusable = true;
        }
        if reusable {
            return Some(Mat::Reuse(v));
        }
        // In-loop def: must be an operand-free pure leaf (GlobalAddr /
        // ParamRef / LabelAddr); clone it into the preheader.
        let mut found: Option<Instruction> = None;
        for block in &func.blocks {
            for inst in &block.instructions {
                if inst.dest() == Some(v) {
                    match inst {
                        Instruction::GlobalAddr { .. }
                        | Instruction::ParamRef { .. }
                        | Instruction::LabelAddr { .. } => {
                            found = Some(inst.clone());
                        }
                        _ => return None,
                    }
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        found.map(Mat::CloneLeaf)
    };

    let store_ptr_src = if m.store_is_bump {
        m.store_init
    } else {
        m.store_base
    };
    let load_ptr_src = if m.load_is_bump {
        m.load_init
    } else {
        m.load_base
    };
    // Bound-load (pointer, type) + pointer materialization, when the
    // bound is computed by a header load.
    let mut bound_clone: Option<(Value, IrType, Mat)> = None;
    if let Some(hl) = m.bound_load {
        let mut found: Option<(Value, IrType)> = None;
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Instruction::Load { dest, ptr, ty, .. } = inst {
                    if *dest == hl {
                        found = Some((*ptr, *ty));
                        break;
                    }
                }
            }
            if found.is_some() {
                break;
            }
        }
        match found {
            Some((p, ty)) => match classify(p) {
                Some(pmat) => bound_clone = Some((p, ty, pmat)),
                None => {
                    dlog!(
                        "{fname} loop@{}: rewrite abort (bound ptr not materializable)",
                        m.header
                    );
                    return false;
                }
            },
            None => {
                dlog!("{fname} loop@{}: rewrite abort (bound load lost)", m.header);
                return false;
            }
        }
    }

    // Classify everything before mutating.
    let store_mat = match classify(store_ptr_src) {
        Some(x) => x,
        None => {
            dlog!(
                "{fname} loop@{}: rewrite abort (store ptr not materializable)",
                m.header
            );
            return false;
        }
    };
    let load_mat = match classify(load_ptr_src) {
        Some(x) => x,
        None => {
            dlog!(
                "{fname} loop@{}: rewrite abort (load ptr not materializable)",
                m.header
            );
            return false;
        }
    };
    // ---- Mutations begin (all remaining steps are infallible). ----
    let mut clone_of: FxHashMap<u32, Value> = FxHashMap::default();
    let mut do_clone = |orig: Value, template: Instruction, func: &mut IrFunction| -> Value {
        let fresh = alloc_value(&mut *func);
        let mut inst = template;
        match &mut inst {
            Instruction::GlobalAddr { dest, .. }
            | Instruction::ParamRef { dest, .. }
            | Instruction::LabelAddr { dest, .. } => *dest = fresh,
            _ => unreachable!("loop-idiom: clone template is always a leaf"),
        }
        func.blocks[m.preheader].instructions.push(inst);
        fresh
    };
    let mut use_val = |orig: Value,
                       mat: Mat,
                       func: &mut IrFunction,
                       clone_of: &mut FxHashMap<u32, Value>|
     -> Value {
        match mat {
            Mat::Reuse(v) => v,
            Mat::CloneLeaf(t) => {
                let c = do_clone(orig, t, func);
                clone_of.insert(orig.0, c);
                c
            }
        }
    };
    let dst = use_val(store_ptr_src, store_mat, &mut *func, &mut clone_of);
    let src = use_val(load_ptr_src, load_mat, &mut *func, &mut clone_of);

    // Bound as a preheader `Ult`-typed operand.
    let bound_ult: Operand = match &m.bound {
        Operand::Const(c) => Operand::Const(*c),
        Operand::Value(v) => match bound_clone {
            Some((p, ty, pmat)) => {
                let ptr = use_val(p, pmat, &mut *func, &mut clone_of);
                let d = alloc_value(&mut *func);
                func.blocks[m.preheader]
                    .instructions
                    .push(Instruction::Load {
                        dest: d,
                        ptr,
                        ty,
                        seg_override: crate::common::types::AddressSpace::Default,
                        volatile: false,
                    });
                Operand::Value(d)
            }
            None => Operand::Value(*v),
        },
    };

    // Length argument, sized to `size_t`. Const bounds are masked to the
    // `Ult` width (unsigned compare semantics) and zero-extended; value
    // bounds widen with a value-preserving `Cast` (unsigned from-types
    // zero-extend; the `Ult` width was proven to fit `size_t` above).
    let len: Operand = match &bound_ult {
        Operand::Const(c) => {
            // Validated in the pre-scan (in-range int, I32/I64 size_t).
            let raw = c.to_i64().expect("loop-idiom: const bound validated") as u64;
            let width_bytes = m.ult_ty.size();
            let mask: u64 = if width_bytes >= 8 {
                u64::MAX
            } else {
                (1u64 << (width_bytes * 8)) - 1
            };
            let v = raw & mask;
            match size_ty {
                IrType::I64 => Operand::Const(IrConst::I64(v as i64)),
                IrType::I32 => Operand::Const(IrConst::I32(v as i32)),
                _ => unreachable!("loop-idiom: size_t form validated"),
            }
        }
        Operand::Value(v) => {
            if m.ult_ty == size_ty {
                Operand::Value(*v)
            } else {
                let d = alloc_value(&mut *func);
                func.blocks[m.preheader]
                    .instructions
                    .push(Instruction::Cast {
                        dest: d,
                        src: Operand::Value(*v),
                        from_ty: m.ult_ty,
                        to_ty: size_ty,
                    });
                Operand::Value(d)
            }
        }
    };

    // The `memcpy`/`memmove` call. Shape mirrors the frontend's
    // `emit_dynamic_memcpy` exactly (plain libc call, no declaration
    // needed — the default link resolves it). Param-rooted copies use
    // memmove for safety (distinct params may still alias in C).
    let call_name = if m.use_memmove { "memmove" } else { "memcpy" };
    let ret = alloc_value(&mut *func);
    func.blocks[m.preheader]
        .instructions
        .push(Instruction::Call {
            func: call_name.to_string(),
            info: crate::ir::reexports::CallInfo {
                dest: Some(ret),
                args: vec![Operand::Value(dst), Operand::Value(src), len],
                arg_types: vec![IrType::Ptr, IrType::Ptr, size_ty],
                return_type: IrType::Ptr,
                is_variadic: false,
                num_fixed_args: 3,
                struct_arg_sizes: vec![None, None, None],
                struct_arg_aligns: vec![None, None, None],
                struct_arg_classes: vec![Vec::new(), Vec::new(), Vec::new()],
                struct_arg_riscv_float_classes: Vec::new(),
                struct_arg_is_f128_sse: vec![false, false, false],
                is_sret: false,
                is_fastcall: false,
                regparm: None,
                is_pure: false,
                is_const: false,
                ret_eightbyte_classes: Vec::new(),
                ret_is_f128_sse: false,
            },
        });

    // Bump-pointer exit values: `GEP(init, bound)` (byte scale, matching
    // the per-iteration `+1`). Always emitted (dead ones fold away);
    // indexed sides need none (their bases are unchanged).
    let mut bump_final: FxHashMap<u32, Value> = FxHashMap::default();
    if m.store_is_bump {
        let init = clone_of
            .get(&store_ptr_src.0)
            .copied()
            .unwrap_or(store_ptr_src);
        let d = alloc_value(&mut *func);
        func.blocks[m.preheader]
            .instructions
            .push(Instruction::GetElementPtr {
                dest: d,
                base: init,
                offset: bound_ult.clone(),
                ty: IrType::Ptr,
            });
        bump_final.insert(m.store_base.0, d);
    }
    if m.load_is_bump {
        let init = clone_of
            .get(&load_ptr_src.0)
            .copied()
            .unwrap_or(load_ptr_src);
        let d = alloc_value(&mut *func);
        func.blocks[m.preheader]
            .instructions
            .push(Instruction::GetElementPtr {
                dest: d,
                base: init,
                offset: bound_ult.clone(),
                ty: IrType::Ptr,
            });
        bump_final.insert(m.load_base.0, d);
    }

    // Exit-phi surgery: the header-edge incoming becomes a
    // preheader-edge incoming carrying the loop's exit value (IV →
    // bound, bump phi → `GEP(init, bound)`, invariant → unchanged).
    // Only exit-block phis can hold loop-edge incomings: the sole
    // loop→outside edge is header→exit (proven in the matcher).
    // Compute first (immutable), apply after (borrow discipline).
    let mut phi_edits: Vec<(usize, usize, Operand)> = Vec::new();
    {
        let mut defs: FxHashMap<u32, &Instruction> = FxHashMap::default();
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Some(d) = inst.dest() {
                    defs.insert(d.0, inst);
                }
            }
        }
        for (pos, inst) in func.blocks[m.exit].instructions.iter().enumerate() {
            if let Instruction::Phi { incoming, .. } = inst {
                for (ipos, (op, label)) in incoming.iter().enumerate() {
                    if *label == header_label {
                        let new_op = match op {
                            Operand::Const(_) => op.clone(),
                            Operand::Value(v) => {
                                let r = resolve_copy(&defs, *v);
                                if r == m.iv_phi {
                                    bound_ult.clone()
                                } else if let Some(g) = bump_final.get(&r.0) {
                                    Operand::Value(*g)
                                } else {
                                    op.clone()
                                }
                            }
                        };
                        phi_edits.push((pos, ipos, new_op));
                    }
                }
            }
        }
    }
    for (pos, ipos, new_op) in phi_edits {
        if let Instruction::Phi { incoming, .. } = &mut func.blocks[m.exit].instructions[pos] {
            incoming[ipos] = (new_op, preheader_label);
        }
    }

    // Rewrite direct outside uses (non-phi instructions + terminators).
    // Phis were handled surgically above; inside-loop uses die with it.
    let mut repl: FxHashMap<u32, Operand> = FxHashMap::default();
    repl.insert(m.iv_phi.0, bound_ult);
    for (phi, g) in &bump_final {
        repl.insert(*phi, Operand::Value(*g));
    }
    for (bi, block) in func.blocks.iter_mut().enumerate() {
        if body.contains(&bi) {
            continue;
        }
        for inst in &mut block.instructions {
            if matches!(inst, Instruction::Phi { .. }) {
                continue;
            }
            inst.for_each_operand_mut(|op| {
                if let Operand::Value(v) = op {
                    if let Some(new) = repl.get(&v.0) {
                        *op = new.clone();
                    }
                }
            });
            // `Value`-typed slots only accept values; a const IV
            // replacement here is unreachable (pre-scan 2 proved none).
            inst.for_each_value_use_mut(|v| {
                if let Some(Operand::Value(nv)) = repl.get(&v.0) {
                    *v = *nv;
                }
            });
        }
        match &mut block.terminator {
            Terminator::CondBranch { cond, .. } => {
                if let Operand::Value(v) = cond {
                    if let Some(new) = repl.get(&v.0) {
                        *cond = new.clone();
                    }
                }
            }
            Terminator::Return(Some(op)) => {
                if let Operand::Value(v) = op {
                    if let Some(new) = repl.get(&v.0) {
                        *op = new.clone();
                    }
                }
            }
            _ => {}
        }
    }

    // Retarget the preheader at the exit and drop the dead loop.
    func.blocks[m.preheader].terminator = Terminator::Branch(exit_label);
    let _ = crate::passes::cfg_simplify::eliminate_unreachable_blocks(func);
    dlog!(
        "{fname} loop@{}: REWROTE to {}",
        m.header,
        if m.use_memmove { "memmove" } else { "memcpy" }
    );
    true
}
