//! Loop Induction Variable Strength Reduction (IVSR) pass.
//!
//! Transforms expensive per-iteration index computations in loops into
//! cheaper pointer increment operations. For array access patterns like:
//!
//!   for (int i = 0; i < n; i++) sum += arr[i];
//!
//! The IR typically generates:
//!   %cast = Cast(%i, I32 -> I64)
//!   %offset = Shl(%cast, 2)          // i * sizeof(int) via shift
//!   %addr = GEP(%base, %offset)      // base + offset
//!   %val = Load(%addr)
//!
//! After IVSR, this becomes:
//!   %ptr = Phi(%initial_ptr, %ptr_next)
//!   %val = Load(%ptr)
//!   %ptr_next = GEP(%ptr, stride)    // ptr += sizeof(int)
//!
//! This eliminates the multiply/shift and cast per iteration, replacing them
//! with a single pointer addition. The dead multiply and cast are then removed
//! by subsequent DCE.

use super::loop_analysis::{self, NaturalLoop};
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::analysis;
use crate::ir::reexports::{Instruction, IrBinOp, IrConst, IrFunction, Operand, Value};

/// Maximum byte stride eligible for pointer induction. Matrix row strides are
/// routinely several KiB (256 doubles = 2048 bytes), and are especially worth
/// reducing because their multiply executes in an enclosing hot loop.
const MAX_IV_STRIDE: i64 = 1 << 20;

/// Maximum number of Cast/Copy instructions to follow when looking through
/// cast chains to find the root value. Guards against infinite loops on
/// malformed IR with cycles.
const MAX_CAST_CHAIN_LENGTH: usize = 10;

/// A basic induction variable: %iv = phi(init, %iv_next) where %iv_next = %iv + step.
struct BasicIV {
    /// The Phi destination value
    phi_dest: Value,
    /// The type of the IV (typically I32 or I64)
    ty: IrType,
    /// The initial value operand (from outside the loop)
    init: Operand,
    /// The step constant (the additive increment per iteration)
    step: i64,
}

type GepAddressUse = (usize, usize, Value, BaseKey);
type VecAddressUse = (usize, usize, bool, BaseKey);

/// A derived expression from a basic IV that can be strength-reduced.
/// Pattern: %derived = %iv_or_cast * const_stride (or Shl), used as offset in GEP
struct DerivedExpr {
    /// The constant stride (element size in bytes)
    stride: i64,
    /// Which basic IV this derives from (index into basic_ivs)
    iv_index: usize,
    /// Constant added to the IV BEFORE the stride multiply, for the affine
    /// pattern `(iv + k) * stride` (e.g. nbody's `(i + 1) * 56`). Zero for the
    /// plain `iv * stride` pattern.
    add_offset: i64,
    /// GEPs that use this multiply/shift result as their offset.
    /// (block_idx, inst_idx, GEP dest, base key)
    gep_uses: Vec<GepAddressUse>,
    /// Vector-memory `(base, index, disp)` address triples whose INDEX reads
    /// this multiply/shift result (possibly through GVN Copy glue).
    /// (block_idx, inst_idx, is_store, base key).
    ///
    /// These are address uses of `base + derived + disp` exactly like a GEP
    /// offset, and must join the same pointer recurrence: nbody's pair loop
    /// kept `imulq $56` alive on every iteration because the SLP/vectorizer
    /// forms `VecLoadF64x2(base, i*56, K)` triples that the GEP-only scan
    /// could not see, so the multiply had live uses and no recurrence formed.
    vec_uses: Vec<VecAddressUse>,
    /// The multiply/shift instruction's destination — the identity of the
    /// derived value for the scalar flavor below.
    mul_dest: Value,
    /// The multiply/shift instruction's type (the derived IV's domain).
    mul_ty: IrType,
    /// Whether the derived value has ANY use in the function (GEP offsets
    /// included). A dead multiply gets no recurrence.
    has_uses: bool,
    /// True only for exprs built by the address-forming-`Add` arm's fresh
    /// scan (`Add(base, iv)` byte-buffer walks). Those recurrences replace a
    /// whole `Add` this backend will NOT sink into SIB addressing (measured:
    /// k_varint 52 -> 42 instructions), so they pay even at stride 1; a
    /// `Mul`-scan group with a SIB-scaleable stride does not (see the
    /// SIB cost rule in `reduce_loop`).
    from_address_add: bool,
}

/// Group-base identity for pointer-recurrence coalescing: the resolved root
/// value plus, for `GlobalAddr` bases, the symbol name.
///
/// GVN unifies two same-symbol `GlobalAddr` materialisations only sometimes;
/// SLP and the vectorizer freely mix `v_a = GlobalAddr("bodies")` defined in
/// the entry block with `v_b = GlobalAddr("bodies")` defined inside the loop
/// body (nbody's pair loop after vectorization). Both name the same link-time
/// object, so `v_b + j*56` and `v_a + j*56` are one recurrence, but value-id
/// keying splits them into two groups that each keep their own multiply.
/// Keying by SYMBOL merges them; the recurrence then materialises its own
/// fresh `GlobalAddr` in the preheader, which dominates trivially and needs
/// no loop-invariance proof on some arbitrary definition site.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum BaseKey {
    /// A plain value base: recurrence init reads this exact value.
    Value(u32),
    /// A `GlobalAddr` base: recurrence init materialises `name` fresh.
    Symbol(String),
}

/// Resolve `val` through single-definition Copy / same-width integer Cast /
/// zero-offset GEP glue to its root value.
///
/// GVN unifies equivalent arithmetic by replacing the redundant def with a
/// `Copy` of the surviving value. By the time IVSR scans, a GEP offset or a
/// vector-address index therefore often reads `Copy(mul)` rather than `mul`
/// itself; use collection that compares operand ids against the mul's dest
/// misses every one of those uses, the group comes up empty, and the
/// recurrence never arms (measured on nbody: five unified `* 56` muls, zero
/// recurrences, `imulq $56` + rematerialised `leaq bodies(%rip)` on every
/// pair-loop iteration). Bounded like `look_through_casts`.
fn alias_root(map: &FxHashMap<u32, u32>, mut val: u32) -> u32 {
    for _ in 0..MAX_CAST_CHAIN_LENGTH {
        match map.get(&val) {
            Some(&next) if next != val => val = next,
            _ => break,
        }
    }
    val
}

/// Build the dest -> source glue map consumed by [`alias_root`].
fn build_alias_map(func: &IrFunction) -> FxHashMap<u32, u32> {
    let mut direct: FxHashMap<u32, u32> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Copy {
                    dest,
                    src: Operand::Value(v),
                } => {
                    direct.insert(dest.0, v.0);
                }
                Instruction::Cast {
                    dest,
                    src: Operand::Value(v),
                    from_ty,
                    to_ty,
                } => {
                    // Value identity, not offset preservation: a same-width
                    // integer/pointer reinterpretation carries the same bits.
                    // A widening cast is a DIFFERENT value (zero/sign bits) and
                    // must not alias.
                    if from_ty.size() == to_ty.size() && !from_ty.is_float() && !to_ty.is_float() {
                        direct.insert(dest.0, v.0);
                    }
                }
                Instruction::GetElementPtr {
                    dest,
                    base,
                    offset: Operand::Const(c),
                    ..
                } => {
                    if c.to_i64() == Some(0) {
                        direct.insert(dest.0, base.0);
                    }
                }
                _ => {}
            }
        }
    }
    direct
}

/// Classify an address-forming value into a group base key: alias root, with
/// same-symbol `GlobalAddr` materialisations collapsing onto one key.
fn base_key_of(
    base: &Value,
    aliases: &FxHashMap<u32, u32>,
    global_names: &FxHashMap<u32, String>,
) -> BaseKey {
    let root = alias_root(aliases, base.0);
    match global_names.get(&root) {
        Some(name) => BaseKey::Symbol(name.clone()),
        None => BaseKey::Value(root),
    }
}

/// `GlobalAddr` dest -> symbol name, for [`base_key_of`].
fn global_addr_names(func: &IrFunction) -> FxHashMap<u32, String> {
    let mut names = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::GlobalAddr { dest, name } = inst {
                names.insert(dest.0, name.clone());
            }
        }
    }
    names
}

/// Reuse only an SSA definition available at the preheader insertion point.
/// CFG edges do not change during IVSR, so the caller's dominators remain valid.
fn dominating_global_addr(
    func: &IrFunction,
    cfg: &analysis::CfgAnalysis,
    name: &str,
    preheader: usize,
) -> Option<Value> {
    for (block_index, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            if let Instruction::GlobalAddr { dest, name: symbol } = inst {
                if symbol == name && analysis::dominates_idom(&cfg.idom, block_index, preheader) {
                    return Some(*dest);
                }
            }
        }
    }
    None
}

/// Run IVSR on a single function (test wrapper: PRODUCTION defaults — the
/// scalar derived-IV flavor is OFF, matching the pipeline's default).
#[cfg(test)]
pub(crate) fn ivsr_function(func: &mut IrFunction) -> usize {
    let num_blocks = func.blocks.len();
    if num_blocks < 2 {
        return 0;
    }

    // Build CFG and dominator tree
    let cfg = analysis::CfgAnalysis::build(func);
    ivsr_with_analysis(func, &cfg, false, true, true)
}

/// Run IVSR on a single function with the scalar derived-IV flavor armed
/// (test wrapper for the opt-in path — see the block comment at the scalar
/// section for why production keeps it off by default).
#[cfg(test)]
pub(crate) fn ivsr_function_scalar(func: &mut IrFunction) -> usize {
    let num_blocks = func.blocks.len();
    if num_blocks < 2 {
        return 0;
    }

    // Build CFG and dominator tree
    let cfg = analysis::CfgAnalysis::build(func);
    ivsr_with_analysis(func, &cfg, true, true, true)
}

/// Run IVSR using pre-computed CFG analysis (avoids redundant analysis when
/// called from a pipeline that shares analysis across GVN, LICM, IVSR).
/// `scalar_derived` arms the scalar derived-IV flavor — off in production;
/// the pipeline derives it from `CCC_IVSR_SCALAR_DERIVED=1` (opt-in).
///
/// `ptr_add` arms the address-forming-`Add` scan (IVSR-PTRADD-1) — ON in
/// production. The pipeline derives it from `CCC_NO_IVSR_PTR_ADD` being absent
/// and passes it in, so this file spends no environment read of its own:
/// `check_env_test_hygiene.sh` ratchets env reads in `src/passes` and exempts
/// only `mod.rs`, which is where a new knob is meant to be read.
///
/// `x86_sib_cost` arms the x86 SIB cost rule below. It is a TARGET property
/// (scaled-index addressing is free on x86, absent on RISC-V): the pipeline
/// passes `matches!(target, X86_64 | I686)` and the unit wrappers pin `true`
/// because the rule's contracts are written against x86 codegen.
// Only topology matters to cached dominance: IVSR may rewrite a terminator's
// value operands, but must not change block identity, terminator kind or edges.
#[cfg(any(test, debug_assertions))]
#[derive(Debug, PartialEq, Eq)]
struct IvsrCfgShape {
    labels: Vec<crate::ir::reexports::BlockId>,
    kinds: Vec<std::mem::Discriminant<crate::ir::reexports::Terminator>>,
    edges: Vec<Vec<crate::ir::reexports::BlockId>>,
}

#[cfg(any(test, debug_assertions))]
fn ivsr_cfg_shape(func: &IrFunction) -> IvsrCfgShape {
    use crate::ir::reexports::Terminator;
    IvsrCfgShape {
        labels: func.blocks.iter().map(|block| block.label).collect(),
        kinds: func
            .blocks
            .iter()
            .map(|block| std::mem::discriminant(&block.terminator))
            .collect(),
        edges: func
            .blocks
            .iter()
            .map(|block| match &block.terminator {
                Terminator::Return(_) | Terminator::Unreachable => Vec::new(),
                Terminator::Branch(label) => vec![*label],
                Terminator::CondBranch {
                    true_label,
                    false_label,
                    ..
                } => vec![*true_label, *false_label],
                Terminator::IndirectBranch {
                    possible_targets, ..
                } => possible_targets.clone(),
                Terminator::Switch { cases, default, .. } => std::iter::once(*default)
                    .chain(cases.iter().map(|(_, label)| *label))
                    .collect(),
            })
            .collect(),
    }
}

#[cfg(any(test, debug_assertions))]
fn ivsr_cfg_unchanged(func: &IrFunction, cfg: &analysis::CfgAnalysis) -> bool {
    if func.blocks.len() != cfg.num_blocks {
        return false;
    }
    let labels = analysis::build_label_map(func);
    let (_, successors) = analysis::build_cfg(func, &labels);
    labels.len() == cfg.num_blocks
        && (0..cfg.num_blocks).all(|i| successors.row(i) == cfg.succs.row(i))
}

pub(crate) fn ivsr_with_analysis(
    func: &mut IrFunction,
    cfg: &analysis::CfgAnalysis,
    scalar_derived: bool,
    ptr_add: bool,
    x86_sib_cost: bool,
) -> usize {
    #[cfg(debug_assertions)]
    assert!(
        ivsr_cfg_unchanged(func, cfg),
        "IVSR received stale CFG analysis"
    );
    if cfg.num_blocks < 2 {
        return 0;
    }

    // Find natural loops
    let loops =
        loop_analysis::find_natural_loops(cfg.num_blocks, &cfg.preds, &cfg.succs, &cfg.idom);
    if loops.is_empty() {
        return 0;
    }

    // Merge loops with same header
    let loops = loop_analysis::merge_loops_by_header(loops);
    let debug = std::env::var("CCC_IVSR_DEBUG").is_ok();

    #[cfg(debug_assertions)]
    let original_cfg_shape = ivsr_cfg_shape(func);

    let mut total_reductions = 0;

    // Process innermost-first.  IVSR creates a new pointer phi and recurrence.
    // Applying it independently to overlapping/nested loops can make an outer
    // recurrence consume a base transformed by the inner loop — the nested matmul
    // wrong-result.  Original conservative fix skipped all overlapping loops.
    // Improved policy: keep inner loops (smallest bodies first) and skip any
    // outer that overlaps an already-kept inner.  This retains the more
    // profitable inner-loop optimization while preventing address-doubling.
    let mut sorted_loops = loops;
    sorted_loops.sort_by_key(|l| l.body.len());

    let mut kept_bodies: Vec<FxHashSet<usize>> = Vec::new();

    for natural_loop in &sorted_loops {
        let overlaps_kept = kept_bodies
            .iter()
            .any(|kept| !kept.is_disjoint(&natural_loop.body));
        if overlaps_kept {
            if debug {
                eprintln!(
                    "[IVSR] skip outer overlapping loop header={} blocks={} (overlaps kept inner)",
                    natural_loop.header,
                    natural_loop.body.len()
                );
            }
            continue;
        }
        let changed = reduce_loop(
            func,
            natural_loop,
            cfg,
            scalar_derived,
            ptr_add,
            x86_sib_cost,
        );
        #[cfg(debug_assertions)]
        assert!(
            ivsr_cfg_shape(func) == original_cfg_shape && ivsr_cfg_unchanged(func, cfg),
            "IVSR invalidated cached dominance"
        );
        if changed > 0 {
            kept_bodies.push(natural_loop.body.clone());
        }
        total_reductions += changed;
    }

    total_reductions
}

/// Try to strength-reduce induction variables in a single loop.
fn reduce_loop(
    func: &mut IrFunction,
    natural_loop: &NaturalLoop,
    cfg: &analysis::CfgAnalysis,
    scalar_derived: bool,
    ptr_add: bool,
    x86_sib_cost: bool,
) -> usize {
    let preds = &cfg.preds;
    let header = natural_loop.header;
    let dbg = std::env::var("CCC_IVSR_DEBUG").is_ok();

    // Find the preheader (single predecessor outside the loop)
    let preheader = match loop_analysis::find_preheader(header, &natural_loop.body, preds) {
        Some(ph) => ph,
        None => {
            if dbg {
                eprintln!("[IVSR] header={} no preheader", header);
            }
            return 0;
        }
    };

    // Find back-edge blocks (predecessors of header that are inside the loop)
    let back_blocks: Vec<usize> = preds
        .row(header)
        .iter()
        .map(|&p| p as usize)
        .filter(|p| natural_loop.body.contains(p))
        .collect();

    // Only handle simple single-latch loops
    if back_blocks.len() != 1 {
        if dbg {
            eprintln!("[IVSR] header={} back_blocks={}", header, back_blocks.len());
        }
        return 0;
    }

    // Step 1: Identify basic induction variables from phi nodes in the header.
    let basic_ivs = find_basic_ivs(func, header, &natural_loop.body, preheader, &back_blocks);
    if basic_ivs.is_empty() {
        if dbg {
            eprintln!("[IVSR] header={} no basic ivs", header);
        }
        return 0;
    }
    if dbg {
        eprintln!("[IVSR] header={} ivs={}", header, basic_ivs.len());
    }

    // Step 2: Find derived expressions (iv * const) used in GEPs.
    let derived = find_derived_exprs(
        func,
        &basic_ivs,
        &natural_loop.body,
        ptr_add,
        natural_loop,
        &back_blocks,
    );
    if derived.is_empty() {
        if dbg {
            eprintln!("[IVSR] header={} no derived exprs", header);
        }
        return 0;
    }
    if dbg {
        eprintln!("[IVSR] header={} derived={}", header, derived.len());
    }

    // Step 3: Apply strength reduction transformations.
    let mut reductions = 0;
    let mut next_id = func.next_value_id;
    // Track how many phi nodes we've inserted at the header, so subsequent
    // GEP replacements in the header use the correct adjusted index.
    let mut header_phi_insertions = 0usize;

    let preheader_label = func.blocks[preheader].label;
    let back_block_label = func.blocks[back_blocks[0]].label;

    // Coalesce all equivalent derived expressions, not only GEPs that
    // literally share the same multiply instruction. C frontends commonly
    // materialize `a[i]` twice as two independently numbered `i * stride`
    // expressions. The key is semantic: (basic IV, byte stride, invariant
    // base). GEP offsets and vector-memory address triples are the SAME
    // address use (`base + derived [+ disp]`) and join the one recurrence.
    enum AddrUse {
        /// `GEP(base, derived)` at (block, inst) defining `dest`.
        Gep(usize, usize, Value),
        /// Vector memory `(base, derived, disp)` triple at (block, inst).
        Vec(usize, usize, bool),
    }
    let mut pointer_groups: Vec<(usize, i64, i64, IrType, BaseKey, Vec<AddrUse>, bool)> =
        Vec::new();
    for d in &derived {
        let mut group_uses: Vec<(BaseKey, AddrUse)> = Vec::new();
        for &(gep_block_idx, gep_inst_idx, gep_dest, ref gep_base) in &d.gep_uses {
            group_uses.push((
                gep_base.clone(),
                AddrUse::Gep(gep_block_idx, gep_inst_idx, gep_dest),
            ));
        }
        for &(vec_block_idx, vec_inst_idx, is_store, ref vec_base) in &d.vec_uses {
            group_uses.push((
                vec_base.clone(),
                AddrUse::Vec(vec_block_idx, vec_inst_idx, is_store),
            ));
        }
        for (base_key, use_) in group_uses {
            if let Some((_, _, _, _, _, uses, from_add)) =
                pointer_groups
                    .iter_mut()
                    .find(|(iv_index, stride, off, mty, base, _, _)| {
                        *iv_index == d.iv_index
                            && *stride == d.stride
                            && *off == d.add_offset
                            && *mty == d.mul_ty
                            && *base == base_key
                    })
            {
                uses.push(use_);
                *from_add |= d.from_address_add;
            } else {
                pointer_groups.push((
                    d.iv_index,
                    d.stride,
                    d.add_offset,
                    d.mul_ty,
                    base_key,
                    vec![use_],
                    d.from_address_add,
                ));
            }
        }
    }

    // Deterministic processing order (loop_body set iteration order is not
    // stable): ascending add_offset materialises the smaller-offset parent
    // recurrence first, so affine siblings can chain off it.
    pointer_groups.sort_by_key(|(iv_index, stride, off, _, _, _, _)| (*iv_index, *stride, *off));
    // Recurrences created in THIS call, for sibling chaining: `(iv, stride,
    // mul_ty, base) -> (ptr_iv, k)`. Two groups differing only in the affine
    // offset k satisfy `P_k(t) = P_k0(t) + (k - k0) * stride` for every t —
    // the same IV, the same stride, the same base — so the sibling needs no
    // recurrence of its own; its uses become `GEP(parent, delta)`. (nbody's
    // pair loop: the `(i + 1) * 56` j-init chains off the `i * 56` body
    // pointer and the second marching pointer disappears.)
    let mut made: Vec<(usize, i64, IrType, BaseKey, Value, i64)> = Vec::new();
    for (iv_index, stride, add_offset, mul_ty, gep_base, addr_uses, from_address_add) in
        pointer_groups
    {
        let iv = &basic_ivs[iv_index];
        let ptr_width = crate::common::types::target_ptr_size();
        let narrow_signed =
            |ty: IrType| ty.is_integer() && !ty.is_unsigned() && ty.size() < ptr_width;
        if super::signed_overflow_wraps() && (narrow_signed(iv.ty) || narrow_signed(mul_ty)) {
            // A pointer-width recurrence must not skip a defined signed wrap
            // in either the source counter or its pre-extension product.
            // Pointer-width rings and already-certified unsigned paths remain.
            continue;
        }
        // A narrow unsigned counter is modular: zero-extending it into an
        // address recurrence is linear only while the executed backedges cannot
        // wrap. That is a proof obligation, not an inference from "there is a
        // comparison somewhere". IVs at or above the pointer width, and every
        // sub-32-bit counter (whose C promotion makes the arithmetic 32-bit and
        // therefore already covered by IVSR-WRAP-1's backedge rule), are handled
        // by the other arms.
        // EVERY modular unsigned IV below the pointer width needs the proof,
        // including `U8`/`U16`. The previous `size >= 4` carve-out skipped them
        // on the reasoning that they are "handled by the other arms" — they are
        // not; they are handled by `find_basic_ivs` never *forming* them, because
        // C promotes a sub-int counter to `int` and the backedge truncation is
        // rejected by `look_through_casts` (IVSR-WRAP-1). That is an invariant of
        // a DIFFERENT function, and relying on it here is fail-open: a `U8` IV
        // built by any other means would recur with no no-wrap proof at all.
        // Failing closed costs nothing — `unsigned_iv_bound` requires the header
        // test to be in the IV's OWN type, and a promoted `U8` counter's test is
        // `I32`, so it returns `None` and the recurrence is declined.
        let modular_iv = iv.ty.size() < crate::common::types::target_ptr_size();
        let unsigned_bound = if modular_iv && iv.ty.is_unsigned() {
            let bound = unsigned_iv_bound(func, natural_loop, iv, &back_blocks);
            if bound.is_none() {
                if dbg {
                    eprintln!(
                        "[IVSR] header={} reject unproven modular iv v{} ty={:?} step={}",
                        header, iv.phi_dest.0, iv.ty, iv.step
                    );
                }
                continue;
            }
            bound
        } else {
            None
        };
        let Some(inc_bytes) = iv.step.checked_mul(stride) else {
            continue;
        };
        if !offset_product_cannot_overflow(mul_ty, stride, add_offset, unsigned_bound) {
            if dbg {
                eprintln!(
                    "[IVSR] header={} reject overflowing {:?} offset product (stride {stride})",
                    header, mul_ty
                );
            }
            continue;
        }

        // Only reduce address groups whose base is loop-invariant (skip
        // loop-variant bases — the historical matmul address-doubling bug, a
        // recurrence whose base is itself transformed by an inner loop). For
        // `Value` bases the proof runs on the alias ROOT, which is the value
        // the recurrence init actually reads. A `Symbol` base is a `GlobalAddr`
        // materialisation of a link-time object: invariant by construction,
        // and the init below materialises its own copy in the preheader, so no
        // definition site needs to dominate anything.
        let base_invariant = match &gep_base {
            BaseKey::Value(root) => is_loop_invariant(*root, &natural_loop.body, func),
            BaseKey::Symbol(_) => true,
        };
        if !base_invariant {
            continue;
        }

        // ── x86 SIB cost rule (measured 2026-10-12) ─────────────────────
        // A GEP-only group with a SIB-scaleable stride (1/2/4/8) is
        // addressed FREE by the backend as `disp(%base, %idx, scale)` — the
        // scaled-index form shares one integer index across every base walk
        // (fp_memfold_stencil5: `vmovss (%rsi,%r10,4)` / `(%rdi,%r10,4)`).
        // Replacing it with a marching pointer per base costs a phi, a
        // preheader init, and a per-iteration step PER BASE — measured as
        // stencil5 +5.7% insns / +6.7% moves (codegen-quality-gate band
        // trip) when this rule was missing.
        //
        // The original exemptions, both measured:
        //   * groups with vec-memory uses always form: the vector emitter's
        //     index is a BYTE offset (`vec_mem_operand` emits
        //     `disp(%base,%index)`, never a scaled SIB), so the multiply is
        //     unavoidable in the index form and the recurrence replaces it
        //     (nbody's `j * 56` pair loop: 1.72x -> 1.09x vs GCC);
        //   * the address-forming-Add arm's fresh byte-walk recurrences
        //     (`from_address_add`): this backend does NOT sink
        //     `Add(base, iv)` into `(%base,%iv)` addressing (k_varint
        //     measured 52 -> 42 instructions with the recurrence).
        // The skip direction is always CORRECT (the index form computes the
        // same address); it is only a profitability choice. On targets whose
        // addressing scales differently (RISC-V has no scaled SIB) the skip
        // forgoes the marching-pointer win AND reshapes constant usage enough
        // to flip downstream RA/GLA decisions — measured on double_reduction
        // (riscv64): weighted constant uses 1 -> 20, global bases reachable
        // at the default reach band, +14 function instructions (the exact
        // RA-GLA-03 hot-loop cascade the cross-reach pin guards). The rule is
        // therefore x86-only (`x86_sib_cost`).
        let sib_scaled = x86_sib_cost
            && matches!(stride, 1 | 2 | 4 | 8)
            && !addr_uses.iter().any(|u| matches!(u, AddrUse::Vec(..)));
        // A narrow symbol index also needs a widening conversion. Reusing a
        // dominating base avoids the duplicate live address that otherwise
        // makes a recurrence lose to SIB through register pressure. Keep the
        // ordinary pointer-base stencil policy and non-x86 materialisation.
        let reusable_symbol_base = match &gep_base {
            BaseKey::Symbol(name) if x86_sib_cost => {
                dominating_global_addr(func, cfg, name, preheader)
            }
            _ => None,
        };
        let narrow_symbol_index = reusable_symbol_base.is_some()
            && iv.ty.size() < mul_ty.size()
            && iv.ty.size() < crate::common::types::target_ptr_size();
        if sib_scaled && !from_address_add && !narrow_symbol_index {
            if dbg {
                eprintln!(
                    "[IVSR] header={} skip SIB-friendly group iv={} stride={} (scaled-index form is free)",
                    header, iv_index, stride
                );
            }
            continue;
        }

        // Sibling chaining: when all uses are GEPs and a recurrence over the
        // same (iv, stride, mul_ty, base) already exists at affine offset k0,
        // every use here is `parent_phi + (k - k0) * stride` — a constant
        // GEP delta, provably identical at every iteration because both
        // recurrences track the same IV with the same stride. The sibling
        // then needs no phi, no init, and no latch step of its own.
        let chainable = made.iter().find_map(|(miv, mstride, mty, mbase, phi, k0)| {
            if *miv == iv_index && *mstride == stride && *mty == mul_ty && *mbase == gep_base {
                add_offset
                    .checked_sub(*k0)
                    .and_then(|d| d.checked_mul(stride))
                    .map(|d| (*phi, d))
            } else {
                None
            }
        });
        if let (Some((parent_phi, delta)), true) = (
            chainable,
            addr_uses.iter().all(|u| matches!(u, AddrUse::Gep(..))),
        ) {
            let mut chained = 0usize;
            for use_ in &addr_uses {
                let (use_block, use_inst, gep_dest) = match use_ {
                    AddrUse::Gep(b, i, d) => (*b, *i, *d),
                    AddrUse::Vec(..) => continue,
                };
                let adjusted_idx = if use_block == header {
                    use_inst + header_phi_insertions
                } else {
                    use_inst
                };
                if adjusted_idx >= func.blocks[use_block].instructions.len() {
                    continue;
                }
                let inst = &func.blocks[use_block].instructions[adjusted_idx];
                if inst.dest() != Some(gep_dest) {
                    continue;
                }
                func.blocks[use_block].instructions[adjusted_idx] = Instruction::GetElementPtr {
                    dest: gep_dest,
                    base: parent_phi,
                    offset: Operand::Const(IrConst::I64(delta)),
                    ty: IrType::I8,
                };
                chained += 1;
            }
            if chained > 0 {
                if dbg {
                    eprintln!(
                        "[IVSR] header={} chained group iv={} k={} -> parent phi v{} + {} ({} uses)",
                        header, iv_index, add_offset, parent_phi.0, delta, chained
                    );
                }
                reductions += chained;
                continue;
            }
        }

        let ptr_iv_val = Value(next_id);
        next_id += 1;
        let ptr_next_val = Value(next_id);
        next_id += 1;
        let init_ptr_val = Value(next_id);
        next_id += 1;

        // Try to resolve init to a constant (looking through copies). The
        // initial byte offset is (init + add_offset) * stride: the affine
        // `(iv + k) * stride` pattern starts at `k * stride` bytes past the
        // base even when the IV starts at zero.
        let init_const = try_resolve_const(&iv.init, func);
        let init_offset = if let Some(v) = init_const {
            // IrConst's signed storage is not the IV's mathematical domain.
            let v = const_in_iv_domain(v, iv.ty);
            let Some(off) = v
                .checked_add(add_offset as i128)
                .and_then(|v| v.checked_mul(stride as i128))
            else {
                continue;
            };
            // An exact offset that no `IrConst::I64` operand can hold cannot be
            // folded into the GEP, so skip the transform rather than truncate
            // it. (Reaching this needs `init * stride >= 2^63` bytes.)
            if i64::try_from(off).is_err() {
                continue;
            }
            Some(off)
        } else {
            None
        };

        // Build preheader instructions for computing the initial pointer
        let mut preheader_insts: Vec<Instruction> = Vec::new();

        // The init reads the alias ROOT for a plain value. On x86 a symbol
        // first reuses an already-dominating GlobalAddr; otherwise materialise
        // a fresh definition in the preheader, which dominates by construction.
        let init_base = match &gep_base {
            BaseKey::Value(root) => Value(*root),
            BaseKey::Symbol(name) => {
                if let Some(gv) = reusable_symbol_base {
                    gv
                } else {
                    let gv = Value(next_id);
                    next_id += 1;
                    preheader_insts.push(Instruction::GlobalAddr {
                        dest: gv,
                        name: name.clone(),
                    });
                    gv
                }
            }
        };

        if let Some(init_off) = init_offset {
            // Constant initial offset case (most common: i = 0)
            if init_off == 0 {
                preheader_insts.push(Instruction::Copy {
                    dest: init_ptr_val,
                    src: Operand::Value(init_base),
                });
            } else {
                preheader_insts.push(Instruction::GetElementPtr {
                    dest: init_ptr_val,
                    base: init_base,
                    offset: Operand::Const(IrConst::I64(init_off as i64)),
                    ty: IrType::I8,
                });
            }
        } else {
            // Non-constant init: compute init * stride at runtime in preheader.
            // The affine `(iv + k) * stride` form with a non-constant init would
            // need `(init + k) * stride` here; that composition is not yet
            // handled, so skip it conservatively (falls back to scalar code,
            // which is always correct).
            if add_offset != 0 {
                continue;
            }
            let init_val = match &iv.init {
                Operand::Value(v) => *v,
                _ => continue,
            };

            if stride == 1 {
                let init_cast_val = Value(next_id);
                next_id += 1;
                if iv.ty != IrType::I64 && iv.ty != IrType::Ptr {
                    preheader_insts.push(Instruction::Cast {
                        dest: init_cast_val,
                        src: Operand::Value(init_val),
                        from_ty: iv.ty,
                        to_ty: IrType::I64,
                    });
                    preheader_insts.push(Instruction::GetElementPtr {
                        dest: init_ptr_val,
                        base: init_base,
                        offset: Operand::Value(init_cast_val),
                        ty: IrType::I8,
                    });
                } else {
                    next_id -= 1; // Undo unused init_cast_val allocation
                    preheader_insts.push(Instruction::GetElementPtr {
                        dest: init_ptr_val,
                        base: init_base,
                        offset: Operand::Value(init_val),
                        ty: IrType::I8,
                    });
                }
            } else {
                // Compute init * stride at runtime.
                // Only allocate a cast value if the IV type needs widening to I64.
                let needs_cast = iv.ty != IrType::I64 && iv.ty != IrType::Ptr;
                let mul_operand = if needs_cast {
                    let init_cast_val = Value(next_id);
                    next_id += 1;
                    preheader_insts.push(Instruction::Cast {
                        dest: init_cast_val,
                        src: Operand::Value(init_val),
                        from_ty: iv.ty,
                        to_ty: IrType::I64,
                    });
                    Operand::Value(init_cast_val)
                } else {
                    Operand::Value(init_val)
                };

                let init_mul_val = Value(next_id);
                next_id += 1;
                preheader_insts.push(Instruction::BinOp {
                    dest: init_mul_val,
                    op: IrBinOp::Mul,
                    lhs: mul_operand,
                    rhs: Operand::Const(IrConst::I64(stride)),
                    ty: IrType::I64,
                });
                preheader_insts.push(Instruction::GetElementPtr {
                    dest: init_ptr_val,
                    base: init_base,
                    offset: Operand::Value(init_mul_val),
                    ty: IrType::I8,
                });
            }
        }

        // Header: ptr_iv = phi(init_ptr from preheader, ptr_next from back_block)
        let phi_inst = Instruction::Phi {
            dest: ptr_iv_val,
            ty: IrType::Ptr,
            incoming: vec![
                (Operand::Value(init_ptr_val), preheader_label),
                (Operand::Value(ptr_next_val), back_block_label),
            ],
        };

        // Back-edge block: ptr_next = GEP(ptr_iv, inc_bytes)
        let inc_inst = Instruction::GetElementPtr {
            dest: ptr_next_val,
            base: ptr_iv_val,
            offset: Operand::Const(IrConst::I64(inc_bytes)),
            ty: IrType::I8,
        };

        // Apply the transformation:
        let ph_has_spans = !func.blocks[preheader].source_spans.is_empty();
        let hdr_has_spans = !func.blocks[header].source_spans.is_empty();
        let bb_has_spans = !func.blocks[back_blocks[0]].source_spans.is_empty();

        // 1. Add init_ptr computation to end of preheader
        for inst in preheader_insts {
            func.blocks[preheader].instructions.push(inst);
            if ph_has_spans {
                func.blocks[preheader]
                    .source_spans
                    .push(crate::common::source::Span::dummy());
            }
        }

        // 2. Add phi at beginning of header (after existing phis)
        let insert_pos = func.blocks[header]
            .instructions
            .iter()
            .position(|inst| !matches!(inst, Instruction::Phi { .. }))
            .unwrap_or(func.blocks[header].instructions.len());
        func.blocks[header]
            .instructions
            .insert(insert_pos, phi_inst);
        if hdr_has_spans {
            func.blocks[header]
                .source_spans
                .insert(insert_pos, crate::common::source::Span::dummy());
        }
        header_phi_insertions += 1;

        // 3. Add increment to end of back-edge block
        func.blocks[back_blocks[0]].instructions.push(inc_inst);
        if bb_has_spans {
            func.blocks[back_blocks[0]]
                .source_spans
                .push(crate::common::source::Span::dummy());
        }

        // 4. Replace every original address use in this semantic group with
        // the one shared pointer IV. GEP uses become a Copy of the pointer;
        // vector-memory `(base, index, disp)` triples become
        // `(ptr, Const(0), disp)` — the same addressing the emitter folds to
        // one `disp(%ptr)` memory operand. Adjust header indices for the
        // inserted phis, but do not add another recurrence per use.
        for use_ in addr_uses {
            let (use_block, use_inst) = match &use_ {
                AddrUse::Gep(b, i, _) | AddrUse::Vec(b, i, _) => (*b, *i),
            };
            let adjusted_idx = if use_block == header {
                use_inst + header_phi_insertions
            } else {
                use_inst
            };
            if adjusted_idx >= func.blocks[use_block].instructions.len() {
                continue;
            }
            match use_ {
                AddrUse::Gep(_, _, gep_dest) => {
                    let inst = &func.blocks[use_block].instructions[adjusted_idx];
                    if let Some(dest) = inst.dest() {
                        if dest == gep_dest {
                            func.blocks[use_block].instructions[adjusted_idx] = Instruction::Copy {
                                dest: gep_dest,
                                src: Operand::Value(ptr_iv_val),
                            };
                            reductions += 1;
                        }
                    }
                }
                AddrUse::Vec(_, _, is_store) => {
                    let inst = &mut func.blocks[use_block].instructions[adjusted_idx];
                    if let Instruction::Intrinsic { args, .. } = inst {
                        // Argument layout is the `vec_intrinsic_access`
                        // contract: stores read (value, base, index, disp?),
                        // loads read (base, index, disp?). Keep the arity
                        // (every consumer indexes slots positionally) and
                        // keep any trailing displacement slot untouched: the
                        // pointer IV already carries `base + derived`, so the
                        // index slot becomes zero and the displacement stays.
                        let (base_slot, index_slot) = if is_store { (1, 2) } else { (0, 1) };
                        if args.len() > index_slot {
                            args[base_slot] = Operand::Value(ptr_iv_val);
                            args[index_slot] = Operand::Const(IrConst::I64(0));
                            reductions += 1;
                        }
                    }
                }
            }
        }
        made.push((iv_index, stride, mul_ty, gep_base, ptr_iv_val, add_offset));
    }

    // ── SCALAR DERIVED IVs — OPT-IN (`CCC_IVSR_SCALAR_DERIVED=1`) ──────
    //
    // WHY THIS IS OFF BY DEFAULT (measured, 2026-09-23, the S46 CI RED):
    // the transformation replaces a per-iteration `imull` (short live range)
    // with a loop-carried recurrence (live the WHOLE loop).  On this backend
    // that trade measured as a NET LOSS everywhere it fired:
    //   * static: 5 golden workloads regressed past the codegen-quality
    //     gate's tolerance bands (gzip_crc32 +6.2% insns, glibc_memcmp +6.4%
    //     insns/+23.5% stackmem, expat_xml_scan +23.5% stackmem, stencil5
    //     +7.5% insns, zlib_ng_adler32 +6.2% stackmem) — CI RED on PR #602.
    //   * runtime (11-rep interleaved A/B, full corpus): expat_xml_scan
    //     +3.9..4.5%, glibc_strstr +4.3%, loop_patterns +2.1%,
    //     linux_find_bit +1.8%, linux_rbtree +1.2% (the motivating case!),
    //     struct_copy +1.1%; zero confirmed wins (hash_table's apparent
    //     −1.6% was pure noise — its assembly is byte-identical).
    // The root cause is the known latch phi-web parking (the Unit-4
    // allocator rock): every added loop-carried web materializes to a slot
    // home at the latch and costs more than the imull it removes.  GCC
    // strength-reduces the identical shapes (oracle: `addq $4051`/
    // `addl $11` in glibc_memcmp's driver) because its RA keeps recurrences
    // in callee-saved registers across calls — revisit this knob AFTER the
    // allocator work; the machinery and its unit tests stay armed here.
    //
    // `iv * C` with NO GEP-offset uses (linux_rbtree's lookup hash
    // `i * 104729`, consumed by an And/mask, not a GEP): the multiply is
    // replaced by a secondary recurrence `j = phi(init*C, j + step*C)`.
    // The primary IV stays (it feeds the exit test); the multiply becomes
    // dead and DCE removes it.  EXACTNESS: the recurrence and the multiply
    // compute the same value in the SAME modulo-2^N domain — the group type
    // must equal the IV's type, so a widening Cast between them (a
    // wraparound divergence: sext(i) after an i32 wrap ≠ the accumulated
    // wide recurrence) disqualifies the group.  Uses after the loop read
    // the last iteration's value on both formulations (SSA dominance).
    //
    // Mixed shapes (a multiply feeding BOTH a GEP offset and other uses)
    // are left alone here: the pointer path above already handled the GEP,
    // and the multiply stays for the scalar readers — always correct.
    if scalar_derived {
        // Group by (iv, stride, add_offset, ty): several equivalent muls
        // (pre-GVN frontends) share ONE derived recurrence.
        let mut scalar_groups: Vec<(usize, i64, i64, IrType, Vec<Value>)> = Vec::new();
        for d in &derived {
            if !d.gep_uses.is_empty() || !d.has_uses {
                continue;
            }
            let iv = &basic_ivs[d.iv_index];
            if iv.ty != d.mul_ty || !d.mul_ty.is_integer() {
                continue;
            }
            if let Some((_, _, _, _, muls)) =
                scalar_groups
                    .iter_mut()
                    .find(|(iv_index, stride, add_offset, ty, _)| {
                        *iv_index == d.iv_index
                            && *stride == d.stride
                            && *add_offset == d.add_offset
                            && *ty == d.mul_ty
                    })
            {
                muls.push(d.mul_dest);
            } else {
                scalar_groups.push((
                    d.iv_index,
                    d.stride,
                    d.add_offset,
                    d.mul_ty,
                    vec![d.mul_dest],
                ));
            }
        }

        for (iv_index, stride, add_offset, group_ty, muls) in scalar_groups {
            let iv = &basic_ivs[iv_index];
            let inc = iv.step.wrapping_mul(stride);
            // Both constants must encode in the group's domain BEFORE any
            // instruction is inserted: a bail-out after the phi insertion
            // would leave j_next undefined (invalid IR).
            let (Some(stride_const), Some(inc_const)) = (
                const_for_int_ty(group_ty, stride),
                const_for_int_ty(group_ty, inc),
            ) else {
                continue;
            };

            let j_val = Value(next_id);
            next_id += 1;
            let j_next_val = Value(next_id);
            next_id += 1;

            // Preheader: j_init = (init + k) * C, constant-folded when both
            // parts are constants.  Executed once — the whole point.
            let init_const = try_resolve_const(&iv.init, func);
            let j_init_op = match init_const
                .map(|init_c| init_c.wrapping_add(add_offset).wrapping_mul(stride))
                .and_then(|folded| const_for_int_ty(group_ty, folded))
            {
                Some(c) => Operand::Const(c),
                None => {
                    // Runtime init in the preheader.  Only the k == 0 shape
                    // (plain iv * C) is built here: a non-constant init with
                    // an affine offset is rare enough to keep the constructor
                    // minimal — fail closed.
                    if add_offset != 0 {
                        continue;
                    }
                    let Operand::Value(init_v) = iv.init else {
                        continue;
                    };
                    let j_init_val = Value(next_id);
                    next_id += 1;
                    func.blocks[preheader]
                        .instructions
                        .push(Instruction::BinOp {
                            dest: j_init_val,
                            op: IrBinOp::Mul,
                            lhs: Operand::Value(init_v),
                            rhs: Operand::Const(stride_const),
                            ty: group_ty,
                        });
                    if !func.blocks[preheader].source_spans.is_empty() {
                        func.blocks[preheader]
                            .source_spans
                            .push(crate::common::source::Span::dummy());
                    }
                    Operand::Value(j_init_val)
                }
            };

            // Header: j = phi(j_init, j_next)
            let phi_inst = Instruction::Phi {
                dest: j_val,
                ty: group_ty,
                incoming: vec![
                    (j_init_op, preheader_label),
                    (Operand::Value(j_next_val), back_block_label),
                ],
            };
            let insert_pos = func.blocks[header]
                .instructions
                .iter()
                .position(|inst| !matches!(inst, Instruction::Phi { .. }))
                .unwrap_or(func.blocks[header].instructions.len());
            func.blocks[header]
                .instructions
                .insert(insert_pos, phi_inst);
            if !func.blocks[header].source_spans.is_empty() {
                func.blocks[header]
                    .source_spans
                    .insert(insert_pos, crate::common::source::Span::dummy());
            }

            // Latch: j_next = j + step*C  (same wraparound domain).
            let inc_inst = Instruction::BinOp {
                dest: j_next_val,
                op: IrBinOp::Add,
                lhs: Operand::Value(j_val),
                rhs: Operand::Const(inc_const),
                ty: group_ty,
            };
            func.blocks[back_blocks[0]].instructions.push(inc_inst);
            if !func.blocks[back_blocks[0]].source_spans.is_empty() {
                func.blocks[back_blocks[0]]
                    .source_spans
                    .push(crate::common::source::Span::dummy());
            }

            // Rewrite every use of every grouped multiply to j.  Operands
            // (incl. phi incomings and terminators) plus the Value-typed
            // pointer positions (Load/Store ptr, GEP base) that the operand
            // walker does not visit.
            let mut rewritten = 0usize;
            for block in &mut func.blocks {
                for rinst in &mut block.instructions {
                    rinst.for_each_operand_mut(|o| {
                        if matches!(o, Operand::Value(v) if v.0 != 0 && muls.contains(v)) {
                            *o = Operand::Value(j_val);
                            rewritten += 1;
                        }
                    });
                    // Bare-`Value` slots (Load/Store ptr, GEP base, Memcpy,
                    // va_list, inline-asm outputs, intrinsic dest_ptr, ...): the
                    // exhaustive walker, never a hand-rolled match.  The three-arm
                    // version this replaces covered only Load/Store/GEP-base and
                    // silently left every other shape reading the grouped multiply
                    // — a missed rewrite, invisible to SSA validation because the
                    // old value is still defined.
                    rinst.for_each_value_use_mut(|v| {
                        if v.0 != 0 && muls.contains(v) {
                            *v = j_val;
                            rewritten += 1;
                        }
                    });
                }
                block.terminator.for_each_operand_mut(|o| {
                    if matches!(o, Operand::Value(v) if v.0 != 0 && muls.contains(v)) {
                        *o = Operand::Value(j_val);
                        rewritten += 1;
                    }
                });
            }
            if rewritten > 0 {
                reductions += rewritten;
            }
        }
    }

    if reductions > 0 {
        func.next_value_id = next_id;
    }

    reductions
}

/// Encode `v` as a constant of integer type `ty`, wrapping into the type's
/// modulo domain (the derived-IV recurrence is exact under wraparound, so
/// the encoding must be the wrapping one).  Non-integer or over-wide types
/// fail closed (`None`).
fn const_for_int_ty(ty: IrType, v: i64) -> Option<IrConst> {
    Some(match ty {
        IrType::I8 | IrType::U8 => IrConst::I8(v as i8),
        IrType::I16 | IrType::U16 => IrConst::I16(v as i16),
        IrType::I32 | IrType::U32 => IrConst::I32(v as i32),
        IrType::I64 | IrType::U64 => IrConst::I64(v),
        _ => return None,
    })
}

/// Find basic induction variables: phis in the header of the form
/// %iv = phi(init from preheader, %iv_next from back_block)
/// where %iv_next = %iv + const_step (possibly through casts)
fn find_basic_ivs(
    func: &IrFunction,
    header: usize,
    loop_body: &FxHashSet<usize>,
    preheader: usize,
    back_blocks: &[usize],
) -> Vec<BasicIV> {
    let mut ivs = Vec::with_capacity(16);

    // Build a map from value to its defining instruction within the loop
    let mut loop_defs: FxHashMap<u32, &Instruction> = FxHashMap::default();
    for &bi in loop_body {
        if bi < func.blocks.len() {
            for inst in &func.blocks[bi].instructions {
                if let Some(dest) = inst.dest() {
                    loop_defs.insert(dest.0, inst);
                }
            }
        }
    }

    // Scan phi nodes in the header
    for inst in &func.blocks[header].instructions {
        if let Instruction::Phi { dest, ty, incoming } = inst {
            // Must be integer type (induction variables are integers).
            //
            // 128-bit types are excluded STRUCTURALLY, not incidentally. Every
            // constant in this pass travels through `IrConst::to_i64()`, so an
            // `I128`/`U128` step or init would be silently truncated before the
            // no-wrap and product-overflow proofs ever saw it. Today the cast
            // gate already rejects the narrowing that a 128-bit IV would need,
            // so no group can form — but that is a consequence of another
            // function's rule, and this pass should not depend on it.
            if !ty.is_integer() && *ty != IrType::Ptr {
                continue;
            }
            if ty.is_128bit() {
                continue;
            }

            // Find the init value (from preheader) and the back-edge value
            let mut init_op = None;
            let mut back_val = None;

            for (op, block_id) in incoming {
                let bi_opt = func
                    .blocks
                    .iter()
                    .enumerate()
                    .find(|(_, b)| b.label == *block_id)
                    .map(|(i, _)| i);
                if let Some(bi) = bi_opt {
                    if bi == preheader {
                        init_op = Some(*op);
                    } else if back_blocks.contains(&bi) {
                        if let Operand::Value(v) = op {
                            back_val = Some(*v);
                        }
                    }
                }
            }

            let init_op = match init_op {
                Some(op) => op,
                None => continue,
            };
            let back_val = match back_val {
                Some(v) => v,
                None => continue,
            };

            // Check if back_val = dest + const_step
            // Also handle: back_val = Cast(Add(dest, step)) or
            //              back_val = Cast(Add(Cast(dest), step))
            // These patterns arise from C integer promotion rules.
            let add_val = look_through_casts(back_val.0, &loop_defs);
            if let Some(Instruction::BinOp { op, lhs, rhs, .. }) = loop_defs.get(&add_val) {
                if *op == IrBinOp::Add {
                    let phi_id = dest.0;
                    let lhs_root = match lhs {
                        Operand::Value(v) => look_through_casts(v.0, &loop_defs),
                        _ => u32::MAX,
                    };
                    let rhs_root = match rhs {
                        Operand::Value(v) => look_through_casts(v.0, &loop_defs),
                        _ => u32::MAX,
                    };

                    let step_operand = if lhs_root == phi_id {
                        Some(rhs)
                    } else if rhs_root == phi_id {
                        Some(lhs)
                    } else {
                        None
                    };

                    if let Some(Operand::Const(c)) = step_operand {
                        if let Some(step) = c.to_i64() {
                            ivs.push(BasicIV {
                                phi_dest: *dest,
                                ty: *ty,
                                init: init_op,
                                step,
                            });
                        }
                    }
                }
            }
        }
    }

    ivs
}

/// Find derived expressions: %mul = %iv * const_stride (or Shl by const)
/// that are used as offsets in GEPs within the loop.
fn find_derived_exprs(
    func: &IrFunction,
    basic_ivs: &[BasicIV],
    loop_body: &FxHashSet<usize>,
    ptr_add: bool,
    natural_loop: &NaturalLoop,
    back_blocks: &[usize],
) -> Vec<DerivedExpr> {
    let mut derived = Vec::with_capacity(16);
    // Read the debug switch ONCE per function, not once per instruction: this
    // scan is O(instructions) and the old spelling did an environment lookup
    // inside the hottest loop in the pass. It also frees two slots against the
    // `check_env_test_hygiene` ratchet (155 -> 153).
    let dbg = std::env::var_os("CCC_IVSR_DEBUG").is_some();

    // GVN unification glue (Copy / same-width reinterpret / zero-GEP) and
    // GlobalAddr symbol names: use-site collection must see a `Copy(mul)` or
    // a second `GlobalAddr("sym")` as the same address value the recurrence
    // is built for, or the group forms empty and the multiply survives.
    let aliases = build_alias_map(func);
    let global_names = global_addr_names(func);

    // Index the immutable input once. Candidate discovery below never mutates
    // IR, so definitions and raw use identities stay valid for this scan. In
    // particular, raw uses are NOT alias-canonicalized: that would change the
    // old has_uses contract for Copy glue and terminator-only consumers.
    let mut definitions = FxHashMap::default();
    let mut used_values = FxHashSet::default();
    let mut gep_uses_by_index: FxHashMap<u32, Vec<GepAddressUse>> = FxHashMap::default();
    let mut vec_uses_by_index: FxHashMap<u32, Vec<VecAddressUse>> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            inst.for_each_used_value(|value| {
                used_values.insert(value);
            });
        }
        block.terminator.for_each_used_value(|value| {
            used_values.insert(value);
        });
    }
    for &bi in loop_body {
        let Some(block) = func.blocks.get(bi) else {
            continue;
        };
        for (ii, inst) in block.instructions.iter().enumerate() {
            if let Some(dest) = inst.dest() {
                definitions.insert(dest.0, inst);
            }
            match inst {
                Instruction::GetElementPtr {
                    dest,
                    base,
                    offset: Operand::Value(offset),
                    ..
                } => {
                    gep_uses_by_index
                        .entry(alias_root(&aliases, offset.0))
                        .or_default()
                        .push((bi, ii, *dest, base_key_of(base, &aliases, &global_names)));
                }
                Instruction::Intrinsic { op, args, .. } => {
                    if let Some((is_store, _, Operand::Value(base), Operand::Value(offset), _)) =
                        super::alias::vec_intrinsic_access(op, args)
                    {
                        vec_uses_by_index
                            .entry(alias_root(&aliases, offset.0))
                            .or_default()
                            .push((bi, ii, is_store, base_key_of(base, &aliases, &global_names)));
                    }
                }
                _ => {}
            }
        }
    }

    // Build a set of IV phi values for quick lookup
    let mut iv_values: FxHashMap<u32, usize> = FxHashMap::default();
    for (i, iv) in basic_ivs.iter().enumerate() {
        iv_values.insert(iv.phi_dest.0, i);
    }

    // Find Casts and Copies of IV values (common pattern: Cast(i32_iv -> i64),
    // Copy(Cast(iv))). Multiple passes to handle chains.
    let mut iv_derived: FxHashMap<u32, usize> = FxHashMap::default();
    for _ in 0..3 {
        let mut new_entries = Vec::with_capacity(16);
        for &bi in loop_body {
            if bi >= func.blocks.len() {
                continue;
            }
            for inst in &func.blocks[bi].instructions {
                match inst {
                    Instruction::Cast {
                        dest,
                        src: Operand::Value(v),
                        from_ty,
                        to_ty,
                    } => {
                        // Value preservation, not equal storage width. In
                        // particular U32 -> I32 -> I64 maps UINT_MAX to -1,
                        // whereas widening the original U32 maps it to 2^32-1.
                        if from_ty.cast_preserves_offset_value(*to_ty) {
                            let idx = iv_values.get(&v.0).or_else(|| iv_derived.get(&v.0));
                            if let Some(&iv_idx) = idx {
                                if !iv_derived.contains_key(&dest.0) {
                                    new_entries.push((dest.0, iv_idx));
                                }
                            }
                        }
                    }
                    Instruction::Copy {
                        dest,
                        src: Operand::Value(v),
                    } => {
                        let idx = iv_values.get(&v.0).or_else(|| iv_derived.get(&v.0));
                        if let Some(&iv_idx) = idx {
                            if !iv_derived.contains_key(&dest.0) {
                                new_entries.push((dest.0, iv_idx));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        if new_entries.is_empty() {
            break;
        }
        for (k, v) in new_entries {
            iv_derived.insert(k, v);
        }
    }

    // Look up whether a value derives from an IV
    let find_iv = |val_id: u32| -> Option<usize> {
        iv_values
            .get(&val_id)
            .or_else(|| iv_derived.get(&val_id))
            .copied()
    };
    // Affine form: `add(iv, k)` (or `add(k, iv)`) — recognized so that
    // `(iv + k) * stride` strength-reduces with an initial offset of k*stride.
    //
    // The `j = i + 1` pair-loop shape reaches the multiply ONE WIDENING CAST
    // below it (`mul(sext(i + 1), stride)`), so the affine Add is located by
    // walking Copy glue and offset-preserving Casts from the multiply's
    // operand. Each hop is checked by `cast_preserves_offset_value`, the same
    // certificate the `iv_derived` closure uses. A narrow unsigned Add is a
    // separate proof obligation: `U32(i + 2)` may wrap before it is widened to
    // the pointer ring, while the pointer recurrence keeps counting linearly.
    // `affine_add_offset_cannot_wrap` proves the Add's own range from the IV's
    // header bound; if it cannot, this affine form is declined. The later
    // `offset_product_cannot_overflow` check still independently certifies the
    // multiply/product ring.
    let find_affine = |val_id: u32, _result_ty: IrType| -> Option<(usize, i64)> {
        let mut candidates = vec![val_id];
        let mut cur = val_id;
        for _ in 0..MAX_CAST_CHAIN_LENGTH {
            let next = match definitions.get(&cur).copied() {
                Some(Instruction::Copy {
                    src: Operand::Value(v),
                    ..
                }) => v.0,
                Some(Instruction::Cast {
                    src: Operand::Value(v),
                    from_ty,
                    to_ty,
                    ..
                }) if from_ty.cast_preserves_offset_value(*to_ty) => v.0,
                _ => break,
            };
            if candidates.contains(&next) {
                break;
            }
            candidates.push(next);
            cur = next;
        }
        for cid in candidates {
            if let Some(Instruction::BinOp {
                op: IrBinOp::Add,
                lhs,
                rhs,
                ty,
                ..
            }) = definitions.get(&cid).copied()
            {
                let (v, c) = match (lhs, rhs) {
                    (Operand::Value(v), Operand::Const(c))
                    | (Operand::Const(c), Operand::Value(v)) => (v, c),
                    _ => continue,
                };
                if let (Some(idx), Some(raw_k)) = (find_iv(v.0), c.to_i64())
                    && let Some(k) = affine_add_offset_cannot_wrap(
                        func,
                        natural_loop,
                        back_blocks,
                        &basic_ivs[idx],
                        raw_k,
                        *ty,
                    )
                {
                    return Some((idx, k));
                }
            }
        }
        None
    };
    if dbg {
        let mut d: Vec<u32> = iv_derived.keys().copied().collect();
        d.sort_unstable();
        eprintln!(
            "[IVSR-DERIV] iv_values={:?} iv_derived={:?}",
            iv_values.keys().collect::<Vec<_>>(),
            d
        );
    }

    // Find multiplications/shifts of IV values by constants
    for &bi in loop_body {
        if bi >= func.blocks.len() {
            continue;
        }
        for inst in func.blocks[bi].instructions.iter() {
            if dbg {
                eprintln!(
                    "[IVSR-DERIV] b{} label={} inst={:?}",
                    bi,
                    func.blocks[bi].label.0,
                    std::mem::discriminant(inst)
                );
                if let Instruction::BinOp {
                    dest, op, lhs, rhs, ..
                } = inst
                {
                    eprintln!(
                        "[IVSR-DERIV] b{} label={} BinOp v{} op={:?} lhs={:?} rhs={:?}",
                        bi, func.blocks[bi].label.0, dest.0, op, lhs, rhs
                    );
                }
            }
            let (mul_dest, mul_ty, iv_idx, stride, add_offset) = match inst {
                // Multiply by constant (plain `iv * s`, or affine
                // `(iv + k) * s` via find_affine).
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::Mul,
                    lhs,
                    rhs,
                    ty,
                } => match (lhs, rhs) {
                    (Operand::Value(v), Operand::Const(c))
                    | (Operand::Const(c), Operand::Value(v)) => {
                        let s = c.to_i64();
                        if let (Some(idx), Some(s)) = (find_iv(v.0), s) {
                            (*dest, *ty, idx, s, 0)
                        } else if let (Some((idx, k)), Some(s)) = (find_affine(v.0, *ty), s) {
                            (*dest, *ty, idx, s, k)
                        } else {
                            continue;
                        }
                    }
                    _ => continue,
                },
                // Shift left by constant (= multiply by 2^k)
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::Shl,
                    lhs: Operand::Value(v),
                    rhs: Operand::Const(c),
                    ty,
                } => {
                    if let (Some(idx), Some(shift)) = (find_iv(v.0), c.to_i64()) {
                        if (0..64).contains(&shift) {
                            (*dest, *ty, idx, 1i64 << shift, 0)
                        } else {
                            continue;
                        }
                    } else {
                        continue;
                    }
                }
                // Add of a value with itself: %x = add %v, %v  ==  %v * 2.
                // The frontend/GVN pipeline canonically folds `i * 2` into
                // `i + i` BEFORE IVSR runs, so the Mul/Shl arms above never
                // see the common stride-2 case (fill_window's prev[]/head[]
                // loops were not strength-reduced at all — measured 13
                // instructions per element vs GCC's 7).
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::Add,
                    lhs: Operand::Value(l),
                    rhs: Operand::Value(r),
                    ty,
                } => {
                    if l.0 == r.0 {
                        if let Some(idx) = find_iv(l.0) {
                            (*dest, *ty, idx, 2, 0)
                        } else {
                            continue;
                        }
                    } else {
                        continue;
                    }
                }
                _ => continue,
            };

            // Only worthwhile for strides that are common element sizes
            if stride <= 0 || stride > MAX_IV_STRIDE {
                continue;
            }

            // Find GEPs and vector-memory address triples that use this
            // multiply/shift result as their offset/index. GVN leaves the
            // surviving multiply behind Copy glue for old value ids, so
            // operands are resolved through `aliases` before matching.
            let mul_dest_id = mul_dest.0;
            let gep_uses = gep_uses_by_index
                .get(&mul_dest_id)
                .cloned()
                .unwrap_or_default();
            let vec_uses = vec_uses_by_index
                .get(&mul_dest_id)
                .cloned()
                .unwrap_or_default();
            let has_uses = used_values.contains(&mul_dest_id);

            if !gep_uses.is_empty() || !vec_uses.is_empty() || has_uses {
                derived.push(DerivedExpr {
                    stride,
                    iv_index: iv_idx,
                    add_offset,
                    gep_uses,
                    vec_uses,
                    mul_dest,
                    mul_ty,
                    has_uses,
                    from_address_add: false,
                });
            } else if dbg {
                eprintln!(
                    "[IVSR-DERIV] mul v{} stride {} has no GEP uses",
                    mul_dest.0, stride
                );
            }
        }
    }

    // ── Address-forming Adds: `Add(ptr, byte_offset)` ──────────────────────
    //
    // LCCC lowers EVERY C subscript to pointer-width integer arithmetic rather
    // than to `GetElementPtr`: `try_lower_pointer_arithmetic` in
    // `src/ir/lowering/expr_ops.rs` emits `Add(ptr, scale_index(i, elem_size))`,
    // where `scale_index` is the identity for `elem_size == 1` and a
    // `Mul(i, elem_size)` otherwise. The scan above only collects
    // `GetElementPtr` offsets, so on this IR the pointer recurrence never fired
    // for the most common addressing idiom in C: byte-buffer walks got no
    // recurrence at all, and wider-element arrays found the `Mul` but no GEP
    // consuming it, so the whole group was dropped (`has_uses` alone only arms
    // the opt-in scalar flavor, which is off by default).
    //
    // Measured on `tests/bench/k_varint.c` `-O3`, `bench_run`: the address was
    // rebuilt every iteration as `movslq`/`leaq v(%rip)`/`addq` against GCC's
    // single pointer bump — 52 vs 42 instructions, 66.5 ms vs 39.9 ms (0.60x,
    // the worst kernel in `scripts/bench_kernels.py`).
    //
    // Two guards keep this from becoming the scalar-derived-IV net loss that
    // `CCC_IVSR_SCALAR_DERIVED` documents:
    //   * the Add must be in the POINTER ring — a narrower Add is ordinary
    //     integer arithmetic; but a pointer-ring Add of two integers is not
    //     distinguishable from an address by type alone, so additionally
    //   * its result must actually be USED AS AN ADDRESS (`Load.ptr`,
    //     `Store.ptr`, `GetElementPtr.base`, or a memory intrinsic's pointer
    //     argument). An integer accumulation `invariant + iv` that nobody
    //     dereferences is left exactly alone.
    //
    // The rewrite itself reuses the existing machinery: an address-forming Add
    // is recorded just like a GEP use, so the transform replaces the Add's dest
    // with a `Copy` of the shared pointer phi, and every constant-offset GEP
    // reading it (`&p[i+1]`, `&p[i+2]`) keeps working untouched.
    // ILP32 is excluded: the recurrence adds a loop-carried pointer web, and on
    // a 6-GPR register file that web is parked in a slot at the latch — the
    // documented cost that keeps `CCC_IVSR_SCALAR_DERIVED` opt-in. Measured on
    // tests/regression/simd_crc_adler.c -O2 -m32 the frame grew 142 -> 191
    // stack refs (+49) for 2 extra instructions, while the same source on
    // x86-64 was +2 insns / -1 stack. LP64 has 15 usable GPRs and the exchange
    // pays; ILP32 does not. `target_is_32bit()` is the same predicate
    // slot_assignment uses to partition its own width classes.
    //
    // `ptr_add` arrives as a PARAMETER, read once in the pipeline, because
    // tests/regression/check_env_test_hygiene.sh ratchets the number of
    // environment reads in src/passes (mod.rs excepted) and the sanctioned way
    // to add a knob is to thread it in like `scalar_derived` — not to spend
    // another read here.
    if ptr_add && !crate::common::types::target_is_32bit() {
        // `mul_dest` (alias root) -> index into `derived`, so an Add whose
        // offset is an already-recognised scaled index JOINS that group
        // instead of forming a second recurrence for the same address. GVN
        // Copy glue is transparent here for the same reason as in the GEP
        // scan: the group identity is the surviving multiply.
        let mut by_mul_dest: FxHashMap<u32, usize> = FxHashMap::default();
        for (i, d) in derived.iter().enumerate() {
            by_mul_dest
                .entry(alias_root(&aliases, d.mul_dest.0))
                .or_insert(i);
        }
        let ptr_ring = crate::common::types::target_ptr_size();
        let mut attaches: Vec<(usize, usize, usize, Value, BaseKey)> = Vec::new();
        let mut fresh: Vec<DerivedExpr> = Vec::new();
        for &bi in loop_body {
            if bi >= func.blocks.len() {
                continue;
            }
            for (ii, inst) in func.blocks[bi].instructions.iter().enumerate() {
                let Instruction::BinOp {
                    dest,
                    op: IrBinOp::Add,
                    lhs,
                    rhs,
                    ty,
                } = inst
                else {
                    continue;
                };
                if ty.size() != ptr_ring {
                    continue;
                }
                let (Operand::Value(a), Operand::Value(b)) = (lhs, rhs) else {
                    continue;
                };
                if !is_used_as_address(func, dest.0) {
                    continue;
                }
                // Exactly one side may be IV-derived. If both are, this is
                // `iv1 + iv2` and there is no invariant base to bump; if the
                // candidate base is loop-variant the address is not an
                // induction of a single pointer either. The invariance proof
                // runs on the alias ROOT (the value the recurrence init will
                // actually read), and a `GlobalAddr` base is invariant by
                // construction — the recurrence materialises its own copy in
                // the preheader.
                for (off, base) in [(a, b), (b, a)] {
                    let key = base_key_of(base, &aliases, &global_names);
                    let base_root = match &key {
                        BaseKey::Value(r) => r.to_owned(),
                        // Fresh preheader materialisation dominates trivially.
                        BaseKey::Symbol(_) => base.0,
                    };
                    let base_invariant = matches!(&key, BaseKey::Symbol(_))
                        || is_loop_invariant(base_root, loop_body, func);
                    if !base_invariant {
                        continue;
                    }
                    let off_root = alias_root(&aliases, off.0);
                    if let Some(&di) = by_mul_dest.get(&off_root) {
                        attaches.push((di, bi, ii, *dest, key));
                        break;
                    }
                    let (idx, add_off) = if let Some(i) = find_iv(off_root) {
                        (i, 0)
                    } else {
                        match find_affine(off_root, *ty) {
                            Some(pair) => pair,
                            None => continue,
                        }
                    };
                    fresh.push(DerivedExpr {
                        stride: 1,
                        iv_index: idx,
                        add_offset: add_off,
                        gep_uses: vec![(bi, ii, *dest, key)],
                        vec_uses: Vec::new(),
                        mul_dest: *dest,
                        mul_ty: *ty,
                        has_uses: true,
                        from_address_add: true,
                    });
                    break;
                }
            }
        }

        for (di, bi, ii, dest, key) in attaches {
            derived[di].gep_uses.push((bi, ii, dest, key));
        }
        derived.extend(fresh);
    }

    derived
}

/// Whether `val_id` is dereferenced or used as an addressing base anywhere in
/// the function — i.e. whether it is an ADDRESS and not merely an integer that
/// happens to be pointer-width. A value used both as an address and as an
/// integer still qualifies (the rewrite preserves its value either way); a pure
/// integer accumulation does not.
///
/// This is DELIBERATELY an over-approximation, and the reverse of the
/// "conservative by construction: anything not positively identified as a memory
/// operand returns false" this comment used to claim. The intrinsic leg
/// delegates to `IntrinsicOp::reads_pointer_arg`, which is the allowlist for a
/// callee-READ-ONLY proof and is therefore true for pure vector arithmetic that
/// never touches memory. Narrowing that leg to `may_read_memory()` was tried and
/// measured, and it cost real code: `simd_crc_adler` +7 instructions and +4
/// stack references per iteration, `simd_vecreg` +2 and +2, because a hoisted
/// `leaq` and a register-resident `movdqa` degraded into a per-iteration
/// `movslq`/`leaq` pair plus a spill/reload. Over-approximating is also SOUND
/// here: only pointer-width values can be armed, and for those the integer and
/// pointer rings agree in any defined program (C17 6.5.6p8). Pinned by
/// `address_classification_deliberately_over_approximates_intrinsics`; do not
/// "fix" this predicate without re-running that measurement.
fn is_used_as_address(func: &IrFunction, val_id: u32) -> bool {
    let is_target = |o: &Operand| matches!(o, Operand::Value(v) if v.0 == val_id);
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                // `Load`/`Store` carry a `Value` pointer; the atomic forms
                // carry an `Operand`, so they cannot share an arm.
                Instruction::Load { ptr, .. } | Instruction::GetElementPtr { base: ptr, .. } => {
                    if ptr.0 == val_id {
                        return true;
                    }
                }
                Instruction::Store { ptr, .. } => {
                    if ptr.0 == val_id {
                        return true;
                    }
                }
                Instruction::AtomicLoad { ptr, .. } | Instruction::AtomicStore { ptr, .. } => {
                    if is_target(ptr) {
                        return true;
                    }
                }
                Instruction::Memcpy { dest, src, .. } => {
                    if dest.0 == val_id || src.0 == val_id {
                        return true;
                    }
                }
                // Vector/SSE loads and stores carry their pointer as a
                // positional argument rather than in a dedicated slot; these
                // are exactly the IVOPTS-1 shapes the backlog calls out as
                // invisible to the GEP-only scan. The classification is
                // delegated to `IntrinsicOp`'s own allowlists rather than
                // hand-listed here, so a new vector memory opcode is picked up
                // automatically instead of silently missing.
                Instruction::Intrinsic {
                    dest_ptr, args, op, ..
                } => {
                    if dest_ptr.is_some_and(|d| d.0 == val_id) {
                        return true;
                    }
                    // `reads_pointer_arg()`, which is the allowlist for a
                    // callee-READ-ONLY proof and is deliberately true for pure
                    // vector arithmetic (`VecAddF64x4` and friends, via
                    // `produces_vector_value`). A review of this code proposed
                    // narrowing it to `may_read_memory()` on the grounds that
                    // over-approximation is the unsafe direction HERE. Measured,
                    // it is not: it removed the pointer induction from
                    // `simd_vecreg`, `simd_crc_adler` and
                    // `temp_promotion_window`, replacing a hoisted
                    // `leaq 16(%r9), %r9` recurrence with a per-iteration
                    // `movslq`/`leaq (%rdi,%r9,4)` re-derivation plus an xmm
                    // spill/reload pair (+7 insns, +4 stack refs on
                    // `simd_crc_adler`). And the soundness argument for keeping
                    // the wide predicate is positive, not merely pragmatic: this
                    // classification can only arm a recurrence on a value that
                    // `try_lower_pointer_arithmetic` already put in POINTER-width
                    // arithmetic, and for a program without UB the integer and
                    // pointer rings agree at that width (a wrapping
                    // `base + i*elem` is UB in the source per C17 6.5.6p8, so no
                    // defined program can distinguish the two recurrences).
                    // Over-approximation here costs nothing and buys the vector
                    // induction; the narrow product ring is guarded separately,
                    // by `offset_product_cannot_overflow`.
                    if (op.reads_pointer_arg() || op.writes_memory_via_args())
                        && args.iter().any(is_target)
                    {
                        return true;
                    }
                }
                _ => {}
            }
        }
    }
    false
}

/// The mathematical value of an IR integer constant **in the domain of `ty`**.
///
/// `IrConst` carries every integer in a signed `i64` carrier, so a `U32` limit of
/// `0xFFFFFFFF` can arrive as `I32(-1)` and `to_i64()` reads `-1`. The frontend's
/// constant evaluator normalises unsigned constants to a non-negative `I64`, so
/// this is unreachable from C source today — but a bound certificate built on the
/// raw carrier would certify `hi = -2` for a limit of `0xFFFFFFFF` and then
/// *wrongly admit* an overflowing product, because `-2 < 2^32` passes the check.
///
/// One helper, used by BOTH the initial-offset computation and the bound, so the
/// two sites cannot disagree about what a constant means. That disagreement was
/// real: `init_offset` reinterpreted by `iv.ty` while the bound used the raw
/// carrier.
///
/// SCOPE OF "the single definition": narrow IV domains. The carrier is read with
/// `to_i64()`, which truncates an `IrConst::I128` (`constants.rs`:
/// `IrConst::I128(v) => Some(v as i64)`). That cannot reach a bound here — the
/// bound is only consulted for a MODULAR IV, i.e. `iv.ty.size() <
/// target_ptr_size()`, at most 32 bits on every supported target, and the exit
/// test must compare in `iv.ty` — and `bits >= 128` returns the raw value
/// unchanged, so a 128-bit domain never reaches the masking branch either.
/// Documented rather than widened, because an `i128`-exact reader would be dead
/// code with a worse failure mode: silently believing a truncated 128-bit
/// constant is a narrow one.
fn const_in_iv_domain(raw: i64, ty: IrType) -> i128 {
    let bits = (ty.size() * 8) as u32;
    if bits == 0 || bits >= 128 {
        return raw as i128;
    }
    let masked = (raw as i128) & ((1i128 << bits) - 1);
    if ty.is_unsigned() {
        masked
    } else if masked >= 1i128 << (bits - 1) {
        masked - (1i128 << bits)
    } else {
        masked
    }
}

/// The closed interval a small unsigned IV takes on every iteration that the
/// exit test does not reject.
///
/// * `lo..=hi` is inclusive and SOUND in both directions: `lo` is at or below
///   the smallest value the body observes and `hi` is at or above the largest.
///   An interval that UNDER-approximates is not a loose proof, it is a
///   miscompile: that was A1, where the ascending formula `hi = limit - 1` was
///   applied to a descending IV and landed BELOW the range, so a product
///   certificate checked a product the loop never computes and passed.
/// * `bounded` says whether `hi` is the exact body maximum, i.e. whether it was
///   derived from a literal endpoint. When it is `false`, `hi` has degraded to
///   the type maximum: still sound, but too loose to certify a product at any
///   `stride >= 2`, so the numeric half of the proof is withdrawn and only the
///   IV no-wrap half stands.
///
/// `bounded` is deliberately NOT "the limit was a constant", which is what it
/// used to mean. The limit fixes the ascending ceiling and the descending
/// FLOOR; the descending ceiling is the INIT. Inheriting the limit's constness
/// withdrew a certificate the init had already earned whenever the limit was
/// loop-invariant rather than literal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct UnsignedIvBound {
    lo: i128,
    hi: i128,
    bounded: bool,
}

/// Local no-wrap proof for a narrow unsigned IV.
///
/// The HEADER's exit test must guard every taken backedge, compare this exact
/// phi in the phi's own unsigned type, have the polarity that matches the step
/// sign, and step by exactly one. A self-loop header, an interior test, a
/// non-unit step or an inclusive limit is not a proof — those shapes are left
/// to the (already modular) index recurrence rather than a pointer recurrence.
/// IVs that `iv_widen` promoted to the pointer width need no new proof here:
/// their recurrence already lives in the pointer ring.
fn unsigned_iv_bound(
    func: &IrFunction,
    lp: &NaturalLoop,
    iv: &BasicIV,
    back_blocks: &[usize],
) -> Option<UnsignedIvBound> {
    use crate::ir::reexports::{IrCmpOp as C, Terminator};
    // This interval is interpreted in the IV's own unsigned domain below.
    // The main IV-widening caller currently enforces this too, but affine-Add
    // proofs call the helper independently; accepting a signed phi paired with
    // an unsigned Cmp would decode signed carrier bits as an unsigned range.
    if !iv.ty.is_integer() || !iv.ty.is_unsigned() || iv.ty.size() == 0 || iv.ty.size() > 8 {
        return None;
    }
    if back_blocks.contains(&lp.header) {
        return None;
    }
    let width = (iv.ty.size() * 8) as u32;
    let max = (1i128 << width) - 1;
    let header = &func.blocks[lp.header];
    let Terminator::CondBranch {
        cond: Operand::Value(cond),
        true_label,
        false_label,
    } = header.terminator
    else {
        return None;
    };
    let inside = |label| {
        func.blocks
            .iter()
            .position(|b| b.label == label)
            .is_some_and(|i| lp.body.contains(&i))
    };
    let true_inside = inside(true_label);
    // Both targets on the same side of the loop boundary proves nothing about
    // which edge is the backedge.
    if true_inside == inside(false_label) {
        return None;
    }
    for inst in &header.instructions {
        let Instruction::Cmp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        } = inst
        else {
            continue;
        };
        // The branch condition must be THIS comparison, in the IV's own
        // unsigned type: a comparison of a cast copy says nothing about the
        // phi's own domain.
        if *dest != cond || *ty != iv.ty {
            continue;
        }
        let (mut op, other) = if *lhs == Operand::Value(iv.phi_dest) {
            (*op, rhs)
        } else if *rhs == Operand::Value(iv.phi_dest) {
            let reversed = match op {
                C::Ult => C::Ugt,
                C::Ugt => C::Ult,
                C::Ule => C::Uge,
                C::Uge => C::Ule,
                _ => return None,
            };
            (reversed, lhs)
        } else {
            return None;
        };
        // Whether the LIMIT is a compile-time constant. It fixes the ascending
        // ceiling and the descending floor, but it does not decide `bounded` on
        // its own: each arm below names the endpoint that fixes ITS ceiling.
        let limit_is_const = matches!(other, Operand::Const(_));
        if let Operand::Value(v) = other
            && !is_loop_invariant(v.0, &lp.body, func)
        {
            return None;
        }
        // Normalise to "the loop continues while <op> holds".
        if !true_inside {
            op = match op {
                C::Ult => C::Uge,
                C::Ugt => C::Ule,
                C::Ule => C::Ugt,
                C::Uge => C::Ult,
                _ => return None,
            };
        }
        // Strict limit, unit step, polarity matching the step sign. An
        // inclusive limit (`<= max`) or a multi-unit step can still wrap.
        if !matches!((op, iv.step), (C::Ult, 1) | (C::Ugt, -1)) {
            return None;
        }
        // ASCENDING (`i < limit`, step +1): the body's values are
        // `[init, limit - 1]`, so a constant limit yields the exact maximum
        // `limit - 1`, and `lo = 0` because an `init` above the limit simply
        // never enters.
        //
        // DESCENDING (`i > limit`, step -1): the body's values are
        // `[limit + 1, init]`, so the ceiling comes from the INIT and the floor
        // from the limit. Deriving `hi` from the limit here — the ascending
        // formula — was a real defect: `limit - 1` is not merely loose, it is
        // BELOW the range, so a product certificate built on it checked `99 * 4`
        // for a loop descending from 2^30 toward a limit of 100 whose executed
        // maximum was `2^30 * 4`, admitting a narrow unsigned product that really
        // does wrap.
        //
        // With the init available as a literal the ceiling IS the body maximum,
        // so the descending arm is certified rather than refused — a recovered
        // optimization, not only a fix — and that holds whether or not the LIMIT
        // is a literal too: a loop-invariant limit only moves the floor, and `1`
        // is a sound floor for every descending unsigned IV. Without a literal
        // init there is no provable numeric maximum, so `hi` degrades to the type
        // maximum and `bounded` goes false: that withdraws the numeric product
        // certificate while keeping the IV *no-wrap* half of the proof, which
        // never depended on the init (entry still requires `init > limit`, and
        // the floor is still `limit + 1 >= 1`).
        //
        // `const_in_iv_domain` maps any constant into `[0, max]` for an unsigned
        // `iv.ty`, so no further range check is needed on the init; an init at or
        // below the limit never enters the body, and `init.max(floor)` is a sound
        // over-approximation of that empty interval.
        let (lo, hi, bounded) = if iv.step > 0 {
            let hi = match other {
                Operand::Const(c) => const_in_iv_domain(c.to_i64().unwrap_or(i64::MAX), iv.ty)
                    .checked_sub(1)
                    .unwrap_or(max),
                Operand::Value(_) => max,
            };
            (0i128, hi, limit_is_const)
        } else {
            let floor = match other {
                Operand::Const(c) => const_in_iv_domain(c.to_i64().unwrap_or(i64::MAX), iv.ty)
                    .checked_add(1)
                    .filter(|v| *v >= 1)
                    .unwrap_or(1i128),
                Operand::Value(_) => 1i128,
            };
            match iv.init {
                Operand::Const(c) => {
                    let init = const_in_iv_domain(c.to_i64().unwrap_or(i64::MAX), iv.ty);
                    // `init >= floor` is exactly the condition for `hi` to be the
                    // body maximum. Below the floor the body never executes, so
                    // there is no body maximum to be exact about and `hi` is only
                    // a sound over-approximation of an empty interval.
                    (floor, init.max(floor), init >= floor)
                }
                Operand::Value(_) => (floor, max, false),
            }
        };
        return Some(UnsignedIvBound { lo, hi, bounded });
    }
    None
}

/// Decode an affine Add's constant in the Add type's own domain and prove
/// that a narrow unsigned `iv + k` cannot wrap over any value the loop body
/// may observe. The returned offset is the mathematical `k` used to seed the
/// pointer recurrence; `None` means the affine form must remain untouched.
fn affine_add_offset_cannot_wrap(
    func: &IrFunction,
    lp: &NaturalLoop,
    back_blocks: &[usize],
    iv: &BasicIV,
    raw_offset: i64,
    add_ty: IrType,
) -> Option<i64> {
    if !add_ty.is_integer() {
        return None;
    }
    if super::signed_overflow_wraps()
        && !add_ty.is_unsigned()
        && add_ty.size() < crate::common::types::target_ptr_size()
        && raw_offset != 0
    {
        return None;
    }
    let narrow_unsigned =
        add_ty.is_unsigned() && (add_ty.size() as usize) < crate::common::types::target_ptr_size();
    if !narrow_unsigned {
        return Some(raw_offset);
    }
    // The pre-widening Add's no-wrap interval below is an unsigned-IV proof.
    // Do not let a signed phi enter it just because the comparison happens to
    // use Ult/Ugt; that pair has no unsigned-phi range semantics.
    if !iv.ty.is_unsigned() {
        return None;
    }

    // An IrConst's signed carrier is not its unsigned mathematical value.
    // For example, U32's 0xffff_ffff arrives as -1; when widened after the
    // Add it denotes +4294967295, not pointer-ring -1.
    let k = const_in_iv_domain(raw_offset, add_ty);
    let k = i64::try_from(k).ok()?;
    if k == 0 {
        return Some(0);
    }

    // Only an unsigned IV with a header-test certificate supplies a sound
    // interval for the pre-widening Add. The certificate may be an over-approx
    // (for a dynamic endpoint); that is fine, since a range that is too wide
    // merely refuses more cases.
    let bound = unsigned_iv_bound(func, lp, iv, back_blocks)?;
    let limit = 1i128 << ((add_ty.size() * 8) as u32);
    let lo = bound.lo.checked_add(k as i128)?;
    let hi = bound.hi.checked_add(k as i128)?;
    (lo >= 0 && hi < limit).then_some(k)
}

/// Whether replacing `offset(iv) = (iv + add_offset) * stride`, evaluated in
/// `mul_ty`, by the pointer recurrence `p += step * stride` can diverge.
///
/// Divergence needs the PRODUCT to wrap inside `mul_ty` while the pointer ring
/// keeps counting linearly. A signed `mul_ty` cannot wrap in a defined program
/// (C17 6.5/5 — the same UB theorem `iv_widen` uses), and a product at or above
/// the pointer width wraps in exactly the ring the recurrence lives in, so both
/// are equivalent by construction. Only an UNSIGNED product NARROWER than the
/// pointer ring is a genuine hazard, and a bounded interval discharges it.
fn offset_product_cannot_overflow(
    mul_ty: IrType,
    stride: i64,
    add_offset: i64,
    bound: Option<UnsignedIvBound>,
) -> bool {
    let ptr = crate::common::types::target_ptr_size();
    if !mul_ty.is_integer() || (mul_ty.size() as usize) >= ptr || !mul_ty.is_unsigned() {
        return true;
    }
    let Some(b) = bound.filter(|b| b.bounded) else {
        return false;
    };
    let limit = 1i128 << ((mul_ty.size() * 8) as u32);
    let lo = (b.lo + add_offset as i128).checked_mul(stride as i128);
    let hi = (b.hi + add_offset as i128).checked_mul(stride as i128);
    matches!((lo, hi), (Some(lo), Some(hi)) if lo >= 0 && hi < limit)
}

/// Look through Cast and Copy instructions to find the root value.
/// Used to match `Cast(Add(Cast(phi_dest), step))` patterns.
fn look_through_casts(val_id: u32, loop_defs: &FxHashMap<u32, &Instruction>) -> u32 {
    let mut current = val_id;
    for _ in 0..MAX_CAST_CHAIN_LENGTH {
        if let Some(inst) = loop_defs.get(&current) {
            match inst {
                // A truncation on the backedge is part of the recurrence,
                // not a transparent copy. In particular, C promotes u8/u16
                // increments to int and then truncates: the wrap is DEFINED.
                // Replacing that recurrence with a pointer bump walks off
                // the array at 255 -> 0 (IVSR-WRAP-1).
                Instruction::Cast {
                    src: Operand::Value(v),
                    from_ty,
                    to_ty,
                    ..
                } if from_ty == to_ty => current = v.0,
                Instruction::Copy {
                    src: Operand::Value(v),
                    ..
                } => current = v.0,
                _ => break,
            }
        } else {
            break;
        }
    }
    current
}

/// Try to resolve an operand to a constant i64 value, looking through Copies.
fn try_resolve_const(op: &Operand, func: &IrFunction) -> Option<i64> {
    match op {
        Operand::Const(c) => c.to_i64(),
        Operand::Value(v) => {
            for block in &func.blocks {
                for inst in &block.instructions {
                    if let Instruction::Copy { dest, src } = inst {
                        if *dest == *v {
                            return try_resolve_const(src, func);
                        }
                    }
                }
            }
            None
        }
    }
}

/// Check if a value is loop-invariant (defined outside the loop).
fn is_loop_invariant(val_id: u32, loop_body: &FxHashSet<usize>, func: &IrFunction) -> bool {
    for &bi in loop_body {
        if bi >= func.blocks.len() {
            continue;
        }
        for inst in &func.blocks[bi].instructions {
            if let Some(dest) = inst.dest() {
                if dest.0 == val_id {
                    return false;
                }
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fx_hash::FxHashSet;
    use crate::common::types::{AddressSpace, IrType};
    use crate::ir::reexports::{BasicBlock, BlockId, IrCmpOp, Terminator};

    #[test]
    fn nested_or_overlapping_loops_inner_kept_outer_skipped() {
        // Inner loop bodies are subsets of outer.  New policy keeps inner and
        // skips outer that overlaps an already-kept inner, rather than skipping
        // both.  This test validates the disjoint checks used by that policy.
        let outer = NaturalLoop {
            header: 1,
            body: [1usize, 2, 3, 4].into_iter().collect::<FxHashSet<_>>(),
        };
        let inner = NaturalLoop {
            header: 2,
            body: [2usize, 3].into_iter().collect::<FxHashSet<_>>(),
        };
        let separate = NaturalLoop {
            header: 7,
            body: [7usize, 8].into_iter().collect::<FxHashSet<_>>(),
        };
        // Simulate sorted asc processing: inner first, then outer, then separate
        let mut kept: Vec<FxHashSet<usize>> = Vec::new();
        // inner does not overlap kept (empty) -> keep
        assert!(
            !kept
                .iter()
                .any(|b: &FxHashSet<usize>| !b.is_disjoint(&inner.body))
        );
        kept.push(inner.body.clone());
        // outer overlaps kept inner -> should be skipped
        assert!(kept.iter().any(|b| !b.is_disjoint(&outer.body)));
        // separate does not overlap kept -> keep
        assert!(!kept.iter().any(|b| !b.is_disjoint(&separate.body)));
    }

    /// Exact promote/add/truncate backedge, including Copy wrappers. IDs are
    /// deliberately independent of block numbers; detection must follow SSA.
    fn cast_backedge(phi_ty: IrType, arithmetic_ty: IrType) -> IrFunction {
        let mut f = IrFunction::new("cast_backedge".into(), IrType::I32, vec![], false);
        f.blocks = vec![
            BasicBlock {
                label: BlockId(10),
                instructions: vec![],
                terminator: Terminator::Branch(BlockId(20)),
                source_spans: vec![],
            },
            BasicBlock {
                label: BlockId(20),
                instructions: vec![
                    Instruction::Phi {
                        dest: Value(0),
                        ty: phi_ty,
                        incoming: vec![
                            (Operand::Const(IrConst::I32(0)), BlockId(10)),
                            (Operand::Value(Value(4)), BlockId(30)),
                        ],
                    },
                    Instruction::Cmp {
                        dest: Value(5),
                        op: IrCmpOp::Ult,
                        lhs: Operand::Value(Value(0)),
                        rhs: Operand::Const(IrConst::I32(8)),
                        ty: phi_ty,
                    },
                ],
                terminator: Terminator::CondBranch {
                    cond: Operand::Value(Value(5)),
                    true_label: BlockId(30),
                    false_label: BlockId(40),
                },
                source_spans: vec![],
            },
            BasicBlock {
                label: BlockId(30),
                instructions: vec![
                    Instruction::Cast {
                        dest: Value(1),
                        src: Operand::Value(Value(0)),
                        from_ty: phi_ty,
                        to_ty: arithmetic_ty,
                    },
                    Instruction::BinOp {
                        dest: Value(2),
                        op: IrBinOp::Add,
                        lhs: Operand::Value(Value(1)),
                        rhs: Operand::Const(IrConst::I32(1)),
                        ty: arithmetic_ty,
                    },
                    Instruction::Cast {
                        dest: Value(3),
                        src: Operand::Value(Value(2)),
                        from_ty: arithmetic_ty,
                        to_ty: phi_ty,
                    },
                    Instruction::Copy {
                        dest: Value(4),
                        src: Operand::Value(Value(3)),
                    },
                ],
                terminator: Terminator::Branch(BlockId(20)),
                source_spans: vec![],
            },
            BasicBlock {
                label: BlockId(40),
                instructions: vec![],
                terminator: Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
                source_spans: vec![],
            },
        ];
        f.next_value_id = 6;
        f
    }

    #[test]
    fn promoted_narrow_backedges_are_not_linear_ivs() {
        for ty in [IrType::U8, IrType::U16, IrType::I8, IrType::I16] {
            let f = cast_backedge(ty, IrType::I32);
            assert!(
                find_basic_ivs(&f, 1, &[1, 2].into_iter().collect(), 0, &[2]).is_empty(),
                "{ty:?}"
            );
        }
    }

    #[test]
    fn identity_casts_and_copies_keep_basic_ivs_but_signedness_needs_proof() {
        for ty in [IrType::I32, IrType::U32, IrType::I64, IrType::U64] {
            let f = cast_backedge(ty, ty);
            let ivs = find_basic_ivs(&f, 1, &[1, 2].into_iter().collect(), 0, &[2]);
            assert_eq!(ivs.len(), 1);
            assert_eq!(ivs[0].step, 1);
        }
        for (from, to) in [
            (IrType::I32, IrType::U32),
            (IrType::U32, IrType::I32),
            (IrType::I64, IrType::U64),
            (IrType::U64, IrType::I64),
        ] {
            let f = cast_backedge(from, to);
            assert!(find_basic_ivs(&f, 1, &[1, 2].into_iter().collect(), 0, &[2]).is_empty());
        }
    }

    #[test]
    fn const_in_iv_domain_reads_a_constant_in_the_ivs_own_domain() {
        // The A4 disagreement: the bound used the raw carrier, the initial offset
        // reinterpreted by type. One helper, both sites.
        assert_eq!(const_in_iv_domain(-1, IrType::U32), 4294967295);
        assert_eq!(const_in_iv_domain(-1, IrType::I32), -1);
        assert_eq!(const_in_iv_domain(-1, IrType::U8), 255);
        assert_eq!(const_in_iv_domain(-1, IrType::I8), -1);
        assert_eq!(const_in_iv_domain(97, IrType::I8), 97);
        assert_eq!(const_in_iv_domain(65535, IrType::U16), 65535);
        assert_eq!(const_in_iv_domain(-1, IrType::U64), (1i128 << 64) - 1);
        assert_eq!(const_in_iv_domain(-1, IrType::I64), -1);
        // Round-trips: reinterpreting a value already in domain is the identity.
        for ty in [
            IrType::I8,
            IrType::U8,
            IrType::I16,
            IrType::U16,
            IrType::I32,
            IrType::U32,
            IrType::I64,
            IrType::U64,
        ] {
            for raw in [
                0i64,
                1,
                -1,
                127,
                -128,
                65535,
                i32::MAX as i64,
                i32::MIN as i64,
            ] {
                let once = const_in_iv_domain(raw, ty);
                assert_eq!(const_in_iv_domain(once as i64, ty), once, "{ty:?} {raw}");
            }
        }
    }

    #[test]
    fn offset_cast_proof_adds_only_the_pointer_ring_reinterpretation() {
        let ints = [
            IrType::I8,
            IrType::U8,
            IrType::I16,
            IrType::U16,
            IrType::I32,
            IrType::U32,
            IrType::I64,
            IrType::U64,
        ];
        let ptr = crate::common::types::target_ptr_size();
        for from in ints {
            for to in ints {
                // Model: value-preserving widening, or a reinterpretation that
                // cannot change the pattern reaching the pointer ring.
                let ring = from.is_integer()
                    && to.is_integer()
                    && from.size() == to.size()
                    && (from.size() as usize) >= ptr;
                assert_eq!(
                    from.cast_preserves_offset_value(to),
                    from.cast_preserves_integer_value(to) || ring,
                    "{from:?}->{to:?}"
                );
            }
        }
        // The exact IVSR-DOMAIN-1 composition stays rejected at every width.
        for (from, to) in [
            (IrType::U32, IrType::I32),
            (IrType::I32, IrType::U32),
            (IrType::U16, IrType::I16),
            (IrType::I16, IrType::U16),
            (IrType::U8, IrType::I8),
            (IrType::I8, IrType::U8),
        ] {
            assert!(!from.cast_preserves_offset_value(to), "{from:?}->{to:?}");
        }
        // The pointer-ring reinterpretation is admitted only at pointer width,
        // which is what keeps the 64-bit `size_t`/`ptrdiff_t` spelling reducible.
        let wide = (IrType::U64.size() as usize) >= ptr;
        assert_eq!(IrType::U64.cast_preserves_offset_value(IrType::I64), wide);
        assert_eq!(IrType::I64.cast_preserves_offset_value(IrType::U64), wide);
        assert!(!IrType::I32.cast_preserves_offset_value(IrType::F32));
        assert!(!IrType::F64.cast_preserves_offset_value(IrType::I64));
        assert!(!IrType::I64.cast_preserves_offset_value(IrType::Ptr));
    }

    #[test]
    fn offset_product_proof_needs_a_bounded_interval_only_below_the_pointer_ring() {
        let ptr = crate::common::types::target_ptr_size();
        let bounded = Some(UnsignedIvBound {
            lo: 0,
            hi: 1023,
            bounded: true,
        });
        let unbounded = Some(UnsignedIvBound {
            lo: 0,
            hi: (1i128 << 32) - 1,
            bounded: false,
        });
        // Signed and pointer-ring products are UB/ring-equivalent: always OK.
        for ty in [IrType::I32, IrType::I64, IrType::U64] {
            if ty.size() >= ptr || !ty.is_unsigned() {
                assert!(offset_product_cannot_overflow(ty, 4, 0, None), "{ty:?}");
            }
        }
        // Unsigned 32-bit product: needs a bounded interval, and the product
        // must really fit.
        assert!(offset_product_cannot_overflow(IrType::U32, 4, 0, bounded));
        assert!(!offset_product_cannot_overflow(
            IrType::U32,
            4,
            0,
            unbounded
        ));
        assert!(!offset_product_cannot_overflow(IrType::U32, 4, 0, None));
        let too_big = Some(UnsignedIvBound {
            lo: 0,
            hi: (1i128 << 31) - 1,
            bounded: true,
        });
        assert!(!offset_product_cannot_overflow(IrType::U32, 4, 0, too_big));
        // An affine offset participates in the product bound.
        assert!(offset_product_cannot_overflow(IrType::U32, 4, 8, bounded));
        let edge = Some(UnsignedIvBound {
            lo: 0,
            hi: (1i128 << 30) - 9,
            bounded: true,
        });
        assert!(offset_product_cannot_overflow(IrType::U32, 4, 8, edge));
        let over = Some(UnsignedIvBound {
            lo: 0,
            hi: (1i128 << 30) - 8,
            bounded: true,
        });
        assert!(!offset_product_cannot_overflow(IrType::U32, 4, 8, over));
        // The predicate only ever sees the integer type of a Mul/Shl/Add;
        // a non-integer `mul_ty` is vacuously admitted and cannot arise.
        assert!(offset_product_cannot_overflow(IrType::F32, 4, 0, bounded));
    }

    #[test]
    fn unsigned_bound_is_bounded_by_whichever_endpoint_fixes_the_ceiling() {
        use crate::ir::reexports::IrCmpOp as C;
        let lp = NaturalLoop {
            header: 1,
            body: [1, 2].into_iter().collect(),
        };
        let iv = BasicIV {
            phi_dest: Value(0),
            ty: IrType::U32,
            init: Operand::Const(IrConst::I32(0)),
            step: 1,
        };
        let mut f = cast_backedge(IrType::U32, IrType::U32);
        // ASCENDING, constant limit 8: body values are exactly 0..=7, so the
        // limit is the endpoint that fixes the ceiling.
        let b = unsigned_iv_bound(&f, &lp, &iv, &[2]).expect("constant bound");
        assert!(b.bounded && b.lo == 0 && b.hi == 7, "{b:?}");
        // Replace the limit with a loop-invariant value: still monotone, but the
        // ceiling is no longer known, so `hi` degrades to the type max.
        f.blocks[0].instructions.push(Instruction::Copy {
            dest: Value(7),
            src: Operand::Const(IrConst::I32(8)),
        });
        if let Instruction::Cmp { rhs, .. } = &mut f.blocks[1].instructions[1] {
            *rhs = Operand::Value(Value(7));
        }
        let b = unsigned_iv_bound(&f, &lp, &iv, &[2]).expect("invariant bound");
        assert!(!b.bounded && b.hi == (1i128 << 32) - 1, "{b:?}");
        // A loop-VARIANT limit is no bound at all.
        if let Instruction::Cmp { rhs, .. } = &mut f.blocks[1].instructions[1] {
            *rhs = Operand::Value(Value(3));
        }
        assert!(unsigned_iv_bound(&f, &lp, &iv, &[2]).is_none());
        // Inclusive limits and multi-unit steps can still wrap.
        let mut g = cast_backedge(IrType::U32, IrType::U32);
        if let Instruction::Cmp { op, .. } = &mut g.blocks[1].instructions[1] {
            *op = C::Ule;
        }
        assert!(unsigned_iv_bound(&g, &lp, &iv, &[2]).is_none());
    }

    /// The limit constant is decoded in the IV's OWN ring, and the product
    /// certificate is built from the decoded value: `uint32_t` `0x80000003`
    /// arrives as `I32(-2147483645)`. Reading the carrier raw gave
    /// `hi = -2147483646`, a "proof" that `2*i` cannot exceed 2^32 -- exactly
    /// the wrap the certificate exists to refuse (IVSR-WRAP-2). End to end on
    /// the shipped shape, both directions.
    #[test]
    fn unsigned_wrap_certificate_uses_the_decoded_limit() {
        use crate::ir::reexports::IrCmpOp as C;
        let lp = NaturalLoop {
            header: 1,
            body: [1, 2].into_iter().collect(),
        };
        let iv = BasicIV {
            phi_dest: Value(0),
            ty: IrType::U32,
            init: Operand::Const(IrConst::I32(0)),
            step: 1,
        };
        let mut f = cast_backedge(IrType::U32, IrType::U32);
        // Limit 0x80000003 in the U32 ring: 2*i wraps on the top iterations.
        if let Instruction::Cmp { rhs, op, .. } = &mut f.blocks[1].instructions[1] {
            *op = C::Ult;
            *rhs = Operand::Const(IrConst::I32(-2147483645));
        }
        let b = unsigned_iv_bound(&f, &lp, &iv, &[2]).expect("constant bound");
        assert!(b.bounded && b.lo == 0 && b.hi == 0x8000_0002, "{b:?}");
        assert!(
            !offset_product_cannot_overflow(IrType::U32, 2, 0, Some(b)),
            "2*i over a bound above 2^31 must not be certified"
        );
        // The same product under a small bound is genuinely linear.
        if let Instruction::Cmp { rhs, .. } = &mut f.blocks[1].instructions[1] {
            *rhs = Operand::Const(IrConst::I32(8));
        }
        let b = unsigned_iv_bound(&f, &lp, &iv, &[2]).expect("constant bound");
        assert!(offset_product_cannot_overflow(IrType::U32, 2, 0, Some(b)));
        // An unbounded interval is never a certificate...
        assert!(!offset_product_cannot_overflow(
            IrType::U32,
            2,
            0,
            Some(UnsignedIvBound {
                lo: 0,
                hi: 10,
                bounded: false,
            })
        ));
        // ...and a small bounded one is (2 * 7 == 14 < 2^32).
        assert!(offset_product_cannot_overflow(
            IrType::U32,
            2,
            0,
            Some(UnsignedIvBound {
                lo: 0,
                hi: 7,
                bounded: true,
            })
        ));
        // Products at or above the pointer ring are exact by construction.
        assert!(offset_product_cannot_overflow(IrType::U64, 2, 0, None));
        assert!(offset_product_cannot_overflow(IrType::I32, 2, 0, None));
    }

    #[test]
    fn derived_cast_proof_is_about_values_not_storage_size() {
        let ints = [
            IrType::I8,
            IrType::U8,
            IrType::I16,
            IrType::U16,
            IrType::I32,
            IrType::U32,
            IrType::I64,
            IrType::U64,
        ];
        // Exhaust the extrema of every integer domain against every cast domain.
        let range = |ty: IrType| {
            let bits = ty.size() * 8;
            if ty.is_unsigned() {
                (0_i128, (1_i128 << bits) - 1)
            } else {
                (-(1_i128 << (bits - 1)), (1_i128 << (bits - 1)) - 1)
            }
        };
        for from in ints {
            for to in ints {
                let (lo, hi) = range(from);
                let (tl, th) = range(to);
                assert_eq!(
                    from.cast_preserves_integer_value(to),
                    lo >= tl && hi <= th,
                    "{from:?}->{to:?}"
                );
            }
        }
        assert!(!IrType::I32.cast_preserves_integer_value(IrType::F32));
        assert!(!IrType::F64.cast_preserves_integer_value(IrType::I64));
    }

    #[test]
    fn unsigned_no_wrap_requires_header_polarity_unit_step_and_unsigned_cmp() {
        let lp = NaturalLoop {
            header: 1,
            body: [1, 2].into_iter().collect(),
        };
        for (op, truth_stays, step, expected) in [
            (IrCmpOp::Ult, true, 1, true),
            (IrCmpOp::Uge, false, 1, true),
            (IrCmpOp::Ult, false, 1, false),
            (IrCmpOp::Ule, true, 1, false),
            (IrCmpOp::Slt, true, 1, false),
            (IrCmpOp::Ult, true, 2, false),
            (IrCmpOp::Ugt, true, -1, true),
            (IrCmpOp::Uge, true, -1, false),
        ] {
            let mut f = cast_backedge(IrType::U32, IrType::U32);
            if let Instruction::Cmp { op: cmp, .. } = &mut f.blocks[1].instructions[1] {
                *cmp = op;
            }
            if !truth_stays {
                f.blocks[1].terminator = Terminator::CondBranch {
                    cond: Operand::Value(Value(5)),
                    true_label: BlockId(40),
                    false_label: BlockId(30),
                };
            }
            let iv = BasicIV {
                phi_dest: Value(0),
                ty: IrType::U32,
                init: Operand::Const(IrConst::I32(0)),
                step,
            };
            assert_eq!(
                unsigned_iv_bound(&f, &lp, &iv, &[2]).is_some(),
                expected,
                "{op:?} {truth_stays} {step}"
            );
            assert!(unsigned_iv_bound(&f, &lp, &iv, &[1]).is_none());
        }
    }

    #[test]
    fn unsigned_bound_rejects_signed_iv_even_with_unsigned_comparison() {
        use crate::ir::reexports::IrCmpOp as C;
        let lp = NaturalLoop {
            header: 1,
            body: [1, 2].into_iter().collect(),
        };
        let mut f = cast_backedge(IrType::I32, IrType::I32);
        if let Instruction::Cmp { op, .. } = &mut f.blocks[1].instructions[1] {
            *op = C::Ult;
        }
        let iv = BasicIV {
            phi_dest: Value(0),
            ty: IrType::I32,
            init: Operand::Const(IrConst::I32(0)),
            step: 1,
        };
        assert!(
            unsigned_iv_bound(&f, &lp, &iv, &[2]).is_none(),
            "an unsigned comparison does not turn a signed IV into an unsigned range"
        );
        assert!(
            affine_add_offset_cannot_wrap(&f, &lp, &[2], &iv, 1, IrType::U8).is_none(),
            "the narrow-unsigned Add proof must fail closed on a signed IV"
        );
    }

    #[test]
    fn derived_signedness_then_widening_is_not_a_pointer_iv() {
        use crate::common::types::AddressSpace;
        for (middle, expected) in [(IrType::I32, 0), (IrType::U32, 1)] {
            let mut f = cast_backedge(IrType::U32, IrType::U32);
            f.blocks[0].instructions.push(Instruction::Copy {
                dest: Value(10),
                src: Operand::Const(IrConst::I64(4096)),
            });
            f.blocks[2].instructions.extend([
                Instruction::Cast {
                    dest: Value(6),
                    src: Operand::Value(Value(0)),
                    from_ty: IrType::U32,
                    to_ty: middle,
                },
                Instruction::Cast {
                    dest: Value(7),
                    src: Operand::Value(Value(6)),
                    from_ty: middle,
                    to_ty: IrType::I64,
                },
                Instruction::BinOp {
                    dest: Value(8),
                    op: IrBinOp::Mul,
                    lhs: Operand::Value(Value(7)),
                    rhs: Operand::Const(IrConst::I64(24)),
                    ty: IrType::I64,
                },
                Instruction::GetElementPtr {
                    dest: Value(9),
                    base: Value(10),
                    offset: Operand::Value(Value(8)),
                    ty: IrType::I32,
                },
                Instruction::Load {
                    dest: Value(11),
                    ptr: Value(9),
                    ty: IrType::I32,
                    volatile: true,
                    seg_override: AddressSpace::Default,
                },
            ]);
            f.next_value_id = 12;
            assert_eq!(ivsr_function(&mut f), expected);
        }
    }

    /// Test basic IV detection on a simple counting loop.
    #[test]
    fn test_find_basic_iv() {
        let mut func = IrFunction::new("test".to_string(), IrType::I32, vec![], false);

        // Block 0 (preheader): init = 0
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::Copy {
                dest: Value(0),
                src: Operand::Const(IrConst::I32(0)),
            }],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });

        // Block 1 (header): i = phi(0, i_next)
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(1),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Value(Value(0)), BlockId(0)),
                        (Operand::Value(Value(3)), BlockId(2)),
                    ],
                },
                Instruction::Cmp {
                    dest: Value(2),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(100)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(2)),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
            source_spans: Vec::new(),
        });

        // Block 2 (body): i_next = i + 1
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![Instruction::BinOp {
                dest: Value(3),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::I32,
            }],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });

        // Block 3 (exit)
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![],
            terminator: Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
            source_spans: Vec::new(),
        });

        func.next_value_id = 4;

        let ivs = find_basic_ivs(&func, 1, &[1, 2].iter().copied().collect(), 0, &[2]);
        assert_eq!(ivs.len(), 1);
        assert_eq!(ivs[0].phi_dest, Value(1));
        assert_eq!(ivs[0].step, 1);
    }

    /// `Add(invariant_ptr, byte_offset)` used as a memory address — the shape
    /// LCCC's own lowering emits for EVERY C subscript
    /// (`try_lower_pointer_arithmetic`), and the shape the GEP-only scan was
    /// blind to. `deref` selects whether the address is actually loaded
    /// through, which is what `is_used_as_address` keys on.
    fn address_add_loop(deref: bool, ptr_width_iv: bool) -> IrFunction {
        let mut f = IrFunction::new("address_add".into(), IrType::I64, vec![], false);
        let iv_ty = if ptr_width_iv {
            IrType::I64
        } else {
            IrType::I32
        };
        // v0 = &buffer (invariant base), defined in the preheader.
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Copy {
                    dest: Value(0),
                    src: Operand::Const(IrConst::I64(0x4000)),
                },
                Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Const(IrConst::I32(0)),
                },
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: vec![],
        });
        // header: i = phi(0, i_next); while (i < 64)
        f.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(2),
                    ty: iv_ty,
                    incoming: vec![
                        (Operand::Const(IrConst::I32(0)), BlockId(0)),
                        (Operand::Value(Value(9)), BlockId(2)),
                    ],
                },
                Instruction::Cmp {
                    dest: Value(3),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Const(IrConst::I32(64)),
                    ty: iv_ty,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(3)),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
            source_spans: vec![],
        });
        // body: off = (I64)i; addr = base + off; [sum += *addr]; i_next = i + 1
        let mut body = vec![
            Instruction::Cast {
                dest: Value(4),
                src: Operand::Value(Value(2)),
                from_ty: iv_ty,
                to_ty: IrType::I64,
            },
            Instruction::BinOp {
                dest: Value(5),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(0)),
                rhs: Operand::Value(Value(4)),
                ty: IrType::I64,
            },
        ];
        if deref {
            body.push(Instruction::Load {
                volatile: false,
                dest: Value(6),
                ptr: Value(5),
                ty: IrType::I8,
                seg_override: AddressSpace::Default,
            });
        } else {
            // Same arithmetic, but nobody dereferences it: an integer
            // accumulation that happens to be pointer-width.
            body.push(Instruction::Copy {
                dest: Value(6),
                src: Operand::Value(Value(5)),
            });
        }
        body.push(Instruction::BinOp {
            dest: Value(9),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(2)),
            rhs: Operand::Const(IrConst::I32(1)),
            ty: iv_ty,
        });
        f.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: body,
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: vec![],
        });
        f.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![Instruction::Copy {
                dest: Value(7),
                src: Operand::Value(Value(6)),
            }],
            terminator: Terminator::Return(Some(Operand::Value(Value(7)))),
            source_spans: vec![],
        });
        f.next_value_id = 10;
        f
    }

    /// An unsigned U32 affine index is formed BEFORE widening it into the
    /// pointer ring: `addr = base + (I64)(i + 2)`.  The loop and comparison
    /// deliberately admit values for which the narrow Add wraps.
    fn narrow_unsigned_affine_address_loop(init: i32, limit: i32) -> IrFunction {
        let mut f = IrFunction::new("narrow_unsigned_affine".into(), IrType::I32, vec![], false);
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::Copy {
                dest: Value(0),
                src: Operand::Const(IrConst::I64(0x4000)),
            }],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: vec![],
        });
        f.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(2),
                    ty: IrType::U32,
                    incoming: vec![
                        (Operand::Const(IrConst::I32(init)), BlockId(0)),
                        (Operand::Value(Value(10)), BlockId(2)),
                    ],
                },
                Instruction::Cmp {
                    dest: Value(3),
                    op: IrCmpOp::Ult,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Const(IrConst::I32(limit)),
                    ty: IrType::U32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(3)),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
            source_spans: vec![],
        });
        f.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![
                Instruction::BinOp {
                    dest: Value(4),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Const(IrConst::I32(2)),
                    ty: IrType::U32,
                },
                Instruction::Cast {
                    dest: Value(5),
                    src: Operand::Value(Value(4)),
                    from_ty: IrType::U32,
                    to_ty: IrType::I64,
                },
                Instruction::BinOp {
                    dest: Value(6),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Value(Value(5)),
                    ty: IrType::I64,
                },
                Instruction::Load {
                    volatile: false,
                    dest: Value(7),
                    ptr: Value(6),
                    ty: IrType::I8,
                    seg_override: AddressSpace::Default,
                },
                Instruction::BinOp {
                    dest: Value(10),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::U32,
                },
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: vec![],
        });
        f.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![],
            terminator: Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
            source_spans: vec![],
        });
        f.next_value_id = 11;
        f
    }

    #[test]
    fn narrow_unsigned_affine_add_requires_an_operative_no_wrap_proof() {
        // U32 i runs through UINT32_MAX-1; `i + 2` wraps in U32 on its last
        // body iteration before the cast to I64.  A pointer-width product
        // check alone cannot see that intermediate wrap.
        let mut wrapping = narrow_unsigned_affine_address_loop(-4, -1);
        assert_eq!(
            ivsr_function(&mut wrapping),
            0,
            "the affine pointer recurrence must be declined when U32 i+2 may wrap"
        );
        assert!(
            wrapping.blocks[2].instructions.iter().any(|i| matches!(
                i,
                Instruction::BinOp { dest, op: IrBinOp::Add, ty: IrType::I64, .. }
                    if *dest == Value(6)
            )),
            "the original address Add must remain intact"
        );

        // A smaller, statically bounded interval proves the same U32 Add
        // cannot wrap, so LP64 retains the strength reduction.
        let mut bounded = narrow_unsigned_affine_address_loop(0, 64);
        let fired = ivsr_function(&mut bounded) > 0;
        assert_eq!(fired, !crate::common::types::target_is_32bit());
    }

    #[test]
    fn address_forming_add_becomes_a_pointer_recurrence() {
        // The exact k_varint shape: a byte-offset Add feeding a Load, with a
        // narrow signed IV. This is what the GEP-only scan could not see.
        let mut f = address_add_loop(true, false);
        assert!(
            ivsr_function(&mut f) > 0,
            "stride-1 address Add must reduce"
        );
        // The recurrence is a pointer phi bumped by the step, and the original
        // Add is replaced by a Copy of it — not left computing the address.
        let phis = f
            .blocks
            .iter()
            .flat_map(|b| b.instructions.iter())
            .filter(|i| {
                matches!(i, Instruction::Phi { ty, .. } if *ty == IrType::Ptr
                || i.result_type() == Some(IrType::I64))
            })
            .count();
        assert!(phis >= 1, "expected an inserted pointer phi");
        assert!(
            f.blocks[2].instructions.iter().any(|i| matches!(i,
            Instruction::Copy { dest, .. } if *dest == Value(5))),
            "the address Add must be rewritten to a Copy of the pointer IV"
        );
    }

    #[test]
    fn pointer_width_add_that_nobody_dereferences_is_left_alone() {
        // Same arithmetic, no memory operand: turning an integer accumulation
        // into a loop-carried recurrence is the net loss that keeps
        // CCC_IVSR_SCALAR_DERIVED opt-in, so is_used_as_address must refuse it.
        let mut f = address_add_loop(false, true);
        assert_eq!(ivsr_function(&mut f), 0);
    }

    #[test]
    fn address_add_arm_is_gated_by_ilp32() {
        // LP64: the arm fires. This pins that the gate reads the TARGET rather
        // than a hard-coded width, since the suite runs on the host default. It
        // does NOT test the kill switch — `ivsr_function` hardcodes
        // `ptr_add = true`, which is why the switch needs its own test below.
        let mut f = address_add_loop(true, false);
        let fired = ivsr_function(&mut f) > 0;
        assert_eq!(fired, !crate::common::types::target_is_32bit());
    }

    #[test]
    fn ptr_add_parameter_is_a_real_kill_switch() {
        // `CCC_NO_IVSR_PTR_ADD` is threaded from the pipeline as this parameter.
        // Before this test existed the switch had ZERO automated coverage: both
        // test wrappers pass `true`, so the only exercise of it was a manual
        // gzip A/B. A dead kill switch is worse than none — it promises an
        // escape hatch that may not work. This drives the parameter directly, so
        // it needs no environment mutation (and therefore no serialisation
        // against the rest of the suite).
        let cfg = analysis::CfgAnalysis::build(&address_add_loop(true, false));
        let mut on = address_add_loop(true, false);
        assert!(
            ivsr_with_analysis(&mut on, &cfg, false, true, true) > 0,
            "ptr_add=true must reduce the address-forming Add"
        );
        let mut off = address_add_loop(true, false);
        assert_eq!(
            ivsr_with_analysis(&mut off, &cfg, false, false, true),
            0,
            "ptr_add=false must disable the arm entirely"
        );
        // And the address Add must survive untouched when the arm is off.
        assert!(
            off.blocks[2].instructions.iter().any(|i| matches!(
                i,
                Instruction::BinOp { dest, op: IrBinOp::Add, .. } if *dest == Value(5)
            )),
            "with the arm off the original Add must still compute the address"
        );
    }

    #[test]
    fn descending_unsigned_iv_bounds_come_from_the_init_not_the_limit() {
        use crate::ir::reexports::IrCmpOp as C;
        let lp = NaturalLoop {
            header: 1,
            body: [1, 2].into_iter().collect(),
        };
        // `while (i > limit) i += -1` over a U32. The body observes
        // `[limit + 1, init]`, so the ceiling comes from the INIT and the floor
        // from the limit. (LCCC's frontend currently emits `Sub` for `i--`,
        // which `find_basic_ivs` does not accept, so no descending BasicIV is
        // formed from C source yet — see FOLLOWUP-2026-10-07 §"descending IVs".
        // The canonical `Add(i, -1)` spelling is what this pins, and it is what
        // any future canonicalisation would produce.)
        let fixture = |limit: i32| {
            let mut f = cast_backedge(IrType::U32, IrType::U32);
            if let Instruction::Cmp { op, rhs, .. } = &mut f.blocks[1].instructions[1] {
                *op = C::Ugt;
                *rhs = Operand::Const(IrConst::I32(limit));
            }
            if let Instruction::BinOp { rhs, .. } = &mut f.blocks[2].instructions[1] {
                *rhs = Operand::Const(IrConst::I32(-1));
            }
            f
        };
        let bound_of = |f: &IrFunction, init: Operand| {
            let iv = BasicIV {
                phi_dest: Value(0),
                ty: IrType::U32,
                init,
                step: -1,
            };
            unsigned_iv_bound(f, &lp, &iv, &[2])
        };

        // A1. `hi = limit - 1` — the ascending formula — is BELOW this range, so
        // a product certificate built on it checked 100*4 while the executed
        // maximum is 2^30*4, admitting a narrow product that really does wrap.
        // The bound now comes from the init, and the certificate is declined for
        // the right reason: 2^30 * 4 == 2^32 does not fit the U32 ring.
        let wide = fixture(100);
        let big = bound_of(&wide, Operand::Const(IrConst::I32(1 << 30))).expect("IV no-wrap holds");
        assert!(
            big.bounded,
            "a literal init makes `hi` the exact body maximum: {big:?}"
        );
        assert_eq!(big.lo, 101, "the floor is limit + 1");
        assert_eq!(big.hi, 1i128 << 30, "the ceiling is the init");
        assert!(
            !offset_product_cannot_overflow(IrType::U32, 4, 0, Some(big)),
            "2^30 * 4 overflows the U32 ring, so no certificate"
        );

        // The same shape with a small init IS certifiable, so the descending arm
        // is recovered rather than refused wholesale: `for (unsigned i = 64;
        // i-- > 0;) p[i]` has every value in [1, 64] and 64 * 4 fits U32.
        let narrow = fixture(0);
        let small = bound_of(&narrow, Operand::Const(IrConst::I32(64))).expect("bound");
        assert!(small.bounded, "{small:?}");
        assert_eq!((small.lo, small.hi), (1, 64), "{small:?}");
        assert!(
            offset_product_cannot_overflow(IrType::U32, 4, 0, Some(small)),
            "64 * 4 fits the U32 ring, so the descending arm is certified"
        );

        // An init at or below the limit never enters the body, so the interval
        // is empty; `hi = max(init, floor)` is a sound over-approximation of it
        // and NOT an exact body maximum, so no numeric certificate is issued.
        let empty = bound_of(&wide, Operand::Const(IrConst::I32(64))).expect("bound");
        assert_eq!((empty.lo, empty.hi), (101, 101), "{empty:?}");
        assert!(!empty.bounded, "an empty body has no maximum: {empty:?}");

        // F2. `bounded` follows the endpoint that fixes the CEILING, not the
        // limit's constness. With a loop-INVARIANT limit and a literal init the
        // body maximum is still exactly the init; only the floor degrades, and
        // `1` is a sound floor for every descending unsigned IV (the body runs
        // while `i > limit_var` and `limit_var >= 0`, so every observed value is
        // `>= 1`). Inheriting the limit's constness withdrew this certificate.
        let mut var_limit = fixture(0);
        var_limit.blocks[0].instructions.push(Instruction::Copy {
            dest: Value(7),
            src: Operand::Const(IrConst::I32(0)),
        });
        if let Instruction::Cmp { rhs, .. } = &mut var_limit.blocks[1].instructions[1] {
            *rhs = Operand::Value(Value(7));
        }
        let vl = bound_of(&var_limit, Operand::Const(IrConst::I32(64))).expect("bound");
        assert!(
            vl.bounded,
            "a literal init bounds the ceiling on its own: {vl:?}"
        );
        assert_eq!((vl.lo, vl.hi), (1, 64), "the floor degrades to 1: {vl:?}");
        assert!(
            offset_product_cannot_overflow(IrType::U32, 4, 0, Some(vl)),
            "64 * 4 fits the U32 ring, so the certificate stands on the init"
        );

        // A non-literal init (`for (unsigned i = n; i-- > 0;)`) has no provable
        // numeric maximum, so the certificate is withdrawn and the loop keeps
        // the modular index recurrence. The IV no-wrap half of the proof is
        // unaffected: entry still requires init > limit and the floor is still
        // limit + 1 >= 1.
        let unknown = bound_of(&narrow, Operand::Value(Value(7))).expect("IV no-wrap still holds");
        assert!(!unknown.bounded, "{unknown:?}");
        assert_eq!(unknown.hi, (1i128 << 32) - 1, "hi degrades to the type max");
        assert!(!offset_product_cannot_overflow(
            IrType::U32,
            4,
            0,
            Some(unknown)
        ));

        // The ascending case is unchanged: hi = limit - 1 is its real maximum.
        let mut g = cast_backedge(IrType::U32, IrType::U32);
        if let Instruction::Cmp { rhs, .. } = &mut g.blocks[1].instructions[1] {
            *rhs = Operand::Const(IrConst::I32(100));
        }
        let up = BasicIV {
            phi_dest: Value(0),
            ty: IrType::U32,
            init: Operand::Const(IrConst::I32(0)),
            step: 1,
        };
        let bg = unsigned_iv_bound(&g, &lp, &up, &[2]).expect("ascending bound");
        assert!(bg.bounded && bg.hi == 99, "ascending: {bg:?}");
        assert!(offset_product_cannot_overflow(IrType::U32, 4, 0, Some(bg)));
    }

    #[test]
    fn offset_product_certificate_decision_table() {
        // Pins the whole decision table of `offset_product_cannot_overflow`, so
        // a future edit cannot silently reopen a cell.
        //
        // TWO different guards, and confusing them produces a wrong audit either
        // way. THIS function's exemption is pointer-width
        // (`mul_ty.size() >= target_ptr_size()`), so on LP64 every unsigned IV
        // narrower than 64 bits (U8, U16, U32) must present a bounded interval. The
        // `>= 4` carve-out a review flagged was in the CALLER's gate
        // (`modular_iv && size >= 4 && is_unsigned()`), which let a U8/U16 IV
        // skip `unsigned_iv_bound` entirely; that is now removed, so the proof is
        // total. Only pointer-width IVs are exempt here, and that exemption is
        // sound at any
        // width: if `iv * stride` wrapped the POINTER ring then the source's own
        // `p[iv]` would not point into or one past any object, which C17
        // 6.5.6p8 already makes undefined — so a defined source program cannot
        // reach the wrap. The same argument covers ILP32, where U32 is the
        // pointer width and is therefore exempt there.
        let b = |lo: i128, hi: i128, bounded: bool| Some(UnsignedIvBound { lo, hi, bounded });

        // (1) Pointer-width and signed IVs: exempt, no bound needed.
        for ty in [
            IrType::I64,
            IrType::U64,
            IrType::Ptr,
            IrType::I8,
            IrType::I16,
            IrType::I32,
        ] {
            assert!(
                offset_product_cannot_overflow(ty, 4, 0, None),
                "{ty:?} is exempt (pointer width, or signed where a wrapping \
                 product is UB in the source: C17 6.5p5)"
            );
        }

        // (2) Sub-pointer-width UNSIGNED IVs need a bounded interval, and the
        //     product must fit THEIR OWN ring — not merely u64.
        //     U8, bound [0,255], stride 1: max 255 < 256 -> sound, admitted.
        assert!(offset_product_cannot_overflow(
            IrType::U8,
            1,
            0,
            b(0, 255, true)
        ));
        //     U8, bound [0,255], stride 4: max 1020 >= 256 -> wraps, declined.
        assert!(!offset_product_cannot_overflow(
            IrType::U8,
            4,
            0,
            b(0, 255, true)
        ));
        //     U16, bound [0,255], stride 4: max 1020 < 65536 -> admitted. This
        //     is CORRECT, not a hole: the recurrence cannot wrap the U16 ring.
        assert!(offset_product_cannot_overflow(
            IrType::U16,
            4,
            0,
            b(0, 255, true)
        ));
        //     U16, full range, stride 4: max 262140 >= 65536 -> declined.
        assert!(!offset_product_cannot_overflow(
            IrType::U16,
            4,
            0,
            b(0, 65535, true)
        ));
        //     U32, bounded interval, product inside the ring -> admitted.
        assert!(offset_product_cannot_overflow(
            IrType::U32,
            4,
            0,
            b(0, (1i128 << 30) - 1, true)
        ));
        //     U32, bounded interval, product one past the ring -> declined.
        assert!(!offset_product_cannot_overflow(
            IrType::U32,
            4,
            0,
            b(0, 1i128 << 30, true)
        ));

        // (3) No bound, or an unbounded one, certifies nothing below pointer
        //     width.
        assert!(!offset_product_cannot_overflow(IrType::U32, 4, 0, None));
        assert!(!offset_product_cannot_overflow(
            IrType::U16,
            1,
            0,
            b(0, 255, false)
        ));

        // (4) The affine `+ k` term is included in the certificate, so a bound
        //     that only fits WITHOUT it is declined.
        assert!(!offset_product_cannot_overflow(
            IrType::U8,
            1,
            1,
            b(0, 255, true)
        ));
        assert!(offset_product_cannot_overflow(
            IrType::U8,
            1,
            1,
            b(0, 254, true)
        ));
    }

    /// A counting-down address loop: `for (unsigned i = n; i-- > 0;) p[i]`.
    ///
    /// Same body as [`address_add_loop`] but the IV descends from its initial
    /// value to an exclusive zero limit, which is the shape A1 is about: the
    /// values the body observes are `[limit, init - 1]`, NOT `[limit + 1, init]`
    /// read backwards, so a bound derived from the limit alone is not the range.
    fn descending_address_add_loop(
        iv_ty: IrType,
        literal_init: bool,
        narrow_offset: bool,
    ) -> IrFunction {
        // A non-literal init has to be genuinely unresolvable, so it comes from a
        // real parameter: IVSR resolves preheader copies, and a `Copy(Const(..))`
        // would (correctly) be seen through and certified as a literal.
        let params = if literal_init {
            Vec::new()
        } else {
            vec![crate::ir::module::IrParam {
                ty: IrType::U32,
                noalias: false,
                struct_size: None,
                struct_align: None,
                param_align: None,
                struct_eightbyte_classes: Vec::new(),
                is_f128_sse: false,
                riscv_float_class: None,
            }]
        };
        let mut f = IrFunction::new("desc_address_add".into(), IrType::I64, params, false);
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Copy {
                    dest: Value(0),
                    src: Operand::Const(IrConst::I64(0x4000)),
                },
                Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Const(IrConst::I32(0)),
                },
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: vec![],
        });
        // A non-literal init models `for (unsigned i = n; i-- > 0;)`: the phi
        // takes its first value from a preheader definition rather than from a
        // constant in the phi itself.
        if !literal_init {
            f.blocks[0].instructions.push(Instruction::ParamRef {
                dest: Value(8),
                param_idx: 0,
                ty: IrType::U32,
            });
        }
        let init_op = if literal_init {
            Operand::Const(IrConst::I32(64))
        } else {
            Operand::Value(Value(8))
        };
        // header: i = phi(64, i_next); while (i > 0)
        f.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(2),
                    ty: iv_ty,
                    incoming: vec![
                        (init_op, BlockId(0)),
                        (Operand::Value(Value(9)), BlockId(2)),
                    ],
                },
                Instruction::Cmp {
                    dest: Value(3),
                    op: IrCmpOp::Ugt,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Const(IrConst::I32(0)),
                    ty: iv_ty,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(3)),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
            source_spans: vec![],
        });
        // body: off = (I64)i; addr = base + off; sum += *addr; i_next = i - 1
        f.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: {
                // `narrow_offset` scales the index INSIDE the IV's own ring
                // before widening, so the derived offset's `mul_ty` is the narrow
                // type and the numeric product certificate is the operative
                // proof. Without it the index is widened first and the offset
                // arithmetic happens at pointer width, where a product cannot
                // wrap the address space (C17 6.5.6p8) and no certificate is
                // needed. Same C source shape, two different proof obligations.
                let mut v = Vec::new();
                let scaled = if narrow_offset {
                    v.push(Instruction::BinOp {
                        dest: Value(4),
                        op: IrBinOp::Shl,
                        lhs: Operand::Value(Value(2)),
                        rhs: Operand::Const(IrConst::I32(2)),
                        ty: iv_ty,
                    });
                    v.push(Instruction::Cast {
                        dest: Value(11),
                        src: Operand::Value(Value(4)),
                        from_ty: iv_ty,
                        to_ty: IrType::I64,
                    });
                    Value(11)
                } else {
                    v.push(Instruction::Cast {
                        dest: Value(4),
                        src: Operand::Value(Value(2)),
                        from_ty: iv_ty,
                        to_ty: IrType::I64,
                    });
                    Value(4)
                };
                v.push(Instruction::BinOp {
                    dest: Value(5),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Value(scaled),
                    ty: IrType::I64,
                });
                v.push(Instruction::Load {
                    volatile: false,
                    dest: Value(6),
                    ptr: Value(5),
                    ty: IrType::I8,
                    seg_override: AddressSpace::Default,
                });
                // Canonical `i += -1`: `find_basic_ivs` matches an Add with a
                // constant step, which is the spelling a canonicalising frontend
                // produces. LCCC's own frontend currently emits `Sub` here, so
                // descending C loops form no BasicIV at all (recorded in
                // FOLLOWUP-2026-10-07); this fixture pins the proof for the day
                // they do.
                v.push(Instruction::BinOp {
                    dest: Value(9),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Const(IrConst::I32(-1)),
                    ty: iv_ty,
                });
                v
            },
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: vec![],
        });
        f.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![Instruction::Copy {
                dest: Value(7),
                src: Operand::Value(Value(6)),
            }],
            terminator: Terminator::Return(Some(Operand::Value(Value(7)))),
            source_spans: vec![],
        });
        f.next_value_id = 12;
        f
    }

    #[test]
    fn descending_unsigned_address_loop_end_to_end() {
        // A1, end to end, and the two facts that bound its blast radius.
        //
        // (1) A DESCENDING unsigned counter whose offset is formed by widening
        //     first (`addr = base + (I64)i`) does get the pointer recurrence, and
        //     the caller's `bound.is_none()` gate is what lets it through: a
        //     descending modular unsigned IV still has to PRESENT a bound.
        //
        //     What this shape does NOT exercise is the bound's VALUES. The offset
        //     is widened before the address Add, so the derived expression's
        //     `mul_ty` is pointer width and `offset_product_cannot_overflow`
        //     exempts that ring before reading `lo`/`hi` (C17 6.5.6p8: a product
        //     at pointer width cannot wrap the address space). Every assertion
        //     below therefore also held against the pre-fix bound
        //     `{lo: 1, hi: limit - 1, bounded: true}`, which is why the numeric
        //     certificate has its own end-to-end test:
        //     `descending_narrow_offset_product_certificate_is_the_operative_proof`.
        //     Claiming otherwise here — "the bound was inexact, the certificate
        //     was withdrawn and the loop was declined" — was wrong on all three
        //     counts: pre-fix `bounded` was inherited from the LIMIT's constness
        //     and was `true` for this literal limit, nothing was withdrawn, and
        //     `hi = limit - 1 = -1` sits BELOW the range, which is an admission,
        //     not a refusal.
        let fired = |literal_init: bool, narrow_offset: bool| {
            let mut f = descending_address_add_loop(IrType::U32, literal_init, narrow_offset);
            ivsr_function(&mut f) > 0
        };
        assert!(
            fired(true, false),
            "descending, literal init: recovered by A1"
        );
        assert!(
            fired(false, false),
            "descending, runtime init: pointer-ring recurrence needs no range"
        );

        // (2) The narrow-ring cell is declined in BOTH directions, so it says
        //     nothing about A1: `find_derived_exprs` does not collect an offset
        //     that is scaled INSIDE the narrow ring and widened afterwards
        //     (`(I64)(i << 2)`). Verified by probe, ascending and descending
        //     alike. That is a separate, pre-existing collection gap and it is
        //     recorded in FOLLOWUP-2026-10-07 rather than papered over here.
        assert!(!fired(true, true), "narrow-scaled offset: not collected");
        assert!(!fired(false, true), "narrow-scaled offset: not collected");

        // Direction alone never decides the outcome: the ascending spelling of
        // the collected shape fires as before.
        let mut up = address_add_loop(true, false);
        assert!(ivsr_function(&mut up) > 0, "the ascending arm still fires");
    }

    /// A DESCENDING unsigned IV whose byte offset is scaled INSIDE the IV's own
    /// ring and handed to a GEP: `for (unsigned i = init; i > 0; i += -1)
    /// sum += base[i]` with the shift at U32 width. This is the shape for which
    /// `offset_product_cannot_overflow` is the operative proof, because the
    /// derived expression's `mul_ty` is the narrow ring instead of the pointer
    /// ring. Compare `descending_address_add_loop`, which widens the index first
    /// and therefore exempts the product check before it reads the bound.
    fn descending_narrow_gep_loop(init: i32) -> IrFunction {
        let mut f = IrFunction::new("desc_narrow_gep".into(), IrType::I64, Vec::new(), false);
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::Copy {
                dest: Value(0),
                src: Operand::Const(IrConst::I64(0x4000)),
            }],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: vec![],
        });
        // header: i = phi(init, i_next); while (i > 0)
        f.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(2),
                    ty: IrType::U32,
                    incoming: vec![
                        (Operand::Const(IrConst::I32(init)), BlockId(0)),
                        (Operand::Value(Value(9)), BlockId(2)),
                    ],
                },
                Instruction::Cmp {
                    dest: Value(3),
                    op: IrCmpOp::Ugt,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Const(IrConst::I32(0)),
                    ty: IrType::U32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(3)),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
            source_spans: vec![],
        });
        // body: off = i * 12 in U32; p = GEP(base, off); v = *p; i_next = i + -1
        // (Stride 12, not a SIB-scaleable 2^k: the SIB cost rule keeps
        // `i << k` walks in scaled-index form, so a shift-based fixture would
        // never exercise the wrap certificate this test is about.)
        f.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![
                Instruction::BinOp {
                    dest: Value(4),
                    op: IrBinOp::Mul,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Const(IrConst::I32(12)),
                    ty: IrType::U32,
                },
                Instruction::GetElementPtr {
                    dest: Value(5),
                    base: Value(0),
                    offset: Operand::Value(Value(4)),
                    ty: IrType::I64,
                },
                Instruction::Load {
                    volatile: false,
                    dest: Value(6),
                    ptr: Value(5),
                    ty: IrType::I8,
                    seg_override: AddressSpace::Default,
                },
                Instruction::BinOp {
                    dest: Value(9),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Const(IrConst::I32(-1)),
                    ty: IrType::U32,
                },
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: vec![],
        });
        f.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![Instruction::Copy {
                dest: Value(7),
                src: Operand::Value(Value(6)),
            }],
            terminator: Terminator::Return(Some(Operand::Value(Value(7)))),
            source_spans: vec![],
        });
        f.next_value_id = 12;
        f
    }

    #[test]
    fn descending_narrow_offset_product_certificate_is_the_operative_proof() {
        // W1/F7. The end-to-end fixture above widens the index before the address
        // Add, so its `mul_ty` is pointer width and `offset_product_cannot_overflow`
        // exempts that ring before it ever reads `lo`/`hi`: every assertion there
        // also holds against the pre-fix bound. THIS test scales inside the IV's
        // own U32 ring, so the numeric certificate decides, and the second cell
        // fails when the two A1 hunks are reverted.
        let reduced = |init: i32| {
            let mut f = descending_narrow_gep_loop(init);
            ivsr_function(&mut f) > 0
        };
        // init = 64: the executed offsets are [12, 768], inside U32 on both
        // arms, so the loop is reduced before and after the fix. This cell
        // pins that the descending arm is certified rather than refused
        // wholesale.
        assert!(
            reduced(64),
            "a small descending index is certified in the U32 ring"
        );
        // init = 2^30 + 1: the executed maximum offset is (2^30 + 1) * 12,
        // which WRAPS the U32 ring. Post-fix `hi` is the init, so the certificate
        // is refused. Pre-fix `hi` was `limit - 1 = -1` — an interval BELOW the
        // range — and `-12 < 2^32` passed, arming a pointer recurrence that bumps by
        // -12 at 64-bit width for an offset the source computes modulo 2^32: the
        // first address would have been `base + 2^32 + 12` where the source says
        // `base + (init*12 mod 2^32)`. That is A1, and this is the cell that
        // discriminates.
        assert!(
            !reduced(0x4000_0001),
            "a descending offset that wraps its own ring must not be reduced"
        );
    }

    #[test]
    fn address_classification_deliberately_over_approximates_intrinsics() {
        // `is_used_as_address` delegates to `IntrinsicOp::reads_pointer_arg()`,
        // which is the allowlist for a callee-READ-ONLY proof and is therefore
        // deliberately true for PURE vector arithmetic. That looks like the wrong
        // predicate for this question, and narrowing it to `may_read_memory()`
        // was tried: it removed the pointer induction from `simd_vecreg`,
        // `simd_crc_adler` and `temp_promotion_window` (+7 insns and +4 stack
        // references on `simd_crc_adler` alone). This test pins the divergence
        // between the two predicates so the choice stays visible, and the
        // soundness argument lives in the comment at the call site: only a
        // pointer-width value can be armed here, where the integer and pointer
        // rings agree for every program without UB.
        use crate::ir::intrinsics::IntrinsicOp;
        assert!(
            IntrinsicOp::VecAddF64x4.reads_pointer_arg(),
            "the read-only allowlist covers pure vector ops by design"
        );
        assert!(
            !IntrinsicOp::VecAddF64x4.may_read_memory(),
            "a pure vector op demonstrably reads no memory: the two predicates \\
             really do differ, so the call site's choice is load-bearing"
        );
        assert!(IntrinsicOp::VecLoadF64x4.may_read_memory());

        let build = |inst: Instruction| {
            let mut f = IrFunction::new("addr_class".into(), IrType::I64, vec![], false);
            f.blocks.push(BasicBlock {
                label: BlockId(0),
                instructions: vec![inst],
                terminator: Terminator::Return(None),
                source_spans: vec![],
            });
            f
        };
        // A vector load's pointer argument is an address.
        let load = build(Instruction::Intrinsic {
            dest: Some(Value(9)),
            op: IntrinsicOp::VecLoadF64x4,
            dest_ptr: None,
            args: vec![Operand::Value(Value(5))],
        });
        assert!(is_used_as_address(&load, 5));
        // A store's dest_ptr slot is an address even though nothing reads it.
        let store = build(Instruction::Intrinsic {
            dest: None,
            op: IntrinsicOp::VecStoreF64x4,
            dest_ptr: Some(Value(5)),
            args: vec![Operand::Value(Value(6))],
        });
        assert!(is_used_as_address(&store, 5));
        // And the over-approximation: a pure vector consumer also classifies as
        // an address. That is INTENTIONAL — it is what keeps the SIMD pointer
        // induction alive — and it is safe because the value is pointer-width.
        let add = build(Instruction::Intrinsic {
            dest: Some(Value(9)),
            op: IntrinsicOp::VecAddF64x4,
            dest_ptr: None,
            args: vec![Operand::Value(Value(5)), Operand::Value(Value(6))],
        });
        assert!(
            is_used_as_address(&add, 5),
            "narrowing this to may_read_memory() is a measured regression"
        );
        // A value with no use at all is still not an address.
        let unused = build(Instruction::Copy {
            dest: Value(9),
            src: Operand::Value(Value(6)),
        });
        assert!(!is_used_as_address(&unused, 5));
    }

    /// Test full IVSR transformation on a sum-array loop.
    #[test]
    fn test_ivsr_sum_array() {
        let mut func = IrFunction::new("sum_array".to_string(), IrType::I64, vec![], false);

        // Block 0 (preheader): base = param, n = param, init = 0
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Copy {
                    dest: Value(0),
                    src: Operand::Const(IrConst::I64(0x1000)),
                },
                Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Const(IrConst::I32(100)),
                },
                Instruction::Copy {
                    dest: Value(2),
                    src: Operand::Const(IrConst::I32(0)),
                },
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });

        // Block 1 (header): i = phi(0, i_next)
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(3),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Value(Value(2)), BlockId(0)),
                        (Operand::Value(Value(10)), BlockId(2)),
                    ],
                },
                Instruction::Cmp {
                    dest: Value(4),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(3)),
                    rhs: Operand::Value(Value(1)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(4)),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
            source_spans: Vec::new(),
        });

        // Block 2 (body): cast, mul, GEP, load, i_next
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![
                Instruction::Cast {
                    dest: Value(5),
                    src: Operand::Value(Value(3)),
                    from_ty: IrType::I32,
                    to_ty: IrType::I64,
                },
                Instruction::BinOp {
                    dest: Value(6),
                    op: IrBinOp::Mul,
                    lhs: Operand::Value(Value(5)),
                    rhs: Operand::Const(IrConst::I64(24)),
                    ty: IrType::I64,
                },
                Instruction::GetElementPtr {
                    dest: Value(7),
                    base: Value(0),
                    offset: Operand::Value(Value(6)),
                    ty: IrType::I32,
                },
                Instruction::Load {
                    volatile: false,
                    dest: Value(8),
                    ptr: Value(7),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                },
                // A second independently-numbered `i * 4` and address
                // materialization models `if (a[i] > 0) sum += a[i]`.
                // It must share the pointer IV despite a distinct Mul value.
                Instruction::BinOp {
                    dest: Value(11),
                    op: IrBinOp::Mul,
                    lhs: Operand::Value(Value(5)),
                    rhs: Operand::Const(IrConst::I64(24)),
                    ty: IrType::I64,
                },
                Instruction::GetElementPtr {
                    dest: Value(12),
                    base: Value(0),
                    offset: Operand::Value(Value(11)),
                    ty: IrType::I32,
                },
                Instruction::Load {
                    volatile: false,
                    dest: Value(13),
                    ptr: Value(12),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                },
                Instruction::BinOp {
                    dest: Value(9),
                    op: IrBinOp::Add,
                    lhs: Operand::Const(IrConst::I64(0)),
                    rhs: Operand::Const(IrConst::I64(0)),
                    ty: IrType::I64,
                },
                Instruction::BinOp {
                    dest: Value(10),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(3)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });

        // Block 3 (exit)
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![],
            terminator: Terminator::Return(Some(Operand::Const(IrConst::I64(0)))),
            source_spans: Vec::new(),
        });

        func.next_value_id = 14;

        let changes = ivsr_function(&mut func);
        assert_eq!(changes, 2, "Expected both same-base GEPs to be reduced");

        // Check that a phi for the pointer IV was added to the header
        let header_phis: Vec<_> = func.blocks[1]
            .instructions
            .iter()
            .filter(|i| matches!(i, Instruction::Phi { .. }))
            .collect();
        assert_eq!(
            header_phis.len(),
            2,
            "Expected one original IV plus one shared pointer IV"
        );

        // Both original GEPs must become Copies from the same pointer IV.
        let body_copies: Vec<_> = func.blocks[2]
            .instructions
            .iter()
            .filter_map(|i| match i {
                Instruction::Copy {
                    dest,
                    src: Operand::Value(src),
                } if *dest == Value(7) || *dest == Value(12) => Some((*dest, *src)),
                _ => None,
            })
            .collect();
        assert_eq!(body_copies.len(), 2, "Expected both GEPs to become Copies");
        assert_eq!(
            body_copies[0].1, body_copies[1].1,
            "Expected one shared pointer IV"
        );
    }

    /// The linux_rbtree lookup-hash shape: `h = i * 104729` consumed by an
    /// And/mask (NOT a GEP offset).  The scalar derived-IV flavor must
    /// replace the multiply with a secondary recurrence.
    /// The hash-multiply loop shape the scalar derived-IV flavor targets:
    /// `for i in 0..16384 { m = (i * 104729) & 16383 }` with the mask
    /// observable.  Shared by the opt-in test and the production-default
    /// pin (audit work order P1-6): if someone inverts the wrapper pair /
    /// the pipeline's `CCC_IVSR_SCALAR_DERIVED` opt-in, the default test
    /// below starts seeing a derived recurrence and fails — the asm gate
    /// that used to pin this was removed because pinning the SHAPE would
    /// pin a measured regression; the default-OFF invariant still needs a
    /// pin, and this differential pair is it.
    fn hash_lookup_fixture() -> IrFunction {
        let mut func = IrFunction::new("hash_lookup".to_string(), IrType::I32, vec![], false);

        // Block 0 (preheader): init = 0
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::Copy {
                dest: Value(0),
                src: Operand::Const(IrConst::I32(0)),
            }],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });

        // Block 1 (header): i = phi(0, i_next), guard
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(1),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Value(Value(0)), BlockId(0)),
                        (Operand::Value(Value(6)), BlockId(2)),
                    ],
                },
                Instruction::Cmp {
                    dest: Value(2),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(16384)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(2)),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
            source_spans: Vec::new(),
        });

        // Block 2 (body/latch): h = i * 104729; m = h & 16383; i_next = i+1
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![
                Instruction::BinOp {
                    dest: Value(3),
                    op: IrBinOp::Mul,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(104729)),
                    ty: IrType::I32,
                },
                Instruction::BinOp {
                    dest: Value(4),
                    op: IrBinOp::And,
                    lhs: Operand::Value(Value(3)),
                    rhs: Operand::Const(IrConst::I32(16383)),
                    ty: IrType::I32,
                },
                // m must be observable or DCE-irrelevant here: store it via
                // the return so the chain has a use.
                Instruction::Copy {
                    dest: Value(5),
                    src: Operand::Value(Value(4)),
                },
                Instruction::BinOp {
                    dest: Value(6),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });

        // Block 3 (exit)
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![],
            terminator: Terminator::Return(Some(Operand::Value(Value(5)))),
            source_spans: Vec::new(),
        });

        func.next_value_id = 7;
        func
    }

    #[test]
    fn test_scalar_derived_iv_hash_shape() {
        let mut func = hash_lookup_fixture();

        let changes = ivsr_function_scalar(&mut func);
        assert!(changes >= 1, "expected the hash multiply to be reduced");

        // The header must hold a second phi (the derived recurrence) whose
        // incoming from the preheader is the constant 0*104729 = 0 and
        // whose latch increment is +104729.
        let derived_phis: Vec<_> = func.blocks[1]
            .instructions
            .iter()
            .filter_map(|i| match i {
                Instruction::Phi { dest, incoming, .. } if *dest != Value(1) => {
                    Some((*dest, incoming.clone()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(derived_phis.len(), 1, "one derived recurrence");
        let (j_dest, j_incoming) = &derived_phis[0];
        assert!(
            j_incoming
                .iter()
                .any(|(op, _)| matches!(op, Operand::Const(IrConst::I32(0)))),
            "constant-folded init 0*104729"
        );
        // The latch must add the wrapped stride.
        let latch_add = func.blocks[2].instructions.iter().any(|i| matches!(i, Instruction::BinOp { op: IrBinOp::Add, rhs: Operand::Const(IrConst::I32(104729)), lhs: Operand::Value(v), .. } if *v == *j_dest));
        assert!(latch_add, "latch increments the derived IV by 104729");
        // The And must read the derived phi, not the multiply.
        let and_reads_j = func.blocks[2].instructions.iter().any(|i| matches!(i, Instruction::BinOp { op: IrBinOp::And, lhs: Operand::Value(v), .. } if *v == *j_dest));
        assert!(and_reads_j, "the mask consumes the derived recurrence");
        // The multiply must have no remaining users (dead; DCE's to remove).
        let mut users = 0usize;
        for b in &func.blocks {
            for inst in &b.instructions {
                inst.for_each_used_value(|u| {
                    if u == 3 {
                        users += 1;
                    }
                });
            }
            b.terminator.for_each_used_value(|u| {
                if u == 3 {
                    users += 1;
                }
            });
        }
        assert_eq!(users, 0, "multiply fully replaced");
    }

    /// A dead `iv * C` earns no recurrence.
    #[test]
    fn test_production_default_leaves_scalar_derived_ivs_off() {
        // Same fixture the opt-in flavor reduces: the PRODUCTION wrapper
        // (scalar derived-IVs off) must leave the loop untouched — one
        // header phi, the multiply alive, the And still reading it.  This
        // pins the default-off invariant the removed rbtree asm gate used
        // to protect.
        let mut func = hash_lookup_fixture();
        let changes = ivsr_function(&mut func);
        assert_eq!(changes, 0, "production IVSR must not derive scalar IVs");
        let header_phis = func.blocks[1]
            .instructions
            .iter()
            .filter(|i| matches!(i, Instruction::Phi { .. }))
            .count();
        assert_eq!(header_phis, 1, "no derived recurrence under defaults");
        let mul_alive = func.blocks[2].instructions.iter().any(
            |i| matches!(i, Instruction::BinOp { dest, op: IrBinOp::Mul, .. } if *dest == Value(3)),
        );
        assert!(mul_alive, "the hash multiply stays");
        let and_reads_mul = func.blocks[2].instructions.iter().any(|i| {
            matches!(i, Instruction::BinOp { op: IrBinOp::And, lhs: Operand::Value(v), .. } if *v == Value(3))
        });
        assert!(and_reads_mul, "the mask still consumes the multiply");
    }

    /// A dead `iv * C` earns no recurrence.
    #[test]
    fn test_scalar_derived_iv_dead_mul_untouched() {
        let mut func = IrFunction::new("dead_mul".to_string(), IrType::I32, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::Copy {
                dest: Value(0),
                src: Operand::Const(IrConst::I32(0)),
            }],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(1),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Value(Value(0)), BlockId(0)),
                        (Operand::Value(Value(5)), BlockId(2)),
                    ],
                },
                Instruction::Cmp {
                    dest: Value(2),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(10)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(2)),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
            source_spans: Vec::new(),
        });
        // The multiply has NO uses at all.
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![
                Instruction::BinOp {
                    dest: Value(3),
                    op: IrBinOp::Mul,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(7)),
                    ty: IrType::I32,
                },
                Instruction::BinOp {
                    dest: Value(5),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![],
            terminator: Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
            source_spans: Vec::new(),
        });
        func.next_value_id = 6;

        let changes = ivsr_function_scalar(&mut func);
        assert_eq!(changes, 0, "a dead multiply must not earn a recurrence");
        let phis = func.blocks[1]
            .instructions
            .iter()
            .filter(|i| matches!(i, Instruction::Phi { .. }))
            .count();
        assert_eq!(phis, 1, "only the original IV phi remains");
    }

    /// A widening cast between the IV and the multiply disqualifies the
    /// scalar flavor (wraparound divergence: sext of a wrapped i32 IV is not
    /// the accumulated wide recurrence).
    #[test]
    fn test_scalar_derived_iv_widening_cast_skipped() {
        let mut func = IrFunction::new("wide_mul".to_string(), IrType::I64, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::Copy {
                dest: Value(0),
                src: Operand::Const(IrConst::I32(0)),
            }],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(1),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Value(Value(0)), BlockId(0)),
                        (Operand::Value(Value(7)), BlockId(2)),
                    ],
                },
                Instruction::Cmp {
                    dest: Value(2),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(100)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(2)),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![
                Instruction::Cast {
                    dest: Value(3),
                    src: Operand::Value(Value(1)),
                    from_ty: IrType::I32,
                    to_ty: IrType::I64,
                },
                Instruction::BinOp {
                    dest: Value(4),
                    op: IrBinOp::Mul,
                    lhs: Operand::Value(Value(3)),
                    rhs: Operand::Const(IrConst::I64(104729)),
                    ty: IrType::I64,
                },
                Instruction::BinOp {
                    dest: Value(5),
                    op: IrBinOp::And,
                    lhs: Operand::Value(Value(4)),
                    rhs: Operand::Const(IrConst::I64(16383)),
                    ty: IrType::I64,
                },
                Instruction::Copy {
                    dest: Value(6),
                    src: Operand::Value(Value(5)),
                },
                Instruction::BinOp {
                    dest: Value(7),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![],
            terminator: Terminator::Return(Some(Operand::Value(Value(6)))),
            source_spans: Vec::new(),
        });
        func.next_value_id = 8;

        let _ = ivsr_function_scalar(&mut func);
        let phis = func.blocks[1]
            .instructions
            .iter()
            .filter(|i| matches!(i, Instruction::Phi { .. }))
            .count();
        assert_eq!(phis, 1, "widened domain must fail closed");
    }

    /// The affine shape `(i + 1) * 56` with no GEP: the derived recurrence
    /// starts at 56 (constant-folded) and steps by 56.
    #[test]
    fn test_scalar_derived_iv_affine_init() {
        let mut func = IrFunction::new("affine".to_string(), IrType::I32, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::Copy {
                dest: Value(0),
                src: Operand::Const(IrConst::I32(0)),
            }],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(1),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Value(Value(0)), BlockId(0)),
                        (Operand::Value(Value(8)), BlockId(2)),
                    ],
                },
                Instruction::Cmp {
                    dest: Value(2),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(5)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(2)),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![
                // k = i + 1
                Instruction::BinOp {
                    dest: Value(3),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                },
                // h = k * 56
                Instruction::BinOp {
                    dest: Value(4),
                    op: IrBinOp::Mul,
                    lhs: Operand::Value(Value(3)),
                    rhs: Operand::Const(IrConst::I32(56)),
                    ty: IrType::I32,
                },
                Instruction::BinOp {
                    dest: Value(5),
                    op: IrBinOp::And,
                    lhs: Operand::Value(Value(4)),
                    rhs: Operand::Const(IrConst::I32(255)),
                    ty: IrType::I32,
                },
                Instruction::Copy {
                    dest: Value(6),
                    src: Operand::Value(Value(5)),
                },
                Instruction::BinOp {
                    dest: Value(8),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![],
            terminator: Terminator::Return(Some(Operand::Value(Value(6)))),
            source_spans: Vec::new(),
        });
        func.next_value_id = 9;

        let changes = ivsr_function_scalar(&mut func);
        assert!(changes >= 1, "affine multiply reduced");
        let derived: Vec<_> = func.blocks[1]
            .instructions
            .iter()
            .filter_map(|i| match i {
                Instruction::Phi { dest, incoming, .. } if *dest != Value(1) => {
                    Some(incoming.clone())
                }
                _ => None,
            })
            .collect();
        assert_eq!(derived.len(), 1);
        assert!(
            derived[0]
                .iter()
                .any(|(op, _)| matches!(op, Operand::Const(IrConst::I32(56)))),
            "init folds to (0+1)*56 = 56"
        );
    }

    // ── address-use collection: GEP glue, vec triples, symbol bases ─────

    use crate::ir::intrinsics::IntrinsicOp;

    /// Shared natural-loop fixture: preheader (b0), header (b1,
    /// `i = phi(0, i_next)`, `Slt i, 100`), body (b2), exit (b3).
    /// `Value(0)` is the address base (an opaque pointer defined in the
    /// preheader); the body is supplied by each test and must end with the
    /// backedge `i_next = i + 1` defining `i_next_id`. Value ids in the body
    /// start at 5; `next_value_id` is one past the body's highest id.
    fn loop_fixture(body: Vec<Instruction>, i_next_id: u32, next_value_id: u32) -> IrFunction {
        let mut func = IrFunction::new("addr_loop".to_string(), IrType::I64, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Copy {
                    dest: Value(0),
                    src: Operand::Const(IrConst::I64(0x1000)),
                },
                Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Const(IrConst::I32(100)),
                },
                Instruction::Copy {
                    dest: Value(2),
                    src: Operand::Const(IrConst::I32(0)),
                },
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(3),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Value(Value(2)), BlockId(0)),
                        (Operand::Value(Value(i_next_id)), BlockId(2)),
                    ],
                },
                Instruction::Cmp {
                    dest: Value(4),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(3)),
                    rhs: Operand::Value(Value(1)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(4)),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: body,
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![],
            terminator: Terminator::Return(Some(Operand::Const(IrConst::I64(0)))),
            source_spans: Vec::new(),
        });
        func.next_value_id = next_value_id;
        func
    }

    /// GVN leaves the surviving multiply behind Copy glue for old value ids;
    /// a GEP whose offset is `Copy(mul)` must still form the recurrence
    /// (the use-scan resolves operands through the alias map).
    #[test]
    fn gvn_copy_glue_gep_offset_still_reduces() {
        let body = vec![
            Instruction::Cast {
                dest: Value(5),
                src: Operand::Value(Value(3)),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
            Instruction::BinOp {
                dest: Value(6),
                op: IrBinOp::Mul,
                lhs: Operand::Value(Value(5)),
                rhs: Operand::Const(IrConst::I64(24)),
                ty: IrType::I64,
            },
            Instruction::Copy {
                dest: Value(7),
                src: Operand::Value(Value(6)),
            },
            Instruction::GetElementPtr {
                dest: Value(8),
                base: Value(0),
                offset: Operand::Value(Value(7)),
                ty: IrType::I64,
            },
            Instruction::Load {
                volatile: false,
                dest: Value(9),
                ptr: Value(8),
                ty: IrType::I64,
                seg_override: AddressSpace::Default,
            },
            Instruction::BinOp {
                dest: Value(10),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(3)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::I32,
            },
        ];
        let mut func = loop_fixture(body, 10, 11);
        let changes = ivsr_function(&mut func);
        assert_eq!(changes, 1, "Copy-glued GEP offset must still reduce");
        let header_phis = func.blocks[1]
            .instructions
            .iter()
            .filter(|i| matches!(i, Instruction::Phi { .. }))
            .count();
        assert_eq!(header_phis, 2, "IV phi + shared pointer phi");
        assert!(
            func.blocks[2].instructions.iter().any(|i| matches!(
                i,
                Instruction::Copy {
                    dest,
                    src: Operand::Value(_)
                } if *dest == Value(8)
            )),
            "the glued GEP must become a Copy of the pointer IV"
        );
    }

    /// `(base, index, disp)` vector-memory triples are address uses: the
    /// load AND store forms join the same recurrence as a GEP over the same
    /// derived offset, and rewrite in place to `(ptr, Const(0), disp)` with
    /// the argument arity (and the store's value) preserved.
    #[test]
    fn vec_addr_index_triple_becomes_pointer_recurrence() {
        let body = vec![
            Instruction::Cast {
                dest: Value(5),
                src: Operand::Value(Value(3)),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
            Instruction::BinOp {
                dest: Value(6),
                op: IrBinOp::Mul,
                lhs: Operand::Value(Value(5)),
                rhs: Operand::Const(IrConst::I64(8)),
                ty: IrType::I64,
            },
            Instruction::Intrinsic {
                dest: Some(Value(7)),
                op: IntrinsicOp::VecLoadF64x2,
                dest_ptr: None,
                args: vec![
                    Operand::Value(Value(0)),
                    Operand::Value(Value(6)),
                    Operand::Const(IrConst::I64(24)),
                ],
            },
            Instruction::Intrinsic {
                dest: None,
                op: IntrinsicOp::VecStoreF64x2,
                dest_ptr: None,
                args: vec![
                    Operand::Value(Value(7)),
                    Operand::Value(Value(0)),
                    Operand::Value(Value(6)),
                    Operand::Const(IrConst::I64(16)),
                ],
            },
            Instruction::GetElementPtr {
                dest: Value(8),
                base: Value(0),
                offset: Operand::Value(Value(6)),
                ty: IrType::I64,
            },
            Instruction::Load {
                volatile: false,
                dest: Value(9),
                ptr: Value(8),
                ty: IrType::I64,
                seg_override: AddressSpace::Default,
            },
            Instruction::BinOp {
                dest: Value(10),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(3)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::I32,
            },
        ];
        let mut func = loop_fixture(body, 10, 11);
        let changes = ivsr_function(&mut func);
        assert_eq!(
            changes, 3,
            "load triple + store triple + GEP share one recurrence"
        );
        let ptr_phi = match func.blocks[1].instructions.iter().find_map(|i| match i {
            Instruction::Phi { dest, ty, .. } if *ty == IrType::Ptr => Some(*dest),
            _ => None,
        }) {
            Some(v) => v,
            None => panic!("expected a pointer recurrence phi"),
        };
        match &func.blocks[2].instructions[2] {
            Instruction::Intrinsic { args, .. } => {
                assert_eq!(args.len(), 3, "load arity preserved");
                assert_eq!(args[0], Operand::Value(ptr_phi), "load base = pointer IV");
                assert_eq!(
                    args[1],
                    Operand::Const(IrConst::I64(0)),
                    "load index zeroed"
                );
                assert_eq!(args[2], Operand::Const(IrConst::I64(24)), "disp untouched");
            }
            other => panic!("load triple must stay an Intrinsic, got {other:?}"),
        }
        match &func.blocks[2].instructions[3] {
            Instruction::Intrinsic { args, .. } => {
                assert_eq!(args.len(), 4, "store arity preserved");
                assert_eq!(args[0], Operand::Value(Value(7)), "store value preserved");
                assert_eq!(args[1], Operand::Value(ptr_phi), "store base = pointer IV");
                assert_eq!(
                    args[2],
                    Operand::Const(IrConst::I64(0)),
                    "store index zeroed"
                );
                assert_eq!(args[3], Operand::Const(IrConst::I64(16)), "disp untouched");
            }
            other => panic!("store triple must stay an Intrinsic, got {other:?}"),
        }
    }

    /// The pair-loop shape: the multiply feeds ONLY vector-memory triples
    /// (no GEP at all). The recurrence must still form — this is exactly
    /// what nbody's `j * 56` needed.
    #[test]
    fn vec_addr_use_alone_forms_recurrence() {
        let body = vec![
            Instruction::Cast {
                dest: Value(5),
                src: Operand::Value(Value(3)),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
            Instruction::BinOp {
                dest: Value(6),
                op: IrBinOp::Mul,
                lhs: Operand::Value(Value(5)),
                rhs: Operand::Const(IrConst::I64(56)),
                ty: IrType::I64,
            },
            Instruction::Intrinsic {
                dest: Some(Value(7)),
                op: IntrinsicOp::VecLoadF64x2,
                dest_ptr: None,
                args: vec![
                    Operand::Value(Value(0)),
                    Operand::Value(Value(6)),
                    Operand::Const(IrConst::I64(24)),
                ],
            },
            Instruction::BinOp {
                dest: Value(8),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(3)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::I32,
            },
        ];
        let mut func = loop_fixture(body, 8, 9);
        let changes = ivsr_function(&mut func);
        assert_eq!(changes, 1, "vec-only use must still form the recurrence");
        assert!(
            func.blocks[1].instructions.iter().any(|i| matches!(
                i,
                Instruction::Phi { ty, .. } if *ty == IrType::Ptr
            )),
            "pointer recurrence phi expected"
        );
    }

    /// Fail-closed negative: the modelled triple shape requires a CONST
    /// trailing displacement. A value displacement is not a shape this
    /// engine models — nothing may be collected and the multiply stays.
    #[test]
    fn vec_addr_value_disp_is_left_alone() {
        let body = vec![
            Instruction::Cast {
                dest: Value(5),
                src: Operand::Value(Value(3)),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
            Instruction::BinOp {
                dest: Value(6),
                op: IrBinOp::Mul,
                lhs: Operand::Value(Value(5)),
                rhs: Operand::Const(IrConst::I64(56)),
                ty: IrType::I64,
            },
            Instruction::GetElementPtr {
                dest: Value(9),
                base: Value(0),
                offset: Operand::Const(IrConst::I64(8)),
                ty: IrType::I64,
            },
            Instruction::Intrinsic {
                dest: Some(Value(7)),
                op: IntrinsicOp::VecLoadF64x2,
                dest_ptr: None,
                args: vec![
                    Operand::Value(Value(0)),
                    Operand::Value(Value(6)),
                    Operand::Value(Value(9)),
                ],
            },
            Instruction::BinOp {
                dest: Value(8),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(3)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::I32,
            },
        ];
        let mut func = loop_fixture(body, 8, 10);
        let changes = ivsr_function(&mut func);
        assert_eq!(changes, 0, "value-disp triple must fail closed");
    }

    /// A `GlobalAddr` materialisation inside the loop body is dataflow
    /// loop-VARIANT (its definition is re-executed), but the base key is the
    /// SYMBOL: a link-time object whose address is loop-invariant. The
    /// recurrence must still form, and the init must materialise its own
    /// fresh `GlobalAddr` in the preheader (dominance by construction).
    #[test]
    fn global_addr_base_defined_in_loop_body_still_reduces_via_symbol() {
        let body = vec![
            Instruction::GlobalAddr {
                dest: Value(5),
                name: "arr".to_string(),
            },
            Instruction::Cast {
                dest: Value(6),
                src: Operand::Value(Value(3)),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
            Instruction::BinOp {
                dest: Value(7),
                op: IrBinOp::Mul,
                lhs: Operand::Value(Value(6)),
                rhs: Operand::Const(IrConst::I64(24)),
                ty: IrType::I64,
            },
            Instruction::GetElementPtr {
                dest: Value(8),
                base: Value(5),
                offset: Operand::Value(Value(7)),
                ty: IrType::I64,
            },
            Instruction::Load {
                volatile: false,
                dest: Value(9),
                ptr: Value(8),
                ty: IrType::I64,
                seg_override: AddressSpace::Default,
            },
            Instruction::BinOp {
                dest: Value(10),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(3)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::I32,
            },
        ];
        let mut func = loop_fixture(body, 10, 11);
        let changes = ivsr_function(&mut func);
        assert_eq!(
            changes, 1,
            "symbol-keyed GlobalAddr base must reduce despite a loop-body def"
        );
        // The preheader must own a FRESH GlobalAddr materialisation (not the
        // body's Value(5)).
        let fresh = func.blocks[0].instructions.iter().find_map(|i| match i {
            Instruction::GlobalAddr { dest, name } if name == "arr" => Some(*dest),
            _ => None,
        });
        let fresh = fresh.expect("preheader must materialise a fresh GlobalAddr");
        assert_ne!(fresh, Value(5), "the materialisation must be a NEW value");
        // The body GEP becomes a Copy of the pointer IV, and the pointer IV's
        // init Copy chain starts at the fresh materialisation.
        assert!(
            func.blocks[2].instructions.iter().any(|i| matches!(
                i,
                Instruction::Copy {
                    dest,
                    src: Operand::Value(_)
                } if *dest == Value(8)
            )),
            "body GEP must become a Copy of the pointer IV"
        );
        let init_ptr = match func.blocks[1].instructions.iter().find_map(|i| match i {
            Instruction::Phi {
                dest: _,
                ty,
                incoming,
                ..
            } if *ty == IrType::Ptr => Some(incoming.clone()),
            _ => None,
        }) {
            Some(incoming) => incoming,
            None => panic!("pointer phi expected"),
        };
        assert_eq!(init_ptr.len(), 2);
        assert!(
            init_ptr
                .iter()
                .any(|(op, _)| matches!(op, Operand::Value(_))),
            "init reads a value"
        );
    }

    /// Sibling chaining: `base + i*56` and `base + (i+1)*56` are ONE
    /// recurrence apart by the constant 56. The `(i+1)` group must chain off
    /// the `i` group's pointer IV — `GEP(parent, 56)` at its use — with NO
    /// second pointer phi, no second init, and no latch step of its own.
    /// This is the nbody pair-loop j-init shape.
    #[test]
    fn affine_sibling_chains_to_parent_recurrence() {
        let body = vec![
            Instruction::Cast {
                dest: Value(5),
                src: Operand::Value(Value(3)),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
            Instruction::BinOp {
                dest: Value(6),
                op: IrBinOp::Mul,
                lhs: Operand::Value(Value(5)),
                rhs: Operand::Const(IrConst::I64(56)),
                ty: IrType::I64,
            },
            Instruction::GetElementPtr {
                dest: Value(7),
                base: Value(0),
                offset: Operand::Value(Value(6)),
                ty: IrType::I64,
            },
            Instruction::Load {
                volatile: false,
                dest: Value(8),
                ptr: Value(7),
                ty: IrType::I64,
                seg_override: AddressSpace::Default,
            },
            Instruction::BinOp {
                dest: Value(9),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(3)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::I32,
            },
            Instruction::Cast {
                dest: Value(10),
                src: Operand::Value(Value(9)),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
            Instruction::BinOp {
                dest: Value(11),
                op: IrBinOp::Mul,
                lhs: Operand::Value(Value(10)),
                rhs: Operand::Const(IrConst::I64(56)),
                ty: IrType::I64,
            },
            Instruction::GetElementPtr {
                dest: Value(12),
                base: Value(0),
                offset: Operand::Value(Value(11)),
                ty: IrType::I64,
            },
            Instruction::Load {
                volatile: false,
                dest: Value(13),
                ptr: Value(12),
                ty: IrType::I64,
                seg_override: AddressSpace::Default,
            },
            Instruction::BinOp {
                dest: Value(14),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(3)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::I32,
            },
        ];
        let mut func = loop_fixture(body, 14, 15);
        let changes = ivsr_function(&mut func);
        assert_eq!(
            changes, 2,
            "both GEPs must rewrite (one recurrence + one chain)"
        );
        let header_phis = func.blocks[1]
            .instructions
            .iter()
            .filter(|i| matches!(i, Instruction::Phi { .. }))
            .count();
        assert_eq!(
            header_phis, 2,
            "IV phi + ONE pointer phi — the sibling must not get its own recurrence"
        );
        let ptr_phi = match func.blocks[1].instructions.iter().find_map(|i| match i {
            Instruction::Phi { dest, ty, .. } if *ty == IrType::Ptr => Some(*dest),
            _ => None,
        }) {
            Some(v) => v,
            None => panic!("expected one pointer recurrence phi"),
        };
        assert!(
            func.blocks[2].instructions.iter().any(|i| matches!(
                i,
                Instruction::Copy {
                    dest,
                    src: Operand::Value(src)
                } if *dest == Value(7) && *src == ptr_phi
            )),
            "parent use must be a Copy of the pointer IV"
        );
        assert!(
            func.blocks[2].instructions.iter().any(|i| matches!(
                i,
                Instruction::GetElementPtr {
                    dest,
                    base,
                    offset: Operand::Const(IrConst::I64(56)),
                    ..
                } if *dest == Value(12) && *base == ptr_phi
            )),
            "sibling use must chain: GEP(pointer IV, 56)"
        );
    }

    /// SIB cost rule pin: a GEP-only group whose stride is SIB-scaleable
    /// (1/2/4/8) keeps the free scaled-index addressing
    /// (`disp(%base, %idx, scale)`) instead of growing a marching pointer —
    /// measured policy (fp_memfold_stencil5 +5.7% insns when the recurrence
    /// formed there, codegen-quality-gate 2026-10-12). Non-SIB strides and
    /// vec-memory groups still reduce (see the sibling tests).
    #[test]
    fn sib_friendly_stride_keeps_scaled_index() {
        let body = vec![
            Instruction::Cast {
                dest: Value(5),
                src: Operand::Value(Value(3)),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
            Instruction::BinOp {
                dest: Value(6),
                op: IrBinOp::Mul,
                lhs: Operand::Value(Value(5)),
                rhs: Operand::Const(IrConst::I64(4)),
                ty: IrType::I64,
            },
            Instruction::GetElementPtr {
                dest: Value(7),
                base: Value(0),
                offset: Operand::Value(Value(6)),
                ty: IrType::I64,
            },
            Instruction::Load {
                volatile: false,
                dest: Value(8),
                ptr: Value(7),
                ty: IrType::I64,
                seg_override: AddressSpace::Default,
            },
            Instruction::BinOp {
                dest: Value(9),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(3)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::I32,
            },
        ];
        let mut func = loop_fixture(body, 9, 10);
        let changes = ivsr_function(&mut func);
        assert_eq!(
            changes, 0,
            "SIB-scaleable stride must keep the scaled index"
        );
        assert!(
            !func.blocks[1]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::Phi { ty, .. } if *ty == IrType::Ptr)),
            "no pointer recurrence for a SIB-friendly GEP-only group"
        );
    }
    fn symbol_recurrence_fixture(stride: i64, entry_base: bool) -> IrFunction {
        let body = vec![
            Instruction::GlobalAddr {
                dest: Value(5),
                name: "arr".to_string(),
            },
            Instruction::Cast {
                dest: Value(6),
                src: Operand::Value(Value(3)),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
            Instruction::BinOp {
                dest: Value(7),
                op: IrBinOp::Mul,
                lhs: Operand::Value(Value(6)),
                rhs: Operand::Const(IrConst::I64(stride)),
                ty: IrType::I64,
            },
            Instruction::GetElementPtr {
                dest: Value(8),
                base: Value(5),
                offset: Operand::Value(Value(7)),
                ty: IrType::I64,
            },
            Instruction::Load {
                volatile: false,
                dest: Value(9),
                ptr: Value(8),
                ty: IrType::I64,
                seg_override: AddressSpace::Default,
            },
            Instruction::BinOp {
                dest: Value(10),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(3)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::I32,
            },
        ];
        let mut func = loop_fixture(body, 10, 11);

        if entry_base {
            let base = func.blocks[2].instructions.remove(0);
            func.blocks[0].instructions.push(base);
        }
        func
    }

    #[test]
    fn symbol_recurrence_reuses_dominating_materialization() {
        let mut func = symbol_recurrence_fixture(24, true);
        assert_eq!(ivsr_function(&mut func), 1);
        let bases: Vec<_> = func.blocks[0]
            .instructions
            .iter()
            .filter_map(|inst| match inst {
                Instruction::GlobalAddr { dest, name } if name == "arr" => Some(*dest),
                _ => None,
            })
            .collect();
        assert_eq!(
            bases,
            [Value(5)],
            "do not add a second live address for the same symbol"
        );
        assert!(
            func.blocks[0].instructions.iter().any(|inst| matches!(
                inst,
                Instruction::Copy {
                    src: Operand::Value(Value(5)),
                    ..
                }
            )),
            "recurrence initializer must reuse the available base"
        );
    }

    #[test]
    fn symbol_recurrence_narrow_index_is_not_free_sib() {
        let mut func = symbol_recurrence_fixture(4, true);
        assert_eq!(
            ivsr_function(&mut func),
            1,
            "widening a narrow symbol index is not free scaled-index addressing"
        );
        assert!(func.blocks[1].instructions.iter().any(|inst| matches!(
            inst,
            Instruction::Phi {
                ty: IrType::Ptr,
                ..
            }
        )));
    }

    #[test]
    fn symbol_recurrence_keeps_sib_without_available_base() {
        let mut func = symbol_recurrence_fixture(4, false);
        assert_eq!(
            ivsr_function(&mut func),
            0,
            "do not trade the widening for an extra live symbol materialization"
        );
    }

    #[test]
    fn symbol_recurrence_rejects_nondominating_materialization() {
        let mut func = symbol_recurrence_fixture(24, false);
        let mut prefix = vec![
            BasicBlock {
                label: BlockId(10),
                instructions: vec![],
                source_spans: vec![],
                terminator: Terminator::CondBranch {
                    cond: Operand::Const(IrConst::I32(1)),
                    true_label: BlockId(11),
                    false_label: BlockId(12),
                },
            },
            BasicBlock {
                label: BlockId(11),
                source_spans: vec![],
                instructions: vec![Instruction::GlobalAddr {
                    dest: Value(11),
                    name: "arr".into(),
                }],
                terminator: Terminator::Branch(BlockId(0)),
            },
            BasicBlock {
                label: BlockId(12),
                instructions: vec![],
                source_spans: vec![],
                terminator: Terminator::Branch(BlockId(0)),
            },
        ];
        prefix.append(&mut func.blocks);
        func.blocks = prefix;
        func.next_value_id = 12;
        assert_eq!(ivsr_function(&mut func), 1);
        let fresh = func.blocks[3]
            .instructions
            .iter()
            .find_map(|inst| match inst {
                Instruction::GlobalAddr { dest, name } if name == "arr" => Some(*dest),
                _ => None,
            })
            .expect("must materialize a fresh base in the actual preheader");
        assert_ne!(
            fresh,
            Value(11),
            "one diamond arm does not dominate the merge"
        );
        assert_ne!(
            fresh,
            Value(5),
            "the loop-body base does not dominate the preheader"
        );
    }

    #[test]
    fn symbol_recurrence_preserves_non_x86_materialization_policy() {
        let mut func = symbol_recurrence_fixture(24, true);
        let cfg = analysis::CfgAnalysis::build(&func);
        assert_eq!(ivsr_with_analysis(&mut func, &cfg, false, true, false), 1);
        assert_eq!(
            func.blocks[0]
                .instructions
                .iter()
                .filter(|inst| matches!(inst,
                    Instruction::GlobalAddr { name, .. } if name == "arr"
                ))
                .count(),
            2,
            "non-x86 codegen policy remains unchanged"
        );
    }

    #[test]
    fn symbol_recurrence_keeps_sib_for_native_width_index() {
        let mut func = symbol_recurrence_fixture(4, true);
        for block in &mut func.blocks {
            for inst in &mut block.instructions {
                match inst {
                    Instruction::Copy {
                        src: Operand::Const(c @ IrConst::I32(_)),
                        ..
                    } => {
                        *c = IrConst::I64(c.to_i64().unwrap());
                    }
                    Instruction::Phi { ty, .. } | Instruction::Cmp { ty, .. } => *ty = IrType::I64,
                    Instruction::Cast { from_ty, .. } => *from_ty = IrType::I64,
                    Instruction::BinOp {
                        ty,
                        rhs: Operand::Const(c @ IrConst::I32(_)),
                        ..
                    } => {
                        *ty = IrType::I64;
                        *c = IrConst::I64(c.to_i64().unwrap());
                    }
                    _ => {}
                }
            }
        }
        assert_eq!(
            ivsr_function(&mut func),
            0,
            "native-width symbol index stays SIB"
        );
    }
}

#[cfg(test)]
mod cfg_shape_contract_tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, Terminator};

    #[test]
    fn topology_guard_is_stronger_than_block_count() {
        let mut func = IrFunction::new("cfg_shape".into(), IrType::Void, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![],
            source_spans: vec![],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(1)),
                true_label: BlockId(0),
                false_label: BlockId(1),
            },
        });
        let original = ivsr_cfg_shape(&func);
        if let Terminator::CondBranch { cond, .. } = &mut func.blocks[0].terminator {
            *cond = Operand::Value(Value(2));
        }
        assert_eq!(
            original,
            ivsr_cfg_shape(&func),
            "value rewrite keeps dominance"
        );
        if let Terminator::CondBranch { true_label, .. } = &mut func.blocks[0].terminator {
            *true_label = BlockId(1);
        }
        assert_ne!(
            original,
            ivsr_cfg_shape(&func),
            "same count, different edges"
        );
        func.blocks[0].terminator = Terminator::Return(None);
        let returned = ivsr_cfg_shape(&func);
        func.blocks[0].terminator = Terminator::Unreachable;
        assert_ne!(
            returned,
            ivsr_cfg_shape(&func),
            "same edges, different kind"
        );
        let unreachable = ivsr_cfg_shape(&func);
        func.blocks[0].label = BlockId(3);
        assert_ne!(
            unreachable,
            ivsr_cfg_shape(&func),
            "same count, different identity"
        );
    }
}

#[cfg(test)]
mod dominance_snapshot_contract {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, Terminator};

    #[test]
    fn checks_edges_as_well_as_block_count() {
        let mut f = IrFunction::new("cfg".into(), IrType::Void, vec![], false);
        for id in 0..2 {
            f.blocks.push(BasicBlock {
                label: BlockId(id),
                instructions: vec![],
                source_spans: vec![],
                terminator: Terminator::Return(None),
            });
        }
        let cfg = analysis::CfgAnalysis::build(&f);
        assert!(ivsr_cfg_unchanged(&f, &cfg));
        f.blocks[0].terminator = Terminator::Branch(BlockId(1));
        assert!(
            !ivsr_cfg_unchanged(&f, &cfg),
            "same block count is not enough"
        );
        f.blocks[0].terminator = Terminator::Return(None);
        f.blocks.push(f.blocks[1].clone());
        assert!(!ivsr_cfg_unchanged(&f, &cfg));
    }
}
