//! Loop-idiom recognition: byte-store loops become `memset(3)`, byte-copy
//! loops become `memcpy(3)`.
//!
//! WHY THIS EXISTS
//! ---------------
//! A byte-granular fill/copy loop (`for (i = 0; i < n; i++) table[i] = 0;`,
//! `while (d < e) *d++ = *s++;`) is the single largest code-quality gap a
//! compiler without loop-idiom recognition has against GCC/Clang: GCC's
//! `-ftree-loop-distribute-patterns` rewrites both shapes into libc calls
//! whose inlined backend expansion runs 8-32 bytes per cycle (ERMSB/FSRM
//! `rep movsb/stosb`), while the scalar loop costs 4-8 instructions per
//! BYTE. Measured on the workload corpus (2026-09-11, callgrind dynamic
//! instruction counts): lz4_compress ran 3.79M instructions per compression
//! pass against GCC's 630K — the literal-copy loop and the hash-table
//! zero-fill loop alone account for the whole 3.3x runtime gap, and neither
//! is reachable by any other pass in this tree (the byte loop is not
//! vectorizable — the trip count is dynamic and the exit is a pointer/
//! counter compare — and no load/store specialization applies).
//!
//! The transform is deliberately CONSERVATIVE; every gate below is a
//! soundness contract, not a heuristic:
//!
//! # Recognized shape (rotated single-block loop)
//!
//! ```text
//!   P:  ...                          Branch -> B_init
//!   B_init: Cmp c0 = (iv op bound)   CondBranch { H, E }     // rotate guard
//!   H:      phis...; body...; Cmp c1 = (iv op bound)
//!          CondBranch { E, H }       // do-while self-loop
//!   E:      phis merge exit values
//! ```
//!
//! exactly what `loop_rotate` + `loop_invert` produce for every `while`
//! loop. The rotation signature on `B_init` (same compare triple, targets
//! `{H, E}`) is REQUIRED: it proves the body never runs on a zero-trip
//! entry, which is what makes the guard rewrite below semantics-preserving.
//! A do-while without the guard is refused (its first iteration would be
//! lost by the rewrite).
//!
//! # Rewrite
//!
//! ```text
//!   P:  ...                       Branch -> G
//!   G:  n = trip_count(iv, bound)          // exact, no counting loop
//!       Cmp cg = (n != 0)         CondBranch { M, B_init }
//!   M:  Call memcpy(dst0, src0, n) / memset(dst0, val0, n)  Branch -> E
//!   B_init: ...                   CondBranch { E, E }     // never taken
//!   H:  REMOVED
//!   E:  phis lose the (·, H) incoming, gain (exit-value, M)
//! ```
//!
//! `n` is computed directly from the exit compare — no trip-counting loop:
//! for a `Ne` bound `n = bound - init` (mod 2^width, exact because the iv
//! steps by 1 and stops exactly at `bound`); for an `Ult` bound
//! `n = (init <ult bound) ? bound - init : 0`. The `B_init` path keeps the
//! exact zero-trip semantics (its own check fails and E is entered with the
//! init values), and the `M` path reproduces the post-loop state through
//! the reconstructed exit values below.
//!
//! # Exit-value reconstruction
//!
//! In the rotated shape the guard's `B_init -> E` edge bypasses `H`, so no
//! value defined in `H` can dominate a use outside `H` except through an
//! E-phi (the pass verifies this mechanically before rewriting). For each
//! E-phi the `(·, H)` incoming — the post-loop value — is replaced by an
//! `(exit-value, M)` incoming:
//!
//! * the iv the exit compare uses → `bound` (both supported exit forms stop
//!   with `iv == bound` after `n >= 1` iterations);
//! * any other recognized iv → `init + n` (one past the last element);
//! * anything else → the whole transform is refused.
//!
//! All reconstructed values are defined in `G`, which dominates both `M`
//! and `E` in the rewritten graph.
//!
//! # Address forms
//!
//! The store (and, for copies, the load) address must be one of:
//!
//! * **Indexed**: `GEP(base, offs)` with `offs` affine in a counter iv
//!   (`counter`, `Cast(counter)`, `Cast << k`, `Cast * c`, sums thereof),
//!   `base` defined outside the loop. The byte scale must equal the
//!   memory-access width (element-wise semantics); memsets may use any
//!   width (a zero element is zero bytes), copies are byte-width only.
//! * **Advancing**: a phi stepping `+1` (bytes) per iteration.
//!
//! # Alias / legality gates
//!
//! * body instructions ⊆ {phis, IV updates, exit Cmp, address GEPs/Casts,
//!   the one Load (copies), the one Store} — calls, allocas, intrinsics,
//!   atomics, volatiles, inline asm, extra memory ops: refuse;
//! * copies: load base and store base must be *distinct objects* (distinct
//!   allocas, distinct globals, or alloca vs global). Unknown roots
//!   (parameters, loaded pointers) refuse — `memcpy` has no-overlap
//!   semantics while the byte loop is a correct forward `memmove`;
//! * the store value must be a compile-time constant (0 for any width, an
//!   all-same-byte constant for width-1 stores) or exactly the load's
//!   result (copy);
//! * a trip count whose byte total could overflow is refused when the
//!   bound is a compile-time constant; dynamic bounds rely on the LLVM-LIR
//!   argument that the original loop would overrun the address space
//!   before finishing;
//! * every E-phi must classify (recognized iv), else refuse.
//!
//! Safe degradation is "don't transform". `CCC_NO_LOOP_IDIOM` disables the
//! pass entirely (kill switch for bisection).

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::analysis::CfgAnalysis;
use crate::ir::reexports::{
    BasicBlock, CallInfo, Instruction, IrBinOp, IrCmpOp, IrConst, IrFunction, Operand, Terminator,
    Value,
};
use crate::passes::loop_analysis;

/// Per-gate refusal tracing (`CCC_LOOP_IDIOM_TRACE=1`): makes it diagnosable
/// why a real-world loop fails to match a memset/memcpy idiom.
fn trace_enabled() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var("CCC_LOOP_IDIOM_TRACE").is_ok_and(|v| v != "0"))
}

macro_rules! refuse {
    ($($arg:tt)*) => {{
        if trace_enabled() {
            eprintln!("[loop-idiom] refuse: {}", format_args!($($arg)*));
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

#[derive(Clone, Debug)]
enum Idiom {
    /// All bytes written are the same constant byte.
    Memset { byte: u8, width: u32 },
    /// Byte load → byte store.
    Memcpy,
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
    /// Load side addressing (copies only).
    load_addr: Option<AddrForm>,
    idiom: Idiom,
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
            "[loop-idiom] fn='{}': {} natural loop(s), {} self-loop candidate(s)",
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
            eprintln!("[loop-idiom] refuse: no rotate guard ({e})");
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
                eprintln!("[loop-idiom] guard pred = B{}", p);
            }
        })
        .ok_or("guard has no non-header predecessor")
        .map_err(|e| {
            if trace_enabled() {
                eprintln!("[loop-idiom] refuse: {e}");
            }
        })
        .ok()?;

    // Body census: classify every non-phi instruction.
    let body = census_body(func, header_idx, &ivs, &defs)
        .map_err(|e| {
            if trace_enabled() {
                eprintln!("[loop-idiom] refuse: body census ({e})");
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
    let (store_addr, store_width, store_byte, load_addr, load_width) =
        resolve_memory_ops(func, header_idx, &body, &ivs, &defs)
            .map_err(|e| {
                if trace_enabled() {
                    eprintln!("[loop-idiom] refuse: memory-op resolution ({e})");
                }
            })
            .ok()?;

    let idiom = match body.store_val.expect("census guarantees a store") {
        StoreVal::Const(b) => {
            if let AddrForm::Indexed { scale, .. } = store_addr {
                if scale != store_width as i64 {
                    refuse!(
                        "memset GEP scale {scale} != store width {} (strided fill)",
                        store_width
                    );
                }
            }
            Idiom::Memset {
                byte: b,
                width: store_width,
            }
        }
        StoreVal::Loaded => {
            if store_width != 1 || load_width != Some(1) {
                refuse!(
                    "memcpy needs byte-wide load/store (store {store_width}, load {:?})",
                    load_width
                );
            }
            let Some(ref la) = load_addr else {
                refuse!("store loads but load address unresolved");
            };
            let la = la.clone();
            if !distinct_objects(func, address_base(&store_addr), address_base(&la)) {
                refuse!("copy source/destination may alias (not provably distinct)");
            }
            for form in [&store_addr, &la] {
                if let AddrForm::Indexed { scale, .. } = form {
                    if *scale != 1 {
                        refuse!("memcpy GEP scale {scale} != 1 (strided copy)");
                    }
                }
            }
            Idiom::Memcpy
        }
    };

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
                let bytes_per = match idiom {
                    Idiom::Memset { width, .. } => u64::from(width),
                    Idiom::Memcpy => 1,
                };
                if n > 0 && n > u64::MAX / bytes_per {
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
        load_addr: if matches!(idiom, Idiom::Memcpy) {
            load_addr
        } else {
            None
        },
        idiom,
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
    if !bound_invariant {
        refuse!("bound defined inside the loop");
    }
    if !matches!(cmp_op, IrCmpOp::Ne | IrCmpOp::Ult) {
        refuse!("unsupported guard predicate {:?} (want Ne/Ult)", cmp_op);
    }

    // Body census: straight-line fill/copy work only.
    let body = census_body(func, body_idx, &ivs, &defs)
        .map_err(|e| {
            if trace_enabled() {
                eprintln!("[loop-idiom] refuse: body census ({e})");
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
    let (store_addr, store_width, store_byte, load_addr, load_width) =
        resolve_memory_ops(func, body_idx, &body, &ivs, &defs)
            .map_err(|e| {
                if trace_enabled() {
                    eprintln!("[loop-idiom] refuse: memory-op resolution ({e})");
                }
            })
            .ok()?;

    let idiom = match body.store_val.expect("census guarantees a store") {
        StoreVal::Const(b) => {
            if let AddrForm::Indexed { scale, .. } = store_addr {
                if scale != store_width as i64 {
                    refuse!(
                        "memset GEP scale {scale} != store width {} (strided fill)",
                        store_width
                    );
                }
            }
            Idiom::Memset {
                byte: b,
                width: store_width,
            }
        }
        StoreVal::Loaded => {
            if store_width != 1 || load_width != Some(1) {
                refuse!(
                    "memcpy needs byte-wide load/store (store {store_width}, load {:?})",
                    load_width
                );
            }
            let Some(ref la) = load_addr else {
                refuse!("store loads but load address unresolved");
            };
            let la = la.clone();
            if !distinct_objects(func, address_base(&store_addr), address_base(&la)) {
                refuse!("copy source/destination may alias (not provably distinct)");
            }
            for form in [&store_addr, &la] {
                if let AddrForm::Indexed { scale, .. } = form {
                    if *scale != 1 {
                        refuse!("memcpy GEP scale {scale} != 1 (strided copy)");
                    }
                }
            }
            Idiom::Memcpy
        }
    };

    // A copy loop NESTED in an enclosing loop refuses: the per-run call
    // barrier forces every enclosing-loop value out of its register at each
    // iteration, and the RA's span planning is not call-aware yet (measured
    // lz4 literal-copy: +12.6M Ir from enclosing-loop reloads alone, dwarfing
    // the byte-loop saving). Top-level fill/copy loops keep the transform:
    // the call barrier costs nothing outside hot nests. Memset stays allowed
    // in nests (init code, measured net win).
    if nested && matches!(idiom, Idiom::Memcpy) {
        refuse!("nested copy loop: call barrier would evict enclosing-loop homes");
    }

    // External uses: every loop-defined value used outside {H, B} must be a
    // recognized iv phi (reconstructible as init / init + n at the exit).
    let mut used_ivs: Vec<usize> = Vec::new();
    {
        let mut fail = |what: &str| {
            if trace_enabled() {
                eprintln!(
                    "[loop-idiom] refuse: external use of non-iv {} (while-form)",
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
                let bytes_per = match idiom {
                    Idiom::Memset { width, .. } => u64::from(width),
                    Idiom::Memcpy => 1,
                };
                if n > 0 && n > u64::MAX / bytes_per {
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
        load_addr,
        idiom,
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

#[derive(Clone, Debug)]
enum StoreVal {
    /// Constant byte (0 → width-agnostic memset; nonzero → width-1).
    Const(u8),
    /// The load's result (copy).
    Loaded,
}

/// Classified body of the self-loop block.
#[derive(Clone, Debug, Default)]
struct BodyCensus {
    store: Option<usize>,
    load: Option<usize>,
    store_val: Option<StoreVal>,
}

fn census_body(
    func: &IrFunction,
    block_idx: usize,
    ivs: &[Iv],
    defs: &FxHashMap<u32, usize>,
) -> Result<BodyCensus, String> {
    let header = &func.blocks[block_idx];
    let mut c = BodyCensus::default();

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
            // real by resolve_affine when the store/load address resolves;
            // any that end up unused die with the removed block.
            Instruction::BinOp {
                op: IrBinOp::Shl | IrBinOp::Mul,
                rhs: Operand::Const(_),
                ..
            } => {}
            Instruction::Cmp { .. } => {}
            Instruction::GetElementPtr { .. } => {}
            Instruction::Cast { .. } => {}
            Instruction::Load {
                volatile, dest: _, ..
            } => {
                if c.load.is_some() || *volatile {
                    return Err("multiple/volatile loads".to_string());
                }
                c.load = Some(ii);
            }
            Instruction::Store { val, volatile, .. } => {
                if c.store.is_some() || *volatile {
                    return Err("multiple/volatile stores".to_string());
                }
                c.store = Some(ii);
                c.store_val = Some(match val {
                    Operand::Const(cv) => {
                        let Some(b) = const_to_u64(cv) else {
                            return Err("store constant not an integer".to_string());
                        };
                        if b == 0 {
                            StoreVal::Const(0)
                        } else if width_filled_bytes(b) {
                            StoreVal::Const(b as u8)
                        } else {
                            return Err(format!(
                                "store constant {b:#x} is not a filled byte pattern"
                            ));
                        }
                    }
                    Operand::Value(v) => {
                        let is_load_dest = c.load.is_some_and(|li| {
                            matches!(&header.instructions[li],
                                Instruction::Load { dest, .. } if dest.0 == v.0)
                        });
                        if !is_load_dest || defs.get(&v.0) != Some(&block_idx) {
                            return Err(
                                "stored value is neither a byte constant nor this iteration's load"
                                    .to_string(),
                            );
                        }
                        StoreVal::Loaded
                    }
                });
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

/// True when every byte of the constant is the same (memset-able).
fn width_filled_bytes(v: u64) -> bool {
    let low = v as u8;
    (1..8).all(|s| ((v >> (s * 8)) as u8) == low)
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
/// (store_addr, store_width, store_const_byte, load_addr, load_width).
fn resolve_memory_ops(
    func: &IrFunction,
    op_block: usize,
    body: &BodyCensus,
    ivs: &[Iv],
    defs: &FxHashMap<u32, usize>,
) -> Result<(AddrForm, u32, u8, Option<AddrForm>, Option<u32>), String> {
    let store_ii = body
        .store
        .ok_or_else(|| "census lost the store".to_string())?;
    let Instruction::Store {
        ptr, ty: store_ty, ..
    } = &func.blocks[op_block].instructions[store_ii]
    else {
        return Err("census index not a store".to_string());
    };
    let store_const_byte = match body.store_val.as_ref() {
        Some(StoreVal::Const(b)) => *b,
        Some(StoreVal::Loaded) | None => 0,
    };

    let mut load_addr = None;
    let mut load_width = None;
    if let Some(li) = body.load {
        let Instruction::Load {
            ptr: lptr, ty: lty, ..
        } = &func.blocks[op_block].instructions[li]
        else {
            unreachable!("census-validated index");
        };
        load_width = Some(lty.size() as u32);
        load_addr = Some(resolve_addr(func, op_block, lptr, ivs, defs, "load")?);
    }

    let store_addr = resolve_addr(func, op_block, ptr, ivs, defs, "store")?;
    Ok((
        store_addr,
        store_ty.size() as u32,
        store_const_byte,
        load_addr,
        load_width,
    ))
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

/// The object root behind an address form, for distinctness checks.
fn address_base(form: &AddrForm) -> Option<Value> {
    match form {
        AddrForm::Indexed { base, .. } => Some(*base),
        AddrForm::Advancing { init } => Some(*init),
    }
}

/// Conservative distinct-object check: both bases must resolve to root sets
/// of *distinct* allocas / globals. Roots propagate through GEPs, casts,
/// pointer-offset arithmetic and phi merges (marching pointers); a root set
/// that cannot be fully resolved refuses. `memcpy` has no-overlap semantics
/// while the byte loop is a forward `memmove`, so overlapping copies refuse.
fn distinct_objects(func: &IrFunction, a: Option<Value>, b: Option<Value>) -> bool {
    let (Some(a), Some(b)) = (a, b) else {
        return false;
    };
    let Some(ra) = root_set(func, a) else {
        return false;
    };
    let Some(rb) = root_set(func, b) else {
        return false;
    };
    ra.is_disjoint(&rb)
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum MemRoot {
    Alloca(u32),
    Global(u64),
}

/// The set of memory objects `start` may point into, or None when the chain
/// is not fully resolvable to allocas/globals (parameters, loads, calls…).
fn root_set(func: &IrFunction, start: Value) -> Option<FxHashSet<MemRoot>> {
    // Index definitions once per call (the walker may revisit values).
    let mut def_block: FxHashMap<u32, usize> = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            if let Some(d) = inst.dest() {
                def_block.insert(d.0, bi);
            }
        }
    }
    let find_def = |v: Value| -> Option<&Instruction> {
        let bi = *def_block.get(&v.0)?;
        func.blocks[bi]
            .instructions
            .iter()
            .find(|i| i.dest().is_some_and(|d| d.0 == v.0))
    };

    let mut roots: FxHashSet<MemRoot> = FxHashSet::default();
    let mut work: Vec<Value> = vec![start];
    let mut visited: FxHashSet<u32> = FxHashSet::default();
    let mut fuel = 64usize;
    while let Some(v) = work.pop() {
        if !visited.insert(v.0) {
            continue;
        }
        if fuel == 0 {
            return None;
        }
        fuel -= 1;
        match find_def(v)? {
            Instruction::GetElementPtr { base, .. } => work.push(*base),
            Instruction::Cast { src, .. } => match src {
                Operand::Value(s) => work.push(*s),
                Operand::Const(_) => return None,
            },
            // Arithmetic keeps the object identity of whichever operand is
            // the pointer; the other is an offset. Union over both sides is
            // a sound over-approximation (integer chains carry no roots:
            // only Ptr-typed loads/params can introduce unknown objects).
            Instruction::BinOp { lhs, rhs, .. } => {
                for op in [lhs, rhs] {
                    match op {
                        Operand::Value(x) => work.push(*x),
                        Operand::Const(_) => {}
                    }
                }
            }
            Instruction::Select {
                true_val,
                false_val,
                ..
            } => {
                for op in [true_val, false_val] {
                    match op {
                        Operand::Value(x) => work.push(*x),
                        Operand::Const(_) => {}
                    }
                }
            }
            // A load produces the memory object's *contents*. Integer loads
            // are offsets/values (no roots); pointer loads could point
            // anywhere.
            Instruction::Load { ty, .. } => {
                if *ty == IrType::Ptr {
                    return None;
                }
            }
            Instruction::GlobalAddr { name, .. } => {
                // FNV-1a of the name as the global identity.
                let mut h: u64 = 0xcbf2_9ce4_8422_2325;
                for byte in name.bytes() {
                    h ^= u64::from(byte);
                    h = h.wrapping_mul(0x0000_0100_0000_01b3);
                }
                roots.insert(MemRoot::Global(h));
            }
            Instruction::Alloca { dest, .. } => {
                roots.insert(MemRoot::Alloca(dest.0));
            }
            Instruction::Phi { incoming, .. } => {
                for (op, _) in incoming {
                    match op {
                        Operand::Value(x) => work.push(*x),
                        Operand::Const(_) => return None,
                    }
                }
            }
            _ => return None,
        }
    }
    if roots.is_empty() { None } else { Some(roots) }
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
    let bound_arith = match &plan.bound {
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
    let elem = match plan.idiom {
        Idiom::Memset { width, .. } => u64::from(width),
        Idiom::Memcpy => 1,
    };
    let bytes = if elem == 1 {
        n_size
    } else {
        em.binop(
            IrBinOp::Mul,
            Operand::Value(n_size),
            Operand::Const(IrConst::I64(elem as i64)),
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
    let call_args = match (&plan.idiom, plan.load_addr.as_ref()) {
        (Idiom::Memset { byte, .. }, _) => vec![
            Operand::Value(dst0),
            Operand::Const(IrConst::I32(i32::from(*byte))),
            Operand::Value(bytes),
        ],
        (Idiom::Memcpy, Some(la)) => {
            let src0 = start_of(&mut em, la);
            vec![
                Operand::Value(dst0),
                Operand::Value(src0),
                Operand::Value(bytes),
            ]
        }
        (Idiom::Memcpy, None) => unreachable!("census guarantees a load for copies"),
    };
    let callee = match plan.idiom {
        Idiom::Memset { .. } => "memset",
        Idiom::Memcpy => "memcpy",
    };

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
                plan.bound.clone()
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
    fn copy_loop_requires_distinct_objects() {
        // Byte copy from one global to another: recognized.
        // values: 1 src base, 2 dst base, 3 gep src, 4 load, 5 gep dst,
        // 6 store, 7 i+1, 10 i phi, 11 guard cmp, 3x cmp reuse...
        let build = |same_base: bool| {
            let mut f = IrFunction::new("copy_loop".into(), IrType::I32, vec![], false);
            f.next_value_id = 30;
            f.next_label = 4;
            f.blocks.push(block(
                0,
                vec![
                    Instruction::GlobalAddr {
                        dest: Value(1),
                        name: "src".into(),
                    },
                    Instruction::GlobalAddr {
                        dest: Value(2),
                        name: if same_base { "src" } else { "dst" }.into(),
                    },
                ],
                Terminator::Branch(BlockId(1)),
            ));
            f.blocks.push(block(
                1,
                vec![Instruction::Cmp {
                    dest: Value(11),
                    op: IrCmpOp::Ult,
                    lhs: i64c(0),
                    rhs: i64c(64),
                    ty: IrType::U64,
                }],
                Terminator::CondBranch {
                    cond: val(11),
                    true_label: BlockId(2),
                    false_label: BlockId(3),
                },
            ));
            f.blocks.push(block(
                2,
                vec![
                    Instruction::Phi {
                        dest: Value(10),
                        ty: IrType::U64,
                        incoming: vec![(i64c(0), BlockId(1)), (val(7), BlockId(2))],
                    },
                    Instruction::GetElementPtr {
                        dest: Value(3),
                        base: Value(1),
                        offset: val(10),
                        ty: IrType::Ptr,
                    },
                    Instruction::Load {
                        dest: Value(4),
                        ptr: Value(3),
                        ty: IrType::U8,
                        seg_override: crate::common::types::AddressSpace::Default,
                        volatile: false,
                    },
                    Instruction::GetElementPtr {
                        dest: Value(5),
                        base: Value(2),
                        offset: val(10),
                        ty: IrType::Ptr,
                    },
                    Instruction::Store {
                        val: val(4),
                        ptr: Value(5),
                        ty: IrType::U8,
                        seg_override: crate::common::types::AddressSpace::Default,
                        volatile: false,
                    },
                    Instruction::BinOp {
                        dest: Value(7),
                        op: IrBinOp::Add,
                        lhs: val(10),
                        rhs: i64c(1),
                        ty: IrType::U64,
                    },
                    Instruction::Cmp {
                        dest: Value(12),
                        op: IrCmpOp::Ult,
                        lhs: val(7),
                        rhs: i64c(64),
                        ty: IrType::U64,
                    },
                ],
                Terminator::CondBranch {
                    cond: val(12),
                    true_label: BlockId(2),
                    false_label: BlockId(3),
                },
            ));
            f.blocks
                .push(block(3, vec![], Terminator::Return(Some(i64c(0)))));
            f
        };
        let mut ok = build(false);
        assert_eq!(run(&mut ok), 1, "distinct globals must transform");
        assert!(ok.blocks.iter().any(|b| {
            b.instructions
                .iter()
                .any(|i| matches!(i, Instruction::Call { func, .. } if func == "memcpy"))
        }));

        let mut aliased = build(true);
        assert_eq!(run(&mut aliased), 0, "same base object must refuse");
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
