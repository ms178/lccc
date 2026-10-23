//! Loop-idiom recognition: proven-disjoint byte copies call `memcpy`; other
//! copy loops retain a scalar fallback behind a safe fast path. The same
//! fixpoint also folds the guarded `ffs` shift-loop
//! (`while (!(x & 1)) { x >>= 1; r++; }` behind `if (!x) …`) into one
//! `CtzNonZero` — the `bsf` shape GCC 16 emits for glibc's `ffs`.
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

use super::module_symbol_facts::ModuleSymbolFacts;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::analysis::{self, CfgAnalysis};
use crate::ir::reexports::{
    BlockId, Instruction, IrBinOp, IrCmpOp, IrConst, IrFunction, IrUnaryOp, Operand, Terminator,
    Value,
};
use crate::passes::loop_analysis::{DominanceChecker, find_natural_loops, merge_loops_by_header};

/// Immutable target permission for loop idioms. The conservative default keeps
/// standalone and alternate pass entry points from synthesizing an operation
/// without a native lowering. Production resolves it once per translation unit.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Capabilities {
    pub native_popcount: bool,
}

/// Per-function entry point for the dirty-tracking pipeline. Two globals
/// prove disjoint through `ModuleSymbolFacts::globals_distinct` (one locally
/// binding strong definition, no alias); extern declarations may resolve to
/// the same storage in a different translation unit. Module-level symbol facts
/// are built once.
pub(crate) fn run_function(
    func: &mut IrFunction,
    symbol_facts: &ModuleSymbolFacts,
    capabilities: Capabilities,
) -> usize {
    recognize_idioms(func, symbol_facts, capabilities)
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
    symbol_facts: &ModuleSymbolFacts,
) -> bool {
    match (a, b) {
        (ObjectRoot::Global(a), ObjectRoot::Global(b)) => symbol_facts.globals_distinct(a, b),
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

impl CopyLoop {
    /// The library function the rewrite calls: `memmove` when the source
    /// and destination may overlap (the guarded rewrite), `memcpy` when
    /// they are proven disjoint.  The one answer both the synthesis gate and
    /// the emitted call use, so the gate always checks the symbol emitted.
    fn callee(&self) -> crate::common::pass_policy::Libcall {
        if self.needs_overlap_guard {
            crate::common::pass_policy::Libcall::Memmove
        } else {
            crate::common::pass_policy::Libcall::Memcpy
        }
    }
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
    symbol_facts: &ModuleSymbolFacts,
    capabilities: Capabilities,
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
            if let Some(matched) = try_match_copy_loop(func, lp, &cfg, symbol_facts) {
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
                // The rewrite creates a library call: only while the callee is
                // the library function (no -fno-builtin, and this TU does not
                // implement the mem* family -- a copy loop inside a TU-defined
                // `memmove` must not become `call memmove`).  Checked before
                // either rewrite touches the IR.
                let callee = matched.callee();
                if !func.pass_policy.builtins.may_synthesize(callee) {
                    dlog!(
                        "{} loop@{}: keep scalar ({} is not the library function here)",
                        func.name,
                        lp.header,
                        callee.name()
                    );
                    continue;
                }
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
            // Trailing-zero-count idiom (the portable `ffs` shape): the
            // shift-and-count loop is exactly `r0 + ctz(x0)` guarded
            // iterations. Matched only behind a dominating nonzero guard so
            // the rewrite emits `CtzNonZero` (baseline x86: `bsf` without a
            // zero fixup — GCC 16's shape for glibc's `ffs`).
            if let Some(matched) = try_match_ctz_loop(func, lp, &cfg) {
                dlog!(
                    "{} loop@{}: MATCH ctz (header={} pre={} latch_exit={} x=phi{} r=phi{})",
                    func.name,
                    lp.header,
                    matched.header,
                    matched.preheader,
                    matched.exit,
                    matched.x_phi.0,
                    matched.r_phi.0,
                );
                if rewrite_ctz_loop(func, &matched, &lp.body) {
                    total += 1;
                    progressed = true;
                    break;
                }
            }
            // Brian Kernighan population-count idiom (`while (x) { x &= x - 1; r++; }`
            // and the guarded `do { x &= x - 1; r++; } while (x)` form): each
            // iteration clears the lowest set bit of `x`, so the trip count is
            // `popcount(x0)` and the final accumulator is `r0 + popcount(x0)`.
            // Gated on target hardware popcount availability (`popcnt` on x86,
            // AdvSIMD `cnt` on AArch64).
            if capabilities.native_popcount {
                if let Some(matched) = try_match_popcount_loop(func, lp, &cfg) {
                    dlog!(
                        "{} loop@{}: MATCH popcount (header={} pre={} exit={} x=phi{} r=phi{} rotated={})",
                        func.name,
                        lp.header,
                        matched.header,
                        matched.preheader,
                        matched.exit,
                        matched.x_phi.0,
                        matched.r_phi.0,
                        matched.rotated,
                    );
                    if rewrite_popcount_loop(func, &matched, &lp.body) {
                        total += 1;
                        progressed = true;
                        break;
                    }
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
    symbol_facts: &ModuleSymbolFacts,
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

    // Header must end in a conditional branch with one successor in the
    // loop (body entry) and one outside (the single exit).
    let (cond_val, _in_succ, exit_idx) = match &func.blocks[header].terminator {
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
    let check_invariant = |v: Value, what: &str| -> Option<()> {
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
    let check_single_threaded = |mut v: Value, end: Value, what: &str| -> Option<()> {
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
    let needs_overlap_guard = !roots_proven_disjoint(func, &store_root, &load_root, symbol_facts);
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
        if !roots_proven_disjoint(func, &bound_root, &store_root, symbol_facts)
            || !roots_proven_disjoint(func, &bound_root, &load_root, symbol_facts)
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
            // Created only under `BuiltinPolicy::may_synthesize`.
            assume_library: true,
            ret_eightbyte_classes: Vec::new(),
            ret_is_f128_sse: false,
            cold: false,
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
    let classify = |v: Value| -> Option<Mat> {
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
    let do_clone = |_orig: Value, template: Instruction, func: &mut IrFunction| -> Value {
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
    let use_val = |orig: Value,
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

    // Only proven-disjoint roots reach this rewrite (`m.callee()` is
    // `memcpy`). A forward-overlap scalar loop cannot be replaced by either
    // memcpy or memmove.
    let call_name = m.callee().name();
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
    // The callee the top-level gate in `recognize_idioms` admitted.
    let call_name = m.callee().name();
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
    fast.instructions
        .push(make_copy_call(func, call_name, dst, src, len, size_ty));
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

// ── Trailing-zero-count (ffs shift-loop) idiom ───────────────────────────────

/// A matched trailing-zero-count loop — the portable `ffs` shape:
///
/// ```c
/// if (!x) return 0;              // dominating nonzero guard (proven below)
/// int r = 1;
/// while (!(x & 1)) { x >>= 1; r++; }
/// return r;                      // == r0 + ctz(x0)
/// ```
///
/// The loop performs exactly `ctz(x0)` iterations, so the counter ends at
/// `r0 + ctz(x0)`. Replacing it with one `CtzNonZero` removes both the
/// data-dependent bit-test branch and the serial shift chain — the `bsf`
/// shape GCC 16 emits for glibc's `ffs` and the kernel's `ffs`/`__ffs`.
#[derive(Clone, Copy)]
enum PreheaderEdge {
    /// `Branch(header)` — retarget the whole terminator.
    Uncond,
    /// `CondBranch` with the loop on the TRUE arm — retarget that arm.
    CondTrue,
    /// `CondBranch` with the loop on the FALSE arm — retarget that arm.
    CondFalse,
}

struct CtzLoop {
    header: usize,
    preheader: usize,
    edge: PreheaderEdge,
    exit: usize,
    /// Shifted word's loop-carried phi.
    x_phi: Value,
    /// The word's value entering the loop (preheader edge operand).
    x_init: Operand,
    x_ty: IrType,
    /// Counter's loop-carried phi.
    r_phi: Value,
    /// The counter's value entering the loop (preheader edge operand).
    r_init: Operand,
    r_ty: IrType,
    /// See `CtzPhis::r_add_ty`.
    r_add_ty: IrType,
}

/// Follow same-width `Cast` chains (cycle-guarded). The frontend's parameter
/// plumbing re-materializes loop-carried words as width-preserving casts;
/// the matcher sees through exactly those and nothing else.
fn strip_noop_casts(defs: &FxHashMap<u32, &Instruction>, mut v: Value) -> Value {
    for _ in 0..32 {
        match defs.get(&v.0) {
            Some(Instruction::Cast {
                src,
                from_ty,
                to_ty,
                ..
            }) if from_ty.size() == to_ty.size() => match src {
                Operand::Value(inner) => v = *inner,
                Operand::Const(_) => break,
            },
            _ => break,
        }
    }
    v
}

/// Header phi pair of a matched ctz loop: the shifted word and the counter,
/// each with its entry operand and type, plus the latch step destinations.
struct CtzPhis {
    x: (Value, Operand, IrType),
    r: (Value, Operand, IrType),
    x_step: Option<Value>,
    r_step: Option<Value>,
    /// Type the counter's `+ 1` is computed in: the phi type, or the
    /// promoted type when the step truncates back (`unsigned char r; r++`).
    r_add_ty: IrType,
}

/// Scan the header phis of a ctz candidate: exactly the shifted word
/// (`x >>= 1`) and the counter (`r += 1`), both fed from {preheader, latch},
/// both steps defined in the latch. Anything else refuses the loop.
fn ctz_collect_header_phis(
    func: &IrFunction,
    header: usize,
    latch: usize,
    latch_label: BlockId,
    defs: &FxHashMap<u32, &Instruction>,
    def_blocks: &FxHashMap<u32, usize>,
) -> Option<CtzPhis> {
    let name = func.name.as_str();
    let strip = |v: Value| strip_noop_casts(defs, resolve_copy(defs, v));
    let mut x: Option<(Value, Operand, IrType)> = None;
    let mut r: Option<(Value, Operand, IrType)> = None;
    let mut x_step_dest: Option<Value> = None;
    let mut r_step_dest: Option<Value> = None;
    let mut r_add_ty: Option<IrType> = None;
    for inst in &func.blocks[header].instructions {
        let Instruction::Phi { dest, ty, incoming } = inst else {
            continue;
        };
        if incoming.len() != 2 {
            dlog!(
                "{name} loop@{header}: ctz bail (phi {} arity {})",
                dest.0,
                incoming.len()
            );
            return None;
        }
        let mut latch_in: Option<&Operand> = None;
        let mut init: Option<&Operand> = None;
        for (op, l) in incoming {
            if *l == latch_label {
                if latch_in.is_some() {
                    dlog!(
                        "{name} loop@{header}: ctz bail (phi {} dup latch edge)",
                        dest.0
                    );
                    return None;
                }
                latch_in = Some(op);
            } else {
                init = Some(op);
            }
        }
        let Some(latch_in) = latch_in else {
            dlog!(
                "{name} loop@{header}: ctz bail (phi {} no latch edge)",
                dest.0
            );
            return None;
        };
        let Some(init) = init else {
            dlog!(
                "{name} loop@{header}: ctz bail (phi {} no entry edge)",
                dest.0
            );
            return None;
        };
        let init = init.clone();
        let step_v = match latch_in {
            Operand::Value(v) => strip(*v),
            Operand::Const(_) => {
                dlog!("{name} loop@{header}: ctz bail (phi {} const step)", dest.0);
                return None;
            }
        };
        // A narrow word or counter steps in its promoted type and truncates
        // back: `(T)((int)x >> 1)`, `(T)((int)r + 1)`. Look through exactly
        // that integer truncation to the phi's width. Wrapping arithmetic
        // makes the low bits equal the narrow step: `r + 1 mod 2^w`, and
        // after k shifts bit 0 is bit k of x0 whatever the fill bit, so the
        // loop still exits after ctz_w(x0) < w iterations.
        let (step_v, through_trunc) = match defs.get(&step_v.0) {
            Some(Instruction::Cast {
                src: Operand::Value(inner),
                from_ty,
                to_ty,
                ..
            }) if from_ty.is_integer()
                && to_ty.is_integer()
                && to_ty.size() == ty.size()
                && from_ty.size() > to_ty.size()
                && from_ty.size() <= 8 =>
            {
                (strip(*inner), true)
            }
            _ => (step_v, false),
        };
        if def_blocks.get(&step_v.0) != Some(&latch) {
            dlog!(
                "{name} loop@{header}: ctz bail (phi {} step outside latch)",
                dest.0
            );
            return None;
        }
        let Some(Instruction::BinOp {
            op,
            lhs,
            rhs,
            ty: step_ty,
            ..
        }) = defs.get(&step_v.0)
        else {
            dlog!(
                "{name} loop@{header}: ctz bail (phi {} step not a BinOp)",
                dest.0
            );
            return None;
        };
        // Through a truncation the step must compute in the promoted (wider)
        // type; the counter's sum is then formed there and truncated back.
        if through_trunc && step_ty.size() <= ty.size() {
            dlog!(
                "{name} loop@{header}: ctz bail (phi {} truncated step not promoted)",
                dest.0
            );
            return None;
        }
        let add_ty = if through_trunc { *step_ty } else { *ty };
        // The phi itself, possibly widened to the promoted step type.
        let feeds_self = |o: &Operand| -> bool {
            let Operand::Value(u) = o else { return false };
            let mut v = strip(*u);
            for _ in 0..4 {
                if v == *dest {
                    return true;
                }
                match defs.get(&v.0) {
                    Some(Instruction::Cast {
                        src: Operand::Value(inner),
                        from_ty,
                        to_ty,
                        ..
                    }) if from_ty.is_integer()
                        && to_ty.is_integer()
                        && from_ty.size() <= to_ty.size() =>
                    {
                        v = strip(*inner);
                    }
                    _ => return false,
                }
            }
            false
        };
        match (*op, lhs, rhs) {
            // Arithmetic or logical: the fill bit never reaches bit 0
            // before the loop exits (x0 != 0), so both count ctz(x0).
            (IrBinOp::LShr | IrBinOp::AShr, l, Operand::Const(c))
                if feeds_self(l) && c.to_i64() == Some(1) =>
            {
                if x.is_some() {
                    dlog!("{name} loop@{header}: ctz bail (two shifted words)");
                    return None;
                }
                x = Some((*dest, init, *ty));
                x_step_dest = Some(step_v);
            }
            (IrBinOp::Add, Operand::Const(c), rr) if c.to_i64() == Some(1) && feeds_self(rr) => {
                if r.is_some() {
                    dlog!("{name} loop@{header}: ctz bail (two counters)");
                    return None;
                }
                r = Some((*dest, init, *ty));
                r_step_dest = Some(step_v);
                r_add_ty = Some(add_ty);
            }
            (IrBinOp::Add, ll, Operand::Const(c)) if c.to_i64() == Some(1) && feeds_self(ll) => {
                if r.is_some() {
                    dlog!("{name} loop@{header}: ctz bail (two counters)");
                    return None;
                }
                r = Some((*dest, init, *ty));
                r_step_dest = Some(step_v);
                r_add_ty = Some(add_ty);
            }
            (op, _, _) => {
                dlog!(
                    "{name} loop@{header}: ctz bail (phi {} step {op:?})",
                    dest.0
                );
                return None;
            }
        }
    }
    let (x, r) = match (x, r) {
        (Some(x), Some(r)) => (x, r),
        (None, _) => {
            dlog!("{name} loop@{header}: ctz bail (no shifted word phi)");
            return None;
        }
        (_, None) => {
            dlog!("{name} loop@{header}: ctz bail (no counter phi)");
            return None;
        }
    };
    Some(CtzPhis {
        r_add_ty: r_add_ty.unwrap_or(r.2),
        x,
        r,
        x_step: x_step_dest,
        r_step: r_step_dest,
    })
}

/// The header test of a ctz candidate: `(x & 1)` re-evaluated in the header,
/// optionally wrapped in `== 0` / `!= 0`. Returns the polarity — `true` means
/// the condition is "low bit set" on the TRUE arm — after checking that the
/// body arm agrees with "continue while the low bit is clear" and the test's
/// defs live in the header (while-shape; rotated forms stay rolled).
fn ctz_header_test_polarity(
    func: &IrFunction,
    header: usize,
    body_arm_idx: usize,
    x_phi: Value,
    defs: &FxHashMap<u32, &Instruction>,
    def_blocks: &FxHashMap<u32, usize>,
    label_to_idx: &FxHashMap<BlockId, usize>,
) -> Option<bool> {
    let name = func.name.as_str();
    let strip = |v: Value| strip_noop_casts(defs, resolve_copy(defs, v));
    let as_val = |o: &Operand| -> Option<Value> {
        match o {
            Operand::Value(v) => Some(*v),
            Operand::Const(_) => None,
        }
    };
    let cond_v = match &func.blocks[header].terminator {
        Terminator::CondBranch { cond, .. } => match cond {
            Operand::Value(v) => strip(*v),
            Operand::Const(_) => return None,
        },
        _ => return None,
    };
    // Bit 0 survives any integer widening (zero- or sign-extending), so a
    // narrow word tested in its promoted type, `((int)x) & 1`, tests `x & 1`.
    let through_widen = |v: Value| -> Value {
        match defs.get(&v.0) {
            Some(Instruction::Cast {
                src: Operand::Value(src),
                from_ty,
                to_ty,
                ..
            }) if from_ty.is_integer() && to_ty.is_integer() && from_ty.size() < to_ty.size() => {
                strip(*src)
            }
            _ => v,
        }
    };
    let header_and = |cv: Value| -> Option<Value> {
        match defs.get(&cv.0)? {
            Instruction::BinOp {
                op: IrBinOp::And,
                lhs,
                rhs,
                ..
            } => {
                let one = |o: &Operand| matches!(o, Operand::Const(c) if c.to_i64() == Some(1));
                if let Some(l) = as_val(lhs) {
                    if one(rhs) {
                        return Some(through_widen(strip(l)));
                    }
                }
                if let Some(rr) = as_val(rhs) {
                    if one(lhs) {
                        return Some(through_widen(strip(rr)));
                    }
                }
                None
            }
            _ => None,
        }
    };
    // The `and` may itself be the branch condition (truthiness form) or sit
    // under a `== 0` / `!= 0` comparison.
    let low_bit_set_exits = if let Some(a) = header_and(cond_v) {
        if a != x_phi {
            dlog!("{name} loop@{header}: ctz bail (cond and is not (x & 1))");
            return None;
        }
        true
    } else {
        match defs.get(&cond_v.0) {
            Some(Instruction::Cmp { op, lhs, rhs, .. }) => {
                let and_on_lhs = as_val(lhs).is_some_and(|v| header_and(v) == Some(x_phi));
                let and_on_rhs = as_val(rhs).is_some_and(|v| header_and(v) == Some(x_phi));
                if !and_on_lhs && !and_on_rhs {
                    dlog!("{name} loop@{header}: ctz bail (cond is not against (x & 1))");
                    return None;
                }
                match op {
                    IrCmpOp::Eq => false, // (x&1)==0: true → continue
                    IrCmpOp::Ne => true,  // (x&1)!=0: true → exit
                    op => {
                        dlog!("{name} loop@{header}: ctz bail (cond cmp {op:?})");
                        return None;
                    }
                }
            }
            _ => {
                dlog!("{name} loop@{header}: ctz bail (cond neither and nor cmp)");
                return None;
            }
        }
    };
    match def_blocks.get(&cond_v.0) {
        Some(&db) if db == header => {}
        _ => {
            dlog!("{name} loop@{header}: ctz bail (test outside header)");
            return None;
        }
    }
    let header_true = match &func.blocks[header].terminator {
        Terminator::CondBranch { true_label, .. } => *label_to_idx.get(true_label)?,
        _ => return None,
    };
    // Truthy-and exits (raw form / `!= 0`) ⇔ the body arm is the FALSE arm;
    // `(x & 1) == 0` continues on TRUE ⇔ the body arm is the TRUE arm.
    if (header_true == body_arm_idx) == low_bit_set_exits {
        dlog!("{name} loop@{header}: ctz bail (test polarity)");
        return None;
    }
    Some(low_bit_set_exits)
}

/// Escape discipline for a ctz candidate: any loop-defined value used outside
/// the loop must be the counter, and only in shapes the rewrite covers — a
/// straight use in the exit block, or a phi operand whose incoming edge is
/// the exit block (the inlined-`ffs` merge shape: the exit branches to a
/// downstream merge phi that also collects the zero path). Returns the
/// number of outside uses of the counter (zero also refuses: the rewrite
/// would be dead code).
fn ctz_escape_discipline(
    func: &IrFunction,
    lp: &crate::passes::loop_analysis::NaturalLoop,
    exit_idx: usize,
    r_phi: Value,
    loop_defs: &FxHashSet<u32>,
) -> Option<usize> {
    let name = func.name.as_str();
    let header = lp.header;
    let exit_label = func.blocks[exit_idx].label;
    let mut r_outside_uses = 0usize;
    for (bi, block) in func.blocks.iter().enumerate() {
        if lp.body.contains(&bi) {
            continue;
        }
        let in_exit = bi == exit_idx;
        for inst in &block.instructions {
            if let Instruction::Phi { incoming, .. } = inst {
                for (op, lbl) in incoming {
                    if let Operand::Value(u) = op {
                        if loop_defs.contains(&u.0) {
                            if *u == r_phi && *lbl == exit_label {
                                r_outside_uses += 1;
                            } else {
                                dlog!(
                                    "{name} loop@{header}: ctz bail (loop value escapes via phi)"
                                );
                                return None;
                            }
                        }
                    }
                }
                continue;
            }
            inst.for_each_used_value(|u| {
                if loop_defs.contains(&u) {
                    if in_exit && u == r_phi.0 {
                        r_outside_uses += 1;
                    } else {
                        r_outside_uses = usize::MAX; // poison: bail below
                    }
                }
            });
        }
        match &block.terminator {
            Terminator::Return(Some(Operand::Value(u))) => {
                if loop_defs.contains(&u.0) {
                    if in_exit && *u == r_phi {
                        r_outside_uses += 1;
                    } else {
                        r_outside_uses = usize::MAX;
                    }
                }
            }
            Terminator::CondBranch { cond, .. } => {
                if let Operand::Value(u) = cond {
                    if loop_defs.contains(&u.0) {
                        r_outside_uses = usize::MAX;
                    }
                }
            }
            _ => {}
        }
    }
    if r_outside_uses == usize::MAX {
        dlog!("{name} loop@{header}: ctz bail (loop value escapes)");
        return None;
    }
    if r_outside_uses == 0 {
        dlog!("{name} loop@{header}: ctz bail (counter unused outside)");
        return None;
    }
    Some(r_outside_uses)
}

/// Walk up from the preheader (at most four single-pred blocks through
/// unconditional branches) looking for the dominating `if (!x) …` guard on
/// the loop word. A conditional preheader IS the guard: the arm entering the
/// loop must be the nonzero arm of a test on the word.
fn ctz_entry_proves_nonzero(
    func: &IrFunction,
    cfg: &CfgAnalysis,
    preheader: usize,
    edge: PreheaderEdge,
    x_init_v: Value,
    defs: &FxHashMap<u32, &Instruction>,
    label_to_idx: &FxHashMap<BlockId, usize>,
) -> bool {
    let name = func.name.as_str();
    let mut proven_nonzero = false;
    if !matches!(edge, PreheaderEdge::Uncond) {
        if let Terminator::CondBranch { cond, .. } = &func.blocks[preheader].terminator {
            if let Operand::Value(cv) = cond {
                match guard_on_value(defs, resolve_copy(defs, *cv), x_init_v) {
                    Some(eq_zero) => {
                        let nonzero_is_true = !eq_zero;
                        proven_nonzero = match edge {
                            PreheaderEdge::CondTrue => nonzero_is_true,
                            PreheaderEdge::CondFalse => !nonzero_is_true,
                            PreheaderEdge::Uncond => unreachable!(),
                        };
                    }
                    None => {
                        dlog!(
                            "{name} loop@{preheader}: ctz bail (cond preheader does not guard the word)"
                        );
                        return false;
                    }
                }
            }
        }
        return proven_nonzero;
    }
    let mut cur = preheader;
    for _ in 0..4 {
        let preds: Vec<usize> = cfg.preds.row(cur).iter().map(|&p| p as usize).collect();
        if preds.len() != 1 {
            break;
        }
        let p = preds[0];
        let (gc, t_lbl, f_lbl) = match &func.blocks[p].terminator {
            Terminator::Branch(t) if label_to_idx.get(t) == Some(&cur) => {
                cur = p;
                continue;
            }
            Terminator::CondBranch {
                cond,
                true_label,
                false_label,
            } => {
                let cond = match cond {
                    Operand::Value(v) => resolve_copy(defs, *v),
                    Operand::Const(_) => break,
                };
                match (label_to_idx.get(true_label), label_to_idx.get(false_label)) {
                    (Some(&t), Some(&f)) => (cond, t, f),
                    _ => break,
                }
            }
            _ => break,
        };
        let down_is_true = t_lbl == cur;
        let down_is_false = f_lbl == cur;
        if !down_is_true && !down_is_false {
            break;
        }
        match guard_on_value(defs, gc, x_init_v) {
            Some(eq_zero) => {
                let nonzero_is_true = !eq_zero;
                proven_nonzero =
                    (down_is_true && nonzero_is_true) || (down_is_false && !nonzero_is_true);
                break;
            }
            None => break,
        }
    }
    proven_nonzero
}

fn try_match_ctz_loop(
    func: &IrFunction,
    lp: &crate::passes::loop_analysis::NaturalLoop,
    cfg: &CfgAnalysis,
) -> Option<CtzLoop> {
    let name = func.name.as_str();
    // Shape: 2–3 blocks (header + latch, optionally one middle block).
    if !(2..=3).contains(&lp.body.len()) {
        return None;
    }
    let header = lp.header;
    let label_to_idx = analysis::build_label_map(func);

    // Header: conditional branch, one arm in the loop, one out.
    let (body_arm_idx, exit_idx) = match &func.blocks[header].terminator {
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => {
            let ti = label_to_idx.get(true_label).copied()?;
            let fi = label_to_idx.get(false_label).copied()?;
            let t_in = lp.body.contains(&ti);
            let f_in = lp.body.contains(&fi);
            if t_in == f_in {
                dlog!("{name} loop@{header}: ctz bail (header arms not split)");
                return None;
            }
            if t_in { (ti, fi) } else { (fi, ti) }
        }
        _ => return None,
    };

    // Header preds: exactly {preheader, latch}; the preheader transfer shape
    // decides how the rewrite retargets it.
    let preds: Vec<usize> = cfg.preds.row(header).iter().map(|&p| p as usize).collect();
    if preds.len() != 2 {
        dlog!(
            "{name} loop@{header}: ctz bail (header preds {})",
            preds.len()
        );
        return None;
    }
    let latch = lp.single_latch(&cfg.preds)?;
    if latch == header {
        dlog!("{name} loop@{header}: ctz bail (self-loop)");
        return None;
    }
    let preheader = if preds[0] == latch {
        preds[1]
    } else {
        preds[0]
    };
    if preheader == latch || lp.body.contains(&preheader) {
        dlog!("{name} loop@{header}: ctz bail (preheader inside loop)");
        return None;
    }
    let latch_label = func.blocks[latch].label;
    // Two preheader shapes: an unconditional branch into the loop (the
    // guard lives upstream), or the guard's own conditional branch with the
    // loop on one arm (`if (!x) return 0;` merged with the entry edge — the
    // common `ffs` shape after CFG simplification). The latter retargets
    // only the loop arm.
    let edge = match &func.blocks[preheader].terminator {
        Terminator::Branch(target) if label_to_idx.get(target) == Some(&header) => {
            PreheaderEdge::Uncond
        }
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => {
            let t_is_header = label_to_idx.get(true_label) == Some(&header);
            let f_is_header = label_to_idx.get(false_label) == Some(&header);
            if t_is_header == f_is_header {
                dlog!("{name} loop@{header}: ctz bail (preheader not Branch header)");
                return None;
            }
            if t_is_header {
                PreheaderEdge::CondTrue
            } else {
                PreheaderEdge::CondFalse
            }
        }
        _ => {
            dlog!("{name} loop@{header}: ctz bail (preheader not Branch header)");
            return None;
        }
    };
    // No side entries; non-header loop blocks branch only within the loop.
    for &b in lp.body.iter() {
        if b == header {
            continue;
        }
        for &p in cfg.preds.row(b).iter() {
            if !lp.body.contains(&(p as usize)) {
                dlog!("{name} loop@{header}: ctz bail (outside entry to {b})");
                return None;
            }
        }
        match &func.blocks[b].terminator {
            Terminator::Branch(target) => {
                let ti = *label_to_idx.get(target)?;
                if !lp.body.contains(&ti) {
                    dlog!("{name} loop@{header}: ctz bail (block {b} exits loop)");
                    return None;
                }
            }
            _ => {
                dlog!("{name} loop@{header}: ctz bail (block {b} not Branch)");
                return None;
            }
        }
    }

    // Definition sites (instructions only; phi defs live in the header).
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
    let strip = |v: Value| strip_noop_casts(&defs, resolve_copy(&defs, v));

    let phis = ctz_collect_header_phis(func, header, latch, latch_label, &defs, &def_blocks)?;
    let (x_phi, x_init, x_ty) = phis.x;
    let (r_phi, r_init, r_ty) = phis.r;
    let x_step_dest = phis.x_step;
    let r_step_dest = phis.r_step;
    let r_add_ty = phis.r_add_ty;

    ctz_header_test_polarity(
        func,
        header,
        body_arm_idx,
        x_phi,
        &defs,
        &def_blocks,
        &label_to_idx,
    )?;

    // Latch/middle scan: only the two steps plus plumbing. A load, store,
    // call or any other computation inside the loop kills the match (the
    // rewrite deletes these blocks).
    for &b in lp.body.iter() {
        if b == header {
            continue;
        }
        for inst in &func.blocks[b].instructions {
            match inst {
                Instruction::Copy { .. } | Instruction::Cast { .. } => {}
                Instruction::BinOp { dest, .. }
                    if Some(*dest) == x_step_dest || Some(*dest) == r_step_dest => {}
                other => {
                    dlog!("{name} loop@{header}: ctz bail (latch inst {other:?})");
                    return None;
                }
            }
        }
    }

    // Escape discipline: only the counter, only in the exit block.
    let loop_defs: FxHashSet<u32> = lp
        .body
        .iter()
        .flat_map(|&b| func.blocks[b].instructions.iter())
        .filter_map(|inst| inst.dest().map(|d| d.0))
        .collect();
    ctz_escape_discipline(func, lp, exit_idx, r_phi, &loop_defs)?;

    // Availability at the preheader: the word and counter entering values
    // must be defined in the preheader itself, in a dominating block, or be
    // function arguments.
    let x_init_v = match &x_init {
        Operand::Value(u) => strip(*u),
        Operand::Const(_) => {
            dlog!("{name} loop@{header}: ctz bail (const word — folds elsewhere)");
            return None;
        }
    };
    let (num_blocks, idom) = {
        let c = CfgAnalysis::build(func);
        (c.num_blocks, c.idom)
    };
    let dom = DominanceChecker::new(num_blocks, &idom);
    let check_available = |v: Value| -> Option<()> {
        match def_blocks.get(&v.0) {
            None => Some(()), // function argument
            Some(&db) if db == preheader || dom.dominates(db, preheader) => Some(()),
            Some(_) => None,
        }
    };
    check_available(x_init_v)?;
    if let Operand::Value(u) = &r_init {
        check_available(strip(*u))?;
    }

    // Dominating nonzero guard: the source `if (!x) …` that proves the loop
    // word is nonzero on the entry path. Without it, x0 == 0 makes the
    // source loop NOT terminate and `Ctz` would fabricate a value — refuse.
    if !ctz_entry_proves_nonzero(func, cfg, preheader, edge, x_init_v, &defs, &label_to_idx) {
        dlog!("{name} loop@{header}: ctz bail (no dominating nonzero guard)");
        return None;
    }

    Some(CtzLoop {
        header,
        preheader,
        edge,
        exit: exit_idx,
        x_phi,
        x_init,
        x_ty,
        r_phi,
        r_init,
        r_ty,
        r_add_ty,
    })
}

/// Is `cmp_v` a guard condition on `v`? Returns `Some(true)` for `v == 0`,
/// `Some(false)` for raw-truthiness / `v != 0`, `None` when the condition
/// does not test `v`.
fn guard_on_value(defs: &FxHashMap<u32, &Instruction>, cmp_v: Value, v: Value) -> Option<bool> {
    let strip = |val: Value| strip_noop_casts(defs, resolve_copy(defs, val));
    if strip(cmp_v) == v {
        return Some(false); // raw truthiness: nonzero on the TRUE arm
    }
    match defs.get(&cmp_v.0)? {
        Instruction::Cmp { op, lhs, rhs, .. } => {
            let zero = |o: &Operand| matches!(o, Operand::Const(c) if c.to_i64() == Some(0));
            let on_x = |o: &Operand| -> bool { matches!(o, Operand::Value(u) if strip(*u) == v) };
            match op {
                IrCmpOp::Ne if (on_x(lhs) && zero(rhs)) || (on_x(rhs) && zero(lhs)) => Some(false),
                IrCmpOp::Eq if (on_x(lhs) && zero(rhs)) || (on_x(rhs) && zero(lhs)) => Some(true),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Rewrite a matched trailing-zero-count loop into one `CtzNonZero` in the
/// preheader plus a straight branch to the loop exit. Abort-safe: every
/// structural check runs before the first mutation.
fn rewrite_ctz_loop(func: &mut IrFunction, m: &CtzLoop, body: &FxHashSet<usize>) -> bool {
    let fname = func.name.clone();
    let preheader_label = func.blocks[m.preheader].label;
    let header_label = func.blocks[m.header].label;
    let exit_label = func.blocks[m.exit].label;

    // Pre-scan 1: exit phis must have no preheader-edge incoming yet and at
    // most one header-edge incoming each.
    for inst in &func.blocks[m.exit].instructions {
        if let Instruction::Phi { incoming, .. } = inst {
            let mut header_entries = 0u32;
            let mut pre_entries = 0u32;
            for (_, label) in incoming {
                if *label == preheader_label {
                    pre_entries += 1;
                }
                if *label == header_label {
                    header_entries += 1;
                }
            }
            if pre_entries > 0 || header_entries > 1 {
                dlog!(
                    "{fname} loop@{}: ctz rewrite abort (exit phi edges pre={} header={})",
                    m.header,
                    pre_entries,
                    header_entries
                );
                return false;
            }
        }
    }
    // Pre-scan 2: the loop's labels are disjoint from preheader/exit by
    // construction; refuse to mutate if that ever changes.
    if body.contains(&m.preheader) || body.contains(&m.exit) {
        dlog!(
            "{fname} loop@{}: ctz rewrite abort (label collision)",
            m.header
        );
        return false;
    }

    // Materialize `res = r0 + ctz_nonzero(x0)` at the end of the preheader.
    // A word narrower than the counter's add type is widened first and
    // counted there: x0 != 0, and zero- or sign-extension keeps its lowest
    // set bit, so ctz(widen(x0)) == ctz(x0) -- and the result needs no
    // extension afterwards (the backend widens a sub-32-bit operand for
    // tzcnt/bsf anyway).
    let mut extra: Vec<Instruction> = Vec::new();
    let widen = m.x_ty.is_integer() && m.r_add_ty.is_integer() && m.x_ty.size() < m.r_add_ty.size();
    let (ctz_src, ctz_ty) = if widen {
        let wide_v = alloc_value(func);
        extra.push(Instruction::Cast {
            dest: wide_v,
            src: m.x_init.clone(),
            from_ty: m.x_ty,
            to_ty: m.r_add_ty,
        });
        (Operand::Value(wide_v), m.r_add_ty)
    } else {
        (m.x_init.clone(), m.x_ty)
    };
    let ctz_v = alloc_value(func);
    extra.push(Instruction::UnaryOp {
        dest: ctz_v,
        op: IrUnaryOp::CtzNonZero,
        src: ctz_src,
        ty: ctz_ty,
    });
    // The sum is formed exactly like the source step: in the counter's
    // add type, truncated back when the counter is narrower (`unsigned
    // char r = 250` wraps). ctz < 64 is representable in every type.
    let mut res = ctz_v;
    if m.r_add_ty != ctz_ty {
        let cast_v = alloc_value(func);
        extra.push(Instruction::Cast {
            dest: cast_v,
            src: Operand::Value(ctz_v),
            from_ty: ctz_ty,
            to_ty: m.r_add_ty,
        });
        res = cast_v;
    }
    let mut res_val = if matches!(&m.r_init, Operand::Const(c) if c.to_i64() == Some(0)) {
        res
    } else {
        let add_v = alloc_value(func);
        extra.push(Instruction::BinOp {
            dest: add_v,
            op: IrBinOp::Add,
            lhs: Operand::Value(res),
            rhs: m.r_init.clone(),
            ty: m.r_add_ty,
        });
        add_v
    };
    if m.r_add_ty != m.r_ty {
        let trunc_v = alloc_value(func);
        extra.push(Instruction::Cast {
            dest: trunc_v,
            src: Operand::Value(res_val),
            from_ty: m.r_add_ty,
            to_ty: m.r_ty,
        });
        res_val = trunc_v;
    }

    // Where the computation goes. A conditional preheader is the guard
    // (`if (!x) return 0;`): computing there runs the count on the zero path
    // too and keeps x live across it for the test (on i686 a callee-saved
    // register). When every predecessor of the exit is a loop block, the
    // exit's only predecessor after the rewrite is the guard's loop arm, so
    // the count sinks into the exit -- test first, count only when nonzero,
    // as GCC does. An exit phi would consume the result before its
    // definition, so exits with phis keep the preheader placement.
    let sink_into_exit = !matches!(m.edge, PreheaderEdge::Uncond)
        && !func.blocks[m.exit]
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::Phi { .. }))
        && {
            let label_to_idx = analysis::build_label_map(func);
            let (preds, _) = analysis::build_cfg(func, &label_to_idx);
            preds
                .row(m.exit)
                .iter()
                .all(|&p| body.contains(&(p as usize)))
        };

    // Retarget the preheader to the exit and install the computation. A
    // conditional preheader keeps its guard arm and only the loop arm moves
    // to the exit (the guard test still selects the zero path).
    {
        // Debug spans stay index-parallel with the instructions (dummy spans
        // for the new ones), or are dropped when they already were not.
        let at = if sink_into_exit { m.exit } else { m.preheader };
        let block = &mut func.blocks[at];
        let pos = if sink_into_exit {
            0
        } else {
            block.instructions.len()
        };
        if !block.source_spans.is_empty() {
            if block.source_spans.len() == block.instructions.len() {
                block.source_spans.splice(
                    pos..pos,
                    (0..extra.len()).map(|_| crate::common::source::Span::dummy()),
                );
            } else {
                block.source_spans.clear();
            }
        }
        block.instructions.splice(pos..pos, extra);
        let pre = &mut func.blocks[m.preheader];
        match m.edge {
            PreheaderEdge::Uncond => {
                debug_assert!(matches!(pre.terminator, Terminator::Branch(_)));
                pre.terminator = Terminator::Branch(exit_label);
            }
            PreheaderEdge::CondTrue => {
                if let Terminator::CondBranch { true_label, .. } = &mut pre.terminator {
                    *true_label = exit_label;
                }
            }
            PreheaderEdge::CondFalse => {
                if let Terminator::CondBranch { false_label, .. } = &mut pre.terminator {
                    *false_label = exit_label;
                }
            }
        }
    }

    // Replace the counter's uses in the exit block: straight uses become the
    // result; phi entries move from the (now dead) header edge to the new
    // preheader edge.
    let exit = m.exit;
    for inst in &mut func.blocks[exit].instructions {
        if let Instruction::Phi { incoming, .. } = inst {
            for (op, lbl) in incoming.iter_mut() {
                if *lbl == header_label {
                    if matches!(op, Operand::Value(u) if *u == m.r_phi) {
                        *op = Operand::Value(res_val);
                    }
                    *lbl = preheader_label;
                }
            }
            continue;
        }
        inst.for_each_operand_mut(|op| {
            if *op == Operand::Value(m.r_phi) {
                *op = Operand::Value(res_val);
            }
        });
        inst.for_each_value_use_mut(|u| {
            if *u == m.r_phi {
                *u = res_val;
            }
        });
    }
    if let Terminator::Return(Some(op)) = &mut func.blocks[exit].terminator {
        if matches!(op, Operand::Value(u) if *u == m.r_phi) {
            *op = Operand::Value(res_val);
        }
    }
    // Merge-block phis fed from the exit block's edge (the inlined-ffs shape
    // merges the zero path and the loop result downstream): the counter
    // operand becomes the result; the edge itself is untouched (the exit
    // block survives with its original terminator).
    let exit_label = func.blocks[exit].label;
    for bi in 0..func.blocks.len() {
        if bi == exit || body.contains(&bi) {
            continue;
        }
        for inst in &mut func.blocks[bi].instructions {
            if let Instruction::Phi { incoming, .. } = inst {
                for (op, lbl) in incoming.iter_mut() {
                    if *lbl == exit_label && matches!(op, Operand::Value(u) if *u == m.r_phi) {
                        *op = Operand::Value(res_val);
                    }
                }
            }
        }
    }

    // Delete the now-unreachable loop blocks and drop their phi edges
    // elsewhere (only exit phis could legally hold them, and those were
    // rewritten above).
    let dead_labels: FxHashSet<BlockId> = body.iter().map(|&b| func.blocks[b].label).collect();
    func.blocks.retain(|b| !dead_labels.contains(&b.label));
    for block in &mut func.blocks {
        for inst in &mut block.instructions {
            if let Instruction::Phi { incoming, .. } = inst {
                incoming.retain(|(_, lbl)| !dead_labels.contains(lbl));
            }
        }
    }
    dlog!(
        "{fname} loop@{}: ctz rewrite done ({} blocks removed)",
        m.header,
        dead_labels.len()
    );
    true
}

/// A matched Brian Kernighan population-count loop (`while (x) { x &= x - 1; r++; }`).
struct PopcountLoop {
    header: usize,
    preheader: usize,
    latch: usize,
    exit: usize,
    x_phi: Value,
    x_ty: IrType,
    x_init: Operand,
    r_phi: Value,
    r_ty: IrType,
    r_init: Operand,
    /// Counter step per cleared bit: `1` or `-1`.
    r_step: i64,
    r_out: Value,
    rotated: bool,
}

/// Follow `Copy` and integer `Cast` instructions whose source and target types
/// are at least `min_bytes` wide (preserving the low `min_bytes * 8` bits).
fn popcnt_unwrap_cast_chain(
    defs: &FxHashMap<u32, &Instruction>,
    mut v: Value,
    min_bytes: usize,
) -> Value {
    for _ in 0..32 {
        match defs.get(&v.0) {
            Some(Instruction::Copy {
                src: Operand::Value(inner),
                ..
            }) => v = *inner,
            Some(Instruction::Cast {
                src: Operand::Value(inner),
                from_ty,
                to_ty,
                ..
            }) if is_int_type(*from_ty)
                && is_int_type(*to_ty)
                && from_ty.size() >= min_bytes
                && to_ty.size() >= min_bytes =>
            {
                v = *inner;
            }
            _ => break,
        }
    }
    v
}

/// Match `x_phi - 1` or `x_phi + (-1)` at width `>= min_bytes`.
fn popcnt_is_minus_one(
    defs: &FxHashMap<u32, &Instruction>,
    v: Value,
    x_phi: Value,
    min_bytes: usize,
) -> bool {
    let v = popcnt_unwrap_cast_chain(defs, v, min_bytes);
    let Some(Instruction::BinOp {
        op, lhs, rhs, ty, ..
    }) = defs.get(&v.0)
    else {
        return false;
    };
    if !is_int_type(*ty) || ty.size() < min_bytes || ty.size() > 8 {
        return false;
    }
    match op {
        IrBinOp::Sub => {
            let Operand::Value(lv) = lhs else {
                return false;
            };
            popcnt_unwrap_cast_chain(defs, *lv, min_bytes) == x_phi
                && operand_const_int(rhs) == Some(1)
        }
        IrBinOp::Add => {
            let all_ones = ty.truncate_i64(-1);
            match (lhs, rhs) {
                (Operand::Value(lv), Operand::Const(c)) => {
                    popcnt_unwrap_cast_chain(defs, *lv, min_bytes) == x_phi
                        && c.to_i64().map(|k| ty.truncate_i64(k)) == Some(all_ones)
                }
                (Operand::Const(c), Operand::Value(rv)) => {
                    popcnt_unwrap_cast_chain(defs, *rv, min_bytes) == x_phi
                        && c.to_i64().map(|k| ty.truncate_i64(k)) == Some(all_ones)
                }
                _ => false,
            }
        }
        _ => false,
    }
}

/// Match `x_next = x_phi & (x_phi - 1)` (Either operand order, allowing
/// C integer promotion casts for 8/16-bit `x_ty`).
fn popcnt_is_clear_lsb(
    defs: &FxHashMap<u32, &Instruction>,
    x_next: Value,
    x_phi: Value,
    x_ty: IrType,
) -> bool {
    let min_bytes = x_ty.size();
    let v = popcnt_unwrap_cast_chain(defs, x_next, min_bytes);
    let Some(Instruction::BinOp {
        op: IrBinOp::And,
        lhs: Operand::Value(a),
        rhs: Operand::Value(b),
        ty,
        ..
    }) = defs.get(&v.0)
    else {
        return false;
    };
    if !is_int_type(*ty) || ty.size() < min_bytes || ty.size() > 8 {
        return false;
    }
    let a_unwrapped = popcnt_unwrap_cast_chain(defs, *a, min_bytes);
    let b_unwrapped = popcnt_unwrap_cast_chain(defs, *b, min_bytes);
    (a_unwrapped == x_phi && popcnt_is_minus_one(defs, *b, x_phi, min_bytes))
        || (b_unwrapped == x_phi && popcnt_is_minus_one(defs, *a, x_phi, min_bytes))
}

/// Match the counter recurrence `r_phi + 1` / `r_phi - 1` (also spelled
/// `r_phi - (-1)` / `r_phi + (-1)`, either operand order for `Add`) at width
/// `>= r_ty`, returning the step.  Copies and casts that preserve the low
/// `r_ty` bits are transparent: after `k` iterations the counter is
/// `(init + step * k) mod 2^bits(r_ty)` whatever the intermediate width or
/// signedness, so a promoted `unsigned char` counter `c = (u8)((int)c + 1)`
/// matches exactly like a native-width one.  Only the literal values `1` and
/// `-1` are accepted (they are +-1 modulo every width); anything else fails
/// closed.
fn popcnt_counter_step(
    defs: &FxHashMap<u32, &Instruction>,
    r_next: Value,
    r_phi: Value,
    r_ty: IrType,
) -> Option<i64> {
    let min_bytes = r_ty.size();
    let v = popcnt_unwrap_cast_chain(defs, r_next, min_bytes);
    let Some(Instruction::BinOp {
        op, lhs, rhs, ty, ..
    }) = defs.get(&v.0)
    else {
        return None;
    };
    if !is_int_type(*ty) || ty.size() < min_bytes || ty.size() > 8 {
        return None;
    }
    let is_r = |op: &Operand| matches!(op, Operand::Value(v) if popcnt_unwrap_cast_chain(defs, *v, min_bytes) == r_phi);
    let step = match op {
        IrBinOp::Add if is_r(lhs) => operand_const_int(rhs)?,
        IrBinOp::Add if is_r(rhs) => operand_const_int(lhs)?,
        IrBinOp::Sub if is_r(lhs) => operand_const_int(rhs)?.checked_neg()?,
        _ => return None,
    };
    matches!(step, 1 | -1).then_some(step)
}

/// Follow only integer conversions that preserve zero in both directions.
/// Same-width casts and extensions preserve zero; a truncation needs a separate
/// range/known-bits proof and must not be stripped merely because it preserves
/// the recurrence's low bits. Bounded traversal fails closed on copy cycles.
fn popcnt_zero_representative(
    defs: &FxHashMap<u32, &Instruction>,
    mut value: Value,
) -> Option<Value> {
    for _ in 0..32 {
        match defs.get(&value.0) {
            Some(Instruction::Copy {
                src: Operand::Value(inner),
                ..
            }) => value = *inner,
            Some(Instruction::Cast {
                src: Operand::Value(inner),
                from_ty,
                to_ty,
                ..
            }) if is_int_type(*from_ty)
                && is_int_type(*to_ty)
                && from_ty.size() <= to_ty.size() =>
            {
                value = *inner
            }
            _ => return Some(value),
        }
    }
    None
}

/// Resolve a `CondBranch` that tests `tested != 0` or `tested == 0`, returning
/// `(nonzero_target_block, zero_target_block)`.
fn popcnt_zero_branch_targets(
    defs: &FxHashMap<u32, &Instruction>,
    term: &Terminator,
    tested: Value,
    x_ty: IrType,
    label_to_idx: &FxHashMap<BlockId, usize>,
) -> Option<(usize, usize)> {
    let Terminator::CondBranch {
        cond: Operand::Value(cv),
        true_label,
        false_label,
    } = term
    else {
        return None;
    };
    let t_idx = *label_to_idx.get(true_label)?;
    let f_idx = *label_to_idx.get(false_label)?;
    let min_bytes = x_ty.size();
    let cv_resolved = resolve_copy(defs, *cv);
    let tested_zero = popcnt_zero_representative(defs, tested)?;
    if popcnt_zero_representative(defs, cv_resolved)? == tested_zero {
        return Some((t_idx, f_idx));
    }
    let Some(Instruction::Cmp {
        op, lhs, rhs, ty, ..
    }) = defs.get(&cv_resolved.0)
    else {
        return None;
    };
    if !is_int_type(*ty) || ty.size() < min_bytes || ty.size() > 8 {
        return None;
    }
    let tests_zero = match (lhs, rhs) {
        (Operand::Value(lv), r) => {
            popcnt_zero_representative(defs, *lv)? == tested_zero && operand_const_int(r) == Some(0)
        }
        (l, Operand::Value(rv)) => {
            popcnt_zero_representative(defs, *rv)? == tested_zero && operand_const_int(l) == Some(0)
        }
        _ => false,
    };
    if !tests_zero {
        return None;
    }
    match op {
        IrCmpOp::Ne => Some((t_idx, f_idx)),
        IrCmpOp::Eq => Some((f_idx, t_idx)),
        _ => None,
    }
}

/// Verify that every instruction in the loop body is side-effect-free and part
/// of the popcount recurrence.
fn popcnt_body_is_pure(
    func: &IrFunction,
    body: &FxHashSet<usize>,
    header: usize,
    x_phi: Value,
    r_phi: Value,
) -> bool {
    for &b in body {
        for inst in &func.blocks[b].instructions {
            match inst {
                Instruction::Phi { dest, .. } => {
                    if b != header || (*dest != x_phi && *dest != r_phi) {
                        return false;
                    }
                }
                Instruction::Copy { .. } => {}
                Instruction::Cast { from_ty, to_ty, .. } => {
                    if !is_int_type(*from_ty)
                        || !is_int_type(*to_ty)
                        || from_ty.size() > 8
                        || to_ty.size() > 8
                    {
                        return false;
                    }
                }
                Instruction::BinOp {
                    op: IrBinOp::Add | IrBinOp::Sub | IrBinOp::And,
                    ty,
                    ..
                } => {
                    if !is_int_type(*ty) || ty.size() > 8 {
                        return false;
                    }
                }
                Instruction::Cmp {
                    op: IrCmpOp::Eq | IrCmpOp::Ne,
                    ty,
                    ..
                } => {
                    if !is_int_type(*ty) || ty.size() > 8 {
                        return false;
                    }
                }
                _ => return false,
            }
        }
    }
    true
}

/// Verify that no value defined inside `body` is used outside `body` except
/// `r_out`, and that any `Phi` in `exit` only takes `r_out` from `exit_pred`.
fn popcnt_external_uses_valid(
    func: &IrFunction,
    body: &FxHashSet<usize>,
    exit: usize,
    exit_pred_label: BlockId,
    r_out: Value,
) -> bool {
    let mut body_defs: FxHashSet<u32> = FxHashSet::default();
    for &b in body {
        for inst in &func.blocks[b].instructions {
            if let Some(d) = inst.dest() {
                body_defs.insert(d.0);
            }
        }
    }
    for inst in &func.blocks[exit].instructions {
        if let Instruction::Phi { incoming, .. } = inst {
            for (op, lbl) in incoming {
                if *lbl == exit_pred_label && *op != Operand::Value(r_out) {
                    return false;
                }
            }
        }
    }
    let mut r_used_outside = false;
    for (bi, block) in func.blocks.iter().enumerate() {
        if body.contains(&bi) {
            continue;
        }
        let mut bad = false;
        for inst in &block.instructions {
            inst.for_each_used_value(|u| {
                if body_defs.contains(&u) {
                    if u == r_out.0 {
                        r_used_outside = true;
                    } else {
                        bad = true;
                    }
                }
            });
        }
        match &block.terminator {
            Terminator::Return(Some(Operand::Value(v)))
            | Terminator::CondBranch {
                cond: Operand::Value(v),
                ..
            }
            | Terminator::Switch {
                val: Operand::Value(v),
                ..
            }
            | Terminator::IndirectBranch {
                target: Operand::Value(v),
                ..
            } => {
                if body_defs.contains(&v.0) {
                    if *v == r_out {
                        r_used_outside = true;
                    } else {
                        bad = true;
                    }
                }
            }
            _ => {}
        }
        if bad {
            return false;
        }
    }
    r_used_outside
}

fn try_match_popcount_loop(
    func: &IrFunction,
    lp: &crate::passes::loop_analysis::NaturalLoop,
    cfg: &CfgAnalysis,
) -> Option<PopcountLoop> {
    let header = lp.header;
    if lp.body.len() > 2 {
        return None;
    }
    let latch = lp.single_latch(&cfg.preds)?;
    let preheaders: Vec<usize> = cfg
        .preds
        .row(header)
        .iter()
        .map(|&p| p as usize)
        .filter(|p| !lp.body.contains(p))
        .collect();
    if preheaders.len() != 1 {
        return None;
    }
    let preheader = preheaders[0];
    let preheader_label = func.blocks[preheader].label;
    let latch_label = func.blocks[latch].label;

    let mut phis = Vec::new();
    for inst in &func.blocks[header].instructions {
        if let Instruction::Phi {
            dest, ty, incoming, ..
        } = inst
        {
            if !is_int_type(*ty) || ty.size() > 8 || incoming.len() != 2 {
                return None;
            }
            let mut init_op = None;
            let mut next_val = None;
            for (op, lbl) in incoming {
                if *lbl == preheader_label {
                    init_op = Some(*op);
                } else if *lbl == latch_label {
                    if let Operand::Value(v) = op {
                        next_val = Some(*v);
                    }
                }
            }
            phis.push((*dest, *ty, init_op?, next_val?));
        }
    }
    if phis.len() != 2 {
        return None;
    }

    let mut defs: FxHashMap<u32, &Instruction> = FxHashMap::default();
    let mut def_blocks: FxHashMap<u32, usize> = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            if let Some(d) = inst.dest() {
                if defs.insert(d.0, inst).is_some() {
                    return None;
                }
                def_blocks.insert(d.0, bi);
            }
        }
    }
    let label_to_idx: FxHashMap<BlockId, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.label, i))
        .collect();

    let pick = |x: usize, r: usize| {
        if popcnt_is_clear_lsb(&defs, phis[x].3, phis[x].0, phis[x].1) {
            popcnt_counter_step(&defs, phis[r].3, phis[r].0, phis[r].1).map(|s| (x, r, s))
        } else {
            None
        }
    };
    let (x_idx, r_idx, r_step) = pick(0, 1).or_else(|| pick(1, 0))?;
    let (x_phi, x_ty, x_init, x_next) = phis[x_idx];
    let (r_phi, r_ty, r_init, r_next) = phis[r_idx];
    if !popcnt_body_is_pure(func, &lp.body, header, x_phi, r_phi) {
        return None;
    }

    let dom = DominanceChecker::new(cfg.num_blocks, &cfg.idom);
    let check_available = |op: &Operand| -> Option<()> {
        let Operand::Value(v) = op else {
            return Some(());
        };
        match def_blocks.get(&v.0) {
            None => Some(()),
            Some(&db) if db == preheader || dom.dominates(db, preheader) => Some(()),
            Some(_) => None,
        }
    };
    check_available(&x_init)?;
    check_available(&r_init)?;

    let edge = match &func.blocks[preheader].terminator {
        Terminator::Branch(target) if label_to_idx.get(target) == Some(&header) => {
            PreheaderEdge::Uncond
        }
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => {
            let t_is_header = label_to_idx.get(true_label) == Some(&header);
            let f_is_header = label_to_idx.get(false_label) == Some(&header);
            if t_is_header == f_is_header {
                return None;
            }
            if t_is_header {
                PreheaderEdge::CondTrue
            } else {
                PreheaderEdge::CondFalse
            }
        }
        _ => return None,
    };

    // Two loop shapes:
    // 1. Unrotated `while (x) { x &= x - 1; r++; }`: header tests `x_phi != 0`
    //    before the body runs in `latch`, and exits on `x_phi == 0` with `r_phi`.
    // 2. Guarded `do { x &= x - 1; r++; } while (x)`: body runs before `latch`
    //    tests `x_next != 0`, and exits on `x_next == 0` with `r_next` (sound
    //    only when the preheader proves `x_init != 0`).
    let (exit, rotated, r_out, exit_pred_label) = if header != latch {
        let (nz_target, z_target) = popcnt_zero_branch_targets(
            &defs,
            &func.blocks[header].terminator,
            x_phi,
            x_ty,
            &label_to_idx,
        )?;
        if nz_target != latch || lp.body.contains(&z_target) {
            return None;
        }
        for inst in &func.blocks[header].instructions {
            if !matches!(inst, Instruction::Phi { .. } | Instruction::Cmp { .. }) {
                return None;
            }
        }
        if !matches!(
            func.blocks[latch].terminator,
            Terminator::Branch(lbl) if lbl == func.blocks[header].label
        ) {
            return None;
        }
        (z_target, false, r_phi, func.blocks[header].label)
    } else {
        let Operand::Value(x_init_val) = x_init else {
            return None;
        };
        let x_init_val = strip_noop_casts(&defs, resolve_copy(&defs, x_init_val));
        if !ctz_entry_proves_nonzero(func, cfg, preheader, edge, x_init_val, &defs, &label_to_idx) {
            return None;
        }
        let (nz_target, z_target) = popcnt_zero_branch_targets(
            &defs,
            &func.blocks[latch].terminator,
            x_next,
            x_ty,
            &label_to_idx,
        )?;
        if nz_target != header || lp.body.contains(&z_target) {
            return None;
        }
        (z_target, true, r_next, func.blocks[latch].label)
    };

    if exit == preheader
        || !popcnt_external_uses_valid(func, &lp.body, exit, exit_pred_label, r_out)
    {
        return None;
    }

    Some(PopcountLoop {
        header,
        preheader,
        latch,
        exit,
        x_phi,
        x_ty,
        x_init,
        r_phi,
        r_ty,
        r_init,
        r_step,
        r_out,
        rotated,
    })
}

/// Append `Cast src: from -> to` to `extra` unless the types already agree.
fn popcnt_cast(
    func: &mut IrFunction,
    extra: &mut Vec<Instruction>,
    src: Operand,
    from_ty: IrType,
    to_ty: IrType,
) -> Operand {
    if from_ty == to_ty {
        return src;
    }
    let dest = alloc_value(func);
    extra.push(Instruction::Cast {
        dest,
        src,
        from_ty,
        to_ty,
    });
    Operand::Value(dest)
}

fn rewrite_popcount_loop(func: &mut IrFunction, m: &PopcountLoop, body: &FxHashSet<usize>) -> bool {
    let header_label = func.blocks[m.header].label;
    let latch_label = func.blocks[m.latch].label;
    let preheader_label = func.blocks[m.preheader].label;
    let exit_label = func.blocks[m.exit].label;
    let exit_pred_label = if m.rotated { latch_label } else { header_label };

    for inst in &func.blocks[m.exit].instructions {
        if let Instruction::Phi { incoming, .. } = inst {
            let mut pred_entries = 0u32;
            let mut pre_entries = 0u32;
            for (_, label) in incoming {
                if *label == preheader_label {
                    pre_entries += 1;
                }
                if *label == exit_pred_label {
                    pred_entries += 1;
                }
            }
            if pre_entries > 0 || pred_entries > 1 {
                return false;
            }
        }
    }

    let mut extra: Vec<Instruction> = Vec::new();
    let (pop_src, pop_ty) = match m.x_ty {
        IrType::I8 => {
            let v_u8 = alloc_value(func);
            let v_u32 = alloc_value(func);
            extra.push(Instruction::Cast {
                dest: v_u8,
                src: m.x_init,
                from_ty: IrType::I8,
                to_ty: IrType::U8,
            });
            extra.push(Instruction::Cast {
                dest: v_u32,
                src: Operand::Value(v_u8),
                from_ty: IrType::U8,
                to_ty: IrType::U32,
            });
            (Operand::Value(v_u32), IrType::U32)
        }
        IrType::U8 => {
            let v_u32 = alloc_value(func);
            extra.push(Instruction::Cast {
                dest: v_u32,
                src: m.x_init,
                from_ty: IrType::U8,
                to_ty: IrType::U32,
            });
            (Operand::Value(v_u32), IrType::U32)
        }
        IrType::I16 => {
            let v_u16 = alloc_value(func);
            let v_u32 = alloc_value(func);
            extra.push(Instruction::Cast {
                dest: v_u16,
                src: m.x_init,
                from_ty: IrType::I16,
                to_ty: IrType::U16,
            });
            extra.push(Instruction::Cast {
                dest: v_u32,
                src: Operand::Value(v_u16),
                from_ty: IrType::U16,
                to_ty: IrType::U32,
            });
            (Operand::Value(v_u32), IrType::U32)
        }
        IrType::U16 => {
            let v_u32 = alloc_value(func);
            extra.push(Instruction::Cast {
                dest: v_u32,
                src: m.x_init,
                from_ty: IrType::U16,
                to_ty: IrType::U32,
            });
            (Operand::Value(v_u32), IrType::U32)
        }
        IrType::I32 | IrType::U32 => (m.x_init, m.x_ty),
        IrType::I64 | IrType::U64 => (m.x_init, m.x_ty),
        _ => return false,
    };

    let v_pop = alloc_value(func);
    extra.push(Instruction::UnaryOp {
        dest: v_pop,
        op: IrUnaryOp::Popcount,
        src: pop_src,
        ty: pop_ty,
    });

    // `r_out = (r_init + r_step * popcount) mod 2^bits(r_ty)`.  Native-width
    // counters compute in `r_ty`; a narrow counter computes in `I32` and
    // truncates, the same promote/truncate shape C gives `c++` on a `char`.
    let calc_ty = if m.r_ty.size() < 4 {
        IrType::I32
    } else {
        m.r_ty
    };
    let pop_c = popcnt_cast(func, &mut extra, Operand::Value(v_pop), pop_ty, calc_ty);
    let res_c = if m.r_step == 1 && operand_const_int(&m.r_init) == Some(0) {
        pop_c
    } else {
        let init_c = popcnt_cast(func, &mut extra, m.r_init, m.r_ty, calc_ty);
        let vr = alloc_value(func);
        extra.push(Instruction::BinOp {
            dest: vr,
            op: if m.r_step == 1 {
                IrBinOp::Add
            } else {
                IrBinOp::Sub
            },
            lhs: init_c,
            rhs: pop_c,
            ty: calc_ty,
        });
        Operand::Value(vr)
    };
    let Operand::Value(v_res) = popcnt_cast(func, &mut extra, res_c, calc_ty, m.r_ty) else {
        return false;
    };

    let sink_into_exit = matches!(
        func.blocks[m.preheader].terminator,
        Terminator::CondBranch { .. }
    ) && !func.blocks[m.exit]
        .instructions
        .iter()
        .any(|i| matches!(i, Instruction::Phi { .. }))
        && {
            let label_to_idx = analysis::build_label_map(func);
            let (preds, _) = analysis::build_cfg(func, &label_to_idx);
            preds
                .row(m.exit)
                .iter()
                .all(|&p| body.contains(&(p as usize)))
        };

    match &mut func.blocks[m.preheader].terminator {
        Terminator::Branch(lbl) if *lbl == header_label => *lbl = exit_label,
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => {
            if *true_label == header_label {
                *true_label = exit_label;
            }
            if *false_label == header_label {
                *false_label = exit_label;
            }
        }
        _ => return false,
    }

    let at = if sink_into_exit { m.exit } else { m.preheader };
    let pos = if sink_into_exit {
        0
    } else {
        func.blocks[at].instructions.len()
    };
    {
        let block = &mut func.blocks[at];
        if !block.source_spans.is_empty() {
            if block.source_spans.len() == block.instructions.len() {
                block.source_spans.splice(
                    pos..pos,
                    (0..extra.len()).map(|_| crate::common::source::Span::dummy()),
                );
            } else {
                block.source_spans.clear();
            }
        }
        block.instructions.splice(pos..pos, extra);
    }

    for inst in &mut func.blocks[m.exit].instructions {
        if let Instruction::Phi { incoming, .. } = inst {
            for (op, lbl) in incoming.iter_mut() {
                if *lbl == exit_pred_label {
                    *op = Operand::Value(v_res);
                    *lbl = preheader_label;
                }
            }
        }
    }

    for (bi, block) in func.blocks.iter_mut().enumerate() {
        if body.contains(&bi) {
            continue;
        }
        for inst in &mut block.instructions {
            if matches!(inst, Instruction::Phi { .. }) && bi == m.exit {
                continue;
            }
            inst.for_each_operand_mut(|op| {
                if *op == Operand::Value(m.r_out) {
                    *op = Operand::Value(v_res);
                }
            });
            inst.for_each_value_use_mut(|u| {
                if *u == m.r_out {
                    *u = v_res;
                }
            });
        }
        match &mut block.terminator {
            Terminator::Return(Some(Operand::Value(v)))
            | Terminator::CondBranch {
                cond: Operand::Value(v),
                ..
            }
            | Terminator::Switch {
                val: Operand::Value(v),
                ..
            }
            | Terminator::IndirectBranch {
                target: Operand::Value(v),
                ..
            } => {
                if *v == m.r_out {
                    *v = v_res;
                }
            }
            _ => {}
        }
    }

    let dead_labels: FxHashSet<BlockId> = body.iter().map(|&b| func.blocks[b].label).collect();
    func.blocks.retain(|b| !dead_labels.contains(&b.label));
    for block in &mut func.blocks {
        for inst in &mut block.instructions {
            if let Instruction::Phi { incoming, .. } = inst {
                incoming.retain(|(_, lbl)| !dead_labels.contains(lbl));
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::AddressSpace;
    use crate::ir::reexports::{BasicBlock, IrParam};

    fn run_function(func: &mut IrFunction) -> usize {
        // Synthetic D/S GlobalAddr instructions represent strong, locally
        // binding definitions in these tests; G is intentionally unproven.
        let symbol_facts = ModuleSymbolFacts {
            aliased_names: FxHashSet::default(),
            identity_witnesses: ["D".to_string(), "S".to_string()].into_iter().collect(),
        };
        super::run_function(
            func,
            &symbol_facts,
            Capabilities {
                native_popcount: true,
            },
        )
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
        let symbol_facts = ModuleSymbolFacts {
            identity_witnesses: aliases.clone(),
            aliased_names: aliases,
        };
        assert_eq!(
            super::run_function(&mut f, &symbol_facts, Capabilities::default()),
            1
        );
        assert!(has_call(&f, "memmove"));
        assert!(!has_call(&f, "memcpy"));
        assert_guarded_scalar_fallback(&f);
    }

    #[test]
    fn extern_global_names_cannot_prove_disjoint() {
        let mut f = self_loop_copy_func(IrType::U8);
        assert_eq!(
            super::run_function(
                &mut f,
                &ModuleSymbolFacts::default(),
                Capabilities::default()
            ),
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
    // ── Trailing-zero-count (ffs shift-loop) idiom ─────────────────────────

    fn u32_param() -> IrParam {
        IrParam {
            ty: IrType::U32,
            noalias: false,
            struct_size: None,
            struct_align: None,
            param_align: None,
            struct_eightbyte_classes: Vec::new(),
            is_f128_sse: false,
            riscv_float_class: None,
        }
    }

    /// The pass-time `ffs` shape (labels match the pipeline's post-simplify
    /// CFG): guard block 0 branches straight into the loop header, the
    /// header carries both phis plus the `(x & 1)` truthiness test, the
    /// latch shifts and counts, the exit returns the counter.
    fn ctz_func(guard_on_word: bool) -> IrFunction {
        let mut f = IrFunction::new("ffs_shape".into(), IrType::I32, vec![u32_param()], false);
        f.next_value_id = 20;
        f.next_label = 5;
        let cond_src = if guard_on_word { val(1) } else { i32c(1) };
        f.blocks.push(block(
            0, // guard: `if (!x) return 0;` with the loop on the TRUE arm
            vec![param_def(1, 0)],
            Terminator::CondBranch {
                cond: cond_src,
                true_label: BlockId(2),
                false_label: BlockId(1),
            },
        ));
        f.blocks.push(block(
            1, // zero path
            vec![],
            Terminator::Return(Some(i32c(0))),
        ));
        f.blocks.push(block(
            2, // header: phis + (x & 1) test
            vec![
                Instruction::Phi {
                    dest: Value(10),
                    ty: IrType::U32,
                    incoming: vec![(val(1), BlockId(0)), (val(12), BlockId(3))],
                },
                Instruction::Phi {
                    dest: Value(11),
                    ty: IrType::I32,
                    incoming: vec![(i32c(1), BlockId(0)), (val(13), BlockId(3))],
                },
                Instruction::BinOp {
                    dest: Value(14),
                    op: IrBinOp::And,
                    lhs: val(10),
                    rhs: Operand::Const(IrConst::I64(1)),
                    ty: IrType::U32,
                },
            ],
            Terminator::CondBranch {
                cond: val(14),
                true_label: BlockId(4),
                false_label: BlockId(3),
            },
        ));
        f.blocks.push(block(
            3, // latch: x >>= 1; r++
            vec![
                Instruction::BinOp {
                    dest: Value(12),
                    op: IrBinOp::LShr,
                    lhs: val(10),
                    rhs: Operand::Const(IrConst::I64(1)),
                    ty: IrType::U32,
                },
                Instruction::BinOp {
                    dest: Value(13),
                    op: IrBinOp::Add,
                    lhs: val(11),
                    rhs: i32c(1),
                    ty: IrType::I32,
                },
            ],
            Terminator::Branch(BlockId(2)),
        ));
        f.blocks.push(block(
            4, // exit: return r
            vec![],
            Terminator::Return(Some(val(11))),
        ));
        f
    }

    #[test]
    fn ctz_guarded_loop_becomes_one_ctz_nonzero() {
        let mut f = ctz_func(true);
        assert_eq!(run_function(&mut f), 1, "the guarded shift loop rewrites");
        // The loop is gone; the guard keeps two arms and the preheader arm
        // now feeds a CtzNonZero feeding the exit value.
        assert_eq!(f.blocks.len(), 3, "header and latch removed");
        assert!(
            f.blocks
                .iter()
                .any(|b| b.instructions.iter().any(|inst| matches!(
                    inst,
                    Instruction::UnaryOp {
                        op: IrUnaryOp::CtzNonZero,
                        ..
                    }
                ))),
            "one CtzNonZero materialized"
        );
        assert!(
            !f.blocks.iter().any(|b| b.label == BlockId(2)),
            "loop header deleted"
        );
    }

    #[test]
    fn ctz_unguarded_loop_is_kept() {
        // The guard tests a constant instead of the word: x0 == 0 would make
        // the source loop not terminate, so the rewrite must refuse.
        let mut f = ctz_func(false);
        assert_eq!(run_function(&mut f), 0, "no nonzero proof, no rewrite");
        assert!(
            f.blocks.iter().any(|b| b.label == BlockId(2)),
            "loop retained"
        );
    }

    /// Mutate the latch step of `ctz_func(true)` that defines `dest`.
    fn ctz_latch_step(f: &mut IrFunction, dest: u32) -> &mut Instruction {
        f.blocks
            .iter_mut()
            .find(|b| b.label == BlockId(3))
            .and_then(|b| {
                b.instructions
                    .iter_mut()
                    .find(|i| i.dest() == Some(Value(dest)))
            })
            .expect("latch step")
    }

    /// The block holding the (single) materialized `CtzNonZero`.
    fn ctz_block(f: &IrFunction) -> &BasicBlock {
        f.blocks
            .iter()
            .find(|b| {
                b.instructions.iter().any(|i| {
                    matches!(
                        i,
                        Instruction::UnaryOp {
                            op: IrUnaryOp::CtzNonZero,
                            ..
                        }
                    )
                })
            })
            .expect("a CtzNonZero")
    }

    #[test]
    fn ctz_count_sinks_below_the_guard() {
        // The guard is the exit's only predecessor after the rewrite: the
        // count runs on the nonzero path only (x is not live past the test).
        let mut f = ctz_func(true);
        assert_eq!(run_function(&mut f), 1);
        assert_eq!(ctz_block(&f).label, BlockId(4), "count in the exit block");
        let guard = f.blocks.iter().find(|b| b.label == BlockId(0)).unwrap();
        assert_eq!(guard.instructions.len(), 1, "guard keeps only its ParamRef");
        // An exit with phis (the counter itself never feeds one: the matcher
        // refuses that escape) keeps the count in the preheader, where every
        // phi of the block is still at its head.
        let mut f = ctz_func(true);
        let exit = f.blocks.iter_mut().find(|b| b.label == BlockId(4)).unwrap();
        exit.instructions.push(Instruction::Phi {
            dest: Value(18),
            ty: IrType::I32,
            incoming: vec![(i32c(7), BlockId(2))],
        });
        assert_eq!(run_function(&mut f), 1);
        assert_eq!(ctz_block(&f).label, BlockId(0), "count stays in the guard");
    }

    fn has_ctz_nonzero(f: &IrFunction) -> bool {
        f.blocks.iter().any(|b| {
            b.instructions.iter().any(|i| {
                matches!(
                    i,
                    Instruction::UnaryOp {
                        op: IrUnaryOp::CtzNonZero,
                        ..
                    }
                )
            })
        })
    }

    #[test]
    fn ctz_arithmetic_shift_of_signed_word_rewrites() {
        // `int x; while (!(x & 1)) x >>= 1;` — sar fills from the sign bit,
        // which never reaches bit 0 before a nonzero word's lowest set bit.
        let mut f = ctz_func(true);
        if let Instruction::BinOp { op, ty, .. } = ctz_latch_step(&mut f, 12) {
            *op = IrBinOp::AShr;
            *ty = IrType::I32;
        }
        assert_eq!(run_function(&mut f), 1, "sar loop rewrites");
        assert!(has_ctz_nonzero(&f));
    }

    #[test]
    fn ctz_narrow_counter_sums_in_promoted_type_and_truncates() {
        // `unsigned char r = 250; ... r++;` steps as `(u8)((int)r + 1)`: the
        // rewrite must form `250 + ctz` in I32 and truncate to U8 (wraps).
        let mut f = ctz_func(true);
        for b in f.blocks.iter_mut() {
            for inst in b.instructions.iter_mut() {
                if let Instruction::Phi { dest, ty, incoming } = inst {
                    if *dest == Value(11) {
                        *ty = IrType::U8;
                        incoming[0].0 = Operand::Const(IrConst::I64(250));
                    }
                }
            }
        }
        if let Instruction::BinOp { dest, .. } = ctz_latch_step(&mut f, 13) {
            *dest = Value(15);
        }
        let latch = f.blocks.iter_mut().find(|b| b.label == BlockId(3)).unwrap();
        latch.instructions.push(Instruction::Cast {
            dest: Value(13),
            src: val(15),
            from_ty: IrType::I32,
            to_ty: IrType::U8,
        });
        assert_eq!(run_function(&mut f), 1, "truncating counter rewrites");
        let at = ctz_block(&f);
        let tail: Vec<_> = at.instructions.iter().rev().take(2).collect();
        assert!(
            matches!(
                tail[0],
                Instruction::Cast {
                    from_ty: IrType::I32,
                    to_ty: IrType::U8,
                    ..
                }
            ) && matches!(
                tail[1],
                Instruction::BinOp {
                    op: IrBinOp::Add,
                    ty: IrType::I32,
                    ..
                }
            ),
            "sum in I32, then truncated to U8: {tail:?}"
        );
    }

    #[test]
    fn ctz_truncated_step_must_be_promoted() {
        // A truncation around a step that is not wider than the phi is not
        // the promotion shape; refuse rather than guess its semantics.
        let mut f = ctz_func(true);
        if let Instruction::BinOp { dest, ty, .. } = ctz_latch_step(&mut f, 13) {
            *dest = Value(15);
            *ty = IrType::I16;
        }
        let latch = f.blocks.iter_mut().find(|b| b.label == BlockId(3)).unwrap();
        latch.instructions.push(Instruction::Cast {
            dest: Value(13),
            src: val(15),
            from_ty: IrType::I64,
            to_ty: IrType::I32,
        });
        assert_eq!(run_function(&mut f), 0, "unpromoted truncated step refused");
        assert!(!has_ctz_nonzero(&f));
    }

    #[test]
    fn ctz_shift_by_two_is_kept() {
        let mut f = ctz_func(true);
        if let Instruction::BinOp { rhs, .. } = ctz_latch_step(&mut f, 12) {
            *rhs = Operand::Const(IrConst::I64(2));
        }
        assert_eq!(run_function(&mut f), 0, "x >>= 2 does not count ctz");
        assert!(!has_ctz_nonzero(&f));
    }

    fn popcnt_while_func(init_count: i32) -> IrFunction {
        let mut f = IrFunction::new("popcnt_test".into(), IrType::I32, vec![], false);
        f.blocks = vec![
            block(
                0,
                vec![Instruction::ParamRef {
                    dest: Value(1),
                    param_idx: 0,
                    ty: IrType::U64,
                }],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![
                    Instruction::Phi {
                        dest: Value(10),
                        ty: IrType::U64,
                        incoming: vec![(val(1), BlockId(0)), (val(12), BlockId(2))],
                    },
                    Instruction::Phi {
                        dest: Value(11),
                        ty: IrType::I32,
                        incoming: vec![(i32c(init_count), BlockId(0)), (val(13), BlockId(2))],
                    },
                ],
                Terminator::CondBranch {
                    cond: val(10),
                    true_label: BlockId(2),
                    false_label: BlockId(3),
                },
            ),
            block(
                2,
                vec![
                    Instruction::BinOp {
                        dest: Value(14),
                        op: IrBinOp::Sub,
                        lhs: val(10),
                        rhs: Operand::Const(IrConst::I64(1)),
                        ty: IrType::U64,
                    },
                    Instruction::BinOp {
                        dest: Value(12),
                        op: IrBinOp::And,
                        lhs: val(10),
                        rhs: val(14),
                        ty: IrType::U64,
                    },
                    Instruction::BinOp {
                        dest: Value(13),
                        op: IrBinOp::Add,
                        lhs: val(11),
                        rhs: i32c(1),
                        ty: IrType::I32,
                    },
                ],
                Terminator::Branch(BlockId(1)),
            ),
            block(3, vec![], Terminator::Return(Some(val(11)))),
        ];
        f.next_value_id = 20;
        f
    }

    fn has_popcount(f: &IrFunction) -> bool {
        f.blocks.iter().any(|b| {
            b.instructions.iter().any(|i| {
                matches!(
                    i,
                    Instruction::UnaryOp {
                        op: IrUnaryOp::Popcount,
                        ..
                    }
                )
            })
        })
    }

    #[test]
    fn popcount_while_loop_rewrites_when_fast_popcount_enabled() {
        let mut f = popcnt_while_func(0);
        assert_eq!(run_function(&mut f), 1);
        assert!(has_popcount(&f));
        assert_eq!(f.blocks.len(), 2);
    }

    #[test]
    fn popcount_while_loop_adds_nonzero_initial_count() {
        let mut f = popcnt_while_func(7);
        assert_eq!(run_function(&mut f), 1);
        assert!(has_popcount(&f));
        let has_add = f.blocks.iter().any(|b| {
            b.instructions.iter().any(|i| {
                matches!(
                    i,
                    Instruction::BinOp {
                        op: IrBinOp::Add,
                        lhs: Operand::Const(IrConst::I32(7)),
                        ty: IrType::I32,
                        ..
                    }
                )
            })
        });
        assert!(has_add);
    }

    #[test]
    fn popcount_while_loop_refused_when_fast_popcount_disabled() {
        let mut f = popcnt_while_func(0);
        assert_eq!(
            super::run_function(
                &mut f,
                &ModuleSymbolFacts::default(),
                Capabilities::default()
            ),
            0
        );
        assert!(!has_popcount(&f));
    }

    #[test]
    fn popcount_while_loop_refused_when_not_clear_lsb() {
        let mut f = popcnt_while_func(0);
        if let Instruction::BinOp { rhs, .. } = &mut f.blocks[2].instructions[0] {
            *rhs = Operand::Const(IrConst::I64(2));
        }
        assert_eq!(run_function(&mut f), 0);
        assert!(!has_popcount(&f));
    }
    mod hardening {
        include!("loop_idiom_hardening_tests.rs");
    }
}
