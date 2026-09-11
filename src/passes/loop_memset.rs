//! Loop-memset recognition: constant-byte fill loops become a `memset`
//! libcall.
//!
//! Companion to [`crate::passes::loop_idiom`] (byte-COPY loops → `memcpy`,
//! which runs early in the pipeline): this pass covers the FILL idiom that
//! pass does not — `for (i) buf[i] = c;` with a compile-time-constant `c`.
//! A stored load result is refused here by construction (census), so the
//! two passes can never claim the same loop.
//!
//! Recognized shapes (both canonical counted-loop forms the pipeline
//! produces):
//! - guard-at-top while-form: header (iv phis + guard compare) + one
//!   straight-line body/latch block branching back unconditionally,
//! - test-at-bottom do-while: self-loop body + separate rotate guard.
//!
//! Soundness contracts (details at each gate):
//! - single straight-line body; census = phis, iv `+1` updates, guard/exit
//!   compare, GEP/cast/const-affine address helpers, exactly one store of a
//!   constant whose every stored byte is equal (0 at any width; nonzero
//!   uniform patterns like `0xAAAAAAAA` fill each byte with `0xAA`);
//! - store address affine in a loop iv (GEP scale == store width for the
//!   indexed form, or a +1-advancing pointer iv);
//! - exact trip count `n = bound - init` for `Ult`/`Ne` exits, static
//!   overflow gate, dynamic `n != 0` guard around the call;
//! - exit values reconstructed (`exit iv → bound`, `other ivs → init + n`)
//!   with mechanical external-use checking.
//!
//! Nested loops stay allowed: unlike the nested-COPY case (where the
//! per-run call barrier evicted enclosing-loop register homes and the
//! transform measured +12.6M Ir on lz4's literal copy), fills are usually
//! cold initialization code and the measured lz4/hash_table effect is a net
//! win. Kill switches: `CCC_NO_MEMSET_LOOP`, per-gate refusal tracing via
//! `CCC_MEMSET_LOOP_TRACE`. -O2+.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::analysis::CfgAnalysis;
use crate::ir::reexports::{
    BasicBlock, CallInfo, Instruction, IrBinOp, IrCmpOp, IrConst, IrFunction, Operand, Terminator,
    Value,
};
use crate::passes::loop_analysis;

/// Per-gate refusal tracing (`CCC_MEMSET_LOOP_TRACE=1`): makes it diagnosable
/// why a real-world loop fails to match the fill idiom.
fn trace_enabled() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var("CCC_MEMSET_LOOP_TRACE").is_ok_and(|v| v != "0"))
}

macro_rules! refuse {
    ($($arg:tt)*) => {{
        if trace_enabled() {
            eprintln!("[loop-memset] refuse: {}", format_args!($($arg)*));
        }
        return None;
    }};
}

/// Variant name of an instruction, for diagnostics only.
trait ShortName {
    fn short_name(&self) -> &'static str;
}
impl ShortName for Instruction {
    fn short_name(&self) -> &'static str {
        // Coarse class via matches! (Debug would be too verbose for traces).
        if matches!(self, Instruction::Phi { .. }) {
            "Phi"
        } else if matches!(self, Instruction::BinOp { .. }) {
            "BinOp"
        } else if matches!(self, Instruction::Cmp { .. }) {
            "Cmp"
        } else if matches!(self, Instruction::Load { .. }) {
            "Load"
        } else if matches!(self, Instruction::Store { .. }) {
            "Store"
        } else if matches!(self, Instruction::GetElementPtr { .. }) {
            "GetElementPtr"
        } else if matches!(self, Instruction::Cast { .. }) {
            "Cast"
        } else if matches!(self, Instruction::Copy { .. }) {
            "Copy"
        } else if matches!(self, Instruction::Call { .. }) {
            "Call"
        } else {
            "other"
        }
    }
}

/// Run to a small fixed point (a transformed loop cannot expose a new one,
/// but the loop bounds the sweep defensively, mirroring sibling passes).
pub fn run(func: &mut IrFunction) -> usize {
    let mut total = 0;
    for _ in 0..4 {
        let n = run_once(func);
        if n == 0 {
            break;
        }
        total += n;
    }
    total
}

fn run_once(func: &mut IrFunction) -> usize {
    let Some(plan) = find_idiom(func) else {
        return 0;
    };
    apply_idiom(func, plan)
}

// ── Recognition structures ───────────────────────────────────────────────────

/// How the loop addresses memory.
#[derive(Clone, Debug, PartialEq)]
enum AddrForm {
    /// `GEP(base, scale*iv + base_off)` — element-wise addressing; `scale`
    /// is in bytes and must equal the memory-access width.
    Indexed {
        base: Value,
        scale: i64,
        base_off: i64,
    },
    /// A phi stepping +1 byte per iteration.
    Advancing { init: Value },
}

/// Which canonical counted-loop shape the pipeline presented.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Form {
    /// test-at-bottom: self-loop body + separate rotate guard.
    DoWhile,
    /// guard-at-top: header (phis + guard cmp) + single straight-line
    /// body/latch block branching back unconditionally. This is the shape
    /// the default (non-`CCC_LOOP_ROTATE`) pipeline produces.
    While,
}

/// A recognized induction variable: phi with a (+1) update.
#[derive(Clone, Debug)]
struct Iv {
    /// The phi value itself.
    phi: Value,
    /// Operand carried into the loop on the guard edge.
    init: Operand,
    /// The per-iteration update value (the Add(phi, 1) dest).
    next: Value,
    /// The phi's IR type (Ptr for advancing pointers).
    ty: IrType,
}

impl Iv {
    fn is_pointer(&self) -> bool {
        self.ty == IrType::Ptr
    }
    /// True when `v` is this iv's phi or its per-iteration update value.
    fn matches(&self, v: Value) -> bool {
        v.0 == self.phi.0 || self.next.0 == v.0
    }
}

#[derive(Clone, Debug)]
struct Plan {
    /// Which loop shape this plan rewrites.
    form: Form,
    /// DoWhile: index of the self-loop body block (removed).
    /// While: index of the guard header block (removed).
    header_idx: usize,
    /// While only: index of the single straight-line body/latch block
    /// (removed). DoWhile: same as `header_idx`.
    body_idx: usize,
    /// Index of the rotation-guard block (DoWhile only).
    guard_idx: usize,
    /// Index of the guard's predecessor whose edge is rerouted.
    pre_idx: usize,
    /// Index of the exit block.
    exit_idx: usize,
    /// The iv the exit compare uses (counter or advancing pointer).
    exit_iv: Iv,
    /// Additional recognized iv (at most one more), for exit-value
    /// reconstruction.
    other_iv: Option<Iv>,
    /// While-form: every iv used outside the loop (reconstructed at the
    /// exit). DoWhile: empty (E-phi incomings already carry the values).
    used_ivs: Vec<Iv>,
    /// Exit compare opcode on the header.
    exit_op: IrCmpOp,
    /// The loop-invariant bound operand.
    bound: Operand,
    /// Store side addressing.
    store_addr: AddrForm,
    /// The fill byte and the store width.
    fill_byte: u8,
    fill_width: u32,
    /// While-form, loop-defined invariant-global bound: (original bound
    /// value, global address value, loaded type). The rewrite re-materializes
    /// the load in the guard block and substitutes it for the original.
    bound_remat: Option<(Value, Value, IrType)>,
}

fn find_idiom(func: &IrFunction) -> Option<Plan> {
    let cfg = CfgAnalysis::build(func);
    let loops = loop_analysis::merge_loops_by_header(loop_analysis::find_natural_loops(
        cfg.num_blocks,
        &cfg.preds,
        &cfg.succs,
        &cfg.idom,
    ));

    // Candidates in deterministic block order; the first fully-validated
    // single-block loop wins (at most one transform per sweep keeps the
    // block-index bookkeeping trivial).
    if trace_enabled() {
        eprintln!(
            "[loop-memset] fn='{}': {} natural loop(s), {} self-loop candidate(s)",
            func.name,
            loops.len(),
            loops.iter().filter(|lp| lp.is_self_loop()).count()
        );
    }
    let mut headers: Vec<usize> = loops
        .iter()
        .filter(|lp| lp.is_self_loop())
        .map(|lp| lp.header)
        .collect();
    headers.sort_unstable();

    for header_idx in headers {
        if let Some(plan) = try_recognize(func, header_idx, &cfg) {
            return Some(plan);
        }
    }

    // Guard-at-top 2-block loops: header + single straight-line body block.
    let mut pairs: Vec<(usize, usize)> = loops
        .iter()
        .filter(|lp| lp.body.len() == 2)
        .map(|lp| {
            let other = lp
                .body
                .iter()
                .copied()
                .find(|&b| b != lp.header)
                .unwrap_or(lp.header);
            (lp.header, other)
        })
        .collect();
    pairs.sort_unstable();
    for (header_idx, body_idx) in pairs {
        // Nested inside an enclosing loop?  An enclosing natural loop must
        // contain the entire inner loop.
        let nested = loops.iter().any(|lp| {
            (lp.header != header_idx || lp.body.len() != 2)
                && lp.contains(header_idx)
                && lp.contains(body_idx)
        });
        if let Some(plan) = try_recognize_while(func, header_idx, body_idx, nested, &cfg) {
            return Some(plan);
        }
    }
    None
}

fn try_recognize(func: &IrFunction, header_idx: usize, cfg: &CfgAnalysis) -> Option<Plan> {
    let header_label = func.blocks[header_idx].label;

    // Exactly one in-loop edge (the self-loop) and exactly one exit edge.
    let mut self_edges = 0usize;
    let mut exit_idx = None;
    for &succ in cfg.succs.row(header_idx) {
        let s = succ as usize;
        if s == header_idx {
            self_edges += 1;
        } else if exit_idx.is_none() {
            exit_idx = Some(s);
        } else if exit_idx != Some(s) {
            refuse!("multiple exit edges");
        }
    }
    if self_edges != 1 {
        refuse!("{} self-edge(s), need exactly 1", self_edges);
    }
    let exit_idx = exit_idx?;

    // Latch check: the terminator condition must be a Cmp on an IV.
    let Terminator::CondBranch {
        cond: Operand::Value(cval),
        true_label,
        false_label,
    } = &func.blocks[header_idx].terminator
    else {
        refuse!("header does not end in CondBranch");
    };
    if *true_label != header_label && *false_label != header_label {
        refuse!("terminator does not branch to itself (not a do-while latch)");
    }

    let defs = collect_defs(func);
    let defined_in_header = |v: Value| defs.get(&v.0).is_some_and(|&b| b == header_idx);

    let (cmp_op, cmp_lhs, cmp_rhs, cmp_ty) =
        func.blocks[header_idx]
            .instructions
            .iter()
            .find_map(|inst| {
                if let Instruction::Cmp {
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty,
                } = inst
                {
                    if dest.0 == cval.0 {
                        return Some((*op, lhs.clone(), rhs.clone(), *ty));
                    }
                }
                None
            })?;
    if cmp_ty != IrType::Ptr && !cmp_ty.is_integer() {
        refuse!("latch condition is not an integer/pointer compare");
    }

    // IV recognition: counters and byte-advancing pointers, (+1) phis.
    let ivs = recognize_ivs(
        func,
        header_idx,
        func.blocks[header_idx].label,
        header_idx,
        &defs,
    );
    if ivs.is_empty() {
        refuse!("no (+1) induction phis in header");
    }

    // Which compare side is a recognized iv, and is the bound invariant?
    let iv_of = |op: &Operand| -> Option<usize> {
        match op {
            Operand::Value(v) => ivs.iter().position(|iv| iv.matches(*v)),
            _ => None,
        }
    };
    let (iv_pos, bound) = match (iv_of(&cmp_lhs), iv_of(&cmp_rhs)) {
        (Some(s), None) => (s, cmp_rhs.clone()),
        (None, Some(s)) => (s, cmp_lhs.clone()),
        _ => refuse!("latch compare is not iv-vs-invariant (both or neither side is an iv)"),
    };
    let bound_invariant = match &bound {
        Operand::Const(_) => true,
        Operand::Value(v) => !defined_in_header(*v),
    };
    if !bound_invariant {
        refuse!("loop bound is defined inside the header");
    }
    // Supported exit forms: the iv stops exactly at `bound` (Ne) or first
    // reaches it from below (Ult). Ugt with a +1 iv never exits; signed
    // forms are refused (ivs here are unsigned or pointers).
    if !matches!(cmp_op, IrCmpOp::Ne | IrCmpOp::Ult) {
        refuse!("unsupported exit predicate {:?} (want Ne/Ult)", cmp_op);
    }

    // Rotate-guard signature: a separate block branching { header, exit }
    // whose compare uses the SAME (op, bound) as the latch and whose iv-side
    // operand is the iv's init value (the latch compares the updated iv).
    let guard_idx = find_rotate_guard(
        func,
        cfg,
        header_idx,
        exit_idx,
        cmp_op,
        &cmp_lhs,
        &cmp_rhs,
        &ivs[iv_pos],
    )
    .map_err(|e| {
        if trace_enabled() {
            eprintln!("[loop-memset] refuse: no rotate guard ({e})");
        }
    })
    .ok()?;

    // The guard's predecessor whose edge we reroute: any non-header pred.
    let pre_idx = cfg
        .preds
        .row(guard_idx)
        .iter()
        .map(|&p| p as usize)
        .find(|&p| p != header_idx)
        .inspect(|&p| {
            if trace_enabled() {
                eprintln!("[loop-memset] guard pred = B{}", p);
            }
        })
        .ok_or("guard has no non-header predecessor")
        .map_err(|e| {
            if trace_enabled() {
                eprintln!("[loop-memset] refuse: {e}");
            }
        })
        .ok()?;

    // Body census: classify every non-phi instruction.
    let body = census_body(func, header_idx, &ivs, &defs)
        .map_err(|e| {
            if trace_enabled() {
                eprintln!("[loop-memset] refuse: body census ({e})");
            }
        })
        .ok()?;

    // The exit compare must be consumed ONLY by the terminator (it dies
    // with the header block).
    {
        let uses = count_uses(func);
        if uses.get(&cval.0).copied() != Some(1) {
            refuse!(
                "exit compare has {} use(s), need exactly 1 (the terminator)",
                uses.get(&cval.0).copied().unwrap_or(0)
            );
        }
    }

    // Mechanical external-use check: in the rotated shape no header-def may
    // be used outside the header except through an E-phi (·, H) incoming.
    check_external_uses(func, header_idx, exit_idx)?;

    // Address forms + idiom classification.
    let (store_addr, store_width, store_byte) =
        resolve_memory_ops(func, header_idx, &body, &ivs, &defs)
            .map_err(|e| {
                if trace_enabled() {
                    eprintln!("[loop-memset] refuse: memory-op resolution ({e})");
                }
            })
            .ok()?;

    // Fill classification. Store width must equal the GEP scale (indexed
    // form) so the elements are contiguous; the fill byte is 0 for zero
    // stores at any width, or a uniform byte for width-1 stores.
    if let AddrForm::Indexed { scale, .. } = store_addr {
        if scale != store_width as i64 {
            refuse!(
                "memset GEP scale {scale} != store width {} (strided fill)",
                store_width
            );
        }
    }

    // Exit-value reconstruction feasibility: every E-phi's (·, H) incoming
    // must be a recognized iv.
    for inst in &func.blocks[exit_idx].instructions {
        if let Instruction::Phi { incoming, .. } = inst {
            let Some((from_h, _)) = incoming.iter().find(|(_, b)| *b == header_label) else {
                // E is a successor of H, so every phi must carry the edge;
                // a missing entry is an IR invariant violation — refuse.
                refuse!("exit phi missing (·, H) incoming (IR invariant violation)");
            };
            let Operand::Value(v) = from_h else {
                refuse!("exit phi's (·, H) incoming is a constant");
            };
            if !ivs.iter().any(|iv| iv.phi.0 == v.0) {
                refuse!("exit phi carries a non-iv value out of the loop");
            }
        }
    }

    // Overflow gate for statically-known trip counts (module docs).
    if let (Operand::Const(k), Operand::Const(i)) = (&bound, &ivs[iv_pos].init) {
        if let (Some(kv), Some(iv0)) = (const_to_u64(k), const_to_u64(i)) {
            let width_bits = (cmp_ty.size() as u64) * 8;
            if width_bits > 0 && width_bits <= 64 {
                let modulus = 1u64 << width_bits;
                let n = match cmp_op {
                    IrCmpOp::Ne => kv.wrapping_sub(iv0) % modulus,
                    _ => kv.checked_sub(iv0).unwrap_or(0),
                };
                if n > 0 && n > u64::MAX / u64::from(store_width) {
                    refuse!("static trip count {n} overflows the byte-range of the call");
                }
            }
        }
    }

    let exit_iv = ivs[iv_pos].clone();
    let other_iv = ivs.iter().find(|iv| iv.phi.0 != exit_iv.phi.0).cloned();

    Some(Plan {
        form: Form::DoWhile,
        header_idx,
        body_idx: header_idx,
        guard_idx,
        pre_idx,
        exit_idx,
        exit_iv,
        other_iv,
        used_ivs: Vec::new(),
        exit_op: cmp_op,
        bound,
        store_addr,
        fill_byte: store_byte,
        fill_width: store_width,
        bound_remat: None,
    })
}

// ── Small IR utilities ───────────────────────────────────────────────────────

/// Values defined per block index.
fn collect_defs(func: &IrFunction) -> FxHashMap<u32, usize> {
    let mut map = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            if let Some(d) = inst.dest() {
                map.insert(d.0, bi);
            }
        }
    }
    map
}

/// Operand-value reference counts across instructions and terminators.
fn count_uses(func: &IrFunction) -> FxHashMap<u32, u32> {
    let mut uses = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            for id in inst.used_values() {
                *uses.entry(id).or_insert(0) += 1;
            }
        }
        for id in block.terminator.used_values() {
            *uses.entry(id).or_insert(0) += 1;
        }
    }
    uses
}

/// Mechanical external-use verification (see `try_recognize`): every use of
/// a header-defined value outside the header must sit in an E-phi incoming
/// keyed on the header edge.
fn check_external_uses(func: &IrFunction, header_idx: usize, exit_idx: usize) -> Option<()> {
    let header_label = func.blocks[header_idx].label;
    let defs = collect_defs(func);

    // E-phi (·, header) operands are the sanctioned post-loop channels.
    let mut e_phi_header_uses: FxHashSet<u32> = FxHashSet::default();
    for inst in &func.blocks[exit_idx].instructions {
        if let Instruction::Phi { incoming, .. } = inst {
            for (op, blk) in incoming {
                if *blk == header_label {
                    if let Operand::Value(v) = op {
                        e_phi_header_uses.insert(v.0);
                    }
                }
            }
        }
    }

    for (bi, block) in func.blocks.iter().enumerate() {
        let in_exit_phi = |id: u32| bi == exit_idx && e_phi_header_uses.contains(&id);
        for inst in &block.instructions {
            for id in inst.used_values() {
                if defs.get(&id) == Some(&header_idx) && bi != header_idx && !in_exit_phi(id) {
                    return None;
                }
            }
        }
        // Terminator uses never legitimately reference header defs (the
        // header's own terminator uses the header-defined compare — allow
        // exactly that one by construction).
        if bi != header_idx {
            for id in block.terminator.used_values() {
                if defs.get(&id) == Some(&header_idx) {
                    return None;
                }
            }
        }
    }
    Some(())
}

/// Recognize induction variables: phis stepping +1 per iteration. At most
/// one counter and one pointer; anything more returns an empty vector.
fn recognize_ivs(
    func: &IrFunction,
    phi_block: usize,
    update_label: crate::ir::reexports::BlockId,
    update_block: usize,
    defs: &FxHashMap<u32, usize>,
) -> Vec<Iv> {
    let mut ivs: Vec<Iv> = Vec::new();

    for inst in &func.blocks[phi_block].instructions {
        let Instruction::Phi { dest, ty, incoming } = inst else {
            continue;
        };
        if incoming.len() != 2 {
            continue;
        }
        let Some(&(init, _)) = incoming.iter().find(|(_, b)| *b != update_label) else {
            continue;
        };
        let Some(&(next, _)) = incoming.iter().find(|(_, b)| *b == update_label) else {
            continue;
        };
        let Operand::Value(nv) = &next else { continue };
        if defs.get(&nv.0) != Some(&update_block) {
            continue;
        }
        let Some(Instruction::BinOp {
            op: IrBinOp::Add,
            lhs: Operand::Value(lv),
            rhs: Operand::Const(r),
            ..
        }) = func.blocks[update_block]
            .instructions
            .iter()
            .find(|i| i.dest().is_some_and(|d| d.0 == nv.0))
        else {
            continue;
        };
        if lv.0 != dest.0 || const_to_u64(r) != Some(1) {
            continue;
        }
        // Only unsigned/pointer ivs (the exit forms are Ne/Ult).
        if *ty == IrType::Ptr || ty.is_integer() {
            ivs.push(Iv {
                phi: *dest,
                init,
                next: *nv,
                ty: *ty,
            });
        }
    }
    let ptrs = ivs.iter().filter(|iv| iv.is_pointer()).count();
    if ptrs > 1 || ivs.len() > 2 {
        return Vec::new();
    }
    ivs
}

/// Recognize the guard-at-top while-form: header H carries the iv phis and
/// the guard compare (`CondBranch { true: B, false: E }`), B is a single
/// straight-line body block ending in an unconditional branch back to H.
/// `for (i = init; i <u bound; i++) { gep/store/copy ops }`.
fn try_recognize_while(
    func: &IrFunction,
    header_idx: usize,
    body_idx: usize,
    nested: bool,
    cfg: &CfgAnalysis,
) -> Option<Plan> {
    let header_label = func.blocks[header_idx].label;
    let body_label = func.blocks[body_idx].label;

    // Header shape: CondBranch with the body as ONE target.
    let Terminator::CondBranch {
        cond: Operand::Value(cval),
        true_label,
        false_label,
    } = &func.blocks[header_idx].terminator
    else {
        return None;
    };
    let (body_target, exit_idx) = if *true_label == body_label {
        (
            *true_label,
            cfg.succs
                .row(header_idx)
                .iter()
                .copied()
                .map(|s| s as usize)
                .find(|&s| s != body_idx && s != header_idx)?,
        )
    } else if *false_label == body_label {
        (
            *false_label,
            cfg.succs
                .row(header_idx)
                .iter()
                .copied()
                .map(|s| s as usize)
                .find(|&s| s != body_idx && s != header_idx)?,
        )
    } else {
        return None;
    };
    let _ = body_target;

    // Body shape: single pred (the header), unconditional branch back.
    if cfg.preds.row(body_idx).len() != 1 || cfg.preds.row(body_idx)[0] as usize != header_idx {
        refuse!(
            "body block has {} pred(s), need exactly the header",
            cfg.preds.row(body_idx).len()
        );
    }
    if !matches!(func.blocks[body_idx].terminator, Terminator::Branch(l) if l == header_label) {
        refuse!("body block does not branch straight back to the header");
    }
    // The exit must be reachable ONLY through the header.
    if cfg.preds.row(exit_idx).len() != 1 || cfg.preds.row(exit_idx)[0] as usize != header_idx {
        refuse!(
            "exit block has {} pred(s), need exactly the header",
            cfg.preds.row(exit_idx).len()
        );
    }

    let defs = collect_defs(func);
    let defined_in_loop = |v: Value| {
        defs.get(&v.0)
            .is_some_and(|&b| b == header_idx || b == body_idx)
    };

    // Guard compare: iv vs loop-invariant bound.
    let Some((cmp_op, cmp_lhs, cmp_rhs, cmp_ty)) = func.blocks[header_idx]
        .instructions
        .iter()
        .find_map(|inst| match inst {
            Instruction::Cmp {
                dest,
                op,
                lhs,
                rhs,
                ty,
            } if dest.0 == cval.0 => Some((*op, lhs.clone(), rhs.clone(), *ty)),
            _ => None,
        })
    else {
        refuse!("header condition is not an SSA compare");
    };
    if cmp_ty != IrType::Ptr && !cmp_ty.is_integer() {
        refuse!("guard compare is not integer/pointer");
    }

    // IVs: phis in the header updated in the body block.
    let ivs = recognize_ivs(func, header_idx, body_label, body_idx, &defs);
    if ivs.is_empty() {
        refuse!("no (+1) induction phis (while-form)");
    }
    let iv_of = |op: &Operand| -> Option<usize> {
        match op {
            Operand::Value(v) => ivs.iter().position(|iv| iv.matches(*v)),
            _ => None,
        }
    };
    let (iv_pos, bound) = match (iv_of(&cmp_lhs), iv_of(&cmp_rhs)) {
        (Some(s), None) => (s, cmp_rhs.clone()),
        (None, Some(s)) => (s, cmp_lhs.clone()),
        _ => refuse!("guard compare is not iv-vs-invariant"),
    };
    let bound_invariant = match &bound {
        Operand::Const(_) => true,
        Operand::Value(v) => !defined_in_loop(*v),
    };
    // A loop-DEFINED bound may still be usable: a Load of a loop-invariant
    // named global (globals are not SSA-promoted, so `i < N` re-loads `N`
    // in the header). Validated after the store address resolves (the
    // disjointness argument needs the fill's root object).
    let bound_loop_defined = !bound_invariant;
    if !matches!(cmp_op, IrCmpOp::Ne | IrCmpOp::Ult) {
        refuse!("unsupported guard predicate {:?} (want Ne/Ult)", cmp_op);
    }

    // Body census: straight-line fill/copy work only.
    let body = census_body(func, body_idx, &ivs, &defs)
        .map_err(|e| {
            if trace_enabled() {
                eprintln!("[loop-memset] refuse: body census ({e})");
            }
        })
        .ok()?;

    // The guard compare must be consumed only by the header terminator.
    {
        let uses = count_uses(func);
        if uses.get(&cval.0).copied() != Some(1) {
            refuse!(
                "guard compare has {} use(s), need exactly 1",
                uses.get(&cval.0).copied().unwrap_or(0)
            );
        }
    }

    // Address forms (GEPs live in the body, iv phis in the header).
    let (store_addr, store_width, store_byte) =
        resolve_memory_ops(func, body_idx, &body, &ivs, &defs)
            .map_err(|e| {
                if trace_enabled() {
                    eprintln!("[loop-memset] refuse: memory-op resolution ({e})");
                }
            })
            .ok()?;

    // Fill classification. Store width must equal the GEP scale (indexed
    // form) so the elements are contiguous; the fill byte is 0 for zero
    // stores at any width, or a uniform byte for width-1 stores.
    if let AddrForm::Indexed { scale, .. } = store_addr {
        if scale != store_width as i64 {
            refuse!(
                "memset GEP scale {scale} != store width {} (strided fill)",
                store_width
            );
        }
    }

    // Loop-defined bound validation (see the marker above): allowed only as
    // a Load of a loop-invariant named global that the loop provably cannot
    // write. The rewrite deletes the header, so the transform re-loads the
    // global in the guard block (exact: nothing in the loop or the call
    // writes the object).
    let mut bound_remat: Option<(Value, Value, IrType)> = None;
    if bound_loop_defined {
        let Operand::Value(bv) = &bound else {
            refuse!("bound {:?} is a non-value loop definition", bound);
        };
        let def = func.blocks[header_idx]
            .instructions
            .iter()
            .chain(func.blocks[body_idx].instructions.iter())
            .find(|i| i.dest().is_some_and(|d| d.0 == bv.0));
        let Some(Instruction::Load { ptr, ty, .. }) = def else {
            refuse!("bound {:?} is a non-load loop definition", bound);
        };
        let defs_all = collect_defs(func);
        if defs_all
            .get(&ptr.0)
            .is_some_and(|&b| b == header_idx || b == body_idx)
        {
            refuse!("bound load address is loop-defined");
        }
        let Some(Instruction::GlobalAddr {
            name: bound_name, ..
        }) = func
            .blocks
            .iter()
            .flat_map(|b| b.instructions.iter())
            .find(|i| i.dest().is_some_and(|d| d.0 == ptr.0))
        else {
            refuse!("bound load does not read a named global directly");
        };
        let store_root = match &store_addr {
            AddrForm::Indexed { base, .. } => Some(*base),
            AddrForm::Advancing { init } => Some(*init),
        };
        let distinct = match store_root {
            Some(root) => match defs_all.get(&root.0).copied() {
                None => true, // defined entirely outside the loop
                Some(bi) if bi != header_idx && bi != body_idx => {
                    let root_def = func.blocks[bi]
                        .instructions
                        .iter()
                        .find(|i| i.dest().is_some_and(|dd| dd.0 == root.0));
                    match root_def {
                        Some(Instruction::GlobalAddr { name, .. }) => name != bound_name,
                        Some(Instruction::Alloca { .. }) => true,
                        _ => false,
                    }
                }
                Some(_) => false, // root defined inside the loop: unresolved
            },
            None => false,
        };
        if !distinct {
            refuse!("bound global may alias the fill target");
        }
        bound_remat = Some((*bv, *ptr, *ty));
    }

    // External uses: every loop-defined value used outside {H, B} must be a
    // recognized iv phi (reconstructible as init / init + n at the exit).
    let mut used_ivs: Vec<usize> = Vec::new();
    {
        let mut fail = |what: &str| {
            if trace_enabled() {
                eprintln!(
                    "[loop-memset] refuse: external use of non-iv {} (while-form)",
                    what
                );
            }
        };
        for (bi, block) in func.blocks.iter().enumerate() {
            if bi == header_idx || bi == body_idx {
                continue;
            }
            for inst in &block.instructions {
                if let Instruction::Phi { incoming, .. } = inst {
                    for (op, blk) in incoming {
                        if *blk == header_label {
                            if let Operand::Value(v) = op {
                                if defined_in_loop(*v) {
                                    match ivs.iter().position(|iv| iv.phi.0 == v.0) {
                                        Some(p) => {
                                            if !used_ivs.contains(&p) {
                                                used_ivs.push(p);
                                            }
                                        }
                                        None => {
                                            fail("phi incoming");
                                            return None;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                for id in inst.used_values() {
                    if defined_in_loop(Value(id)) {
                        match ivs.iter().position(|iv| iv.phi.0 == id) {
                            Some(p) => {
                                if !used_ivs.contains(&p) {
                                    used_ivs.push(p);
                                }
                            }
                            None => {
                                fail("use");
                                return None;
                            }
                        }
                    }
                }
            }
            for id in block.terminator.used_values() {
                if defined_in_loop(Value(id)) {
                    match ivs.iter().position(|iv| iv.phi.0 == id) {
                        Some(p) => {
                            if !used_ivs.contains(&p) {
                                used_ivs.push(p);
                            }
                        }
                        None => {
                            fail("terminator use");
                            return None;
                        }
                    }
                }
            }
        }
    }

    // Overflow gate for statically-known trip counts.
    if let (Operand::Const(k), Operand::Const(i)) = (&bound, &ivs[iv_pos].init) {
        if let (Some(kv), Some(iv0)) = (const_to_u64(k), const_to_u64(i)) {
            let width_bits = (cmp_ty.size() as u64) * 8;
            if width_bits > 0 && width_bits <= 64 {
                let modulus = 1u64 << width_bits;
                let n = match cmp_op {
                    IrCmpOp::Ne => kv.wrapping_sub(iv0) % modulus,
                    _ => kv.checked_sub(iv0).unwrap_or(0),
                };
                if n > 0 && n > u64::MAX / u64::from(store_width) {
                    refuse!("static trip count {n} overflows the byte-range of the call");
                }
            }
        }
    }

    let pre_idx = cfg
        .preds
        .row(header_idx)
        .iter()
        .map(|&p| p as usize)
        .find(|&p| p != body_idx)?;
    let exit_iv = ivs[iv_pos].clone();

    Some(Plan {
        form: Form::While,
        header_idx,
        body_idx,
        guard_idx: usize::MAX,
        pre_idx,
        exit_idx,
        exit_iv,
        other_iv: None,
        used_ivs: used_ivs.into_iter().map(|p| ivs[p].clone()).collect(),
        exit_op: cmp_op,
        bound,
        store_addr,
        fill_byte: store_byte,
        fill_width: store_width,
        bound_remat,
    })
}

/// Find the rotation-guard block: branches {H, E}, compare uses the SAME
/// (op, bound) as the latch, and its iv-side operand is the iv's init value.
fn find_rotate_guard(
    func: &IrFunction,
    cfg: &CfgAnalysis,
    header_idx: usize,
    exit_idx: usize,
    op: IrCmpOp,
    latch_lhs: &Operand,
    latch_rhs: &Operand,
    iv: &Iv,
) -> Result<usize, String> {
    let header_label = func.blocks[header_idx].label;
    let exit_label = func.blocks[exit_idx].label;
    // The latch's bound side (the operand that is NOT the updated iv).
    let latch_bound = if matches!(latch_lhs, Operand::Value(v) if iv.matches(*v)) {
        latch_rhs
    } else {
        latch_lhs
    };

    let mut saw_branch = 0usize;
    let mut saw_cmp_mismatch = false;
    for &p in cfg.preds.row(header_idx) {
        let p = p as usize;
        if p == header_idx {
            continue;
        }
        let b = &func.blocks[p];
        let Terminator::CondBranch {
            cond: Operand::Value(cv),
            true_label,
            false_label,
        } = &b.terminator
        else {
            continue;
        };
        let rotated_targets = (*true_label == header_label && *false_label == exit_label)
            || (*true_label == exit_label && *false_label == header_label);
        if !rotated_targets {
            continue;
        }
        saw_branch += 1;
        let Some((gop, glhs, grhs)) = b.instructions.iter().find_map(|i| match i {
            Instruction::Cmp {
                dest, op, lhs, rhs, ..
            } if dest.0 == cv.0 => Some((*op, lhs.clone(), rhs.clone())),
            _ => None,
        }) else {
            saw_cmp_mismatch = true;
            continue;
        };
        if gop != op {
            saw_cmp_mismatch = true;
            continue;
        }
        // Same bound operand on either side; the OTHER side must be the
        // iv's init operand (the value the phi carries on the guard edge).
        for (g_iv, g_bound) in [(&glhs, &grhs), (&grhs, &glhs)] {
            if g_bound == latch_bound && (*g_iv == iv.init || copy_of(g_iv, &iv.init, b)) {
                return Ok(p);
            }
        }
        saw_cmp_mismatch = true;
    }
    if saw_branch == 0 {
        Err("no predecessor branches {header, exit}".to_string())
    } else if saw_cmp_mismatch {
        Err("guard compare does not share (op, bound, iv-init) with the latch".to_string())
    } else {
        Err("no rotate guard found".to_string())
    }
}

/// True when `op` is `Value(x)` with x defined in `block` as a Copy of the
/// given operand (phi-elimination artifacts in late pipelines).
fn copy_of(op: &Operand, of: &Operand, block: &BasicBlock) -> bool {
    let Operand::Value(v) = op else {
        return false;
    };
    block.instructions.iter().any(|i| {
        matches!(
            i,
            Instruction::Copy {
                dest,
                src,
                ..
            } if dest.0 == v.0 && src == of
        )
    })
}

/// Classified body of the self-loop block.
#[derive(Clone, Debug, Default)]
struct BodyCensus {
    store: Option<usize>,
    /// The stored constant byte (None until the store is classified).
    store_byte: Option<u8>,
}

fn census_body(
    func: &IrFunction,
    block_idx: usize,
    ivs: &[Iv],
    defs: &FxHashMap<u32, usize>,
) -> Result<BodyCensus, String> {
    let header = &func.blocks[block_idx];
    let mut c = BodyCensus {
        store: None,
        store_byte: None,
    };

    for (ii, inst) in header.instructions.iter().enumerate() {
        match inst {
            Instruction::Phi { .. } => {}
            Instruction::BinOp {
                dest: _,
                op: IrBinOp::Add,
                lhs: Operand::Value(v),
                rhs: Operand::Const(r),
                ..
            } if const_to_u64(r) == Some(1) && ivs.iter().any(|iv| iv.phi.0 == v.0) => {}
            // Address-affine helpers (`iv << 2`, `iv * 4`): validated for
            // real by resolve_affine when the store address resolves; any
            // that end up unused die with the removed block.
            Instruction::BinOp {
                op: IrBinOp::Shl | IrBinOp::Mul,
                rhs: Operand::Const(_),
                ..
            } => {}
            Instruction::Cmp { .. } => {}
            Instruction::GetElementPtr { .. } => {}
            Instruction::Cast { .. } => {}
            // A FILL loop reads no memory: any load refuses (this keeps the
            // pass disjoint from the loop_idiom memcpy pass, whose loops
            // store a load result).
            Instruction::Load { .. } => {
                return Err("load in fill body (not a constant fill)".to_string());
            }
            Instruction::Store {
                val: Operand::Const(cv),
                ty,
                volatile,
                ..
            } => {
                if c.store.is_some() || *volatile {
                    return Err("multiple/volatile stores".to_string());
                }
                let Some(raw) = const_to_u64(cv) else {
                    return Err("store constant not an integer".to_string());
                };
                // Interpret the constant at the STORE width: the IR's
                // integer constants are signed carriers (I8(0xAA) is -86),
                // so the byte pattern is the low `ty.size()` bytes. A fill
                // is exact iff every stored byte is equal (0 is always
                // memset-able; a nonzero uniform pattern like 0xAAAAAAAA
                // fills each byte with 0xAA — GCC converts the same shape).
                let width = ty.size() as u64;
                if width == 0 || width > 8 {
                    return Err(format!("store width {width} is not an integer size"));
                }
                let mask = if width == 8 {
                    u64::MAX
                } else {
                    (1u64 << (width * 8)) - 1
                };
                let b = raw & mask;
                if b != 0 && (1..width).any(|sh| (b >> (sh * 8)) as u8 != b as u8) {
                    return Err(format!(
                        "store constant {b:#x} is not a memset-able byte pattern"
                    ));
                }
                c.store = Some(ii);
                c.store_byte = Some(b as u8);
            }
            Instruction::Store { .. } => {
                return Err(
                    "stored value is not a constant (copy loop — loop_idiom's job)".to_string(),
                );
            }
            _ => {
                let name = inst.short_name();
                return Err(format!("unrecognized body instruction #{ii}: {name}"));
            }
        }
    }
    if c.store.is_none() {
        return Err("no store in loop body".to_string());
    }
    Ok(c)
}

fn const_to_u64(c: &IrConst) -> Option<u64> {
    // The IR's integer constants are all signed carriers; the phi/compare
    // types define the unsigned interpretation.
    match c {
        IrConst::I8(v) => Some(*v as u64),
        IrConst::I16(v) => Some(*v as u64),
        IrConst::I32(v) => Some(*v as u64),
        IrConst::I64(v) => Some(*v as u64),
        IrConst::I128(v) => u64::try_from(*v).ok(),
        IrConst::Zero => Some(0),
        _ => None,
    }
}

fn const_to_i64(c: &IrConst) -> Option<i64> {
    match c {
        IrConst::I8(v) => Some(i64::from(*v)),
        IrConst::I16(v) => Some(i64::from(*v)),
        IrConst::I32(v) => Some(i64::from(*v)),
        IrConst::I64(v) => Some(*v),
        IrConst::I128(v) => i64::try_from(*v).ok(),
        IrConst::Zero => Some(0),
        _ => None,
    }
}

/// Resolve memory op addressing. Returns
/// (store_addr, store_width, store_const_byte). Fill loops have no load.
fn resolve_memory_ops(
    func: &IrFunction,
    op_block: usize,
    body: &BodyCensus,
    ivs: &[Iv],
    defs: &FxHashMap<u32, usize>,
) -> Result<(AddrForm, u32, u8), String> {
    let store_ii = body
        .store
        .ok_or_else(|| "census lost the store".to_string())?;
    let Instruction::Store {
        ptr, ty: store_ty, ..
    } = &func.blocks[op_block].instructions[store_ii]
    else {
        return Err("census index not a store".to_string());
    };
    let store_const_byte = body
        .store_byte
        .ok_or_else(|| "census lost the store constant".to_string())?;

    let store_addr = resolve_addr(func, op_block, ptr, ivs, defs, "store")?;
    Ok((store_addr, store_ty.size() as u32, store_const_byte))
}

/// Resolve a pointer operand into an [`AddrForm`].
fn resolve_addr(
    func: &IrFunction,
    op_block: usize,
    ptr: &Value,
    ivs: &[Iv],
    defs: &FxHashMap<u32, usize>,
    which: &str,
) -> Result<AddrForm, String> {
    let v = ptr;
    // Advancing-pointer form: the iv phi itself (lives in the loop header,
    // which for the while-form is a different block than the body ops).
    if let Some(iv) = ivs.iter().find(|iv| iv.phi.0 == v.0) {
        if !iv.is_pointer() {
            return Err(format!("{which} advancing iv is not a pointer"));
        }
        let Operand::Value(init) = iv.init else {
            return Err(format!("{which} pointer iv init is a constant"));
        };
        if defs.get(&init.0) == Some(&op_block) {
            return Err(format!("{which} pointer iv init defined inside the loop"));
        }
        return Ok(AddrForm::Advancing { init });
    }
    if defs.get(&v.0) != Some(&op_block) {
        return Err(format!("{which} address is not defined inside the loop"));
    }
    let def = func.blocks[op_block]
        .instructions
        .iter()
        .find(|i| i.dest().is_some_and(|d| d.0 == v.0))
        .ok_or_else(|| format!("{which} address has no def in the loop"))?;

    match def {
        // Indexed form: GEP(base, offs) with offs affine in a counter iv.
        Instruction::GetElementPtr { base, offset, .. } => {
            let bv = *base;
            if defs.get(&bv.0) == Some(&op_block) {
                // An op-block-defined base is only the advancing phi
                // (handled above); GEP chains refuse.
                return Err(format!("{which} GEP base is loop-defined (GEP chain)"));
            }
            let (scale, off) = resolve_affine(func, op_block, offset, ivs, defs, 0)
                .ok_or_else(|| format!("{which} GEP offset is not affine in a counter iv"))?;
            if !matches!(scale, 1 | 2 | 4 | 8) {
                return Err(format!("{which} GEP scale {scale} is not 1/2/4/8"));
            }
            Ok(AddrForm::Indexed {
                base: bv,
                scale,
                base_off: off,
            })
        }
        _ => Err(format!(
            "{which} address is neither an iv phi nor a GEP: {}",
            def.short_name()
        )),
    }
}

/// Resolve `op` as `scale*counter + base` with recursion fuel.
fn resolve_affine(
    func: &IrFunction,
    header_idx: usize,
    op: &Operand,
    ivs: &[Iv],
    defs: &FxHashMap<u32, usize>,
    fuel: u8,
) -> Option<(i64, i64)> {
    if fuel > 6 {
        return None;
    }
    match op {
        Operand::Const(c) => Some((0, const_to_i64(c)?)),
        Operand::Value(v) => {
            if ivs.iter().any(|iv| iv.phi.0 == v.0) {
                return Some((1, 0));
            }
            if defs.get(&v.0) != Some(&header_idx) {
                // Loop-invariant offsets live in the GEP base, not here.
                return None;
            }
            let def = func.blocks[header_idx]
                .instructions
                .iter()
                .find(|i| i.dest().is_some_and(|d| d.0 == v.0))?;
            match def {
                // Widening/narrowing casts of the counter are transparent.
                Instruction::Cast { src, .. } => {
                    resolve_affine(func, header_idx, src, ivs, defs, fuel + 1)
                }
                Instruction::BinOp {
                    op: IrBinOp::Shl,
                    lhs,
                    rhs: Operand::Const(k),
                    ..
                } => {
                    let k = const_to_i64(k)?;
                    if !(0..4).contains(&k) {
                        return None;
                    }
                    let (s, b) = resolve_affine(func, header_idx, lhs, ivs, defs, fuel + 1)?;
                    if s != 0 && b != 0 {
                        return None;
                    }
                    Some((s << k, b << k))
                }
                Instruction::BinOp {
                    op: IrBinOp::Mul,
                    lhs,
                    rhs: Operand::Const(k),
                    ..
                } => {
                    let k = const_to_i64(k)?;
                    let (s, b) = resolve_affine(func, header_idx, lhs, ivs, defs, fuel + 1)?;
                    Some((s.checked_mul(k)?, b.checked_mul(k)?))
                }
                Instruction::BinOp {
                    op: IrBinOp::Add,
                    lhs,
                    rhs,
                    ..
                } => {
                    let (s1, b1) = resolve_affine(func, header_idx, lhs, ivs, defs, fuel + 1)?;
                    let (s2, b2) = resolve_affine(func, header_idx, rhs, ivs, defs, fuel + 1)?;
                    if s1 != 0 && s2 != 0 {
                        return None;
                    }
                    Some((s1 + s2, b1 + b2))
                }
                _ => None,
            }
        }
    }
}

/// Pending-instruction builder: accumulates the guard block's instruction
/// sequence and hands it over as one vector.
struct Emitter {
    pending: Vec<Instruction>,
    next_id: u32,
}

impl Emitter {
    fn new(func: &IrFunction) -> Self {
        Self {
            pending: Vec::new(),
            next_id: func.next_value_id,
        }
    }

    fn value(&mut self) -> Value {
        let v = Value(self.next_id);
        self.next_id += 1;
        v
    }

    fn push(&mut self, inst: Instruction) -> Value {
        let d = inst.dest().expect("emitter instructions define a dest");
        self.pending.push(inst);
        d
    }

    fn cast(&mut self, src: Operand, from_ty: IrType, to_ty: IrType) -> Value {
        let dest = self.value();
        self.push(Instruction::Cast {
            dest,
            src,
            from_ty,
            to_ty,
        })
    }

    fn binop(&mut self, op: IrBinOp, lhs: Operand, rhs: Operand, ty: IrType) -> Value {
        let dest = self.value();
        self.push(Instruction::BinOp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        })
    }

    fn cmp(&mut self, op: IrCmpOp, lhs: Operand, rhs: Operand, ty: IrType) -> Value {
        let dest = self.value();
        self.push(Instruction::Cmp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        })
    }

    fn select(
        &mut self,
        cond: Operand,
        true_val: Operand,
        false_val: Operand,
        ty: IrType,
    ) -> Value {
        let dest = self.value();
        self.push(Instruction::Select {
            dest,
            cond,
            true_val,
            false_val,
            ty,
        })
    }

    fn load(&mut self, ptr: Value, ty: IrType) -> Value {
        let dest = self.value();
        self.push(Instruction::Load {
            dest,
            ptr,
            ty,
            seg_override: crate::common::types::AddressSpace::Default,
            volatile: false,
        })
    }

    fn gep(&mut self, base: Value, offset: Operand, ty: IrType) -> Value {
        let dest = self.value();
        self.push(Instruction::GetElementPtr {
            dest,
            base,
            offset,
            ty,
        })
    }

    fn take(&mut self) -> Vec<Instruction> {
        std::mem::take(&mut self.pending)
    }
}

fn zero_const(_ty: IrType) -> IrConst {
    IrConst::Zero
}

fn apply_idiom(func: &mut IrFunction, plan: Plan) -> usize {
    let size_ty = crate::common::types::target_int_ir_type();
    let ptr_math_ty = IrType::I64;
    let mut em = Emitter::new(func);

    // ── G: trip count from the exit compare (exactness: module docs) ────
    // A loop-defined invariant-global bound is re-loaded here (the original
    // load died with the removed header; nothing in the loop or the call
    // writes the object, so the reload is exact).
    let bound_eff = if let Some((_orig, gptr, lty)) = plan.bound_remat {
        Operand::Value(em.load(gptr, lty))
    } else {
        plan.bound.clone()
    };
    let iv = &plan.exit_iv;
    let iv_arith_ty = if iv.is_pointer() { ptr_math_ty } else { iv.ty };
    let init_arith = match &iv.init {
        Operand::Value(v) => Operand::Value(em.cast(Operand::Value(*v), iv.ty, iv_arith_ty)),
        Operand::Const(c) => {
            if iv.is_pointer() {
                Operand::Const(IrConst::I64(const_to_i64(c).unwrap_or(0)))
            } else {
                iv.init.clone()
            }
        }
    };
    let bound_arith = match &bound_eff {
        Operand::Value(v) => Operand::Value(em.cast(Operand::Value(*v), iv.ty, iv_arith_ty)),
        Operand::Const(c) => {
            if iv.is_pointer() {
                Operand::Const(IrConst::I64(const_to_i64(c).unwrap_or(0)))
            } else {
                plan.bound.clone()
            }
        }
    };
    let diff = em.binop(
        IrBinOp::Sub,
        bound_arith.clone(),
        init_arith.clone(),
        iv_arith_ty,
    );
    let n_val = match plan.exit_op {
        IrCmpOp::Ne => diff,
        _ => {
            // Ult with a zero init needs no select: n = bound - 0 = bound,
            // and bound == 0 yields n == 0, which the M-guard skips — the
            // select's only job was clamping the init > bound case.
            let zero_init = match &iv.init {
                Operand::Const(c) => const_to_u64(c) == Some(0),
                _ => false,
            };
            if zero_init {
                let zero = Operand::Const(zero_const(iv_arith_ty));
                em.binop(IrBinOp::Sub, bound_arith.clone(), zero, iv_arith_ty)
            } else {
                let c = em.cmp(
                    IrCmpOp::Ult,
                    init_arith.clone(),
                    bound_arith.clone(),
                    iv_arith_ty,
                );
                em.select(
                    Operand::Value(c),
                    Operand::Value(diff),
                    Operand::Const(zero_const(iv_arith_ty)),
                    iv_arith_ty,
                )
            }
        }
    };
    let n_size = if iv_arith_ty == size_ty {
        n_val
    } else {
        em.cast(Operand::Value(n_val), iv_arith_ty, size_ty)
    };

    // ── G: byte count + start addresses ──────────────────────────────────
    let bytes = if plan.fill_width == 1 {
        n_size
    } else {
        em.binop(
            IrBinOp::Mul,
            Operand::Value(n_size),
            Operand::Const(IrConst::I64(i64::from(plan.fill_width))),
            size_ty,
        )
    };

    let mut start_of = |em: &mut Emitter, form: &AddrForm| -> Value {
        match form {
            AddrForm::Indexed { base, base_off, .. } => {
                if *base_off == 0 {
                    *base
                } else {
                    em.gep(*base, Operand::Const(IrConst::I64(*base_off)), IrType::Ptr)
                }
            }
            AddrForm::Advancing { init } => *init,
        }
    };
    let dst0 = start_of(&mut em, &plan.store_addr);
    // Call shape mirrors the frontend's memset lowering (lower.rs
    // emit_memset_zero): args [dst, byte(I32), n(size_ty)].
    let call_args = vec![
        Operand::Value(dst0),
        Operand::Const(IrConst::I32(i32::from(plan.fill_byte))),
        Operand::Value(bytes),
    ];
    let callee = "memset";

    // ── G: the n != 0 guard ──────────────────────────────────────────────
    let nz = em.cmp(
        IrCmpOp::Ne,
        Operand::Value(n_size),
        Operand::Const(zero_const(size_ty)),
        size_ty,
    );
    let guard_insts = em.take();

    if plan.form == Form::DoWhile {
        // ── G: exit-value reconstructions (dominate M and E) ─────────────────
        // Exit iv: post-loop value == bound. Other ivs: init + n.
        let mut exit_values: Vec<(u32, Operand)> = Vec::new();
        exit_values.push((plan.exit_iv.phi.0, plan.bound.clone()));
        if let Some(other) = &plan.other_iv {
            let other_ty = if other.is_pointer() {
                ptr_math_ty
            } else {
                other.ty
            };
            let init_a = match &other.init {
                Operand::Value(v) => {
                    Operand::Value(em.cast(Operand::Value(*v), other.ty, other_ty))
                }
                Operand::Const(c) => {
                    if other.is_pointer() {
                        Operand::Const(IrConst::I64(const_to_i64(c).unwrap_or(0)))
                    } else {
                        other.init.clone()
                    }
                }
            };
            let n_a = if other_ty == size_ty {
                n_size
            } else {
                em.cast(Operand::Value(n_size), size_ty, other_ty)
            };
            let summed = em.binop(IrBinOp::Add, init_a, Operand::Value(n_a), other_ty);
            let final_val = if other_ty == other.ty {
                summed
            } else {
                em.cast(Operand::Value(summed), other_ty, other.ty)
            };
            exit_values.push((other.phi.0, Operand::Value(final_val)));
        }
        let recon_insts = em.take();
        let mut guard_insts = guard_insts;
        guard_insts.extend(recon_insts);

        // ── Block surgery ────────────────────────────────────────────────────
        let guard_label = {
            let l = func.next_label;
            func.next_label += 1;
            crate::ir::reexports::BlockId(l)
        };
        let call_label = {
            let l = func.next_label;
            func.next_label += 1;
            crate::ir::reexports::BlockId(l)
        };

        let header_idx = plan.header_idx;
        let guard_idx = plan.guard_idx;
        let pre_idx = plan.pre_idx;
        let exit_idx = plan.exit_idx;
        let pre_label = func.blocks[pre_idx].label;
        let header_label = func.blocks[header_idx].label;
        let exit_label = func.blocks[exit_idx].label;
        let binit_label = func.blocks[guard_idx].label;

        // Snapshot the E-phi (·, H) operands BEFORE mutating (they name the iv
        // each phi carried out of the loop).
        let mut e_phi_iv: Vec<(u32, u32)> = Vec::new(); // (E-phi dest, iv phi id)
        for inst in &func.blocks[exit_idx].instructions {
            if let Instruction::Phi { dest, incoming, .. } = inst {
                for (op, blk) in incoming {
                    if *blk == header_label {
                        if let Operand::Value(v) = op {
                            e_phi_iv.push((dest.0, v.0));
                        }
                    }
                }
            }
        }

        // P: Branch -> B_init  becomes  Branch -> G.
        func.blocks[pre_idx].terminator = Terminator::Branch(guard_label);

        // B_init phis: rekey the preheader edge (·, P) to (·, G).
        for inst in &mut func.blocks[guard_idx].instructions {
            if let Instruction::Phi { incoming, .. } = inst {
                for (_, blk) in incoming.iter_mut() {
                    if *blk == pre_label {
                        *blk = guard_label;
                    }
                }
            }
        }
        // B_init: the (dead) edge into H retargets to E so the header removal
        // below leaves every terminator target defined.
        if let Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } = &mut func.blocks[guard_idx].terminator
        {
            if *true_label == header_label {
                *true_label = exit_label;
            }
            if *false_label == header_label {
                *false_label = exit_label;
            }
        }

        // E phis: drop the (·, H) incoming; add the reconstructed (·, M) one.
        for inst in &mut func.blocks[exit_idx].instructions {
            if let Instruction::Phi { dest, incoming, .. } = inst {
                incoming.retain(|(_, blk)| *blk != header_label);
                if let Some((_, iv_phi)) = e_phi_iv.iter().find(|(d, _)| *d == dest.0) {
                    if let Some((_, val)) = exit_values.iter().find(|(id, _)| id == iv_phi) {
                        incoming.push((val.clone(), call_label));
                    }
                }
            }
        }

        // G block: n computation + exit-value reconstructions + CondBranch.
        func.blocks.push(BasicBlock {
            label: guard_label,
            instructions: guard_insts,
            terminator: Terminator::CondBranch {
                cond: Operand::Value(nz),
                true_label: call_label,
                false_label: binit_label,
            },
            source_spans: Vec::new(),
        });

        // M block: the call + Branch -> E.
        let call_dest = em.value();
        func.blocks.push(BasicBlock {
            label: call_label,
            instructions: vec![Instruction::Call {
                func: callee.to_string(),
                info: CallInfo {
                    dest: Some(call_dest),
                    args: call_args,
                    arg_types: vec![IrType::Ptr, IrType::I32, size_ty],
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
            }],
            terminator: Terminator::Branch(exit_label),
            source_spans: Vec::new(),
        });

        // Remove the header block (last: all indices above are pre-removal).
        func.blocks.remove(header_idx);
        func.next_value_id = em.next_id;
        1
    } else {
        // ── G: exit-value reconstructions (dominate M and E) ─────────────────
        // While-form: EVERY iv used outside the loop is reconstructed. The exit
        // iv's post-call value is the bound (Ult/Ne both stop exactly there);
        // other ivs end at init + n.
        let mut exit_values: Vec<(u32, Operand)> = Vec::new();
        for iv in &plan.used_ivs {
            let val = if iv.phi.0 == plan.exit_iv.phi.0 {
                bound_eff.clone()
            } else {
                let other_ty = if iv.is_pointer() { ptr_math_ty } else { iv.ty };
                let init_a = match &iv.init {
                    Operand::Value(v) => {
                        Operand::Value(em.cast(Operand::Value(*v), iv.ty, other_ty))
                    }
                    Operand::Const(c) => {
                        if iv.is_pointer() {
                            Operand::Const(IrConst::I64(const_to_i64(c).unwrap_or(0)))
                        } else {
                            iv.init.clone()
                        }
                    }
                };
                let n_a = if other_ty == size_ty {
                    n_size
                } else {
                    em.cast(Operand::Value(n_size), size_ty, other_ty)
                };
                let summed = em.binop(IrBinOp::Add, init_a, Operand::Value(n_a), other_ty);
                let final_val = if other_ty == iv.ty {
                    summed
                } else {
                    em.cast(Operand::Value(summed), other_ty, iv.ty)
                };
                Operand::Value(final_val)
            };
            exit_values.push((iv.phi.0, val));
        }
        let recon_insts = em.take();
        let mut guard_insts = guard_insts;
        guard_insts.extend(recon_insts);

        // ── Block surgery (while-form) ───────────────────────────────────────
        let guard_label = {
            let l = func.next_label;
            func.next_label += 1;
            crate::ir::reexports::BlockId(l)
        };
        let call_label = {
            let l = func.next_label;
            func.next_label += 1;
            crate::ir::reexports::BlockId(l)
        };

        let header_idx = plan.header_idx;
        let body_idx = plan.body_idx;
        let pre_idx = plan.pre_idx;
        let exit_idx = plan.exit_idx;
        let header_label = func.blocks[header_idx].label;
        let exit_label = func.blocks[exit_idx].label;

        // P: Branch -> H  becomes  Branch -> G.
        func.blocks[pre_idx].terminator = Terminator::Branch(guard_label);

        // G block: n computation + reconstructions + CondBranch (n != 0?).
        func.blocks.push(BasicBlock {
            label: guard_label,
            instructions: guard_insts,
            terminator: Terminator::CondBranch {
                cond: Operand::Value(nz),
                true_label: call_label,
                false_label: exit_label,
            },
            source_spans: Vec::new(),
        });

        // M block: the call + Branch -> E.
        let call_dest = em.value();
        func.blocks.push(BasicBlock {
            label: call_label,
            instructions: vec![Instruction::Call {
                func: callee.to_string(),
                info: CallInfo {
                    dest: Some(call_dest),
                    args: call_args,
                    arg_types: vec![IrType::Ptr, IrType::I32, size_ty],
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
            }],
            terminator: Terminator::Branch(exit_label),
            source_spans: Vec::new(),
        });

        // E phis: the header was E's only predecessor, so every existing phi
        // carries a single (·, H) incoming naming an iv — rehome it to the
        // (init · G) / (reconstruction · M) pair.
        let mut covered: FxHashMap<u32, ()> = FxHashMap::default();
        for inst in &mut func.blocks[exit_idx].instructions {
            if let Instruction::Phi { incoming, .. } = inst {
                let Some(pos) = incoming.iter().position(|(_, blk)| *blk == header_label) else {
                    continue;
                };
                let Operand::Value(v) = incoming[pos].0 else {
                    continue;
                };
                if let Some((_, val)) = exit_values.iter().find(|(id, _)| *id == v.0) {
                    incoming.clear();
                    if let Some(iv) = plan.used_ivs.iter().find(|iv| iv.phi.0 == v.0) {
                        incoming.push((iv.init.clone(), guard_label));
                    }
                    incoming.push((val.clone(), call_label));
                    covered.insert(v.0, ());
                }
            }
        }

        // Missing phis: ivs used directly (no E-phi existed) get one now, then
        // all uses outside {H, B} are remapped to the new values.
        let mut new_phis: Vec<(u32, u32)> = Vec::new(); // (iv phi id, new phi dest)
        for iv in &plan.used_ivs {
            if covered.contains_key(&iv.phi.0) {
                continue;
            }
            let dest = em.value();
            let val = &exit_values
                .iter()
                .find(|(id, _)| *id == iv.phi.0)
                .expect("recon exists for every used iv")
                .1;
            new_phis.push((iv.phi.0, dest.0));
            func.blocks[exit_idx].instructions.insert(
                0,
                Instruction::Phi {
                    dest,
                    ty: iv.ty,
                    incoming: vec![(iv.init.clone(), guard_label), (val.clone(), call_label)],
                },
            );
        }
        if !new_phis.is_empty() {
            for bi in 0..func.blocks.len() {
                if bi == header_idx || bi == body_idx {
                    continue;
                }
                let block = &mut func.blocks[bi];
                for inst in &mut block.instructions {
                    if matches!(inst, Instruction::Phi { .. }) {
                        // Existing E phis were rehomed above; their operands no
                        // longer name iv phis.
                        if let Instruction::Phi { incoming, .. } = inst {
                            for (op, _) in incoming.iter_mut() {
                                if let Operand::Value(v) = op {
                                    if let Some(&(_, dest)) =
                                        new_phis.iter().find(|(id, _)| *id == v.0)
                                    {
                                        *op = Operand::Value(Value(dest));
                                    }
                                }
                            }
                        }
                        continue;
                    }
                    inst.for_each_operand_mut(|op| {
                        if let Operand::Value(v) = op {
                            if let Some(&(_, dest)) = new_phis.iter().find(|(id, _)| *id == v.0) {
                                *op = Operand::Value(Value(dest));
                            }
                        }
                    });
                }
                block.terminator.for_each_operand_mut(|op| {
                    if let Operand::Value(v) = op {
                        if let Some(&(_, dest)) = new_phis.iter().find(|(id, _)| *id == v.0) {
                            *op = Operand::Value(Value(dest));
                        }
                    }
                });
            }
        }

        // Remove the loop blocks (higher index first: indices are pre-removal).
        if body_idx > header_idx {
            func.blocks.remove(body_idx);
            func.blocks.remove(header_idx);
        } else {
            func.blocks.remove(header_idx);
            func.blocks.remove(body_idx);
        }
        func.next_value_id = em.next_id;
        1
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::reexports::BlockId;

    fn val(id: u32) -> Operand {
        Operand::Value(Value(id))
    }
    fn i64c(v: i64) -> Operand {
        Operand::Const(IrConst::I64(v))
    }
    fn block(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions,
            terminator,
            source_spans: Vec::new(),
        }
    }

    /// Zero-fill loop over a global byte array:
    ///   P -> B_init -> H (self) -> E
    /// `for (i = 0; i < N; i++) buf[i] = 0;` (rotated form, U64 counter).
    fn zero_loop_func(n: i64) -> IrFunction {
        // values: 1 = buf base (GlobalAddr), 2 = GEP, 3 = Cmp(H),
        // 4 = i+1, 10 = i phi, 11 = Cmp(guard), 20 = E phi of i
        let mut f = IrFunction::new("zero_loop".into(), IrType::I32, vec![], false);
        f.next_value_id = 30;
        f.next_label = 4;
        f.blocks.push(block(
            0, // P
            vec![Instruction::GlobalAddr {
                dest: Value(1),
                name: "buf".into(),
            }],
            Terminator::Branch(BlockId(1)),
        ));
        f.blocks.push(block(
            1, // B_init
            vec![Instruction::Cmp {
                dest: Value(11),
                op: IrCmpOp::Ult,
                lhs: i64c(0),
                rhs: i64c(n),
                ty: IrType::U64,
            }],
            Terminator::CondBranch {
                cond: val(11),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
        ));
        f.blocks.push(block(
            2, // H (self-loop)
            vec![
                Instruction::Phi {
                    dest: Value(10),
                    ty: IrType::U64,
                    incoming: vec![(i64c(0), BlockId(1)), (val(4), BlockId(2))],
                },
                Instruction::GetElementPtr {
                    dest: Value(2),
                    base: Value(1),
                    offset: val(10),
                    ty: IrType::Ptr,
                },
                Instruction::Store {
                    val: i64c(0),
                    ptr: Value(2),
                    ty: IrType::U8,
                    seg_override: crate::common::types::AddressSpace::Default,
                    volatile: false,
                },
                Instruction::BinOp {
                    dest: Value(4),
                    op: IrBinOp::Add,
                    lhs: val(10),
                    rhs: i64c(1),
                    ty: IrType::U64,
                },
                Instruction::Cmp {
                    dest: Value(3),
                    op: IrCmpOp::Ult,
                    lhs: val(4),
                    rhs: i64c(n),
                    ty: IrType::U64,
                },
            ],
            Terminator::CondBranch {
                cond: val(3),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
        ));
        f.blocks.push(block(
            3, // E
            vec![Instruction::Phi {
                dest: Value(20),
                ty: IrType::U64,
                incoming: vec![(i64c(0), BlockId(1)), (val(10), BlockId(2))],
            }],
            Terminator::Return(Some(val(20))),
        ));
        f
    }

    #[test]
    fn zero_loop_is_recognized_and_rewritten() {
        let mut f = zero_loop_func(1024);
        let n = run(&mut f);
        assert_eq!(n, 1, "the zero loop must transform");

        // The header block is gone; G and M exist; P branches to G.
        assert!(!f.blocks.iter().any(|b| b.label.0 == 2), "H removed");
        let g = f.blocks.iter().find(|b| b.label.0 == 4).expect("G block");
        assert!(matches!(g.terminator, Terminator::CondBranch { .. }));
        // G computes n = N - 0 and guards on n != 0.
        assert!(g.instructions.iter().any(|i| matches!(
            i,
            Instruction::BinOp {
                op: IrBinOp::Sub,
                ..
            }
        )));
        let m = f.blocks.iter().find(|b| b.label.0 == 5).expect("M block");
        assert!(
            m.instructions
                .iter()
                .any(|i| matches!(i, Instruction::Call { func, .. } if func == "memset")),
            "M must call memset"
        );
        // E phi carries the exit value (bound) on the M edge.
        let e = &f.blocks[2];
        let Instruction::Phi { incoming, .. } = &e.instructions[0] else {
            panic!("E phi");
        };
        assert!(
            incoming
                .iter()
                .any(|(op, b)| b.0 == 5
                    && matches!(op, Operand::Const(IrConst::I64(v)) if *v == 1024)),
            "E phi must carry the bound on the M edge, got {:?}",
            incoming
        );
        // B_init's dead edge retargets to E and its phis rekeyed.
        assert!(matches!(
            f.blocks[1].terminator,
            Terminator::CondBranch { true_label, false_label, .. }
                if true_label.0 == 3 && false_label.0 == 3
        ));
    }

    #[test]
    fn do_while_without_guard_is_refused() {
        // Same as zero_loop_func but B_init is a plain branch (no rotate
        // signature) — the loop could be a zero-trip-unsafe do-while.
        let mut f = zero_loop_func(1024);
        f.blocks[1].terminator = Terminator::Branch(BlockId(2));
        assert_eq!(run(&mut f), 0, "unguarded loop must refuse");
    }

    #[test]
    fn copy_loop_is_refused() {
        // `for (i) d[i] = s[i];` — the stored value is a LOAD result, which
        // the fill census refuses (loop_idiom's memcpy job).
        let mut f = zero_loop_func(16);
        // Rework the body: Load v5 = [v3]; Store [v2] = v5 instead of const.
        let hdr = &mut f.blocks[2];
        hdr.instructions
            .retain(|inst| !matches!(inst, Instruction::Store { .. }));
        hdr.instructions.push(Instruction::Load {
            dest: Value(5),
            ptr: Value(3),
            ty: IrType::U8,
            seg_override: crate::common::types::AddressSpace::Default,
            volatile: false,
        });
        hdr.instructions.push(Instruction::Store {
            val: val(5),
            ptr: Value(2),
            ty: IrType::U8,
            seg_override: crate::common::types::AddressSpace::Default,
            volatile: false,
        });
        assert_eq!(run(&mut f), 0, "copy loop must refuse");
    }

    #[test]
    fn nonzero_uniform_byte_fill_transforms() {
        // `for (i) buf[i] = 0xAA;` — width-1 uniform nonzero fill.
        let mut f = zero_loop_func(16);
        for inst in &mut f.blocks[2].instructions {
            if let Instruction::Store { val, .. } = inst {
                *val = Operand::Const(IrConst::I8(i8::from_be_bytes([0xAA])));
            }
        }
        assert_eq!(run(&mut f), 1, "uniform byte fill must transform");
    }

    #[test]
    fn non_uniform_wide_fill_is_refused() {
        // `((u32) 0x00AA00AA)`-style wide nonzero pattern: width != 1 and
        // nonzero refuses (memset byte semantics).
        let mut f = zero_loop_func(16);
        for inst in &mut f.blocks[2].instructions {
            if let Instruction::Store { val, ty, .. } = inst {
                *val = Operand::Const(IrConst::I64(0x00AA00AA));
                *ty = IrType::U32;
            }
        }
        assert_eq!(run(&mut f), 0, "wide nonzero pattern must refuse");
    }

    #[test]
    fn non_constant_store_is_refused() {
        let mut f = zero_loop_func(16);
        // Store value 5 (not all-same-byte beyond width 1): still fine for
        // width-1; use a nonzero width-4 const instead.
        if let Some(h) = f.blocks.iter_mut().find(|b| b.label.0 == 2) {
            for inst in &mut h.instructions {
                if let Instruction::Store { val, ty, .. } = inst {
                    *val = i64c(0x01020304);
                    *ty = IrType::U32;
                }
            }
        }
        assert_eq!(run(&mut f), 0, "non-memset-able constant must refuse");
    }
}
