//! AVX2/SSE2 strided-2 32-bit integer sum-reduction vectorizer.
//!
//! Recognizes counted innermost two-block reduction loops whose byte induction
//! variable steps by 8 bytes (two 32-bit dwords per scalar iteration) and
//! accumulates a pure modular 32-bit integer expression into an `I32`/`U32`
//! sum accumulator:
//!
//! ```c
//! for (unsigned long i = 0; i < BYTES; i += 8) /* BYTES: compile-time constant */
//!     acc += *(const uint32_t *)(buf + i) ^ (uint32_t)i;
//! ```
//!
//! ### Admitted exit test
//!
//! [`analyze`] requires the loop header's exit comparison to name the induction
//! variable itself on one side and an `Operand::Const` on the other, so the trip
//! count is known when the transform runs. A run-time bound, including the
//! `(i + 8) <= len` spelling, is declined: admitting it needs grouped-bound
//! machinery and a proof for a non-constant vector prefix.
//!
//! The corpus gate `tests/regression/check_vector_remainder_codegen.sh` pins
//! both dereferenceability paths below on
//! `tests/regression/vectorize_strided_i32_reduce.c`.
//!
//! ### Even-lane SIMD homomorphism (zero shuffles inside the loop)
//!
//! A `VecLoadStrided2I32x8` places four stride-8B dwords (`+0, +8, +16,
//! +24`) in the even 32-bit lanes `0, 2, 4, 6` of a YMM register without
//! reading the gaps; `VecLoadStrided2I32x4` is the two-dword XMM equivalent.
//! With a sufficient contiguous-object proof, SSE2 can instead compact two
//! full XMM loads into four real lanes and omit the odd-lane mask. AVX2 uses
//! a full `VecLoadI32x8` under the same proof. Because modular lane operations
//! (`vpaddd`, `vpsubd`, `vpmulld`, `vpand`, `vpor`, `vpxor`, `vpslld`,
//! `vpsrld`, `vpsrad`) never propagate carries or traps across lane
//! boundaries, evaluating the entire per-element DAG across all 8 lanes in
//! parallel computes the exact scalar values in lanes `0, 2, 4, 6` with zero
//! in-loop shuffles or permutes on the exact-access forms. Once the vector
//! loop exits, the even-lane accumulators mask their odd lanes before the
//! horizontal add; the compact XMM form already contains only real elements.
//!
//! ### Exact-access and guard-page contract
//!
//! Iteration `t` reads `[8t, 8t+3]`. Bounding a wide load by the scalar
//! loop's highest address does not make the *gaps* safe: they may be separate
//! objects concurrently modified by another thread, or device registers.
//! The default load therefore performs the same four 4-byte accesses as four
//! scalar iterations. A contiguous load is used only when the complete vector
//! prefix lies within an unescaped, nonvolatile stack allocation, so its gap
//! bytes cannot be observed independently. The untouched scalar loop executes
//! the remaining iterations.

use super::*;
use crate::common::fx_hash::FxHashSet;
use crate::ir::provenance::{ChainStep, RootLeaf, chain_step, root_leaf};

#[derive(Clone, Debug, PartialEq)]
enum StridedExpr {
    LoadEven32(usize),
    IvLow32,
    Invariant(Operand),
    BinOp(IrBinOp, Box<StridedExpr>, Box<StridedExpr>),
}

struct StridedI32ReducePattern {
    preheader_idx: usize,
    header_idx: usize,
    body_idx: usize,
    iv_phi: Value,
    iv_ty: IrType,
    acc_phi: Value,
    acc_ty: IrType,
    acc_init: Operand,
    safe_vecs: u64,
    load_bases: Vec<Value>,
    wide_load_proven: Vec<bool>,
    expr: StridedExpr,
}

thread_local! {
    /// Whether the strided reduction vectorizer may run, resolved ONCE per
    /// translation unit by `run_passes` (`set_enabled`). The kill switch is not
    /// read per call site: the pass-pipeline env-read ratchet
    /// (`tests/regression/check_env_test_hygiene.sh`) requires a new codegen
    /// switch to be resolved in `run_passes`, which is what keeps a per-loop
    /// `getenv` off the hot path and out of the pipeline's environment budget.
    static ENABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

/// Record whether `CCC_NO_STRIDED_REDUCE` allows the transform. Called once per
/// translation unit; a previous compile on the same worker thread cannot leak
/// its setting into the next one.
pub(crate) fn set_enabled(enabled: bool) {
    ENABLED.with(|cell| cell.set(enabled));
}

fn strided_reduce_disabled() -> bool {
    !ENABLED.with(|cell| cell.get())
}

/// Diagnostic switch: report the analysis stage that declined this loop.
///
/// The transform fails closed at forty-odd preconditions, and a silent `None`
/// makes "the optimization is dormant" indistinguishable from "no candidate
/// loop exists" -- which is exactly how this pass sat unused without anyone
/// noticing.  Setting `LCCC_VEC_STRIDED_TRACE=1` names the line that declined.
fn trace_enabled() -> bool {
    TRACE.with(|cell| cell.get())
}

thread_local! {
    static TRACE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Install the trace switch, resolved once per translation unit in
/// `run_passes` (src/passes/mod.rs, which the env-read ratchet excludes) so
/// this pass reads no environment variable of its own.
pub(crate) fn set_trace_enabled(enabled: bool) {
    TRACE.with(|cell| cell.set(enabled));
}

macro_rules! decline {
    () => {{
        if trace_enabled() {
            eprintln!("[STRIDED] decline at line {}", line!());
        }
        return None;
    }};
}

/// Smallest scalar trip count worth vectorizing: below this, the preheader
/// setup and the masking/horizontal-add epilogue cost more than the four
/// iterations per vector can save.
const MIN_SCALAR_TRIPS: u64 = 16;

/// Bytes advanced by one scalar iteration (a stride-2 `u32` access pair).
const SCALAR_STEP_BYTES: u64 = 8;

/// Bytes covered by one 256-bit vector iteration.
const VECTOR_BYTES: u64 = 32;

/// Scalar iterations consumed by one 256-bit vector iteration (`4`).
const VECTORS_PER_SCALAR_ITER: u64 = VECTOR_BYTES / SCALAR_STEP_BYTES;

/// Number of scalar iterations of `for (iv = 0; iv <cmp> bound; iv += 8)`,
/// or `None` when the exit test is not one this pass models.
///
/// `signed`ness comes from the IV type, and `cmp_op` must agree with it: an
/// unsigned type compared `Slt` (or a signed type compared `Ult`) means the
/// IR is not the loop it appears to be, so the pass declines.
///
/// Every accepted case is also **wrap-free**: the last iteration's
/// `iv + 8` must stay inside the IV type, otherwise the scalar loop the
/// transform resumes into is not a counted loop at all (it either wraps into
/// an endless loop or is UB in the source), and a trip count derived from a
/// wrapping increment would be a fiction. That is checked here once, for both
/// the `<` and `<=` forms, instead of only for `<=`.
fn scalar_trip_count(op: IrCmpOp, iv_ty: IrType, bound: i64) -> Option<u64> {
    if bound <= 0 {
        return None;
    }
    let signed = matches!(iv_ty, IrType::I32 | IrType::I64);
    // The exit constant must fit the IV type in the comparison's own
    // signedness. Canonical IR gives this for free (the compare is typed, and
    // `analyze` requires `cmp_ty == iv_ty`), but a non-canonical yet
    // well-formed constant must not be reinterpreted through an `as` cast.
    let fits = match iv_ty {
        IrType::I32 => i32::try_from(bound).is_ok(),
        IrType::U32 => u32::try_from(bound).is_ok(),
        IrType::I64 | IrType::U64 => true,
        _ => return None,
    };
    if !fits {
        return None;
    }
    // Largest `iv` value the type can hold, in `i64` space; the largest bound
    // for which the final `iv + 8` still fits is `limit - 8`.
    let limit = match iv_ty {
        IrType::I32 => i64::from(i32::MAX),
        IrType::U32 => i64::from(u32::MAX),
        _ => i64::MAX,
    };
    // One wrap-free admission test for both comparison forms: the final
    // `iv + 8` must still fit the IV type.
    if bound > limit.saturating_sub(SCALAR_STEP_BYTES as i64) {
        return None;
    }
    let trips = match (op, signed) {
        (IrCmpOp::Ult, false) | (IrCmpOp::Slt, true) => (bound as u64).div_ceil(SCALAR_STEP_BYTES),
        (IrCmpOp::Ule, false) | (IrCmpOp::Sle, true) => (bound as u64) / SCALAR_STEP_BYTES + 1,
        _ => return None,
    };
    (trips > 0).then_some(trips)
}

/// Groups of four scalar iterations with exact-width loads. Full-width
/// loads have an additional independent private-object extent proof.
fn safe_vec_count(scalar_trips: u64) -> u64 {
    scalar_trips / VECTORS_PER_SCALAR_ITER
}

/// The byte offset the scalar remainder loop resumes at, expressed in the IV's
/// own type.
///
/// A 4-byte IV takes an `I32` constant whose *bit pattern* is the `u32` value:
/// for an unsigned IV that is the same bytes the emitter would produce for a
/// `u32` constant, and for a signed IV the trip-count guard has already
/// proved `total < 2^31`, so the conversion is exact rather than wrapping.
/// `None` means "cannot be represented" and the caller declines before
/// mutating anything.
fn resume_offset_const(iv_ty: IrType, total_vec_bytes: i64) -> Option<IrConst> {
    if total_vec_bytes < 0 {
        return None;
    }
    match iv_ty {
        IrType::I32 | IrType::U32 => u32::try_from(total_vec_bytes)
            .ok()
            .map(|v| IrConst::I32(v as i32)),
        IrType::I64 | IrType::U64 => Some(IrConst::I64(total_vec_bytes)),
        _ => None,
    }
}

pub(super) fn try_transform(
    func: &mut IrFunction,
    loop_info: &loop_analysis::NaturalLoop,
    cfg: &CfgAnalysis,
    symbol_facts: &ModuleSymbolFacts,
) -> usize {
    if strided_reduce_disabled() {
        return 0;
    }
    let Some(pattern) = analyze(func, loop_info, cfg, symbol_facts) else {
        return 0;
    };
    transform(func, &pattern).unwrap_or_default()
}

/// Whether one wide `needed`-byte load from `base` can only touch memory the
/// program itself names.  Two proofs qualify:
///
/// * a current-frame alloca of proven extent (`proves_private_stack_object`);
/// * a strong, locally binding global whose defined size covers the window
///   (`ModuleSymbolFacts::global_min_bytes` — interposable, weak, common,
///   alias- and asm-label-redirected symbols and toplevel-asm units are
///   excluded there, so the definition cannot be replaced by a smaller one at
///   link time).
///
/// Everything else declines and keeps the strided load, which reads only the
/// bytes the scalar loop already reads.
fn wide_load_proven(
    func: &IrFunction,
    base: Value,
    needed: u64,
    symbol_facts: &ModuleSymbolFacts,
) -> bool {
    if proves_private_stack_object(func, base) {
        return alloca_extent_at_least(func, base, needed);
    }
    global_root_and_offset(func, base).is_some_and(|(name, byte_offset)| {
        // `ModuleSymbolFacts::from_module` already excludes redirected names
        // from `defined_sizes`; the explicit check keeps this proof locally
        // evident even if that construction invariant drifts. A GEP also
        // consumes part of the object: only the remaining extent after its
        // constant byte offset can cover the vector window.
        !symbol_facts.aliased_names.contains(&name)
            && symbol_facts
                .global_min_bytes(&name)
                .and_then(|sz| u64::try_from(sz).ok())
                .and_then(|sz| sz.checked_sub(byte_offset))
                .is_some_and(|remaining| remaining >= needed)
    })
}

/// Whether `base` is defined by a current-frame `alloca` whose byte size
/// covers `needed`; the extent half of the private-stack proof.
fn alloca_extent_at_least(func: &IrFunction, base: Value, needed: u64) -> bool {
    func.blocks.iter().any(|block| {
        block.instructions.iter().any(|inst| {
            matches!(inst, Instruction::Alloca { dest, size, .. }
                if *dest == base && u64::try_from(*size).is_ok_and(|sz| sz >= needed))
        })
    })
}

/// Resolve `base` to a global and the constant byte offset into that object,
/// following only provenance-preserving edges (the canonical [`root_leaf`] /
/// [`chain_step`] rules). Loads, calls, integer-to-pointer conversions,
/// unknown definitions, nonconstant/negative GEP or pointer-add offsets and
/// cycles fail closed. Keeping the offset is essential: `global + 8` has eight fewer
/// usable bytes than the definition's total size.
fn global_root_and_offset(func: &IrFunction, start: Value) -> Option<(String, u64)> {
    let mut cur = start;
    let mut byte_offset = 0u64;
    let mut seen = FxHashSet::default();
    for _ in 0..128 {
        if !seen.insert(cur) {
            return None;
        }
        let inst = unique_def(func, cur)?;
        match root_leaf(inst) {
            RootLeaf::Global(name) => return Some((name, byte_offset)),
            RootLeaf::NotLeaf => match inst {
                Instruction::GetElementPtr {
                    base,
                    offset: Operand::Const(c),
                    ..
                } => {
                    let delta = u64::try_from(c.to_i64()?).ok()?;
                    byte_offset = byte_offset.checked_add(delta)?;
                    cur = *base;
                }
                Instruction::GetElementPtr { .. } => return None,
                Instruction::BinOp {
                    op: IrBinOp::Add,
                    lhs,
                    rhs,
                    ty: IrType::Ptr | IrType::I64 | IrType::U64,
                    ..
                } => {
                    let (base, c) = match (lhs, rhs) {
                        (Operand::Value(base), Operand::Const(c))
                        | (Operand::Const(c), Operand::Value(base)) => (*base, c),
                        _ => return None,
                    };
                    let delta = u64::try_from(c.to_i64()?).ok()?;
                    byte_offset = byte_offset.checked_add(delta)?;
                    cur = base;
                }
                _ => match chain_step(inst) {
                    ChainStep::Follow(next) => cur = next,
                    ChainStep::Stop => return None,
                },
            },
            _ => return None,
        }
    }
    None
}

/// The unique definition of `value`.  Duplicate definitions make the pointer
/// chain position-dependent, so they cannot support a proof.
fn unique_def(func: &IrFunction, value: Value) -> Option<&Instruction> {
    let mut found = None;
    for block in &func.blocks {
        for inst in &block.instructions {
            if inst.dest() == Some(value) {
                if found.is_some() {
                    return None;
                }
                found = Some(inst);
            }
        }
    }
    found
}

fn trace_iv_offset(
    func: &IrFunction,
    loop_blocks: &FxHashSet<usize>,
    mut v: Value,
    iv_phi: Value,
) -> bool {
    for _ in 0..6 {
        if v == iv_phi {
            return true;
        }
        let Some((_, inst)) = find_inst_in_loop(func, loop_blocks, v) else {
            return false;
        };
        match inst {
            Instruction::Copy {
                src: Operand::Value(s),
                ..
            } => v = *s,
            Instruction::Cast {
                src: Operand::Value(s),
                from_ty,
                to_ty,
                ..
            } if from_ty.size() >= 4
                && to_ty.size() == 8
                && from_ty.cast_preserves_offset_value(*to_ty) =>
            {
                v = *s;
            }
            _ => return false,
        }
    }
    false
}

fn match_base_plus_iv(
    func: &IrFunction,
    loop_blocks: &FxHashSet<usize>,
    loop_defs: &FxHashSet<Value>,
    mut ptr_val: Value,
    iv_phi: Value,
) -> Option<Value> {
    for _ in 0..4 {
        let (_, inst) = find_inst_in_loop(func, loop_blocks, ptr_val)?;
        match inst {
            Instruction::Copy {
                src: Operand::Value(s),
                ..
            } => ptr_val = *s,
            Instruction::GetElementPtr {
                base,
                offset: Operand::Value(off_val),
                ..
            } => {
                if !loop_defs.contains(base) && trace_iv_offset(func, loop_blocks, *off_val, iv_phi)
                {
                    return Some(*base);
                }
                return None;
            }
            Instruction::BinOp {
                op: IrBinOp::Add,
                lhs: Operand::Value(a),
                rhs: Operand::Value(b),
                ty: IrType::I64 | IrType::U64 | IrType::Ptr,
                ..
            } => {
                if !loop_defs.contains(a) && trace_iv_offset(func, loop_blocks, *b, iv_phi) {
                    return Some(*a);
                }
                if !loop_defs.contains(b) && trace_iv_offset(func, loop_blocks, *a, iv_phi) {
                    return Some(*b);
                }
                return None;
            }
            _ => return None,
        }
    }
    None
}

fn match_ptr_const_disp(
    func: &IrFunction,
    loop_blocks: &FxHashSet<usize>,
    mut ptr_val: Value,
) -> (Value, i64) {
    for _ in 0..4 {
        let Some((_, inst)) = find_inst_in_loop(func, loop_blocks, ptr_val) else {
            return (ptr_val, 0);
        };
        match inst {
            Instruction::Copy {
                src: Operand::Value(s),
                ..
            } => ptr_val = *s,
            Instruction::GetElementPtr {
                base,
                offset: Operand::Const(c),
                ..
            } => {
                if let Some(d) = c.to_i64() {
                    return (*base, d);
                }
                return (ptr_val, 0);
            }
            Instruction::BinOp {
                op: IrBinOp::Add,
                lhs: Operand::Value(base),
                rhs: Operand::Const(c),
                ty: IrType::I64 | IrType::U64 | IrType::Ptr,
                ..
            } => {
                if let Some(d) = c.to_i64() {
                    return (*base, d);
                }
                return (ptr_val, 0);
            }
            _ => return (ptr_val, 0),
        }
    }
    (ptr_val, 0)
}

fn match_shifted_u8_byte(
    func: &IrFunction,
    loop_blocks: &FxHashSet<usize>,
    u8_loads: &FxHashMap<Value, Value>,
    mut v: Value,
) -> Option<(Value, usize, Value)> {
    let mut shift = 0i64;
    for _ in 0..4 {
        let (_, inst) = find_inst_in_loop(func, loop_blocks, v)?;
        match inst {
            Instruction::Copy {
                src: Operand::Value(s),
                ..
            }
            | Instruction::Cast {
                src: Operand::Value(s),
                from_ty: IrType::I32 | IrType::U32,
                to_ty: IrType::I32 | IrType::U32,
                ..
            } => v = *s,
            Instruction::BinOp {
                op: IrBinOp::Shl,
                lhs: Operand::Value(l),
                rhs: Operand::Const(c),
                ty: IrType::I32 | IrType::U32,
                ..
            } if shift == 0 => {
                shift = c.to_i64()?;
                if !matches!(shift, 8 | 16 | 24) {
                    return None;
                }
                v = *l;
            }
            Instruction::Cast {
                src: Operand::Value(ld),
                from_ty: IrType::U8,
                to_ty: IrType::I32 | IrType::U32,
                ..
            } => {
                let &ptr_val = u8_loads.get(ld)?;
                let (base_p0, disp) = match_ptr_const_disp(func, loop_blocks, ptr_val);
                let byte_idx = (shift / 8) as usize;
                if disp != shift / 8 {
                    return None;
                }
                return Some((base_p0, byte_idx, *ld));
            }
            _ => return None,
        }
    }
    None
}

fn collect_le_u32_reassembly(
    func: &IrFunction,
    loop_blocks: &FxHashSet<usize>,
    u8_loads: &FxHashMap<Value, Value>,
    v: Value,
    slots: &mut [Option<(Value, Value)>; 4],
    depth: usize,
) -> bool {
    if depth > 6 {
        return false;
    }
    if let Some((p0, idx, ld)) = match_shifted_u8_byte(func, loop_blocks, u8_loads, v) {
        if slots[idx].is_some() {
            return false;
        }
        slots[idx] = Some((p0, ld));
        return true;
    }
    let Some((_, inst)) = find_inst_in_loop(func, loop_blocks, v) else {
        return false;
    };
    match inst {
        Instruction::Copy {
            src: Operand::Value(s),
            ..
        } => collect_le_u32_reassembly(func, loop_blocks, u8_loads, *s, slots, depth + 1),
        Instruction::BinOp {
            op: IrBinOp::Or | IrBinOp::Xor | IrBinOp::Add,
            lhs: Operand::Value(l),
            rhs: Operand::Value(r),
            ty: IrType::I32 | IrType::U32,
            ..
        } => {
            collect_le_u32_reassembly(func, loop_blocks, u8_loads, *l, slots, depth + 1)
                && collect_le_u32_reassembly(func, loop_blocks, u8_loads, *r, slots, depth + 1)
        }
        _ => false,
    }
}

fn trace_iv_low32(
    func: &IrFunction,
    loop_blocks: &FxHashSet<usize>,
    mut v: Value,
    iv_phi: Value,
) -> bool {
    for _ in 0..6 {
        if v == iv_phi {
            return true;
        }
        let Some((_, inst)) = find_inst_in_loop(func, loop_blocks, v) else {
            return false;
        };
        match inst {
            Instruction::Copy {
                src: Operand::Value(s),
                ..
            } => v = *s,
            Instruction::Cast {
                src: Operand::Value(s),
                from_ty,
                to_ty,
                ..
            } if from_ty.is_integer()
                && to_ty.is_integer()
                && from_ty.size() >= 4
                && to_ty.size() >= 4 =>
            {
                v = *s;
            }
            _ => return false,
        }
    }
    false
}

#[allow(clippy::too_many_arguments)]
fn parse_expr(
    func: &IrFunction,
    loop_blocks: &FxHashSet<usize>,
    iv_phi: Value,
    acc_phi: Value,
    load_map: &FxHashMap<Value, usize>,
    operand: &Operand,
    depth: usize,
) -> Option<StridedExpr> {
    if depth > 12 {
        return None;
    }
    match operand {
        Operand::Const(c) => {
            let v = c.to_i64()?;
            Some(StridedExpr::Invariant(Operand::Const(IrConst::I32(
                v as i32,
            ))))
        }
        Operand::Value(v) => {
            if *v == acc_phi {
                return None;
            }
            if let Some(&stream_idx) = load_map.get(v) {
                return Some(StridedExpr::LoadEven32(stream_idx));
            }
            if trace_iv_low32(func, loop_blocks, *v, iv_phi) {
                return Some(StridedExpr::IvLow32);
            }
            let Some((_, inst)) = find_inst_in_loop(func, loop_blocks, *v) else {
                // Defined outside the loop (parameter or preheader value).
                return Some(StridedExpr::Invariant(Operand::Value(*v)));
            };
            match inst {
                Instruction::Copy { src, .. } => {
                    parse_expr(func, loop_blocks, iv_phi, acc_phi, load_map, src, depth + 1)
                }
                Instruction::Cast {
                    src,
                    from_ty,
                    to_ty,
                    ..
                } if matches!(from_ty, IrType::I32 | IrType::U32)
                    && matches!(to_ty, IrType::I32 | IrType::U32) =>
                {
                    parse_expr(func, loop_blocks, iv_phi, acc_phi, load_map, src, depth + 1)
                }
                Instruction::BinOp {
                    op,
                    lhs,
                    rhs,
                    ty: IrType::I32 | IrType::U32,
                    ..
                } => {
                    let supported = matches!(
                        op,
                        IrBinOp::Add
                            | IrBinOp::Sub
                            | IrBinOp::Mul
                            | IrBinOp::And
                            | IrBinOp::Or
                            | IrBinOp::Xor
                            | IrBinOp::Shl
                            | IrBinOp::LShr
                            | IrBinOp::AShr
                    );
                    if !supported {
                        return None;
                    }
                    let l =
                        parse_expr(func, loop_blocks, iv_phi, acc_phi, load_map, lhs, depth + 1)?;
                    if matches!(op, IrBinOp::Shl | IrBinOp::LShr | IrBinOp::AShr) {
                        let Operand::Const(c) = rhs else {
                            return None;
                        };
                        let sh = c.to_i64()?;
                        if !(0..32).contains(&sh) {
                            return None;
                        }
                        Some(StridedExpr::BinOp(
                            *op,
                            Box::new(l),
                            Box::new(StridedExpr::Invariant(Operand::Const(IrConst::I32(
                                sh as i32,
                            )))),
                        ))
                    } else {
                        let r = parse_expr(
                            func,
                            loop_blocks,
                            iv_phi,
                            acc_phi,
                            load_map,
                            rhs,
                            depth + 1,
                        )?;
                        // Unsigned right shift depends on carrier signedness: keep
                        // LShr/AShr exact by using the instruction's own `op`.
                        Some(StridedExpr::BinOp(*op, Box::new(l), Box::new(r)))
                    }
                }
                _ => None,
            }
        }
    }
}

fn expr_has_load(e: &StridedExpr) -> bool {
    match e {
        StridedExpr::LoadEven32(_) => true,
        StridedExpr::IvLow32 | StridedExpr::Invariant(_) => false,
        StridedExpr::BinOp(_, l, r) => expr_has_load(l) || expr_has_load(r),
    }
}

/// Every value the loop's own blocks define.
fn loop_defined_values(
    func: &IrFunction,
    loop_info: &loop_analysis::NaturalLoop,
) -> FxHashSet<Value> {
    let mut defs = FxHashSet::default();
    for &b in &loop_info.body {
        for inst in &func.blocks[b].instructions {
            if let Some(d) = inst.dest() {
                defs.insert(d);
            }
        }
    }
    defs
}

/// Is any loop-defined value other than `acc` read outside the loop?
///
/// The transform rebuilds the loop body from the packed expression and leaves
/// the scalar remainder to resume from the induction variable, so a value the
/// loop defines cannot be consumed by the surrounding code afterwards: its new
/// definition would not match. `acc` is the one exception -- it is the
/// reduction's result and the caller keeps reading it.
fn loop_value_escapes(
    func: &IrFunction,
    loop_info: &loop_analysis::NaturalLoop,
    loop_defs: &FxHashSet<Value>,
    acc: Value,
) -> bool {
    let escapes =
        |op: &Operand| matches!(op, Operand::Value(v) if loop_defs.contains(v) && *v != acc);
    for (b_idx, block) in func.blocks.iter().enumerate() {
        if loop_info.body.contains(&b_idx) {
            continue;
        }
        let mut bad = false;
        for inst in &block.instructions {
            crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                if escapes(op) {
                    bad = true;
                }
            });
            // Bare `Value` fields — Store/Load ptr, GEP base, Memcpy dest/src,
            // Intrinsic dest_ptr, … — are invisible to the operand walk.  The
            // previous GEP-only special case covered one spelling of this; the
            // walker covers all of them (and GEP's base goes through it).
            crate::backend::liveness::for_each_value_use_in_instruction(inst, |v| {
                if loop_defs.contains(v) && *v != acc {
                    bad = true;
                }
            });
        }
        crate::backend::liveness::for_each_operand_in_terminator(&block.terminator, |op| {
            if escapes(op) {
                bad = true;
            }
        });
        if bad {
            return true;
        }
    }
    false
}

/// The header's exit test and its two phis, as
/// `(phis, cmp_op, cmp_lhs, cmp_ty, bound_raw)`.  Every refusal goes through
/// `decline!`, so `LCCC_VEC_STRIDED_TRACE` names the exact precondition line
/// even though the helper is a separate frame from [`analyze`]; the caller
/// re-traces its own line when this returns `None`.
fn header_exit_test(
    func: &IrFunction,
    header_idx: usize,
    cmp_dest: Value,
    preheader_label: BlockId,
    body_label: BlockId,
) -> Option<(
    Vec<(Value, IrType, Operand, Operand)>,
    IrCmpOp,
    Value,
    IrType,
    i64,
)> {
    let mut phis = Vec::new();
    let mut exit_cmp = None;
    for inst in &func.blocks[header_idx].instructions {
        match inst {
            Instruction::Phi {
                dest, ty, incoming, ..
            } => {
                if incoming.len() != 2 {
                    decline!();
                }
                let Some((pre, _)) = incoming.iter().find(|(_, l)| *l == preheader_label) else {
                    decline!();
                };
                let Some((back, _)) = incoming.iter().find(|(_, l)| *l == body_label) else {
                    decline!();
                };
                phis.push((*dest, *ty, pre.clone(), back.clone()));
            }
            Instruction::Cmp {
                dest,
                op,
                lhs,
                rhs,
                ty,
            } if *dest == cmp_dest => {
                exit_cmp = Some((*op, lhs.clone(), rhs.clone(), *ty));
            }
            _ => {
                decline!();
            }
        }
    }
    if phis.len() != 2 {
        decline!();
    }
    let Some(exit_cmp) = exit_cmp else {
        decline!();
    };
    let (cmp_op, Operand::Value(cmp_lhs), Operand::Const(cmp_rhs_c), cmp_ty) = exit_cmp else {
        decline!();
    };
    let Some(bound_raw) = cmp_rhs_c.to_i64() else {
        decline!();
    };
    if bound_raw <= 0 {
        decline!();
    }
    Some((phis, cmp_op, cmp_lhs, cmp_ty, bound_raw))
}

fn analyze(
    func: &IrFunction,
    loop_info: &loop_analysis::NaturalLoop,
    cfg: &CfgAnalysis,
    symbol_facts: &ModuleSymbolFacts,
) -> Option<StridedI32ReducePattern> {
    if loop_info.body.len() != 2 {
        decline!();
    }
    let header_idx = loop_info.header;
    let Some(&body_idx) = loop_info.body.iter().find(|&&b| b != header_idx) else {
        decline!();
    };
    let header_label = func.blocks[header_idx].label;
    let body_label = func.blocks[body_idx].label;
    if !matches!(func.blocks[body_idx].terminator, Terminator::Branch(lbl) if lbl == header_label) {
        decline!();
    }

    // Unique external preheader jumping unconditionally to `header_idx`.
    let Some(preheader_idx) = loop_info.find_preheader(&cfg.preds) else {
        decline!();
    };
    if !matches!(func.blocks[preheader_idx].terminator, Terminator::Branch(lbl) if lbl == header_label)
    {
        decline!();
    }
    let preheader_label = func.blocks[preheader_idx].label;

    // Header must branch to body on true and exit outside the loop on false.
    let Terminator::CondBranch {
        cond: Operand::Value(cmp_dest),
        true_label,
        false_label,
    } = func.blocks[header_idx].terminator
    else {
        decline!();
    };
    if true_label != body_label {
        decline!();
    }
    let Some(exit_idx) = func.blocks.iter().position(|b| b.label == false_label) else {
        decline!();
    };
    if loop_info.body.contains(&exit_idx) {
        decline!();
    }

    // Header must contain only two 2-way Phis (IV and accumulator) plus the exit Cmp.
    let Some((phis, cmp_op, cmp_lhs, cmp_ty, bound_raw)) =
        header_exit_test(func, header_idx, cmp_dest, preheader_label, body_label)
    else {
        decline!();
    };

    // Identify which phi is `iv_phi` (compared in `exit_cmp`, starts at 0, steps by +8).
    let Some(iv_pos) = phis.iter().position(|(d, _, _, _)| *d == cmp_lhs) else {
        decline!();
    };
    let (iv_phi, iv_ty, iv_init, Operand::Value(iv_next)) = phis[iv_pos].clone() else {
        decline!();
    };
    let (acc_phi, acc_ty, acc_init, Operand::Value(acc_next)) = phis[1 - iv_pos].clone() else {
        decline!();
    };
    if !matches!(iv_ty, IrType::U64 | IrType::I64 | IrType::U32 | IrType::I32)
        || cmp_ty != iv_ty
        || !matches!(acc_ty, IrType::I32 | IrType::U32)
    {
        decline!();
    }
    let Operand::Const(iv_init_c) = iv_init else {
        decline!();
    };
    if iv_init_c.to_i64() != Some(0) {
        decline!();
    }
    let Some(scalar_trips) = scalar_trip_count(cmp_op, iv_ty, bound_raw) else {
        decline!();
    };
    if scalar_trips < MIN_SCALAR_TRIPS {
        decline!();
    }

    // Verify `iv_next = iv_phi + 8` in `body_idx`.
    let iv_step_ok = func.blocks[body_idx].instructions.iter().any(|inst| {
        matches!(
            inst,
            Instruction::BinOp {
                dest,
                op: IrBinOp::Add,
                lhs: Operand::Value(l),
                rhs: Operand::Const(c),
                ty,
            } if *dest == iv_next && *l == iv_phi && *ty == iv_ty && c.to_i64() == Some(8)
        )
    });
    if !iv_step_ok {
        decline!();
    }

    // Escape closure: nothing the loop defines may be observed outside it,
    // except the accumulator (which IS the loop's result).
    let loop_defs = loop_defined_values(func, loop_info);
    if loop_value_escapes(func, loop_info, &loop_defs, acc_phi) {
        decline!();
    }

    // Scan `body_idx`: allow only pure non-trapping instructions and non-volatile
    // 32-bit loads (or 4-byte little-endian U8 reassemblies) at `base + iv_phi`
    // with loop-invariant `base`.
    let mut load_bases: Vec<Value> = Vec::new();
    let mut load_map: FxHashMap<Value, usize> = FxHashMap::default();
    let mut u8_loads: FxHashMap<Value, Value> = FxHashMap::default();
    let mut consumed_u8_loads: FxHashSet<Value> = FxHashSet::default();
    let mut contrib_op: Option<Operand> = None;
    let mut acc_uses_in_body = 0usize;

    let register_base = |base: Value, load_bases: &mut Vec<Value>| -> Option<usize> {
        if let Some(pos) = load_bases.iter().position(|&b| b == base) {
            Some(pos)
        } else if load_bases.len() < 4 {
            let pos = load_bases.len();
            load_bases.push(base);
            Some(pos)
        } else {
            None
        }
    };

    for inst in &func.blocks[body_idx].instructions {
        crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
            if matches!(op, Operand::Value(v) if *v == acc_phi) {
                acc_uses_in_body += 1;
            }
        });
        match inst {
            Instruction::Load {
                dest,
                ptr,
                ty: IrType::I32 | IrType::U32,
                seg_override: AddressSpace::Default,
                volatile: false,
            } => {
                let Some(base) =
                    match_base_plus_iv(func, &loop_info.body, &loop_defs, *ptr, iv_phi)
                else {
                    decline!();
                };
                let Some(stream_idx) = register_base(base, &mut load_bases) else {
                    decline!();
                };
                load_map.insert(*dest, stream_idx);
            }
            Instruction::Load {
                dest,
                ptr,
                ty: IrType::U8,
                seg_override: AddressSpace::Default,
                volatile: false,
            } => {
                u8_loads.insert(*dest, *ptr);
            }
            Instruction::BinOp {
                dest,
                op: IrBinOp::Add,
                lhs,
                rhs,
                ty,
            } if *dest == acc_next && *ty == acc_ty => {
                if *lhs == Operand::Value(acc_phi) {
                    contrib_op = Some(rhs.clone());
                } else if *rhs == Operand::Value(acc_phi) {
                    contrib_op = Some(lhs.clone());
                } else {
                    decline!();
                }
            }
            Instruction::BinOp { dest, op, ty, .. } if !op.can_trap() && ty.is_integer() => {
                if !u8_loads.is_empty()
                    && matches!(op, IrBinOp::Or | IrBinOp::Xor | IrBinOp::Add)
                    && matches!(ty, IrType::I32 | IrType::U32)
                {
                    let mut slots: [Option<(Value, Value)>; 4] = [None; 4];
                    if collect_le_u32_reassembly(
                        func,
                        &loop_info.body,
                        &u8_loads,
                        *dest,
                        &mut slots,
                        0,
                    ) {
                        if let [
                            Some((p0, l0)),
                            Some((p1, l1)),
                            Some((p2, l2)),
                            Some((p3, l3)),
                        ] = slots
                        {
                            if p0 == p1 && p0 == p2 && p0 == p3 {
                                if let Some(base) = match_base_plus_iv(
                                    func,
                                    &loop_info.body,
                                    &loop_defs,
                                    p0,
                                    iv_phi,
                                ) {
                                    let Some(stream_idx) = register_base(base, &mut load_bases)
                                    else {
                                        decline!();
                                    };
                                    load_map.insert(*dest, stream_idx);
                                    consumed_u8_loads.extend([l0, l1, l2, l3]);
                                }
                            }
                        }
                    }
                }
            }
            Instruction::Cast { from_ty, to_ty, .. }
                if from_ty.is_integer() && to_ty.is_integer() => {}
            Instruction::Copy { .. } | Instruction::GetElementPtr { .. } => {}
            _ => {
                decline!();
            }
        }
    }
    if consumed_u8_loads.len() != u8_loads.len() || acc_uses_in_body != 1 || load_bases.is_empty() {
        decline!();
    }
    let Some(contrib_op) = contrib_op else {
        decline!();
    };
    let Some(expr) = parse_expr(
        func,
        &loop_info.body,
        iv_phi,
        acc_phi,
        &load_map,
        &contrib_op,
        0,
    ) else {
        decline!();
    };
    if !expr_has_load(&expr) {
        decline!();
    }

    let safe_vecs = safe_vec_count(scalar_trips);
    if safe_vecs < 4 {
        decline!();
    }
    let Some(vector_bytes) = safe_vecs.checked_mul(VECTOR_BYTES) else {
        decline!();
    };
    let wide_load_proven = load_bases
        .iter()
        .map(|&base| wide_load_proven(func, base, vector_bytes, symbol_facts))
        .collect();

    Some(StridedI32ReducePattern {
        preheader_idx,
        header_idx,
        body_idx,
        iv_phi,
        iv_ty,
        acc_phi,
        acc_ty,
        acc_init,
        safe_vecs,
        load_bases,
        wide_load_proven,
        expr,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StridedWidth {
    X4,
    X4Compact,
    X8,
}

impl StridedWidth {
    fn vector_bytes(self) -> u64 {
        match self {
            Self::X4 => 16,
            Self::X4Compact | Self::X8 => 32,
        }
    }

    /// `analyze` counts four-dword groups. Exact SSE2 handles the same proven
    /// prefix as twice as many two-dword groups; compact SSE2 and AVX2 retain
    /// the four-dword group count. Any remainder stays in the scalar loop.
    fn safe_vecs(self, x8_vecs: u64) -> u64 {
        match self {
            Self::X4 => x8_vecs * 2,
            Self::X4Compact | Self::X8 => x8_vecs,
        }
    }

    fn load(self, contiguous: bool) -> Option<IntrinsicOp> {
        let op = match (self, contiguous) {
            (Self::X4, true) => IntrinsicOp::VecLoadI32x4,
            (Self::X4, false) => IntrinsicOp::VecLoadStrided2I32x4,
            (Self::X4Compact, true) => IntrinsicOp::VecLoadStrided2CompactI32x4,
            // A compact load consumes the gap bytes. Without the contiguous
            // window proof there is no legal compact operation to emit.
            (Self::X4Compact, false) => return None,
            (Self::X8, true) => IntrinsicOp::VecLoadI32x8,
            (Self::X8, false) => IntrinsicOp::VecLoadStrided2I32x8,
        };
        Some(op)
    }

    fn broadcast(self) -> IntrinsicOp {
        match self {
            Self::X4 | Self::X4Compact => IntrinsicOp::VecBroadcastI32x4,
            Self::X8 => IntrinsicOp::VecBroadcastI32x8,
        }
    }

    fn zero(self) -> IntrinsicOp {
        match self {
            Self::X4 | Self::X4Compact => IntrinsicOp::VecZeroI32x4,
            Self::X8 => IntrinsicOp::VecZeroI32x8,
        }
    }

    fn bin_op(self, op: IrBinOp) -> Option<IntrinsicOp> {
        let intrinsic = match (self, op) {
            (Self::X4 | Self::X4Compact, IrBinOp::Add) => IntrinsicOp::VecAddI32x4,
            (Self::X4 | Self::X4Compact, IrBinOp::Sub) => IntrinsicOp::VecSubI32x4,
            (Self::X4 | Self::X4Compact, IrBinOp::Mul) => IntrinsicOp::VecMulI32x4,
            (Self::X4 | Self::X4Compact, IrBinOp::And) => IntrinsicOp::VecAndI32x4,
            (Self::X4 | Self::X4Compact, IrBinOp::Or) => IntrinsicOp::VecOrI32x4,
            (Self::X4 | Self::X4Compact, IrBinOp::Xor) => IntrinsicOp::VecXorI32x4,
            (Self::X8, IrBinOp::Add) => IntrinsicOp::VecAddI32x8,
            (Self::X8, IrBinOp::Sub) => IntrinsicOp::VecSubI32x8,
            (Self::X8, IrBinOp::Mul) => IntrinsicOp::VecMulI32x8,
            (Self::X8, IrBinOp::And) => IntrinsicOp::VecAndI32x8,
            (Self::X8, IrBinOp::Or) => IntrinsicOp::VecOrI32x8,
            (Self::X8, IrBinOp::Xor) => IntrinsicOp::VecXorI32x8,
            _ => return None,
        };
        Some(intrinsic)
    }

    fn shift_op(self, op: IrBinOp) -> Option<IntrinsicOp> {
        let intrinsic = match (self, op) {
            (Self::X4 | Self::X4Compact, IrBinOp::Shl) => IntrinsicOp::VecShlI32x4,
            (Self::X4 | Self::X4Compact, IrBinOp::LShr) => IntrinsicOp::VecLShrI32x4,
            (Self::X4 | Self::X4Compact, IrBinOp::AShr) => IntrinsicOp::VecAShrI32x4,
            (Self::X8, IrBinOp::Shl) => IntrinsicOp::VecShlI32x8,
            (Self::X8, IrBinOp::LShr) => IntrinsicOp::VecLShrI32x8,
            (Self::X8, IrBinOp::AShr) => IntrinsicOp::VecAShrI32x8,
            _ => return None,
        };
        Some(intrinsic)
    }

    fn horizontal_add(self) -> IntrinsicOp {
        match self {
            Self::X4 | Self::X4Compact => IntrinsicOp::VecHorizontalAddI32x4,
            Self::X8 => IntrinsicOp::VecHorizontalAddI32x8,
        }
    }
}

struct EmitCtx<'a> {
    width: StridedWidth,
    load_bases: &'a [Value],
    wide_load_proven: &'a [bool],
    byte_iv: Value,
    byte_disp: i64,
    vec_iv: Option<Value>,
    bcast_cache: Vec<(Operand, Value)>,
    preheader_insts: Vec<Instruction>,
    body_insts: Vec<Instruction>,
    next_val: u32,
}

impl EmitCtx<'_> {
    fn fresh(&mut self) -> Value {
        let v = Value(self.next_val);
        self.next_val += 1;
        v
    }

    /// Emit one expression into temporary block buffers. `None` refuses the
    /// whole transform; the caller has not mutated the input function yet.
    fn emit(&mut self, expr: &StridedExpr) -> Option<Value> {
        match expr {
            StridedExpr::LoadEven32(idx) => {
                let base = *self.load_bases.get(*idx)?;
                let contiguous = *self.wide_load_proven.get(*idx)?;
                let dest = self.fresh();
                let mut args = vec![Operand::Value(base), Operand::Value(self.byte_iv)];
                if self.byte_disp != 0 {
                    args.push(Operand::Const(IrConst::I64(self.byte_disp)));
                }
                self.body_insts.push(Instruction::Intrinsic {
                    dest: Some(dest),
                    op: self.width.load(contiguous)?,
                    dest_ptr: None,
                    args,
                });
                Some(dest)
            }
            StridedExpr::IvLow32 => self.vec_iv,
            StridedExpr::Invariant(op) => {
                if let Some(&(_, cached)) = self.bcast_cache.iter().find(|(k, _)| k == op) {
                    return Some(cached);
                }
                let bcast_src = match op {
                    Operand::Const(c) => Operand::Const(IrConst::I32(c.to_i64()? as i32)),
                    Operand::Value(v) => Operand::Value(*v),
                };
                let dest = self.fresh();
                self.preheader_insts.push(Instruction::Intrinsic {
                    dest: Some(dest),
                    op: self.width.broadcast(),
                    dest_ptr: None,
                    args: vec![bcast_src],
                });
                self.bcast_cache.push((op.clone(), dest));
                Some(dest)
            }
            StridedExpr::BinOp(op, l, r) => {
                let lv = self.emit(l)?;
                if matches!(op, IrBinOp::Shl | IrBinOp::LShr | IrBinOp::AShr) {
                    let StridedExpr::Invariant(sh_op) = r.as_ref() else {
                        return None;
                    };
                    let vec_op = self.width.shift_op(*op)?;
                    let dest = self.fresh();
                    self.body_insts.push(Instruction::Intrinsic {
                        dest: Some(dest),
                        op: vec_op,
                        dest_ptr: None,
                        args: vec![Operand::Value(lv), sh_op.clone()],
                    });
                    return Some(dest);
                }
                let rv = self.emit(r)?;
                let vec_op = self.width.bin_op(*op)?;
                let dest = self.fresh();
                self.body_insts.push(Instruction::Intrinsic {
                    dest: Some(dest),
                    op: vec_op,
                    dest_ptr: None,
                    args: vec![Operand::Value(lv), Operand::Value(rv)],
                });
                Some(dest)
            }
        }
    }
}

struct VectorSchedule {
    width: StridedWidth,
    vector_bytes: u64,
    unroll: u64,
    step_bytes: i64,
    total_vec_bytes: i64,
    needs_iv_vec: bool,
    multi_iv: bool,
}

impl VectorSchedule {
    fn new(p: &StridedI32ReducePattern) -> Option<Self> {
        let width = if x86_avx2_available() && !sse2_forced() {
            StridedWidth::X8
        } else if p.wide_load_proven.iter().all(|&proven| proven) {
            // Every stream can read its full 32-byte window: compact two XMM
            // halves into four real lanes instead of carrying zero odd lanes.
            StridedWidth::X4Compact
        } else {
            // One unknown stream is enough to keep the whole expression on the
            // exact-access representation; mixing lane meanings is not valid.
            StridedWidth::X4
        };
        let vector_bytes = width.vector_bytes();
        let safe_vecs = width.safe_vecs(p.safe_vecs);
        // Preserve the proven AVX2 schedule exactly. SSE2 uses the same two
        // independent accumulator chains, but only two XMM copies per turn:
        // four copies were measured as a substantial regression.
        let unroll: u64 = if safe_vecs >= 8 {
            match width {
                StridedWidth::X4 | StridedWidth::X4Compact => 2,
                StridedWidth::X8 => 4,
            }
        } else {
            1
        };
        let vec_turns = safe_vecs / unroll;
        if vec_turns == 0 {
            return None;
        }
        let step_bytes = i64::try_from(unroll.checked_mul(vector_bytes)?).ok()?;
        let total_vec_bytes =
            i64::try_from(vec_turns.checked_mul(unroll)?.checked_mul(vector_bytes)?).ok()?;
        fn uses_iv(e: &StridedExpr) -> bool {
            match e {
                StridedExpr::IvLow32 => true,
                StridedExpr::LoadEven32(_) | StridedExpr::Invariant(_) => false,
                StridedExpr::BinOp(_, l, r) => uses_iv(l) || uses_iv(r),
            }
        }
        let needs_iv_vec = uses_iv(&p.expr);
        // Two compact XMM streams can carry adjacent IV vectors directly. This
        // replaces the second stream's per-turn register copy + offset add with
        // one in-place step, while consuming the offset constant's old register.
        let multi_iv = width == StridedWidth::X4Compact && unroll == 2 && needs_iv_vec;
        Some(Self {
            width,
            vector_bytes,
            unroll,
            step_bytes,
            total_vec_bytes,
            needs_iv_vec,
            multi_iv,
        })
    }
}

#[derive(Clone, Copy)]
struct VectorPhi {
    init: Value,
    phi: Value,
    next: Value,
}

struct IvPlan {
    main: VectorPhi,
    step: Value,
    offsets: Vec<Value>,
    extra: Vec<VectorPhi>,
}

struct PreheaderPlan {
    acc0: VectorPhi,
    acc1: Option<VectorPhi>,
    iv: Option<IvPlan>,
    ones_tbl: Option<Value>,
    byte_iv_phi: Value,
    byte_iv_next: Value,
    vec_cmp_dest: Value,
    instructions: Vec<Instruction>,
}

fn build_preheader(schedule: &VectorSchedule, next_value: &mut u32) -> Option<PreheaderPlan> {
    let width = schedule.width;
    let mut next_val = *next_value;
    let mut fresh = || {
        let v = Value(next_val);
        next_val += 1;
        v
    };
    let mut instructions = Vec::new();

    let acc0_init = fresh();
    instructions.push(Instruction::Intrinsic {
        dest: Some(acc0_init),
        op: width.zero(),
        dest_ptr: None,
        args: vec![],
    });
    let acc0 = VectorPhi {
        init: acc0_init,
        phi: fresh(),
        next: fresh(),
    };
    let acc1 = if schedule.unroll > 1 {
        let init = fresh();
        instructions.push(Instruction::Intrinsic {
            dest: Some(init),
            op: width.zero(),
            dest_ptr: None,
            args: vec![],
        });
        Some(VectorPhi {
            init,
            phi: fresh(),
            next: fresh(),
        })
    } else {
        None
    };

    let ones_tbl = (width == StridedWidth::X8).then(|| {
        let tbl = fresh();
        // 16 i16 lanes `[1, 0, 1, 0, ..., 1, 0]` so `vpmaddwd(tbl, ones_tbl)`
        // zero/sign-extends the 8 even i16 lanes of `tbl` into i32x8 class 5.
        let mut bytes = Vec::with_capacity(32);
        for i in 0..16usize {
            let v: i16 = if i % 2 == 0 { 1 } else { 0 };
            for b in v.to_le_bytes() {
                bytes.push(Operand::Const(IrConst::I8(b as i8)));
            }
        }
        instructions.push(Instruction::Intrinsic {
            dest: Some(tbl),
            op: IntrinsicOp::VecConstI8x32,
            dest_ptr: None,
            args: bytes,
        });
        tbl
    });

    let iv = if schedule.needs_iv_vec {
        let init = if width == StridedWidth::X8 {
            let raw_tbl = fresh();
            // 16 i16 lanes `[0, 0, 0, 0, 8, 0, 0, 0, 16, 0, 0, 0, 24, 0, 0, 0]` ->
            // `vpmaddwd(raw_tbl, ones_tbl)` produces i32x8 `[0, 0, 8, 0, 16, 0, 24, 0]`
            // directly in reduction web class 5 (`I32x8`), keeping `iv_vec_phi`
            // register-homed in a YMM register across the loop.
            let lane_i16: [i16; 16] = [0, 0, 0, 0, 8, 0, 0, 0, 16, 0, 0, 0, 24, 0, 0, 0];
            let mut init_bytes = Vec::with_capacity(32);
            for v in lane_i16 {
                for b in v.to_le_bytes() {
                    init_bytes.push(Operand::Const(IrConst::I8(b as i8)));
                }
            }
            instructions.push(Instruction::Intrinsic {
                dest: Some(raw_tbl),
                op: IntrinsicOp::VecConstI8x32,
                dest_ptr: None,
                args: init_bytes,
            });
            let init = fresh();
            instructions.push(Instruction::Intrinsic {
                dest: Some(init),
                op: IntrinsicOp::VecMaddwdI16x16,
                dest_ptr: None,
                args: vec![Operand::Value(raw_tbl), Operand::Value(ones_tbl?)],
            });
            init
        } else {
            let init = fresh();
            let lanes: [i32; 4] = if width == StridedWidth::X4Compact {
                [0, 8, 16, 24]
            } else {
                [0, 0, 8, 0]
            };
            instructions.push(Instruction::Intrinsic {
                dest: Some(init),
                op: IntrinsicOp::VecPackI32x4,
                dest_ptr: None,
                args: lanes
                    .into_iter()
                    .map(|v| Operand::Const(IrConst::I32(v)))
                    .collect(),
            });
            init
        };
        let step = fresh();
        instructions.push(Instruction::Intrinsic {
            dest: Some(step),
            op: width.broadcast(),
            dest_ptr: None,
            args: vec![Operand::Const(IrConst::I32(schedule.step_bytes as i32))],
        });
        let mut offsets = Vec::new();
        let mut extra = Vec::new();
        if schedule.unroll > 1 {
            for copy_idx in 1..schedule.unroll as i32 {
                let byte_offset = copy_idx * schedule.vector_bytes as i32;
                if schedule.multi_iv {
                    // Only the compact XMM pair carries two strided streams;
                    // these four lanes are the complete vector. Refuse an
                    // inconsistent schedule in every build, not just under
                    // debug assertions, before emitting a wrong-width pack.
                    if schedule.width != StridedWidth::X4Compact {
                        return None;
                    }
                    let init = fresh();
                    instructions.push(Instruction::Intrinsic {
                        dest: Some(init),
                        op: IntrinsicOp::VecPackI32x4,
                        dest_ptr: None,
                        args: [
                            byte_offset,
                            byte_offset + 8,
                            byte_offset + 16,
                            byte_offset + 24,
                        ]
                        .into_iter()
                        .map(|v| Operand::Const(IrConst::I32(v)))
                        .collect(),
                    });
                    extra.push(VectorPhi {
                        init,
                        phi: fresh(),
                        next: fresh(),
                    });
                } else {
                    let ov = fresh();
                    instructions.push(Instruction::Intrinsic {
                        dest: Some(ov),
                        op: width.broadcast(),
                        dest_ptr: None,
                        args: vec![Operand::Const(IrConst::I32(byte_offset))],
                    });
                    offsets.push(ov);
                }
            }
        }
        let main = VectorPhi {
            init,
            phi: fresh(),
            next: fresh(),
        };
        Some(IvPlan {
            main,
            step,
            offsets,
            extra,
        })
    } else {
        None
    };

    let byte_iv_phi = fresh();
    let byte_iv_next = fresh();
    let vec_cmp_dest = fresh();
    *next_value = next_val;
    Some(PreheaderPlan {
        acc0,
        acc1,
        iv,
        ones_tbl,
        byte_iv_phi,
        byte_iv_next,
        vec_cmp_dest,
        instructions,
    })
}

fn emit_vector_body(
    ctx: &mut EmitCtx<'_>,
    p: &StridedI32ReducePattern,
    schedule: &VectorSchedule,
    plan: &PreheaderPlan,
) -> Option<()> {
    let width = schedule.width;
    let iv = if schedule.needs_iv_vec {
        Some(plan.iv.as_ref()?)
    } else {
        None
    };
    let acc1 = if schedule.unroll > 1 {
        Some(plan.acc1.as_ref()?)
    } else {
        None
    };

    if schedule.unroll > 1 {
        let mut copy_vals = Vec::with_capacity(schedule.unroll as usize);
        for copy_idx in 0..schedule.unroll as usize {
            ctx.byte_disp = (copy_idx as i64) * schedule.vector_bytes as i64;
            if let Some(iv) = iv {
                if schedule.multi_iv {
                    let selected = if copy_idx == 0 {
                        iv.main.phi
                    } else {
                        iv.extra.get(copy_idx - 1)?.phi
                    };
                    ctx.vec_iv = Some(selected);
                } else if copy_idx == 0 {
                    ctx.vec_iv = Some(iv.main.phi);
                } else {
                    let cur_iv = ctx.fresh();
                    ctx.body_insts.push(Instruction::Intrinsic {
                        dest: Some(cur_iv),
                        op: width.bin_op(IrBinOp::Add)?,
                        dest_ptr: None,
                        args: vec![
                            Operand::Value(iv.main.phi),
                            Operand::Value(*iv.offsets.get(copy_idx - 1)?),
                        ],
                    });
                    ctx.vec_iv = Some(cur_iv);
                }
            }
            copy_vals.push(ctx.emit(&p.expr)?);
        }
        match schedule.unroll {
            4 => {
                let [v0, v1, v2, v3] = copy_vals.as_slice() else {
                    return None;
                };
                let acc1 = acc1?;
                let sum02 = ctx.fresh();
                ctx.body_insts.push(Instruction::Intrinsic {
                    dest: Some(sum02),
                    op: width.bin_op(IrBinOp::Add)?,
                    dest_ptr: None,
                    args: vec![Operand::Value(plan.acc0.phi), Operand::Value(*v0)],
                });
                ctx.body_insts.push(Instruction::Intrinsic {
                    dest: Some(plan.acc0.next),
                    op: width.bin_op(IrBinOp::Add)?,
                    dest_ptr: None,
                    args: vec![Operand::Value(sum02), Operand::Value(*v2)],
                });
                let sum13 = ctx.fresh();
                ctx.body_insts.push(Instruction::Intrinsic {
                    dest: Some(sum13),
                    op: width.bin_op(IrBinOp::Add)?,
                    dest_ptr: None,
                    args: vec![Operand::Value(acc1.phi), Operand::Value(*v1)],
                });
                ctx.body_insts.push(Instruction::Intrinsic {
                    dest: Some(acc1.next),
                    op: width.bin_op(IrBinOp::Add)?,
                    dest_ptr: None,
                    args: vec![Operand::Value(sum13), Operand::Value(*v3)],
                });
            }
            2 => {
                let [v0, v1] = copy_vals.as_slice() else {
                    return None;
                };
                let acc1 = acc1?;
                ctx.body_insts.push(Instruction::Intrinsic {
                    dest: Some(plan.acc0.next),
                    op: width.bin_op(IrBinOp::Add)?,
                    dest_ptr: None,
                    args: vec![Operand::Value(plan.acc0.phi), Operand::Value(*v0)],
                });
                ctx.body_insts.push(Instruction::Intrinsic {
                    dest: Some(acc1.next),
                    op: width.bin_op(IrBinOp::Add)?,
                    dest_ptr: None,
                    args: vec![Operand::Value(acc1.phi), Operand::Value(*v1)],
                });
            }
            _ => return None,
        }
    } else {
        ctx.byte_disp = 0;
        let v0 = ctx.emit(&p.expr)?;
        ctx.body_insts.push(Instruction::Intrinsic {
            dest: Some(plan.acc0.next),
            op: width.bin_op(IrBinOp::Add)?,
            dest_ptr: None,
            args: vec![Operand::Value(plan.acc0.phi), Operand::Value(v0)],
        });
    }

    if schedule.multi_iv {
        let iv = iv?;
        for (phi, next) in std::iter::once((iv.main.phi, iv.main.next))
            .chain(iv.extra.iter().map(|entry| (entry.phi, entry.next)))
        {
            ctx.body_insts.push(Instruction::Intrinsic {
                dest: Some(next),
                op: width.bin_op(IrBinOp::Add)?,
                dest_ptr: None,
                args: vec![Operand::Value(phi), Operand::Value(iv.step)],
            });
        }
    } else if let Some(iv) = iv {
        ctx.body_insts.push(Instruction::Intrinsic {
            dest: Some(iv.main.next),
            op: width.bin_op(IrBinOp::Add)?,
            dest_ptr: None,
            args: vec![Operand::Value(iv.main.phi), Operand::Value(iv.step)],
        });
    }
    ctx.body_insts.push(Instruction::BinOp {
        dest: plan.byte_iv_next,
        op: IrBinOp::Add,
        lhs: Operand::Value(plan.byte_iv_phi),
        rhs: Operand::Const(IrConst::I64(schedule.step_bytes)),
        ty: IrType::U64,
    });
    Some(())
}

fn vector_phi(phi: VectorPhi, preheader: BlockId, body: BlockId) -> Instruction {
    Instruction::Phi {
        dest: phi.phi,
        ty: IrType::I32,
        incoming: vec![
            (Operand::Value(phi.init), preheader),
            (Operand::Value(phi.next), body),
        ],
    }
}

fn build_vector_header(
    schedule: &VectorSchedule,
    plan: &PreheaderPlan,
    preheader_label: BlockId,
    vec_body_label: BlockId,
) -> Vec<Instruction> {
    let mut instructions = vec![
        Instruction::Phi {
            dest: plan.byte_iv_phi,
            ty: IrType::U64,
            incoming: vec![
                (Operand::Const(IrConst::I64(0)), preheader_label),
                (Operand::Value(plan.byte_iv_next), vec_body_label),
            ],
        },
        vector_phi(plan.acc0, preheader_label, vec_body_label),
    ];
    if let Some(acc1) = plan.acc1 {
        instructions.push(vector_phi(acc1, preheader_label, vec_body_label));
    }
    if let Some(iv) = &plan.iv {
        instructions.push(vector_phi(iv.main, preheader_label, vec_body_label));
        for &entry in &iv.extra {
            instructions.push(vector_phi(entry, preheader_label, vec_body_label));
        }
    }
    instructions.push(Instruction::Cmp {
        dest: plan.vec_cmp_dest,
        op: IrCmpOp::Ult,
        lhs: Operand::Value(plan.byte_iv_phi),
        rhs: Operand::Const(IrConst::I64(schedule.total_vec_bytes)),
        ty: IrType::U64,
    });
    instructions
}

fn emit_reduce(
    ctx: &mut EmitCtx<'_>,
    p: &StridedI32ReducePattern,
    schedule: &VectorSchedule,
    plan: &PreheaderPlan,
) -> Option<(Vec<Instruction>, Value)> {
    // Sum accumulators, mask only an even-lane representation, horizontally
    // add, add `acc_init`, and hand off to the scalar header.
    let width = schedule.width;
    let mut instructions = Vec::new();
    let merged_acc = if let Some(acc1) = plan.acc1 {
        let merged = ctx.fresh();
        instructions.push(Instruction::Intrinsic {
            dest: Some(merged),
            op: width.bin_op(IrBinOp::Add)?,
            dest_ptr: None,
            args: vec![Operand::Value(plan.acc0.phi), Operand::Value(acc1.phi)],
        });
        merged
    } else {
        plan.acc0.phi
    };
    let masked_acc = match width {
        StridedWidth::X4Compact => merged_acc,
        StridedWidth::X8 => {
            let mask_tbl = ctx.fresh();
            let mask_i16: [i16; 16] = [-1, 0, 0, 0, -1, 0, 0, 0, -1, 0, 0, 0, -1, 0, 0, 0];
            let mut mask_bytes = Vec::with_capacity(32);
            for v in mask_i16 {
                for b in v.to_le_bytes() {
                    mask_bytes.push(Operand::Const(IrConst::I8(b as i8)));
                }
            }
            instructions.push(Instruction::Intrinsic {
                dest: Some(mask_tbl),
                op: IntrinsicOp::VecConstI8x32,
                dest_ptr: None,
                args: mask_bytes,
            });
            let even_mask = ctx.fresh();
            instructions.push(Instruction::Intrinsic {
                dest: Some(even_mask),
                op: IntrinsicOp::VecMaddwdI16x16,
                dest_ptr: None,
                args: vec![Operand::Value(mask_tbl), Operand::Value(plan.ones_tbl?)],
            });
            let masked = ctx.fresh();
            instructions.push(Instruction::Intrinsic {
                dest: Some(masked),
                op: width.bin_op(IrBinOp::And)?,
                dest_ptr: None,
                args: vec![Operand::Value(merged_acc), Operand::Value(even_mask)],
            });
            masked
        }
        StridedWidth::X4 => {
            let even_mask = ctx.fresh();
            instructions.push(Instruction::Intrinsic {
                dest: Some(even_mask),
                op: IntrinsicOp::VecPackI32x4,
                dest_ptr: None,
                args: vec![
                    Operand::Const(IrConst::I32(-1)),
                    Operand::Const(IrConst::I32(0)),
                    Operand::Const(IrConst::I32(-1)),
                    Operand::Const(IrConst::I32(0)),
                ],
            });
            let masked = ctx.fresh();
            instructions.push(Instruction::Intrinsic {
                dest: Some(masked),
                op: width.bin_op(IrBinOp::And)?,
                dest_ptr: None,
                args: vec![Operand::Value(merged_acc), Operand::Value(even_mask)],
            });
            masked
        }
    };
    let hsum = ctx.fresh();
    instructions.push(Instruction::Intrinsic {
        dest: Some(hsum),
        op: width.horizontal_add(),
        dest_ptr: None,
        args: vec![Operand::Value(masked_acc)],
    });
    let combined_acc = ctx.fresh();
    instructions.push(Instruction::BinOp {
        dest: combined_acc,
        op: IrBinOp::Add,
        lhs: p.acc_init.clone(),
        rhs: Operand::Value(hsum),
        ty: p.acc_ty,
    });
    Some((instructions, combined_acc))
}

/// Validate both scalar incoming edges before performing the first mutation.
/// A malformed internally supplied pattern therefore cannot leave half of the
/// scalar header retargeted.
fn retarget_scalar_header(
    func: &mut IrFunction,
    p: &StridedI32ReducePattern,
    preheader_label: BlockId,
    vec_reduce_label: BlockId,
    rem_iv_const: IrConst,
    combined_acc: Value,
) -> Option<()> {
    let header = func.blocks.get_mut(p.header_idx)?;
    let mut iv_seen = false;
    let mut acc_seen = false;
    for inst in &header.instructions {
        let Instruction::Phi { dest, incoming, .. } = inst else {
            continue;
        };
        let target = if *dest == p.iv_phi {
            &mut iv_seen
        } else if *dest == p.acc_phi {
            &mut acc_seen
        } else {
            continue;
        };
        // The IR verifier requires exactly one incoming per predecessor.
        // Retargeting a duplicated edge would otherwise hide malformed IR
        // behind two identical rewrites instead of refusing transactionally.
        let preheader_edges = incoming
            .iter()
            .filter(|(_, label)| *label == preheader_label)
            .count();
        if *target || preheader_edges != 1 {
            return None;
        }
        *target = true;
    }
    if !iv_seen || !acc_seen {
        return None;
    }

    for inst in &mut header.instructions {
        let Instruction::Phi { dest, incoming, .. } = inst else {
            continue;
        };
        let replacement = if *dest == p.iv_phi {
            Operand::Const(rem_iv_const)
        } else if *dest == p.acc_phi {
            Operand::Value(combined_acc)
        } else {
            continue;
        };
        for (value, label) in incoming {
            if *label == preheader_label {
                *value = replacement;
                *label = vec_reduce_label;
            }
        }
    }
    Some(())
}

fn transform(func: &mut IrFunction, p: &StridedI32ReducePattern) -> Option<usize> {
    let schedule = VectorSchedule::new(p)?;
    // Decline before any mutation: every rewrite below assumes this constant
    // is representable in the IV's own type.
    let rem_iv_const = resume_offset_const(p.iv_ty, schedule.total_vec_bytes)?;
    let preheader_label = func.blocks.get(p.preheader_idx)?.label;
    let scalar_header_label = func.blocks.get(p.header_idx)?.label;

    let mut next_lbl = func.next_label.max(
        func.blocks
            .iter()
            .map(|b| b.label.0)
            .max()
            .unwrap_or(0)
            .saturating_add(1),
    );
    let vec_header_label = BlockId(next_lbl);
    next_lbl += 1;
    let vec_body_label = BlockId(next_lbl);
    next_lbl += 1;
    let vec_reduce_label = BlockId(next_lbl);
    next_lbl += 1;

    let mut next_val = func.next_value_id;
    let mut plan = build_preheader(&schedule, &mut next_val)?;
    let mut ctx = EmitCtx {
        width: schedule.width,
        load_bases: &p.load_bases,
        wide_load_proven: &p.wide_load_proven,
        byte_iv: plan.byte_iv_phi,
        byte_disp: 0,
        vec_iv: plan.iv.as_ref().map(|iv| iv.main.phi),
        bcast_cache: Vec::new(),
        preheader_insts: std::mem::take(&mut plan.instructions),
        body_insts: Vec::new(),
        next_val,
    };
    emit_vector_body(&mut ctx, p, &schedule, &plan)?;
    let header_insts = build_vector_header(&schedule, &plan, preheader_label, vec_body_label);
    let (reduce_insts, combined_acc) = emit_reduce(&mut ctx, p, &schedule, &plan)?;

    // This is the commit point. Every operation above is fail-closed and uses
    // temporary buffers, so refusal leaves the scalar function byte-for-byte
    // unchanged, including its fresh-ID counters.
    retarget_scalar_header(
        func,
        p,
        preheader_label,
        vec_reduce_label,
        rem_iv_const,
        combined_acc,
    )?;
    let preheader_insts = std::mem::take(&mut ctx.preheader_insts);
    let body_insts = std::mem::take(&mut ctx.body_insts);
    let final_next_val = ctx.next_val;
    let preheader = &mut func.blocks[p.preheader_idx];
    if !preheader.instructions.is_empty()
        && preheader.source_spans.len() == preheader.instructions.len()
    {
        // Keep a nonempty parallel-span block parallel.  Empty/empty means
        // source spans were absent, not that dummy rows should be invented.
        preheader.source_spans.extend(std::iter::repeat_n(
            crate::common::source::Span::dummy(),
            preheader_insts.len(),
        ));
    }
    preheader.instructions.extend(preheader_insts);
    preheader.terminator = Terminator::Branch(vec_header_label);

    let change_count = header_insts.len() + body_insts.len() + reduce_insts.len();
    func.blocks.push(BasicBlock {
        label: vec_header_label,
        instructions: header_insts,
        terminator: Terminator::CondBranch {
            cond: Operand::Value(plan.vec_cmp_dest),
            true_label: vec_body_label,
            false_label: vec_reduce_label,
        },
        source_spans: vec![],
    });
    func.blocks.push(BasicBlock {
        label: vec_body_label,
        instructions: body_insts,
        terminator: Terminator::Branch(vec_header_label),
        source_spans: vec![],
    });
    func.blocks.push(BasicBlock {
        label: vec_reduce_label,
        instructions: reduce_insts,
        terminator: Terminator::Branch(scalar_header_label),
        source_spans: vec![],
    });
    func.next_value_id = final_next_val;
    func.next_label = next_lbl;
    Some(change_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A loop-defined pointer consumed outside the loop through a *bare*
    /// `Value` field (a `Load`'s `ptr`) escapes the loop.  The operand walk
    /// never reports those fields, and the pre-audit GEP-only special case
    /// missed this spelling, so the pattern was admitted with a live escape.
    #[test]
    fn bare_value_memory_operands_escape_the_loop() {
        fn fixture(outside_load: bool) -> (IrFunction, loop_analysis::NaturalLoop) {
            let mut func =
                IrFunction::new("escape_fixture".to_string(), IrType::Void, vec![], false);
            func.blocks.push(BasicBlock {
                label: BlockId(0),
                instructions: vec![],
                terminator: Terminator::Branch(BlockId(1)),
                source_spans: vec![],
            });
            func.blocks.push(BasicBlock {
                label: BlockId(1),
                instructions: vec![],
                terminator: Terminator::Branch(BlockId(0)),
                source_spans: vec![],
            });
            if outside_load {
                func.blocks.push(BasicBlock {
                    label: BlockId(2),
                    instructions: vec![Instruction::Load {
                        dest: Value(50),
                        ptr: Value(40),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                        volatile: false,
                    }],
                    terminator: Terminator::Return(None),
                    source_spans: vec![],
                });
            }
            let loop_info = loop_analysis::NaturalLoop {
                header: 0,
                body: [0, 1].into_iter().collect(),
            };
            (func, loop_info)
        }
        let acc = Value(41);
        let mut defs = FxHashSet::default();
        defs.insert(Value(40)); // the loop-defined pointer
        defs.insert(acc);
        let (func, loop_info) = fixture(true);
        assert!(
            loop_value_escapes(&func, &loop_info, &defs, acc),
            "outside Load with a bare loop-defined ptr must count as an escape"
        );
        let (func, loop_info) = fixture(false);
        assert!(
            !loop_value_escapes(&func, &loop_info, &defs, acc),
            "with no outside consumer nothing escapes"
        );
    }

    /// `LCCC_VEC_STRIDED_TRACE` must name a line for every refusal class,
    /// including the ones that used to leave through a bare `?` operator:
    /// run three dropping analyses in an isolated child (so the harness's
    /// capture does not swallow the eprintln stream) and require three
    /// distinct decline lines.
    #[test]
    fn decline_trace_names_every_refusal_class() {
        const NAME: &str =
            "passes::vectorize::strided_i32_reduce::tests::decline_trace_names_every_refusal_class";
        match crate::test_support::run_isolated(NAME) {
            Some(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                let mut lines: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
                for l in stderr.lines() {
                    if let Some(n) = l.strip_prefix("[STRIDED] decline at line ") {
                        lines.insert(n.trim().parse().expect("numeric decline line"));
                    }
                }
                assert!(
                    out.status.success()
                        && stdout.contains("test result: ok. 1 passed; 0 failed")
                        && lines.len() >= 3,
                    "expected a passing child with >= 3 distinct traced decline lines\n\
                     (status {}): {lines:?}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
                    out.status
                );
            }
            None => {
                set_trace_enabled(true);
                let facts = ModuleSymbolFacts::default();
                // (a) a body that is not two blocks: the very first precondition.
                let mut fa = IrFunction::new("t".to_string(), IrType::Void, vec![], false);
                fa.blocks.push(BasicBlock {
                    label: BlockId(0),
                    instructions: vec![],
                    terminator: Terminator::Return(None),
                    source_spans: vec![],
                });
                let la = loop_analysis::NaturalLoop {
                    header: 0,
                    body: [0].into_iter().collect(),
                };
                let ca = crate::ir::analysis::CfgAnalysis::build(&fa);
                assert!(analyze(&fa, &la, &ca, &facts).is_none());
                // (b) two blocks with no external preheader: the converted
                // `find_preheader` decline, silent before the conversion.
                let mut fb = IrFunction::new("t".to_string(), IrType::Void, vec![], false);
                fb.blocks.push(BasicBlock {
                    label: BlockId(0),
                    instructions: vec![],
                    terminator: Terminator::Branch(BlockId(1)),
                    source_spans: vec![],
                });
                fb.blocks.push(BasicBlock {
                    label: BlockId(1),
                    instructions: vec![],
                    terminator: Terminator::Branch(BlockId(0)),
                    source_spans: vec![],
                });
                let lb = loop_analysis::NaturalLoop {
                    header: 0,
                    body: [0, 1].into_iter().collect(),
                };
                let cb = crate::ir::analysis::CfgAnalysis::build(&fb);
                assert!(analyze(&fb, &lb, &cb, &facts).is_none());
                // (c) a proper preheader/header/body/exit shape whose header
                // carries a stray non-phi instruction: the converted
                // `_ => decline` arm, a bare `return None` before the audit.
                let mut fc = IrFunction::new("t".to_string(), IrType::Void, vec![], false);
                fc.blocks.push(BasicBlock {
                    label: BlockId(0),
                    instructions: vec![],
                    terminator: Terminator::Branch(BlockId(1)),
                    source_spans: vec![],
                });
                fc.blocks.push(BasicBlock {
                    label: BlockId(1),
                    instructions: vec![Instruction::Copy {
                        dest: Value(7),
                        src: Operand::Const(IrConst::I32(0)),
                    }],
                    terminator: Terminator::CondBranch {
                        cond: Operand::Value(Value(9)),
                        true_label: BlockId(2),
                        false_label: BlockId(3),
                    },
                    source_spans: vec![],
                });
                fc.blocks.push(BasicBlock {
                    label: BlockId(2),
                    instructions: vec![],
                    terminator: Terminator::Branch(BlockId(1)),
                    source_spans: vec![],
                });
                fc.blocks.push(BasicBlock {
                    label: BlockId(3),
                    instructions: vec![],
                    terminator: Terminator::Return(None),
                    source_spans: vec![],
                });
                let lc = loop_analysis::NaturalLoop {
                    header: 1,
                    body: [1, 2].into_iter().collect(),
                };
                let cc = crate::ir::analysis::CfgAnalysis::build(&fc);
                assert!(analyze(&fc, &lc, &cc, &facts).is_none());
            }
        }
    }

    /// Every exact load is one of the scalar program's own dword accesses,
    /// including the last group when the final gap is unmapped.
    #[test]
    fn exact_load_groups_never_add_a_scalar_iteration() {
        for scalar_trips in MIN_SCALAR_TRIPS..=200_000 {
            let x8_vecs = safe_vec_count(scalar_trips);
            for width in [StridedWidth::X4, StridedWidth::X4Compact, StridedWidth::X8] {
                let vecs = width.safe_vecs(x8_vecs);
                let lanes = width.vector_bytes() / SCALAR_STEP_BYTES;
                if vecs > 0 {
                    for lane in 0..lanes {
                        let scalar_iteration = (vecs - 1) * lanes + lane;
                        assert!(scalar_iteration < scalar_trips, "{width:?}");
                        let vector_address =
                            (vecs - 1) * width.vector_bytes() + lane * SCALAR_STEP_BYTES;
                        assert_eq!(vector_address, scalar_iteration * SCALAR_STEP_BYTES);
                    }
                }
            }
        }
    }

    #[test]
    fn wide_load_requires_a_proven_object_of_sufficient_size() {
        let facts = |sizes: &[(&str, usize)]| {
            let mut facts = ModuleSymbolFacts::default();
            for (name, size) in sizes {
                facts.identity_witnesses.insert((*name).to_string());
                facts.defined_sizes.insert((*name).to_string(), *size);
            }
            facts
        };
        let mut f = IrFunction::new("private".into(), IrType::Void, vec![], false);
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::Alloca {
                dest: Value(0),
                ty: IrType::U8,
                size: 128,
                align: 16,
                volatile: false,
                semantic_volatile: false,
            }],
            terminator: Terminator::Return(None),
            source_spans: vec![],
        });
        let empty = facts(&[]);
        assert!(wide_load_proven(&f, Value(0), 128, &empty));
        assert!(!wide_load_proven(&f, Value(0), 129, &empty));
        f.blocks[0].terminator = Terminator::Return(Some(Operand::Value(Value(0))));
        assert!(!wide_load_proven(&f, Value(0), 128, &empty));
        f.blocks[0].terminator = Terminator::Return(None);
        f.blocks[0].instructions[0] = Instruction::GlobalAddr {
            dest: Value(0),
            name: "arr".into(),
        };
        // No witness: a global the module cannot prove is declined.
        assert!(!wide_load_proven(&f, Value(0), 128, &empty));
        // A strong, locally binding definition of sufficient size is admitted.
        assert!(wide_load_proven(&f, Value(0), 128, &facts(&[("arr", 128)])));
        assert!(wide_load_proven(
            &f,
            Value(0),
            128,
            &facts(&[("arr", 4096)])
        ));
        // An undersized definition is declined.
        assert!(!wide_load_proven(
            &f,
            Value(0),
            129,
            &facts(&[("arr", 128)])
        ));
        // An alias- or label-redirected name is never a size proof.
        let mut redirected = facts(&[("arr", 4096)]);
        redirected.aliased_names.insert("arr".to_string());
        assert!(!wide_load_proven(&f, Value(0), 128, &redirected));
        // A constant GEP off the global keeps the root, but its byte offset
        // consumes part of the object extent. With 16 scalar iterations the
        // final scalar dword ends at offset 131, so a 132-byte object is a
        // valid scalar input; the 128-byte vector window ends at offset 135
        // and must be refused. The exact 136-byte boundary is admitted.
        f.blocks[0].instructions.push(Instruction::GetElementPtr {
            dest: Value(1),
            base: Value(0),
            offset: Operand::Const(IrConst::I64(8)),
            ty: IrType::U8,
        });
        assert!(wide_load_proven(
            &f,
            Value(1),
            128,
            &facts(&[("arr", 4096)])
        ));
        assert!(!wide_load_proven(
            &f,
            Value(1),
            128,
            &facts(&[("arr", 132)])
        ));
        assert!(!wide_load_proven(
            &f,
            Value(1),
            128,
            &facts(&[("arr", 135)])
        ));
        assert!(wide_load_proven(&f, Value(1), 128, &facts(&[("arr", 136)])));
        // The frontend may canonicalize the same constant displacement as a
        // pointer-typed integer add instead of a GEP; the extent is identical.
        *f.blocks[0].instructions.last_mut().unwrap() = Instruction::BinOp {
            dest: Value(1),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Const(IrConst::I64(8)),
            ty: IrType::I64,
        };
        assert!(!wide_load_proven(
            &f,
            Value(1),
            128,
            &facts(&[("arr", 135)])
        ));
        assert!(wide_load_proven(&f, Value(1), 128, &facts(&[("arr", 136)])));
        // A nonconstant offset has no statically checkable remaining extent.
        *f.blocks[0].instructions.last_mut().unwrap() = Instruction::GetElementPtr {
            dest: Value(1),
            base: Value(0),
            offset: Operand::Value(Value(2)),
            ty: IrType::U8,
        };
        assert!(!wide_load_proven(
            &f,
            Value(1),
            128,
            &facts(&[("arr", 4096)])
        ));
    }

    #[test]
    fn load_emission_selects_exact_or_proven_private_access() {
        for (width, private) in [
            (StridedWidth::X8, false),
            (StridedWidth::X8, true),
            (StridedWidth::X4, false),
            (StridedWidth::X4, true),
            (StridedWidth::X4Compact, true),
        ] {
            let mut ctx = EmitCtx {
                width,
                load_bases: &[Value(0)],
                wide_load_proven: &[private],
                byte_iv: Value(1),
                byte_disp: 32,
                vec_iv: Some(Value(2)),
                bcast_cache: vec![],
                preheader_insts: vec![],
                body_insts: vec![],
                next_val: 3,
            };
            assert!(ctx.emit(&StridedExpr::LoadEven32(0)).is_some());
            let Instruction::Intrinsic { op, args, .. } = &ctx.body_insts[0] else {
                panic!("expected vector load");
            };
            let expected = match (width, private) {
                (StridedWidth::X8, true) => Some(IntrinsicOp::VecLoadI32x8),
                (StridedWidth::X8, false) => Some(IntrinsicOp::VecLoadStrided2I32x8),
                (StridedWidth::X4, true) => Some(IntrinsicOp::VecLoadI32x4),
                (StridedWidth::X4, false) => Some(IntrinsicOp::VecLoadStrided2I32x4),
                (StridedWidth::X4Compact, true) => Some(IntrinsicOp::VecLoadStrided2CompactI32x4),
                (StridedWidth::X4Compact, false) => None,
            };
            assert_eq!(Some(*op), expected, "{width:?} private={private}");
            assert_eq!(args[2], Operand::Const(IrConst::I64(32)));
        }
    }

    #[test]
    fn unsupported_intrinsic_forms_are_refused() {
        for width in [StridedWidth::X4, StridedWidth::X4Compact, StridedWidth::X8] {
            assert!(width.bin_op(IrBinOp::SDiv).is_none(), "{width:?}");
            assert!(width.shift_op(IrBinOp::Add).is_none(), "{width:?}");
        }
        assert!(StridedWidth::X4Compact.load(false).is_none());
    }

    fn emission_test_function() -> IrFunction {
        let mut func = IrFunction::new("emit".into(), IrType::I32, vec![], false);
        func.next_value_id = 100;
        func.next_label = 3;
        func.blocks = vec![
            BasicBlock {
                label: BlockId(0),
                instructions: vec![],
                terminator: Terminator::Branch(BlockId(1)),
                source_spans: vec![],
            },
            BasicBlock {
                label: BlockId(1),
                instructions: vec![
                    Instruction::Phi {
                        dest: Value(1),
                        ty: IrType::I64,
                        incoming: vec![
                            (Operand::Const(IrConst::I64(0)), BlockId(0)),
                            (Operand::Value(Value(90)), BlockId(2)),
                        ],
                    },
                    Instruction::Phi {
                        dest: Value(2),
                        ty: IrType::I32,
                        incoming: vec![
                            (Operand::Const(IrConst::I32(0)), BlockId(0)),
                            (Operand::Value(Value(91)), BlockId(2)),
                        ],
                    },
                ],
                terminator: Terminator::Return(None),
                source_spans: vec![],
            },
            BasicBlock {
                label: BlockId(2),
                instructions: vec![],
                terminator: Terminator::Branch(BlockId(1)),
                source_spans: vec![],
            },
        ];
        func
    }

    fn emission_test_pattern(expr: StridedExpr) -> StridedI32ReducePattern {
        StridedI32ReducePattern {
            preheader_idx: 0,
            header_idx: 1,
            body_idx: 2,
            iv_phi: Value(1),
            iv_ty: IrType::I64,
            acc_phi: Value(2),
            acc_ty: IrType::I32,
            acc_init: Operand::Const(IrConst::I32(0)),
            safe_vecs: 8,
            load_bases: vec![Value(3)],
            wide_load_proven: vec![false],
            expr,
        }
    }

    /// Internal pattern corruption must not turn a formerly validated
    /// assumption into a panic or a half-rewritten function. These cases are
    /// deliberately below `analyze`: they test the emitter's own fail-closed
    /// contract on every host, independently of the admitted source syntax.
    #[test]
    fn invalid_emission_forms_are_transactional() {
        let load = || StridedExpr::LoadEven32(0);
        let one = || StridedExpr::Invariant(Operand::Const(IrConst::I32(1)));
        let cases = [
            StridedExpr::BinOp(IrBinOp::SDiv, Box::new(load()), Box::new(one())),
            StridedExpr::BinOp(IrBinOp::Shl, Box::new(load()), Box::new(load())),
            StridedExpr::LoadEven32(1),
        ];

        for expr in cases {
            let mut func = emission_test_function();
            let pattern = emission_test_pattern(expr);
            let before = func.clone();
            assert_eq!(transform(&mut func, &pattern), None);
            assert_eq!(func.next_value_id, before.next_value_id);
            assert_eq!(func.next_label, before.next_label);
            assert_eq!(format!("{:?}", func.blocks), format!("{:?}", before.blocks));
        }
    }

    #[test]
    fn successful_emission_keeps_preheader_source_spans_parallel() {
        let mut func = emission_test_function();
        func.blocks[0].instructions.push(Instruction::Copy {
            dest: Value(80),
            src: Operand::Const(IrConst::I64(0)),
        });
        func.blocks[0]
            .source_spans
            .push(crate::common::source::Span::dummy());
        let expr = StridedExpr::BinOp(
            IrBinOp::Xor,
            Box::new(StridedExpr::LoadEven32(0)),
            Box::new(StridedExpr::IvLow32),
        );
        let pattern = emission_test_pattern(expr);
        assert!(transform(&mut func, &pattern).is_some());
        assert_eq!(func.blocks.len(), 6);
        let preheader = &func.blocks[0];
        assert!(preheader.instructions.len() > 1);
        assert_eq!(preheader.source_spans.len(), preheader.instructions.len());
    }

    #[test]
    fn successful_emission_keeps_empty_spanless_preheader_spanless() {
        let mut func = emission_test_function();
        let expr = StridedExpr::BinOp(
            IrBinOp::Xor,
            Box::new(StridedExpr::LoadEven32(0)),
            Box::new(StridedExpr::IvLow32),
        );
        let pattern = emission_test_pattern(expr);
        assert!(func.blocks[0].instructions.is_empty());
        assert!(transform(&mut func, &pattern).is_some());
        let preheader = &func.blocks[0];
        assert!(!preheader.instructions.is_empty());
        assert!(preheader.source_spans.is_empty());
    }

    #[test]
    fn duplicated_scalar_header_phi_edge_is_transactional() {
        let mut func = emission_test_function();
        let Some(Instruction::Phi { incoming, .. }) = func.blocks[1].instructions.first_mut()
        else {
            panic!("scalar reduction header must start with its accumulator phi");
        };
        let duplicated = incoming[0].clone();
        incoming.push(duplicated);
        let before = format!("{func:?}");
        let expr = StridedExpr::BinOp(
            IrBinOp::Xor,
            Box::new(StridedExpr::LoadEven32(0)),
            Box::new(StridedExpr::IvLow32),
        );
        let pattern = emission_test_pattern(expr);
        assert!(transform(&mut func, &pattern).is_none());
        assert_eq!(format!("{func:?}"), before);
    }

    /// The tightened admission must never hand back a trip count whose final
    /// `iv + 8` leaves the IV type: that loop is either UB in the source or a
    /// wrap in the IR, and a trip count derived from it is a fiction.
    #[test]
    fn accepted_trip_counts_never_wrap_the_induction_variable() {
        let types = [
            (IrType::I32, i64::from(i32::MAX)),
            (IrType::U32, i64::from(u32::MAX)),
            (IrType::I64, i64::MAX),
            (IrType::U64, i64::MAX),
        ];
        for (ty, limit) in types {
            let signed = matches!(ty, IrType::I32 | IrType::I64);
            let ops = if signed {
                [IrCmpOp::Slt, IrCmpOp::Sle]
            } else {
                [IrCmpOp::Ult, IrCmpOp::Ule]
            };
            for op in ops {
                // Sweep the whole legal range densely near the top, where a
                // wrap would live, plus a coarse sweep below it.
                let mut bounds: Vec<i64> = (1i64..=1_000).map(|x| x * 4096).collect();
                bounds.extend((limit - 64)..=limit);
                for bound in bounds {
                    let Some(trips) = scalar_trip_count(op, ty, bound) else {
                        continue;
                    };
                    // Last iteration's value, then the increment the scalar
                    // loop performs after its body: it must fit the type.
                    let last_iv = (trips - 1) * SCALAR_STEP_BYTES;
                    let after = last_iv + SCALAR_STEP_BYTES;
                    assert!(
                        after <= u64::try_from(limit).unwrap(),
                        "{ty:?} {op:?} bound={bound}: last iv {last_iv} + 8 = {after} \
                         leaves the type (limit {limit})"
                    );
                    // ... and the kept iterations must not overshoot the bound
                    // in the comparison's own sense: iteration `trips - 1` was
                    // taken, so its IV satisfies the predicate.
                    let satisfies = match op {
                        IrCmpOp::Ult | IrCmpOp::Ule => last_iv <= u64::try_from(bound).unwrap(),
                        _ => last_iv as i64 <= bound,
                    };
                    assert!(
                        satisfies,
                        "{ty:?} {op:?} bound={bound}: iteration {} (iv {last_iv}) \
                         does not satisfy the exit test",
                        trips - 1
                    );
                }
            }
        }
    }

    /// Trip-count formulae, pinned case by case (the property test above
    /// covers the invariants; this pins the exact arithmetic).
    #[test]
    fn trip_counts_match_the_counted_loop_semantics() {
        // `<` takes ceil(bound / 8) iterations: iv = 0, 8, ..., < bound.
        assert_eq!(scalar_trip_count(IrCmpOp::Ult, IrType::U64, 8), Some(1));
        assert_eq!(scalar_trip_count(IrCmpOp::Ult, IrType::U64, 9), Some(2));
        assert_eq!(
            scalar_trip_count(IrCmpOp::Ult, IrType::U64, 1024),
            Some(128)
        );
        assert_eq!(
            scalar_trip_count(IrCmpOp::Slt, IrType::I64, 1024),
            Some(128)
        );
        // `<=` takes floor(bound / 8) + 1: iv = 0, 8, ..., <= bound.
        assert_eq!(scalar_trip_count(IrCmpOp::Ule, IrType::U32, 8), Some(2));
        assert_eq!(
            scalar_trip_count(IrCmpOp::Ule, IrType::U32, 1023),
            Some(128)
        );
        assert_eq!(
            scalar_trip_count(IrCmpOp::Sle, IrType::I32, 1023),
            Some(128)
        );
        // Signedness must agree with the IV type.
        assert_eq!(scalar_trip_count(IrCmpOp::Ult, IrType::I64, 1024), None);
        assert_eq!(scalar_trip_count(IrCmpOp::Slt, IrType::U64, 1024), None);
        // Non-positive bounds are not counted loops this pass models.
        assert_eq!(scalar_trip_count(IrCmpOp::Ult, IrType::U64, 0), None);
        assert_eq!(scalar_trip_count(IrCmpOp::Slt, IrType::I64, -8), None);
        // A bound outside the IV type is refused rather than reinterpreted.
        assert_eq!(
            scalar_trip_count(IrCmpOp::Ult, IrType::U32, 1i64 << 32),
            None
        );
        assert_eq!(
            scalar_trip_count(IrCmpOp::Slt, IrType::I32, 1i64 << 32),
            None
        );
        assert_eq!(scalar_trip_count(IrCmpOp::Slt, IrType::I32, -1), None);
        // A bound one past the wrap-free limit is refused; the limit itself is
        // accepted for `<=` (last iteration sits exactly at the maximum).
        assert_eq!(
            scalar_trip_count(IrCmpOp::Ule, IrType::U32, i64::from(u32::MAX) - 7),
            None
        );
        assert_eq!(
            scalar_trip_count(IrCmpOp::Ule, IrType::U32, i64::from(u32::MAX) - 8),
            Some((i64::from(u32::MAX) - 8) as u64 / 8 + 1)
        );
    }

    /// The resume offset must be an exact value of the IV type: the scalar
    /// remainder loop continues from it, so a wrapped constant would restart
    /// the loop at a negative byte offset.
    #[test]
    fn resume_offset_is_exact_in_the_induction_variable_type() {
        for ty in [IrType::U32, IrType::I32] {
            for v in [0u32, 1, 31, 0x7fff_ffff, 0x8000_0000, 0xffff_fff8] {
                let c = resume_offset_const(ty, i64::from(v)).expect("in range");
                let IrConst::I32(raw) = c else {
                    panic!("4-byte IV must resume through an I32 constant");
                };
                assert_eq!(
                    raw as u32, v,
                    "{ty:?}: bit pattern must carry the unsigned value {v}"
                );
            }
            assert_eq!(
                resume_offset_const(ty, i64::from(u32::MAX) + 1),
                None,
                "{ty:?}: a value past the type must be refused, not truncated"
            );
        }
        for ty in [IrType::U64, IrType::I64] {
            assert_eq!(
                resume_offset_const(ty, 1i64 << 40),
                Some(IrConst::I64(1i64 << 40))
            );
        }
        assert_eq!(resume_offset_const(IrType::U32, -1), None);
    }
}
