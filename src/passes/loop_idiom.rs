//! Loop-idiom recognition: proven-disjoint byte copies call `memcpy`; other
//! copy loops retain a scalar fallback behind a safe fast path.
//!
//! Real-world C copies bytes in loops everywhere (compressors, codecs, string
//! routines, network buffers, kernels). A byte-at-a-time loop costs ~7-9
//! instructions per byte; glibc's copy routines are highly tuned. The pass
//! recognizes small counted byte-copy loops (indexed and pointer-bump forms,
//! including LZ4 literals) before vectorization at -O2 and above.
//!
//! Legality is deliberately structural: a single-entry/single-exit natural
//! loop with a zero-based unsigned IV, a loop-invariant count, exactly one
//! nonvolatile byte load feeding exactly one nonvolatile byte store, and no
//! trapping or escaping instructions apart from reconstructible IV/pointer
//! live-outs. Everything else keeps its original loop. See the matcher below
//! for the full list of fail-closed checks.
//!
//! A forward scalar loop has *smear* semantics for `src < dst < src+n`:
//! `memmove` is NOT an unconditional replacement. Only known-disjoint roots
//! use `memcpy`. Uncertain pointers use unsigned address subtraction to
//! select a `memmove` fast path for safe addresses, keeping the original
//! scalar loop for forward overlap and zero length. Roots are disjoint only
//! when justified by fresh allocations, `restrict` where applicable, or
//! distinct strong private globals that do not name a linker alias. Extern,
//! common, weak and arbitrary-asm names cannot prove global disjointness.
//! A disjoint copy with a nullable pointer parameter guards `memcpy` on
//! `n > 0` so a zero-trip source loop never calls libc with null pointers.
//!
//! Guarded rewriting versions the preheader, extends existing exit phis,
//! creates SSA merge phis for live-out IV/bump pointers and retains the
//! original scalar loop. A per-pass header set prevents repeatedly versioning
//! the same fallback. Bound loads that can alias either copy operand cannot
//! be hoisted. Unsupported CFG/phi/bound forms are rejected before mutation.
//!
//! Default-on at -O2+ (also -Os/-Oz); `CCC_NO_LOOP_IDIOM=1` disables the pass.
//! `CCC_LOOP_IDIOM=1` is a legacy accepted no-op; `CCC_DEBUG_LOOP_IDIOM=1`
//! logs matches, rewrites, and bail reasons. No byte-compare rewrite is
//! implemented: widening comparison loads without a range proof can overread.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::analysis::{self, CfgAnalysis};
use crate::ir::reexports::{
    BlockId, Instruction, IrBinOp, IrCmpOp, IrConst, IrFunction, Operand, Terminator, Value,
};
use crate::passes::loop_analysis::{DominanceChecker, find_natural_loops, merge_loops_by_header};

/// Per-function entry point for the dirty-tracking pipeline. Two globals
/// prove disjoint only when *both* are strong private definitions and neither
/// is an alias; extern declarations may resolve to the same storage in a
/// different translation unit. Module-level symbol facts are built once.
pub(crate) fn run_function(
    func: &mut IrFunction,
    aliased_names: &FxHashSet<String>,
    local_globals: &FxHashSet<String>,
) -> usize {
    recognize_idioms(func, aliased_names, local_globals)
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
/// `Alloca(id)` for static stack slots, `Param(idx)` for POINTER-typed
/// function parameters (an integer parameter converted to a pointer names
/// no object — the frontend erases the conversion — so it is `Other`;
/// different `Param` roots need a `restrict` proof to call `memcpy`),
/// `Other` for everything else (dynamic allocas, computed
/// pointers, unknowns).
///
/// Leaf typing and chain following come from `crate::ir::provenance` (shared
/// with the vectorizer's tracer so the two cannot drift): the walk sees
/// through `Phi` (preheader-edge init only — the latch edge marches within
/// the same object), `Copy`, `Ptr -> Ptr` `Cast`, `GEP` (base), and
/// `Add`/`Sub` (the side that resolves; an integer offset contributes no
/// root, a known-pointer offset rejects the root, and two rooted sides is
/// nonsense → `Other`). Offsets are deliberately ignored: cross-object
/// disjointness needs roots only, and same-object copies bail regardless
/// of offsets (v1).
#[derive(Clone, PartialEq, Eq, Debug)]
enum ObjectRoot {
    Global(String),
    Alloca(u32),
    Param(u32),
    Other,
}

/// Proof for a call to `memcpy`, not merely a guess that two pointer values
/// have different SSA IDs. In particular, distinct global *names* are not
/// distinct objects when either is a linker alias. A pointer from a parameter
/// cannot name a fresh alloca created during the current activation. Two
/// different pointer parameters require the frontend's restrict contract;
/// without it the original forward loop can smear overlapped bytes.
fn roots_proven_disjoint(
    func: &IrFunction,
    a: &ObjectRoot,
    b: &ObjectRoot,
    aliased_names: &FxHashSet<String>,
    local_globals: &FxHashSet<String>,
) -> bool {
    match (a, b) {
        (ObjectRoot::Global(a), ObjectRoot::Global(b)) => {
            a != b
                && local_globals.contains(a)
                && local_globals.contains(b)
                && !aliased_names.contains(a)
                && !aliased_names.contains(b)
        }
        (ObjectRoot::Alloca(a), ObjectRoot::Alloca(b)) => a != b,
        (ObjectRoot::Global(_), ObjectRoot::Alloca(_))
        | (ObjectRoot::Alloca(_), ObjectRoot::Global(_))
        | (ObjectRoot::Param(_), ObjectRoot::Alloca(_))
        | (ObjectRoot::Alloca(_), ObjectRoot::Param(_)) => true,
        (ObjectRoot::Param(a), ObjectRoot::Param(b)) if a != b => {
            func.params.get(*a as usize).is_some_and(|p| p.noalias)
                || func.params.get(*b as usize).is_some_and(|p| p.noalias)
        }
        _ => false,
    }
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
    match crate::ir::provenance::root_leaf(inst) {
        crate::ir::provenance::RootLeaf::Global(name) => {
            return ObjectRoot::Global(name);
        }
        crate::ir::provenance::RootLeaf::Alloca(id) => {
            return ObjectRoot::Alloca(id);
        }
        crate::ir::provenance::RootLeaf::Param(idx) => {
            return ObjectRoot::Param(idx as u32);
        }
        crate::ir::provenance::RootLeaf::NotLeaf => {}
    }
    if let crate::ir::provenance::ChainStep::Follow(inner) = crate::ir::provenance::chain_step(inst)
    {
        return object_root_inner(defs, label_to_idx, preheader_label, inner, depth + 1);
    }
    match inst {
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
            op: op @ (IrBinOp::Add | IrBinOp::Sub),
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
            // An unrooted side that is a KNOWN pointer (loaded pointer,
            // call result, outer phi of pointer type, `Ptr`-typed cast of
            // an integer) rejects the root: adding two addresses is
            // meaningless. Immediate definitions only — looking through
            // `Copy` would misclassify the valid `p + (int)q` idiom (whose
            // `ptrtoint` is an erased `Copy` of a pointer-typed value).
            // Any other unrooted side is an integer offset or UB in valid
            // IR (C17 6.5.6 admits only `ptr ± int` / `int + ptr`), so it
            // cannot disturb the rooted side's object identity. See
            // `crate::ir::provenance`.
            let side_known_pointer = |side: &Operand| -> bool {
                match side {
                    Operand::Const(_) => false,
                    Operand::Value(v) => defs
                        .get(&v.0)
                        .is_some_and(|d| crate::ir::provenance::produces_pointer(d)),
                }
            };
            match (l, r) {
                (ObjectRoot::Other, ObjectRoot::Other) => ObjectRoot::Other,
                // `ptr +/- int` keeps the LHS root: it stays in/near the
                // object and going out-of-bounds is UB, exactly like GEP.
                (root, ObjectRoot::Other) if !side_known_pointer(rhs) => root,
                // `int + ptr` is commutative with the above.  But `int -
                // ptr` (a rooted RHS under `Sub`) fails closed: not
                // valid pointer arithmetic (C17 6.5.6), it can only arise
                // from integer laundering (`(T*)(c - (intptr_t)p)` cast
                // back to a pointer), and an int-derived address may
                // alias anything.
                (ObjectRoot::Other, root)
                    if matches!(op, IrBinOp::Add) && !side_known_pointer(lhs) =>
                {
                    root
                }
                // Two rooted sides (ptr + ptr, ptr - ptr used as an
                // address) is nonsense; fail closed.
                _ => ObjectRoot::Other,
            }
        }
        _ => {
            let _ = label_to_idx;
            ObjectRoot::Other
        }
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
    /// Preserve the scalar loop for forward overlap (memmove fast path).
    needs_overlap_guard: bool,
    /// For a provably disjoint copy with a pointer parameter, the parameter
    /// may be null on a zero trip. Calling even memcpy(_, null, 0) is not
    /// universally valid, while the source loop never dereferences it.
    needs_zero_guard: bool,
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

pub(crate) fn recognize_idioms(
    func: &mut IrFunction,
    aliased_names: &FxHashSet<String>,
    local_globals: &FxHashSet<String>,
) -> usize {
    if std::env::var("CCC_NO_LOOP_IDIOM").is_ok() {
        return 0;
    }
    // Default-on. For maybe-overlapping pointers, a guarded fast path keeps
    // the original scalar loop as the forward-overlap fallback.
    // The old CCC_LOOP_IDIOM=1 opt-in knob is retained as a no-op for
    // compatibility but no longer gates the pass.
    if func.blocks.len() < 3 {
        return 0;
    }
    // M2: match + rewrite fixpoint. One rewrite per rescan (block
    // indices shift when the dead loop is removed), innermost-first,
    // bounded (same discipline as loop_rotate).
    let mut total = 0;
    // A guarded rewrite retains its scalar loop. Do not version it again on
    // the next fixpoint scan (nor let another matched loop suppress this one).
    let mut versioned_headers: FxHashSet<BlockId> = FxHashSet::default();
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
            let header_label = func.blocks[lp.header].label;
            if versioned_headers.contains(&header_label) {
                continue;
            }
            if let Some(matched) = try_match_copy_loop(func, lp, &cfg, aliased_names, local_globals)
            {
                dlog!(
                    "{} loop@{}: MATCH copy (header={} pre={} latch={} exit={} bound={:?} store_bump={} load_bump={} overlap_guard={} zero_guard={})",
                    func.name,
                    lp.header,
                    matched.header,
                    matched.preheader,
                    matched.latch,
                    matched.exit,
                    matched.bound,
                    matched.store_is_bump,
                    matched.load_is_bump,
                    matched.needs_overlap_guard,
                    matched.needs_zero_guard,
                );
                let rewritten = if matched.needs_overlap_guard || matched.needs_zero_guard {
                    rewrite_guarded_copy_loop(func, &matched, &lp.body, &cfg)
                } else {
                    rewrite_copy_loop(func, &matched, &lp.body)
                };
                if rewritten {
                    if matched.needs_overlap_guard || matched.needs_zero_guard {
                        versioned_headers.insert(header_label);
                    }
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
    aliased_names: &FxHashSet<String>,
    local_globals: &FxHashSet<String>,
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
                    Instruction::BinOp { op, .. }
                        if matches!(
                            op,
                            IrBinOp::SDiv | IrBinOp::UDiv | IrBinOp::SRem | IrBinOp::URem
                        ) =>
                    {
                        dlog!("{name} loop@{header}: bail (trapping div/rem)");
                        return None;
                    }
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
            Instruction::BinOp { op, .. }
                if matches!(
                    op,
                    IrBinOp::SDiv | IrBinOp::UDiv | IrBinOp::SRem | IrBinOp::URem
                ) =>
            {
                dlog!("{name} loop@{header}: bail (trapping div/rem)");
                return None;
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

    // A forward loop and memmove differ for dst in (src, src + n): the
    // loop repeatedly reads bytes it has already overwritten. Even two
    // distinct parameter names, `Other` roots, or ELF names aliasing the
    // same object can have that relationship. Do not turn them into an
    // unconditional memmove; preserve the scalar loop behind a runtime
    // non-smear guard. Proven-disjoint roots still use plain memcpy.
    let store_root = object_root(&defs, &label_to_idx, preheader_label, store_init);
    let load_root = object_root(&defs, &label_to_idx, preheader_label, load_init);
    if store_init == load_init {
        dlog!("{name} loop@{header}: bail (same init {store_init:?})");
        return None;
    }
    if store_root == load_root && !matches!(store_root, ObjectRoot::Other) {
        dlog!("{name} loop@{header}: bail (same root {store_root:?})");
        return None;
    }
    let needs_overlap_guard =
        !roots_proven_disjoint(func, &store_root, &load_root, aliased_names, local_globals);
    let needs_zero_guard = !needs_overlap_guard
        && (matches!(store_root, ObjectRoot::Param(_))
            || matches!(load_root, ObjectRoot::Param(_)));

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
        // Hoisting a header load is valid only when its address is provably
        // independent of BOTH copy operands, not just a different SSA/name:
        // a parameter may point into the bound global; two global names can
        // be linker aliases. Failing this test retains the original loop.
        if !roots_proven_disjoint(func, &bound_root, &store_root, aliased_names, local_globals)
            || !roots_proven_disjoint(func, &bound_root, &load_root, aliased_names, local_globals)
        {
            dlog!("{name} loop@{header}: bail (bound load may alias copy)");
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
                    // With a const bound the IV rewrites to a const, which
                    // only fits `Operand` slots; a `Value`-slot use would
                    // be unrewritable. That check lives in the M2 pre-scan
                    // (pre-scan 2), which uses the canonical exhaustive
                    // `Value`-slot walker and aborts pre-mutation — the
                    // matcher cannot distinguish slot kinds without a
                    // fragile hand-rolled walk, so it allows the use here.
                    // A Copy defined inside the deleted loop does not
                    // dominate a new fast exit merely because it *resolves*
                    // to the IV. Only the phi itself can be reconstructed.
                    if (Value(v) == iv_phi || bump_phis.contains_key(&v))
                        && dom.dominates(exit_idx, bi)
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
        // Canonical walker includes Return, CondBranch, Switch and indirect
        // branches. The old Return-only check missed a loop value escaping
        // through a different terminator and deleting its definition.
        let mut term_escape = None;
        block.terminator.for_each_used_value(|id| {
            let v = Value(id);
            if loop_defined(v)
                && !((v == iv_phi || bump_phis.contains_key(&id)) && dom.dominates(exit_idx, bi))
            {
                term_escape = Some(id);
            }
        });
        if let Some(id) = term_escape {
            dlog!("{name} loop@{header}: bail (terminator escapes v{id})");
            return None;
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
        needs_overlap_guard,
        needs_zero_guard,
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

/// Both the statically-disjoint rewrite and the guarded fast edge use the
/// same ABI/side-effect description for the libc copy call.
fn make_copy_call(
    func: &mut IrFunction,
    name: &str,
    dst: Value,
    src: Value,
    len: Operand,
    size_ty: IrType,
) -> Instruction {
    Instruction::Call {
        func: name.to_string(),
        info: crate::ir::reexports::CallInfo {
            dest: Some(alloc_value(func)),
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
    }
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
            // Function argument: available in the preheader. An incoming
            // argument can be reused or cloned as a ParamRef leaf.
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
            // Fail-closed: if the const is not an int (e.g. float that
            // slipped through), abort this idiom rewrite rather than panic.
            let Some(raw_i) = c.to_i64() else {
                dlog!(
                    "{fname} loop@{}: rewrite abort (const bound not int)",
                    m.header
                );
                return false;
            };
            let raw = raw_i as u64;
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

    // Only proven-disjoint roots reach this rewrite. A forward-overlap
    // scalar loop cannot be replaced by either memcpy or memmove.
    let call_name = "memcpy";
    let call = make_copy_call(func, call_name, dst, src, len, size_ty);
    func.blocks[m.preheader].instructions.push(call);

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
        block.terminator.for_each_operand_mut(|op| {
            if let Operand::Value(v) = op {
                if let Some(new) = repl.get(&v.0) {
                    *op = new.clone();
                }
            }
        });
    }

    // Retarget the preheader at the exit and drop the dead loop.
    func.blocks[m.preheader].terminator = Terminator::Branch(exit_label);
    let _ = crate::passes::cfg_simplify::eliminate_unreachable_blocks(func);
    dlog!("{fname} loop@{}: REWROTE to memcpy", m.header);
    true
}

/// Version a byte-copy loop when the original loop must remain available:
/// forward overlap (src < dst < src+n) needs the scalar smear semantics; a
/// provably disjoint copy with a nullable parameter must skip the libc call
/// on zero trips, where the source loop never dereferences that parameter.
///
/// In unsigned address arithmetic, `delta = dst - src`. If `dst > src`,
/// `delta >= n` proves non-overlap. If `dst < src`, the wrapped delta may
/// spuriously send us to the scalar loop (safe); otherwise a forward copy is
/// equivalent to memmove. For `n == 0`, `n - 1 == UINTPTR_MAX`, so the guard
/// is false and even null pointers take the zero-trip scalar path without a
/// library call. Unsigned subtraction avoids C pointer-order/subtraction UB.
///
/// The single-header-predecessor exit and preheader-dominating operands are
/// intentional fail-closed restrictions. We add a fast->exit edge, extend
/// existing exit phis with the fast values, and create merge phis for any
/// direct uses of the loop's IV or bump pointers after the exit. The slow
/// loop/its backedge and their incoming values are left unmodified.
fn rewrite_guarded_copy_loop(
    func: &mut IrFunction,
    m: &CopyLoop,
    body: &FxHashSet<usize>,
    cfg: &CfgAnalysis,
) -> bool {
    let size_ty = crate::common::types::target_int_ir_type();
    let guard_ty = match size_ty {
        IrType::I32 => IrType::U32,
        IrType::I64 => IrType::U64,
        _ => return false,
    };
    if !matches!(m.ult_ty, IrType::U32 | IrType::U64)
        || m.ult_ty.size() > size_ty.size()
        || m.bound_load.is_some()
        || cfg.preds.row(m.exit) != [m.header as u32]
    {
        dlog!(
            "{} loop@{}: guard declines (bounds/exit)",
            func.name,
            m.header
        );
        return false;
    }
    let header_label = func.blocks[m.header].label;
    let exit_label = func.blocks[m.exit].label;
    let dst = if m.store_is_bump {
        m.store_init
    } else {
        m.store_base
    };
    let src = if m.load_is_bump {
        m.load_init
    } else {
        m.load_base
    };
    let dom = DominanceChecker::new(func.blocks.len(), &cfg.idom);
    let mut defs: FxHashMap<u32, &Instruction> = FxHashMap::default();
    let mut def_blocks: FxHashMap<u32, usize> = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            if let Some(v) = inst.dest() {
                defs.insert(v.0, inst);
                def_blocks.insert(v.0, bi);
            }
        }
    }
    let available = |v: Value| match def_blocks.get(&v.0) {
        Some(&bi) => !body.contains(&bi) && dom.dominates(bi, m.preheader),
        None => true, // an incoming parameter, available in every block
    };
    if !available(dst) || !available(src) || matches!(&m.bound, Operand::Value(v) if !available(*v))
    {
        dlog!(
            "{} loop@{}: guard declines (operand not in preheader)",
            func.name,
            m.header
        );
        return false;
    }
    // The guarded fast path can merge only the IV and the two copy-pointer
    // phis. Decline an additional loop-carried value rather than fabricate
    // its final value on the fast edge.
    for inst in &func.blocks[m.header].instructions {
        if let Instruction::Phi { dest, ty, .. } = inst {
            if *dest == m.iv_phi {
                if *ty != m.ult_ty {
                    return false;
                }
            } else if !(m.store_is_bump && *dest == m.store_base
                || m.load_is_bump && *dest == m.load_base)
                || *ty != IrType::Ptr
            {
                return false;
            }
        }
    }

    // Validate every old exit phi before ANY mutation. Its header-edge input
    // may be an IV, a bump pointer, a constant or a preheader-invariant.
    // Copy chains are resolved only on this incoming edge; they are not
    // globally treated as available on the new fast edge.
    enum FastIncoming {
        Iv,
        StoreBump,
        LoadBump,
        Invariant(Operand),
    }
    let mut exit_incomings = Vec::new();
    for (pos, inst) in func.blocks[m.exit].instructions.iter().enumerate() {
        if let Instruction::Phi { incoming, .. } = inst {
            let mut header_inputs = incoming.iter().filter(|(_, b)| *b == header_label);
            let Some((op, _)) = header_inputs.next() else {
                return false;
            };
            if header_inputs.next().is_some() || incoming.len() != 1 {
                return false;
            }
            let fast = match op {
                Operand::Value(v) => {
                    let resolved = resolve_copy(&defs, *v);
                    if resolved == m.iv_phi {
                        FastIncoming::Iv
                    } else if m.store_is_bump && resolved == m.store_base {
                        FastIncoming::StoreBump
                    } else if m.load_is_bump && resolved == m.load_base {
                        FastIncoming::LoadBump
                    } else if available(*v) {
                        // Not merely outside the loop: it must dominate the
                        // new fast edge, which bypasses the loop header.
                        FastIncoming::Invariant(op.clone())
                    } else {
                        return false;
                    }
                }
                Operand::Const(_) => FastIncoming::Invariant(op.clone()),
            };
            exit_incomings.push((pos, fast));
        }
    }
    // The original exit is dominated by the loop header. The new edge must
    // not bypass a loop-defined value other than the phis merged below;
    // the matcher already checks non-phi escape via every instruction.
    let Some(max_label) = func.blocks.iter().map(|b| b.label.0).max() else {
        return false;
    };
    let Some(after_max) = max_label.checked_add(1) else {
        return false;
    };
    let fast_id = func.next_label.max(after_max);
    let Some(next_label) = fast_id.checked_add(1) else {
        return false;
    };
    // Constants must be representable and carry only the Ult bit width.
    let const_bound = match &m.bound {
        Operand::Const(c) => {
            let Some(raw) = c.to_i64() else { return false };
            Some(if m.ult_ty == IrType::U32 {
                raw as u32 as u64
            } else {
                raw as u64
            })
        }
        Operand::Value(_) => None,
    };

    // ---- All bailouts above this line: mutation starts here. ----
    func.next_label = next_label;
    let fast_label = BlockId(fast_id);
    let mut pre_insts = Vec::new();
    let len = if let Some(n) = const_bound {
        match size_ty {
            IrType::I32 => Operand::Const(IrConst::I32(n as i32)),
            IrType::I64 => Operand::Const(IrConst::I64(n as i64)),
            _ => unreachable!(),
        }
    } else if let Operand::Value(v) = m.bound {
        let len_val = alloc_value(func);
        pre_insts.push(Instruction::Cast {
            dest: len_val,
            src: Operand::Value(v),
            from_ty: m.ult_ty,
            to_ty: size_ty,
        });
        Operand::Value(len_val)
    } else {
        unreachable!("guarded copy bound was checked above")
    };
    let len_unsigned = alloc_value(func);
    pre_insts.push(Instruction::Cast {
        dest: len_unsigned,
        src: len.clone(),
        from_ty: size_ty,
        to_ty: guard_ty,
    });
    let use_fast = if m.needs_overlap_guard {
        let dst_int = alloc_value(func);
        pre_insts.push(Instruction::Cast {
            dest: dst_int,
            src: Operand::Value(dst),
            from_ty: IrType::Ptr,
            to_ty: guard_ty,
        });
        let src_int = alloc_value(func);
        pre_insts.push(Instruction::Cast {
            dest: src_int,
            src: Operand::Value(src),
            from_ty: IrType::Ptr,
            to_ty: guard_ty,
        });
        let distance = alloc_value(func);
        pre_insts.push(Instruction::BinOp {
            dest: distance,
            op: IrBinOp::Sub,
            lhs: Operand::Value(dst_int),
            rhs: Operand::Value(src_int),
            ty: guard_ty,
        });
        let len_minus_one = alloc_value(func);
        pre_insts.push(Instruction::BinOp {
            dest: len_minus_one,
            op: IrBinOp::Sub,
            lhs: Operand::Value(len_unsigned),
            rhs: Operand::Const(match guard_ty {
                IrType::U32 => IrConst::I32(1),
                IrType::U64 => IrConst::I64(1),
                _ => unreachable!(),
            }),
            ty: guard_ty,
        });
        let test = alloc_value(func);
        pre_insts.push(Instruction::Cmp {
            dest: test,
            op: IrCmpOp::Ugt,
            lhs: Operand::Value(distance),
            rhs: Operand::Value(len_minus_one),
            ty: guard_ty,
        });
        test
    } else {
        // When roots are disjoint but one pointer is nullable, calling
        // memcpy(_, null, 0) would add UB to a skipped C loop. Zero trips
        // take the original (zero-iteration) edge; nonzero trips are proven
        // non-null by the source program's dereferences.
        let test = alloc_value(func);
        pre_insts.push(Instruction::Cmp {
            dest: test,
            op: IrCmpOp::Ugt,
            lhs: Operand::Value(len_unsigned),
            rhs: Operand::Const(match guard_ty {
                IrType::U32 => IrConst::I32(0),
                IrType::U64 => IrConst::I64(0),
                _ => unreachable!(),
            }),
            ty: guard_ty,
        });
        test
    };
    {
        let pre = &mut func.blocks[m.preheader];
        if !pre.source_spans.is_empty() {
            if pre.source_spans.len() == pre.instructions.len() {
                pre.source_spans
                    .extend((0..pre_insts.len()).map(|_| crate::common::source::Span::dummy()));
            } else {
                pre.source_spans.clear();
            }
        }
        pre.instructions.extend(pre_insts);
        pre.terminator = Terminator::CondBranch {
            cond: Operand::Value(use_fast),
            true_label: fast_label,
            false_label: header_label,
        };
    }

    let mut fast = crate::ir::reexports::BasicBlock {
        label: fast_label,
        instructions: Vec::new(),
        source_spans: Vec::new(),
        terminator: Terminator::Branch(exit_label),
    };
    fast.instructions.push(make_copy_call(
        func,
        if m.needs_overlap_guard {
            "memmove"
        } else {
            "memcpy"
        },
        dst,
        src,
        len,
        size_ty,
    ));
    let mut bump_final: FxHashMap<u32, Value> = FxHashMap::default();
    if m.store_is_bump {
        let v = alloc_value(func);
        fast.instructions.push(Instruction::GetElementPtr {
            dest: v,
            base: dst,
            offset: m.bound.clone(),
            ty: IrType::Ptr,
        });
        bump_final.insert(m.store_base.0, v);
    }
    if m.load_is_bump {
        let v = alloc_value(func);
        fast.instructions.push(Instruction::GetElementPtr {
            dest: v,
            base: src,
            offset: m.bound.clone(),
            ty: IrType::Ptr,
        });
        bump_final.insert(m.load_base.0, v);
    }

    // Existing exit phis receive a fast-edge input; slow-edge inputs remain
    // unchanged. Then merge the loop-header values used directly after the
    // exit. All resulting phis have exactly the two real predecessors.
    for (pos, value) in exit_incomings {
        let fast_op = match value {
            FastIncoming::Iv => m.bound.clone(),
            FastIncoming::StoreBump => Operand::Value(bump_final[&m.store_base.0]),
            FastIncoming::LoadBump => Operand::Value(bump_final[&m.load_base.0]),
            FastIncoming::Invariant(op) => op,
        };
        if let Instruction::Phi { incoming, .. } = &mut func.blocks[m.exit].instructions[pos] {
            incoming.push((fast_op, fast_label));
        }
    }
    let mut replacements: FxHashMap<u32, Value> = FxHashMap::default();
    let mut merge_phis = Vec::new();
    let mut add_merge = |old: Value, ty: IrType, fast_op: Operand, func: &mut IrFunction| {
        let dest = alloc_value(func);
        replacements.insert(old.0, dest);
        merge_phis.push(Instruction::Phi {
            dest,
            ty,
            incoming: vec![(Operand::Value(old), header_label), (fast_op, fast_label)],
        });
    };
    add_merge(m.iv_phi, m.ult_ty, m.bound.clone(), func);
    if m.store_is_bump {
        add_merge(
            m.store_base,
            IrType::Ptr,
            Operand::Value(bump_final[&m.store_base.0]),
            func,
        );
    }
    if m.load_is_bump && (!m.store_is_bump || m.load_base != m.store_base) {
        add_merge(
            m.load_base,
            IrType::Ptr,
            Operand::Value(bump_final[&m.load_base.0]),
            func,
        );
    }
    {
        let exit = &mut func.blocks[m.exit];
        if !exit.source_spans.is_empty() {
            if exit.source_spans.len() == exit.instructions.len() {
                exit.source_spans.splice(
                    0..0,
                    (0..merge_phis.len()).map(|_| crate::common::source::Span::dummy()),
                );
            } else {
                exit.source_spans.clear();
            }
        }
        exit.instructions.splice(0..0, merge_phis);
    }
    // No loop-defined Copy chain is permitted to escape; only the actual
    // header phis are renamed. Exit phis retain their header-edge operands.
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
                    if let Some(&new) = replacements.get(&v.0) {
                        *v = new;
                    }
                }
            });
            inst.for_each_value_use_mut(|v| {
                if let Some(&new) = replacements.get(&v.0) {
                    *v = new;
                }
            });
        }
        block.terminator.for_each_operand_mut(|op| {
            if let Operand::Value(v) = op {
                if let Some(&new) = replacements.get(&v.0) {
                    *v = new;
                }
            }
        });
    }
    func.blocks.push(fast);
    dlog!(
        "{} loop@{}: REWROTE with {} guard and scalar fallback",
        func.name,
        m.header,
        if m.needs_overlap_guard {
            "non-smear"
        } else {
            "nonzero"
        }
    );
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::AddressSpace;
    use crate::ir::reexports::{BasicBlock, IrParam};

    fn run_function(func: &mut IrFunction) -> usize {
        // Synthetic D/S GlobalAddr instructions represent private, strong
        // definitions in these tests; G is intentionally unproven.
        let local_globals = ["D".to_string(), "S".to_string()].into_iter().collect();
        super::run_function(func, &FxHashSet::default(), &local_globals)
    }

    fn val(id: u32) -> Operand {
        Operand::Value(Value(id))
    }
    fn i32c(v: i32) -> Operand {
        Operand::Const(IrConst::I32(v))
    }
    fn block(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions,
            terminator,
            source_spans: Vec::new(),
        }
    }
    fn ptr_param() -> IrParam {
        IrParam {
            ty: IrType::Ptr,
            noalias: false,
            struct_size: None,
            struct_align: None,
            param_align: None,
            struct_eightbyte_classes: Vec::new(),
            is_f128_sse: false,
            riscv_float_class: None,
        }
    }
    fn load(dest: u32, ptr: u32, ty: IrType) -> Instruction {
        Instruction::Load {
            dest: Value(dest),
            ptr: Value(ptr),
            ty,
            seg_override: AddressSpace::Default,
            volatile: false,
        }
    }
    fn store(v: Operand, ptr: u32, ty: IrType) -> Instruction {
        Instruction::Store {
            val: v,
            ptr: Value(ptr),
            ty,
            seg_override: AddressSpace::Default,
            volatile: false,
        }
    }
    fn has_call(f: &IrFunction, name: &str) -> bool {
        f.blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .any(|i| matches!(i, Instruction::Call { func, .. } if func == name))
    }

    fn assert_guarded_scalar_fallback(f: &IrFunction) {
        assert!(
            f.blocks.iter().any(|b| b.label == BlockId(1)),
            "scalar loop retained"
        );
        assert!(
            matches!(
                f.blocks[0].terminator,
                Terminator::CondBranch {
                    false_label: BlockId(1),
                    ..
                }
            ),
            "guard must route overlap to the original loop"
        );
    }

    /// Single-block indexed copy over two globals:
    ///   P -> H (self) -> E   `for (i = 0; i < 16; i++) D[i] = S[i];`
    /// Values: 1 = D base, 2 = S base, 10 = iv phi, 11 = Ult,
    /// 12 = dst GEP, 13 = iv bump, 14 = src GEP, 15 = loaded byte.
    fn self_loop_copy_func(elem_ty: IrType) -> IrFunction {
        let mut f = IrFunction::new("self_loop_copy".into(), IrType::I32, vec![], false);
        f.next_value_id = 30;
        f.next_label = 3;
        f.blocks.push(block(
            0, // P
            vec![
                Instruction::GlobalAddr {
                    dest: Value(1),
                    name: "D".into(),
                },
                Instruction::GlobalAddr {
                    dest: Value(2),
                    name: "S".into(),
                },
            ],
            Terminator::Branch(BlockId(1)),
        ));
        f.blocks.push(block(
            1, // H (self-loop: header == latch)
            vec![
                Instruction::Phi {
                    dest: Value(10),
                    ty: IrType::U32,
                    incoming: vec![(i32c(0), BlockId(0)), (val(13), BlockId(1))],
                },
                Instruction::Cmp {
                    dest: Value(11),
                    op: IrCmpOp::Ult,
                    lhs: val(10),
                    rhs: i32c(16),
                    ty: IrType::U32,
                },
                Instruction::GetElementPtr {
                    dest: Value(12),
                    base: Value(1),
                    offset: val(10),
                    ty: IrType::Ptr,
                },
                Instruction::GetElementPtr {
                    dest: Value(14),
                    base: Value(2),
                    offset: val(10),
                    ty: IrType::Ptr,
                },
                load(15, 14, elem_ty),
                store(val(15), 12, elem_ty),
                Instruction::BinOp {
                    dest: Value(13),
                    op: IrBinOp::Add,
                    lhs: val(10),
                    rhs: i32c(1),
                    ty: IrType::U32,
                },
            ],
            Terminator::CondBranch {
                cond: val(11),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
        ));
        f.blocks
            .push(block(2, vec![], Terminator::Return(Some(i32c(0)))));
        f
    }

    #[test]
    fn self_loop_global_copy_rewrites_to_memcpy() {
        let mut f = self_loop_copy_func(IrType::U8);
        let n = run_function(&mut f);
        assert_eq!(n, 1, "single-block distinct-global copy must rewrite");
        assert!(has_call(&f, "memcpy"), "distinct globals lower to memcpy");
        assert!(
            !f.blocks.iter().any(|b| b.label.0 == 1),
            "dead loop header removed"
        );
    }

    #[test]
    fn global_names_that_alias_cannot_prove_disjoint() {
        let mut f = self_loop_copy_func(IrType::U8);
        let aliases: FxHashSet<String> = ["D".to_string(), "S".to_string()].into_iter().collect();
        let local_globals = aliases.clone();
        assert_eq!(super::run_function(&mut f, &aliases, &local_globals), 1);
        assert!(has_call(&f, "memmove"));
        assert!(!has_call(&f, "memcpy"));
        assert_guarded_scalar_fallback(&f);
    }

    #[test]
    fn extern_global_names_cannot_prove_disjoint() {
        let mut f = self_loop_copy_func(IrType::U8);
        assert_eq!(
            super::run_function(&mut f, &FxHashSet::default(), &FxHashSet::default()),
            1
        );
        assert!(has_call(&f, "memmove"));
        assert!(!has_call(&f, "memcpy"));
        assert_guarded_scalar_fallback(&f);
    }

    #[test]
    fn self_loop_i8_copy_rewrites() {
        // Pins the U8 -> byte-type (I8|U8) widening: signed bytes match too.
        let mut f = self_loop_copy_func(IrType::I8);
        let n = run_function(&mut f);
        assert_eq!(n, 1, "I8 copy loop must rewrite");
        assert!(has_call(&f, "memcpy"), "distinct globals lower to memcpy");
    }

    #[test]
    fn self_loop_param_copy_keeps_scalar_overlap_fallback() {
        // Two params may overlap in the forward-smear direction. A fast
        // memmove alone is not equivalent: the original loop must remain.
        let mut f = IrFunction::new(
            "self_loop_pcopy".into(),
            IrType::I32,
            vec![ptr_param(), ptr_param()],
            false,
        );
        f.next_value_id = 30;
        f.next_label = 3;
        f.blocks.push(block(
            0, // P
            vec![
                Instruction::ParamRef {
                    dest: Value(1),
                    param_idx: 0,
                    ty: IrType::Ptr,
                },
                Instruction::ParamRef {
                    dest: Value(2),
                    param_idx: 1,
                    ty: IrType::Ptr,
                },
            ],
            Terminator::Branch(BlockId(1)),
        ));
        // Header identical to the global shape (values 10..=15).
        let mut g = self_loop_copy_func(IrType::U8);
        let header = g.blocks.remove(1);
        f.blocks.push(header);
        f.blocks
            .push(block(2, vec![], Terminator::Return(Some(i32c(0)))));
        let n = run_function(&mut f);
        assert_eq!(n, 1, "single-block param copy must rewrite");
        assert!(has_call(&f, "memmove"), "non-smear fast path is retained");
        assert_guarded_scalar_fallback(&f);
        assert!(
            !has_call(&f, "memcpy"),
            "param roots must never lower to memcpy"
        );
    }

    #[test]
    fn header_div_bails() {
        // A dead trapping UDiv in the header: the rewrite DCEs the loop,
        // which would delete a potential trap. Must not match.
        let mut f = self_loop_copy_func(IrType::U8);
        f.blocks[1].instructions.push(Instruction::BinOp {
            dest: Value(20),
            op: IrBinOp::UDiv,
            lhs: val(10),
            rhs: i32c(2),
            ty: IrType::U32,
        });
        // The div rides after the iv bump (scan order is irrelevant: every
        // header BinOp is classified).
        let n = run_function(&mut f);
        assert_eq!(n, 0, "trapping div in header must bail");
    }

    /// Two-block indexed copy (header test + body latch) with a dead UDiv
    /// in the body: pins the body-scan trapping bail.
    fn two_block_copy_with_body_div() -> IrFunction {
        let mut f = IrFunction::new("two_block_div".into(), IrType::I32, vec![], false);
        f.next_value_id = 30;
        f.next_label = 4;
        f.blocks.push(block(
            0, // P
            vec![
                Instruction::GlobalAddr {
                    dest: Value(1),
                    name: "D".into(),
                },
                Instruction::GlobalAddr {
                    dest: Value(2),
                    name: "S".into(),
                },
            ],
            Terminator::Branch(BlockId(1)),
        ));
        f.blocks.push(block(
            1, // H: phis + test only
            vec![
                Instruction::Phi {
                    dest: Value(10),
                    ty: IrType::U32,
                    incoming: vec![(i32c(0), BlockId(0)), (val(13), BlockId(2))],
                },
                Instruction::Cmp {
                    dest: Value(11),
                    op: IrCmpOp::Ult,
                    lhs: val(10),
                    rhs: i32c(16),
                    ty: IrType::U32,
                },
            ],
            Terminator::CondBranch {
                cond: val(11),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
        ));
        f.blocks.push(block(
            2, // B (latch): copy + iv bump + dead trapping div
            vec![
                Instruction::GetElementPtr {
                    dest: Value(12),
                    base: Value(1),
                    offset: val(10),
                    ty: IrType::Ptr,
                },
                Instruction::GetElementPtr {
                    dest: Value(14),
                    base: Value(2),
                    offset: val(10),
                    ty: IrType::Ptr,
                },
                load(15, 14, IrType::U8),
                store(val(15), 12, IrType::U8),
                Instruction::BinOp {
                    dest: Value(13),
                    op: IrBinOp::Add,
                    lhs: val(10),
                    rhs: i32c(1),
                    ty: IrType::U32,
                },
                Instruction::BinOp {
                    dest: Value(20),
                    op: IrBinOp::UDiv,
                    lhs: val(10),
                    rhs: i32c(2),
                    ty: IrType::U32,
                },
            ],
            Terminator::Branch(BlockId(1)),
        ));
        f.blocks
            .push(block(3, vec![], Terminator::Return(Some(i32c(0)))));
        f
    }

    #[test]
    fn body_div_bails() {
        let mut f = two_block_copy_with_body_div();
        let n = run_function(&mut f);
        assert_eq!(n, 0, "trapping div in body must bail");
        assert!(
            f.blocks.iter().any(|b| b.label.0 == 2),
            "bailed loop left intact"
        );
    }

    /// Single-block indexed copy with caller-chosen preheader defs for the
    /// D (value 1) and S (value 2) bases; header is the shared values
    /// 10..=15 shape. `n_params` pointer params are declared.
    fn self_loop_copy_with_bases(
        d_def: Instruction,
        s_def: Instruction,
        n_params: usize,
    ) -> IrFunction {
        let mut f = IrFunction::new(
            "self_loop_bases".into(),
            IrType::I32,
            vec![ptr_param(); n_params],
            false,
        );
        f.next_value_id = 30;
        f.next_label = 3;
        f.blocks.push(block(
            0, // P
            vec![d_def, s_def],
            Terminator::Branch(BlockId(1)),
        ));
        let mut g = self_loop_copy_func(IrType::U8);
        let header = g.blocks.remove(1);
        f.blocks.push(header);
        f.blocks
            .push(block(2, vec![], Terminator::Return(Some(i32c(0)))));
        f
    }
    fn alloca_def(dest: u32) -> Instruction {
        Instruction::Alloca {
            dest: Value(dest),
            ty: IrType::U8,
            size: 16,
            align: 1,
            volatile: false,
            semantic_volatile: false,
        }
    }
    fn param_def(dest: u32, idx: usize) -> Instruction {
        param_ty_def(dest, idx, IrType::Ptr)
    }
    fn param_ty_def(dest: u32, idx: usize, ty: IrType) -> Instruction {
        Instruction::ParamRef {
            dest: Value(dest),
            param_idx: idx,
            ty,
        }
    }
    fn load_def(dest: u32, ptr: u32, ty: IrType) -> Instruction {
        load(dest, ptr, ty)
    }
    fn global_def(dest: u32, name: &str) -> Instruction {
        Instruction::GlobalAddr {
            dest: Value(dest),
            name: name.into(),
        }
    }

    #[test]
    fn distinct_restrict_params_are_disjoint() {
        let mut f = self_loop_copy_with_bases(param_def(1, 0), param_def(2, 1), 2);
        f.params[0].noalias = true;
        assert_eq!(run_function(&mut f), 1);
        assert!(has_call(&f, "memcpy"));
        assert!(!has_call(&f, "memmove"));
        assert_guarded_scalar_fallback(&f); // n == 0 may have null params
    }

    #[test]
    fn param_to_alloca_copy_uses_memcpy() {
        // Fresh-object disjointness: the alloca postdates the param value,
        // so no `restrict` is needed for memcpy's no-overlap contract.
        let mut f = self_loop_copy_with_bases(alloca_def(1), param_def(2, 0), 1);
        let n = run_function(&mut f);
        assert_eq!(n, 1, "param->alloca copy must rewrite");
        assert!(has_call(&f, "memcpy"), "disjoint pair lowers to memcpy");
        assert!(!has_call(&f, "memmove"), "no memmove for disjoint pair");
        assert_guarded_scalar_fallback(&f); // nullable source, zero trip
    }

    #[test]
    fn alloca_to_param_copy_uses_memcpy() {
        // Same rule, store side: alloca destination, param source.
        let mut f = self_loop_copy_with_bases(param_def(1, 0), alloca_def(2), 1);
        let n = run_function(&mut f);
        assert_eq!(n, 1, "alloca->param copy must rewrite");
        assert!(has_call(&f, "memcpy"), "disjoint pair lowers to memcpy");
        assert!(!has_call(&f, "memmove"), "no memmove for disjoint pair");
        assert_guarded_scalar_fallback(&f); // nullable destination, zero trip
    }

    #[test]
    fn param_to_global_copy_uses_memmove() {
        // A caller may legally pass `&global+k` for a plain `T *p`; overlap
        // is real, so (Param,Global) must NOT take the memcpy pair rule.
        let mut f = self_loop_copy_with_bases(global_def(1, "G"), param_def(2, 0), 1);
        let n = run_function(&mut f);
        assert_eq!(n, 1, "param->global copy must rewrite");
        assert!(has_call(&f, "memmove"), "non-smear fast path");
        assert_guarded_scalar_fallback(&f);
        assert!(!has_call(&f, "memcpy"), "no memcpy for maybe-overlap");
    }

    #[test]
    fn global_to_param_copy_uses_memmove() {
        let mut f = self_loop_copy_with_bases(param_def(1, 0), global_def(2, "G"), 1);
        let n = run_function(&mut f);
        assert_eq!(n, 1, "global->param copy must rewrite");
        assert!(has_call(&f, "memmove"), "non-smear fast path");
        assert_guarded_scalar_fallback(&f);
        assert!(!has_call(&f, "memcpy"), "no memcpy for maybe-overlap");
    }

    /// Trace `v` through a def map with no phis/labels (pure chain test).
    fn trace_root(defs: &FxHashMap<u32, &Instruction>, v: u32) -> ObjectRoot {
        object_root_inner(defs, &FxHashMap::default(), BlockId(0), Value(v), 0)
    }

    #[test]
    fn int_param_to_alloca_copy_uses_memmove() {
        // P1: an INTEGER parameter converted to a pointer (the frontend
        // erases the conversion) names no object — it must not present a
        // `Param` root, so the freshness rule cannot fire and the copy
        // takes memmove.
        let mut f = self_loop_copy_with_bases(alloca_def(1), param_ty_def(2, 0, IrType::U64), 1);
        let n = run_function(&mut f);
        assert_eq!(n, 1, "laundered-int-param copy must rewrite");
        assert!(has_call(&f, "memmove"), "non-smear fast path");
        assert_guarded_scalar_fallback(&f);
        assert!(!has_call(&f, "memcpy"), "no memcpy for unknown provenance");
    }

    #[test]
    fn int_param_to_param_copy_uses_memmove() {
        // Same hole, both sides laundered: still memmove, never memcpy.
        let mut f = self_loop_copy_with_bases(
            param_ty_def(1, 0, IrType::U64),
            param_ty_def(2, 1, IrType::U64),
            2,
        );
        let n = run_function(&mut f);
        assert_eq!(n, 1, "int-param copy must rewrite");
        assert!(has_call(&f, "memmove"), "non-smear fast path");
        assert_guarded_scalar_fallback(&f);
        assert!(!has_call(&f, "memcpy"), "no memcpy for unknown provenance");
    }

    #[test]
    fn typed_chain_matrix() {
        // Integer param through a laundering chain: no root, even though
        // the chain ends in pointer-typed operations.
        let iparam = param_ty_def(1, 0, IrType::U64);
        let copy = Instruction::Copy {
            dest: Value(2),
            src: val(1),
        };
        let gep = Instruction::GetElementPtr {
            dest: Value(3),
            base: Value(2),
            offset: i32c(0),
            ty: IrType::Ptr,
        };
        let defs: FxHashMap<u32, &Instruction> =
            [(1, &iparam), (2, &copy), (3, &gep)].into_iter().collect();
        assert_eq!(trace_root(&defs, 3), ObjectRoot::Other);
        // Pointer -> int -> pointer roundtrip through typed casts: the
        // int↔pointer casts end the proof (Ptr -> Ptr only is followed).
        let pparam = param_def(4, 0);
        let ptr_to_int = Instruction::Cast {
            dest: Value(5),
            src: val(4),
            from_ty: IrType::Ptr,
            to_ty: IrType::U64,
        };
        let int_to_ptr = Instruction::Cast {
            dest: Value(6),
            src: val(5),
            from_ty: IrType::U64,
            to_ty: IrType::Ptr,
        };
        let defs: FxHashMap<u32, &Instruction> = [(4, &pparam), (5, &ptr_to_int), (6, &int_to_ptr)]
            .into_iter()
            .collect();
        assert_eq!(trace_root(&defs, 6), ObjectRoot::Other);
        // Pointer-preserving Ptr -> Ptr cast: still roots.
        let ppcast = Instruction::Cast {
            dest: Value(7),
            src: val(4),
            from_ty: IrType::Ptr,
            to_ty: IrType::Ptr,
        };
        let defs: FxHashMap<u32, &Instruction> = [(4, &pparam), (7, &ppcast)].into_iter().collect();
        assert_eq!(trace_root(&defs, 7), ObjectRoot::Param(0));
    }

    #[test]
    fn typed_add_side_matrix() {
        // `p + dynamic_int_param` keeps the root (integer offsets, even
        // dynamic ones, cannot disturb object identity — wild is UB).
        let pparam = param_def(1, 0);
        let iparam = param_ty_def(2, 1, IrType::U64);
        let p_plus_dyn = Instruction::BinOp {
            dest: Value(3),
            op: IrBinOp::Add,
            lhs: val(1),
            rhs: val(2),
            ty: IrType::U64,
        };
        let defs: FxHashMap<u32, &Instruction> = [(1, &pparam), (2, &iparam), (3, &p_plus_dyn)]
            .into_iter()
            .collect();
        assert_eq!(trace_root(&defs, 3), ObjectRoot::Param(0));
        // `p + loaded_int` keeps the root (loaded integer offset).
        let loaded_int = load_def(4, 1, IrType::I32);
        let p_plus_load = Instruction::BinOp {
            dest: Value(5),
            op: IrBinOp::Add,
            lhs: val(1),
            rhs: val(4),
            ty: IrType::U64,
        };
        let defs: FxHashMap<u32, &Instruction> =
            [(1, &pparam), (4, &loaded_int), (5, &p_plus_load)]
                .into_iter()
                .collect();
        assert_eq!(trace_root(&defs, 5), ObjectRoot::Param(0));
        // `p + loaded_pointer` rejects the root (adding two addresses is
        // meaningless), both operand orders.
        let loaded_ptr = load_def(6, 1, IrType::Ptr);
        let p_plus_ptr = Instruction::BinOp {
            dest: Value(7),
            op: IrBinOp::Add,
            lhs: val(1),
            rhs: val(6),
            ty: IrType::Ptr,
        };
        let ptr_plus_p = Instruction::BinOp {
            dest: Value(8),
            op: IrBinOp::Add,
            lhs: val(6),
            rhs: val(1),
            ty: IrType::Ptr,
        };
        let defs: FxHashMap<u32, &Instruction> = [
            (1, &pparam),
            (6, &loaded_ptr),
            (7, &p_plus_ptr),
            (8, &ptr_plus_p),
        ]
        .into_iter()
        .collect();
        assert_eq!(trace_root(&defs, 7), ObjectRoot::Other);
        assert_eq!(trace_root(&defs, 8), ObjectRoot::Other);
        // The side check is immediate-def-only: `p + Copy(loaded_ptr)` —
        // the shape of the valid `p + (int)q` idiom, whose ptrtoint is an
        // erased Copy — keeps the root. (Copy is transparent; through-Copy
        // checking would regress valid integer-offset code.)
        let copy_of_ptr = Instruction::Copy {
            dest: Value(9),
            src: val(6),
        };
        let p_plus_copy = Instruction::BinOp {
            dest: Value(10),
            op: IrBinOp::Add,
            lhs: val(1),
            rhs: val(9),
            ty: IrType::U64,
        };
        let defs: FxHashMap<u32, &Instruction> = [
            (1, &pparam),
            (6, &loaded_ptr),
            (9, &copy_of_ptr),
            (10, &p_plus_copy),
        ]
        .into_iter()
        .collect();
        assert_eq!(trace_root(&defs, 10), ObjectRoot::Param(0));
    }

    #[test]
    fn sub_traced_roots() {
        // `ptr - int` keeps the pointer's root (stays in/near the object;
        // out-of-bounds is UB), exactly like `Add`.
        let pref = param_def(1, 0);
        let p_minus_c = Instruction::BinOp {
            dest: Value(2),
            op: IrBinOp::Sub,
            lhs: val(1),
            rhs: i32c(4),
            ty: IrType::Ptr,
        };
        let defs: FxHashMap<u32, &Instruction> =
            [(1, &pref), (2, &p_minus_c)].into_iter().collect();
        assert_eq!(trace_root(&defs, 2), ObjectRoot::Param(0));
        // `int - ptr` is integer laundering, not pointer arithmetic: the
        // int-derived address may alias anything, so it fails closed.
        let c_minus_p = Instruction::BinOp {
            dest: Value(3),
            op: IrBinOp::Sub,
            lhs: i32c(100),
            rhs: val(1),
            ty: IrType::Ptr,
        };
        let defs: FxHashMap<u32, &Instruction> =
            [(1, &pref), (3, &c_minus_p)].into_iter().collect();
        assert_eq!(trace_root(&defs, 3), ObjectRoot::Other);
        // `int + ptr` is commutative with `ptr + int`: keeps the root.
        let c_plus_p = Instruction::BinOp {
            dest: Value(4),
            op: IrBinOp::Add,
            lhs: i32c(100),
            rhs: val(1),
            ty: IrType::Ptr,
        };
        let defs: FxHashMap<u32, &Instruction> = [(1, &pref), (4, &c_plus_p)].into_iter().collect();
        assert_eq!(trace_root(&defs, 4), ObjectRoot::Param(0));
        // Two rooted sides (ptr+ptr, or ptrdiff reused as an address).
        let pref2 = param_def(5, 1);
        let p_plus_p = Instruction::BinOp {
            dest: Value(6),
            op: IrBinOp::Add,
            lhs: val(1),
            rhs: val(5),
            ty: IrType::Ptr,
        };
        let defs: FxHashMap<u32, &Instruction> = [(1, &pref), (5, &pref2), (6, &p_plus_p)]
            .into_iter()
            .collect();
        assert_eq!(trace_root(&defs, 6), ObjectRoot::Other);
    }
}
