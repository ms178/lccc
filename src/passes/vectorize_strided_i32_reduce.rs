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
    transform(func, &pattern)
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
            if let Instruction::GetElementPtr { base, .. } = inst {
                if loop_defs.contains(base) && *base != acc {
                    bad = true;
                }
            }
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

    /// Load for one stream.  `contiguous` is the reducer's per-stream proof:
    /// with it, [`Self::X4`] may read all four lanes of its 16-byte window
    /// (the two gap dwords are computed speculatively and masked off before
    /// the horizontal add) and [`Self::X4Compact`] may merge two proven
    /// halves; without it both stay on the exact-access forms, which never
    /// touch a gap.
    ///
    /// `None` is the combination the schedule cannot produce — the compact
    /// form is *selected* only when every stream is proven, because it has no
    /// gap-free two-dword lowering.  Returning it keeps that a decline rather
    /// than an abort, and `debug_assert!` keeps the invariant loud in tests.
    fn load(self, contiguous: bool) -> Option<IntrinsicOp> {
        let intrinsic = match (self, contiguous) {
            (Self::X4, true) => IntrinsicOp::VecLoadI32x4,
            (Self::X4, false) => IntrinsicOp::VecLoadStrided2I32x4,
            (Self::X4Compact, true) => IntrinsicOp::VecLoadStrided2CompactI32x4,
            (Self::X4Compact, false) => {
                debug_assert!(
                    false,
                    "compact strided load requires the contiguous-window proof"
                );
                return None;
            }
            (Self::X8, true) => IntrinsicOp::VecLoadI32x8,
            (Self::X8, false) => IntrinsicOp::VecLoadStrided2I32x8,
        };
        Some(intrinsic)
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

    /// Lane operation for `op`, or `None` when this width has no lowering.
    ///
    /// Today `None` is unreachable: the expression analyser builds only the
    /// operations the reducers below can emit.  Spelling the failure as a
    /// value rather than an abort keeps a future widening of that set a
    /// *missed optimisation* instead of a compiler crash, and the caller's
    /// contract is exactly "decline the whole loop" — `transform` performs no
    /// mutation before its last block, so a decline leaves the scalar loop
    /// untouched.  `debug_assert!` keeps the invariant visible in test builds
    /// without turning it into a release-mode abort.
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
            _ => {
                debug_assert!(
                    false,
                    "unvalidated strided lane operation {op:?} for {self:?}"
                );
                return None;
            }
        };
        Some(intrinsic)
    }

    /// Shift operation for `op`; `None` mirrors [`Self::bin_op`]'s contract.
    /// The analyser guarantees a *constant* shift count (`StridedExpr::Invariant`
    /// as the right operand), which is why the vector form takes the count as
    /// an immediate operand instead of a lane.
    fn shift_op(self, op: IrBinOp) -> Option<IntrinsicOp> {
        let intrinsic = match (self, op) {
            (Self::X4 | Self::X4Compact, IrBinOp::Shl) => IntrinsicOp::VecShlI32x4,
            (Self::X4 | Self::X4Compact, IrBinOp::LShr) => IntrinsicOp::VecLShrI32x4,
            (Self::X4 | Self::X4Compact, IrBinOp::AShr) => IntrinsicOp::VecAShrI32x4,
            (Self::X8, IrBinOp::Shl) => IntrinsicOp::VecShlI32x8,
            (Self::X8, IrBinOp::LShr) => IntrinsicOp::VecLShrI32x8,
            (Self::X8, IrBinOp::AShr) => IntrinsicOp::VecAShrI32x8,
            _ => {
                debug_assert!(false, "unvalidated strided shift {op:?} for {self:?}");
                return None;
            }
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
    vec_iv: Value,
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

    /// Emit one lane-DAG node, or `None` when the DAG contains something this
    /// width cannot lower.
    ///
    /// Every failure is a value, never an abort: a `None` propagates to
    /// `transform`, which declines the loop *before* its first mutation.  The
    /// only shape that can produce it today is a shift whose right operand is
    /// not a proven invariant, which the analyser also refuses — the check is
    /// kept because it is what makes the args below well-typed (a lane-valued
    /// shift count is not expressible in the vector form).
    fn emit(&mut self, expr: &StridedExpr) -> Option<Value> {
        match expr {
            StridedExpr::LoadEven32(idx) => {
                let dest = self.fresh();
                let mut args = vec![
                    Operand::Value(self.load_bases[*idx]),
                    Operand::Value(self.byte_iv),
                ];
                if self.byte_disp != 0 {
                    args.push(Operand::Const(IrConst::I64(self.byte_disp)));
                }
                self.body_insts.push(Instruction::Intrinsic {
                    dest: Some(dest),
                    op: self.width.load(self.wide_load_proven[*idx])?,
                    dest_ptr: None,
                    args,
                });
                Some(dest)
            }
            StridedExpr::IvLow32 => Some(self.vec_iv),
            StridedExpr::Invariant(op) => {
                if let Some(&(_, cached)) = self.bcast_cache.iter().find(|(k, _)| k == op) {
                    return Some(cached);
                }
                let bcast_src = match op {
                    Operand::Const(c) => {
                        Operand::Const(IrConst::I32(c.to_i64().unwrap_or(0) as i32))
                    }
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
                        debug_assert!(false, "lane-valued strided shift count reached emission");
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

/// One accumulator chain. `init`, `phi` and `next` exist together or not at
/// all, so the emitter never re-establishes an invariant the schedule already
/// decided — and never has to `unwrap` it.
#[derive(Clone, Copy)]
struct AccChain {
    init: Value,
    phi: Value,
    next: Value,
}

/// The vector induction stream, present exactly when the body reads the IV.
struct IvStream {
    init: Value,
    phi: Value,
    next: Value,
    /// Broadcast `step_bytes`: the single-stream update's operand.
    step: Value,
    /// Per-copy broadcast offsets when every copy shares one IV stream.
    offs: Vec<Value>,
    /// Per-copy chains when each copy carries its own adjacent IV vector
    /// (`multi_iv`): `extra[i]` belongs to copy `i + 1`.
    extra: Vec<AccChain>,
}

/// Everything the preheader seeds, grouped by the schedule that owns it.
///
/// The two `Option`s are decided *here*, once, from `width`/`unroll`/
/// `needs_iv_vec`; that is what removes them — and every `unwrap` of them —
/// from the body emitter downstream.
struct Seeds {
    acc0: AccChain,
    acc1: Option<AccChain>,
    /// `VecConstI8x32([1, 0, …])`: the `vpmaddwd` partner that sign-extends
    /// the packed i16 tables into i32 lanes. YMM (`X8`) only — the XMM
    /// schedules build their seeds with `VecPackI32x4` and need no table.
    ones: Option<Value>,
    iv: Option<IvStream>,
}

/// Append the preheader seed instructions and return the values they define.
///
/// `None` means the schedule asked for a seed its width cannot build. No such
/// combination exists today (`X8` is the only width that creates `ones`, and
/// it is also the only one that consumes it), so the two sites that rely on
/// that implication assert it, and the caller's contract is "decline the
/// loop" — nothing has been mutated yet.
///
/// Value-numbering order is part of this pass's output (ids feed the register
/// allocator's ordering), so it is preserved exactly: accumulators, then the
/// YMM table, then the IV stream's init/step/offsets and finally its
/// phi/next pair.
fn seed_preheader(
    width: StridedWidth,
    unroll: u64,
    needs_iv_vec: bool,
    multi_iv: bool,
    vector_bytes: u64,
    step_bytes: i64,
    fresh: &mut impl FnMut() -> Value,
    out: &mut Vec<Instruction>,
) -> Option<Seeds> {
    let acc0 = AccChain {
        init: fresh(),
        phi: fresh(),
        next: fresh(),
    };
    out.push(Instruction::Intrinsic {
        dest: Some(acc0.init),
        op: width.zero(),
        dest_ptr: None,
        args: vec![],
    });

    let acc1 = if unroll > 1 {
        let chain = AccChain {
            init: fresh(),
            phi: fresh(),
            next: fresh(),
        };
        out.push(Instruction::Intrinsic {
            dest: Some(chain.init),
            op: width.zero(),
            dest_ptr: None,
            args: vec![],
        });
        Some(chain)
    } else {
        None
    };

    let ones = if width == StridedWidth::X8 {
        let tbl = fresh();
        // 16 i16 lanes `[1, 0, 1, 0, …]` so `vpmaddwd(tbl, ones)` sign-extends
        // the 8 even i16 lanes of `tbl` into i32x8, reduction web class 5,
        // with no shuffle.
        let mut bytes = Vec::with_capacity(32);
        for i in 0..16usize {
            let v: i16 = if i % 2 == 0 { 1 } else { 0 };
            for b in v.to_le_bytes() {
                bytes.push(Operand::Const(IrConst::I8(b as i8)));
            }
        }
        out.push(Instruction::Intrinsic {
            dest: Some(tbl),
            op: IntrinsicOp::VecConstI8x32,
            dest_ptr: None,
            args: bytes,
        });
        Some(tbl)
    } else {
        None
    };

    let iv = if needs_iv_vec {
        let init = if width == StridedWidth::X8 {
            let raw_tbl = fresh();
            // 16 i16 lanes `[0,0,0,0, 8,0,0,0, 16,0,0,0, 24,0,0,0]`, fed
            // through the same `vpmaddwd` partner: i32x8 `[0,0,8,0,16,0,24,0]`
            // in reduction web class 5, so `phi` stays register-homed in a YMM
            // across the loop instead of round-tripping through a stack slot.
            let lane_i16: [i16; 16] = [0, 0, 0, 0, 8, 0, 0, 0, 16, 0, 0, 0, 24, 0, 0, 0];
            let mut init_bytes = Vec::with_capacity(32);
            for v in lane_i16 {
                for b in v.to_le_bytes() {
                    init_bytes.push(Operand::Const(IrConst::I8(b as i8)));
                }
            }
            out.push(Instruction::Intrinsic {
                dest: Some(raw_tbl),
                op: IntrinsicOp::VecConstI8x32,
                dest_ptr: None,
                args: init_bytes,
            });
            let Some(ones_tbl) = ones else {
                debug_assert!(false, "X8 induction seed without its i16 ones table");
                return None;
            };
            let widened = fresh();
            out.push(Instruction::Intrinsic {
                dest: Some(widened),
                op: IntrinsicOp::VecMaddwdI16x16,
                dest_ptr: None,
                args: vec![Operand::Value(raw_tbl), Operand::Value(ones_tbl)],
            });
            widened
        } else {
            let init = fresh();
            // X4Compact: four real strided elements `[0, 8, 16, 24]`.
            // X4: the exact two-dword form, so the odd lanes start at zero.
            let lanes: [i32; 4] = if width == StridedWidth::X4Compact {
                [0, 8, 16, 24]
            } else {
                [0, 0, 8, 0]
            };
            out.push(Instruction::Intrinsic {
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
        out.push(Instruction::Intrinsic {
            dest: Some(step),
            op: width.broadcast(),
            dest_ptr: None,
            args: vec![Operand::Const(IrConst::I32(step_bytes as i32))],
        });
        let mut offs = Vec::new();
        let mut extra = Vec::new();
        if unroll > 1 {
            for copy_idx in 1..unroll as i32 {
                let byte_offset = copy_idx * vector_bytes as i32;
                if multi_iv {
                    // Only the compact XMM pair carries two strided streams, so
                    // this packer's four i32 lanes are the complete vector.  Any
                    // wider width has to go through `width.broadcast()` below.
                    debug_assert!(
                        width == StridedWidth::X4Compact,
                        "multi-IV offset chains are an X4Compact-only shape"
                    );
                    let chain_init = fresh();
                    out.push(Instruction::Intrinsic {
                        dest: Some(chain_init),
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
                    extra.push(AccChain {
                        init: chain_init,
                        phi: fresh(),
                        next: fresh(),
                    });
                } else {
                    let ov = fresh();
                    out.push(Instruction::Intrinsic {
                        dest: Some(ov),
                        op: width.broadcast(),
                        dest_ptr: None,
                        args: vec![Operand::Const(IrConst::I32(byte_offset))],
                    });
                    offs.push(ov);
                }
            }
        }
        let phi = fresh();
        let next = fresh();
        Some(IvStream {
            init,
            phi,
            next,
            step,
            offs,
            extra,
        })
    } else {
        None
    };

    Some(Seeds {
        acc0,
        acc1,
        ones,
        iv,
    })
}

/// Reduce-block instructions: merge the accumulator chains, kill the odd lanes
/// that only a gap-carrying representation has, horizontally add, then fold in
/// the scalar seed.
///
/// `None` mirrors [`seed_preheader`]'s contract: a width asked for a mask seed
/// it does not own. Every mask is derived from constants this function builds
/// itself, so the reduce block never depends on a value the vector loop
/// carries.
fn emit_reduce_epilogue(
    ctx: &mut EmitCtx<'_>,
    width: StridedWidth,
    acc0_phi: Value,
    acc1_phi: Option<Value>,
    ones: Option<Value>,
    acc_init: &Operand,
    acc_ty: IrType,
    out: &mut Vec<Instruction>,
) -> Option<Value> {
    let merged_acc = if let Some(acc1_v) = acc1_phi {
        let Some(op) = width.bin_op(IrBinOp::Add) else {
            return None;
        };
        let m = ctx.fresh();
        out.push(Instruction::Intrinsic {
            dest: Some(m),
            op,
            dest_ptr: None,
            args: vec![Operand::Value(acc0_phi), Operand::Value(acc1_v)],
        });
        m
    } else {
        acc0_phi
    };

    let masked_acc = if width == StridedWidth::X4Compact {
        // All four lanes are real strided elements; no odd-lane mask exists.
        merged_acc
    } else if width == StridedWidth::X8 {
        let mask_tbl = ctx.fresh();
        let mask_i16: [i16; 16] = [-1, 0, 0, 0, -1, 0, 0, 0, -1, 0, 0, 0, -1, 0, 0, 0];
        let mut mask_bytes = Vec::with_capacity(32);
        for v in mask_i16 {
            for b in v.to_le_bytes() {
                mask_bytes.push(Operand::Const(IrConst::I8(b as i8)));
            }
        }
        out.push(Instruction::Intrinsic {
            dest: Some(mask_tbl),
            op: IntrinsicOp::VecConstI8x32,
            dest_ptr: None,
            args: mask_bytes,
        });
        let Some(ones_tbl) = ones else {
            debug_assert!(false, "X8 reduce mask without its i16 ones table");
            return None;
        };
        let even_mask = ctx.fresh();
        out.push(Instruction::Intrinsic {
            dest: Some(even_mask),
            op: IntrinsicOp::VecMaddwdI16x16,
            dest_ptr: None,
            args: vec![Operand::Value(mask_tbl), Operand::Value(ones_tbl)],
        });
        let Some(op) = width.bin_op(IrBinOp::And) else {
            return None;
        };
        let masked_acc = ctx.fresh();
        out.push(Instruction::Intrinsic {
            dest: Some(masked_acc),
            op,
            dest_ptr: None,
            args: vec![Operand::Value(merged_acc), Operand::Value(even_mask)],
        });
        masked_acc
    } else {
        let even_mask = ctx.fresh();
        out.push(Instruction::Intrinsic {
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
        let Some(op) = width.bin_op(IrBinOp::And) else {
            return None;
        };
        let masked_acc = ctx.fresh();
        out.push(Instruction::Intrinsic {
            dest: Some(masked_acc),
            op,
            dest_ptr: None,
            args: vec![Operand::Value(merged_acc), Operand::Value(even_mask)],
        });
        masked_acc
    };

    let hsum = ctx.fresh();
    out.push(Instruction::Intrinsic {
        dest: Some(hsum),
        op: width.horizontal_add(),
        dest_ptr: None,
        args: vec![Operand::Value(masked_acc)],
    });
    let combined_acc = ctx.fresh();
    out.push(Instruction::BinOp {
        dest: combined_acc,
        op: IrBinOp::Add,
        lhs: acc_init.clone(),
        rhs: Operand::Value(hsum),
        ty: acc_ty,
    });
    Some(combined_acc)
}

fn transform(func: &mut IrFunction, p: &StridedI32ReducePattern) -> usize {
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
        return 0;
    }
    let step_bytes = (unroll * vector_bytes) as i64;
    let total_vec_bytes = (vec_turns * unroll * vector_bytes) as i64;
    // Decline *before* any mutation: every rewrite below assumes this constant
    // is representable in the IV's own type.
    let Some(rem_iv_const) = resume_offset_const(p.iv_ty, total_vec_bytes) else {
        return 0;
    };
    let needs_iv_vec = {
        fn uses_iv(e: &StridedExpr) -> bool {
            match e {
                StridedExpr::IvLow32 => true,
                StridedExpr::LoadEven32(_) | StridedExpr::Invariant(_) => false,
                StridedExpr::BinOp(_, l, r) => uses_iv(l) || uses_iv(r),
            }
        }
        uses_iv(&p.expr)
    };
    // Two compact XMM streams can carry adjacent IV vectors directly. This
    // replaces the second stream's per-turn register copy + offset add with
    // one in-place step, while consuming the offset constant's old register.
    let multi_iv = width == StridedWidth::X4Compact && unroll == 2 && needs_iv_vec;

    let mut next_val = func.next_value_id;
    let mut fresh = || {
        let v = Value(next_val);
        next_val += 1;
        v
    };
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

    let preheader_label = func.blocks[p.preheader_idx].label;
    let mut preheader_insts = Vec::new();
    let Some(Seeds {
        acc0,
        acc1,
        ones,
        iv,
    }) = seed_preheader(
        width,
        unroll,
        needs_iv_vec,
        multi_iv,
        vector_bytes,
        step_bytes,
        &mut fresh,
        &mut preheader_insts,
    )
    else {
        return 0;
    };

    let byte_iv_phi = fresh();
    let byte_iv_next = fresh();
    let vec_cmp_dest = fresh();

    let mut ctx = EmitCtx {
        width,
        load_bases: &p.load_bases,
        wide_load_proven: &p.wide_load_proven,
        byte_iv: byte_iv_phi,
        byte_disp: 0,
        vec_iv: iv.as_ref().map_or(Value(0), |stream| stream.phi),
        bcast_cache: Vec::new(),
        preheader_insts,
        body_insts: Vec::new(),
        next_val,
    };

    if let Some(chain) = acc1 {
        let mut copy_vals = Vec::with_capacity(unroll as usize);
        for copy_idx in 0..unroll as usize {
            ctx.byte_disp = (copy_idx as i64) * vector_bytes as i64;
            if let Some(stream) = iv.as_ref() {
                ctx.vec_iv = if copy_idx == 0 {
                    stream.phi
                } else if multi_iv {
                    // `multi_iv` implies one extra chain per additional copy.
                    let Some(extra) = stream.extra.get(copy_idx - 1) else {
                        debug_assert!(false, "multi-IV copy {copy_idx} has no chain");
                        return 0;
                    };
                    extra.phi
                } else {
                    let Some(&offset) = stream.offs.get(copy_idx - 1) else {
                        debug_assert!(false, "copy {copy_idx} has no IV offset");
                        return 0;
                    };
                    let Some(op) = width.bin_op(IrBinOp::Add) else {
                        return 0;
                    };
                    let cur_iv = ctx.fresh();
                    ctx.body_insts.push(Instruction::Intrinsic {
                        dest: Some(cur_iv),
                        op,
                        dest_ptr: None,
                        args: vec![Operand::Value(stream.phi), Operand::Value(offset)],
                    });
                    cur_iv
                };
            }
            let Some(lane_value) = ctx.emit(&p.expr) else {
                return 0;
            };
            copy_vals.push(lane_value);
        }
        let Some(add_op) = width.bin_op(IrBinOp::Add) else {
            return 0;
        };
        if unroll == 4 {
            // IEEE-style pairwise schedule: (copy0 + copy2) and
            // (copy1 + copy3) feed the two independent accumulator chains, so
            // the per-turn dependency chain stays two adds deep.
            let [c0, c1, c2, c3] = copy_vals[..] else {
                debug_assert!(false, "unroll 4 emitted {} copies", copy_vals.len());
                return 0;
            };
            let sum02 = ctx.fresh();
            ctx.body_insts.push(Instruction::Intrinsic {
                dest: Some(sum02),
                op: add_op,
                dest_ptr: None,
                args: vec![Operand::Value(acc0.phi), Operand::Value(c0)],
            });
            ctx.body_insts.push(Instruction::Intrinsic {
                dest: Some(acc0.next),
                op: add_op,
                dest_ptr: None,
                args: vec![Operand::Value(sum02), Operand::Value(c2)],
            });
            let sum13 = ctx.fresh();
            ctx.body_insts.push(Instruction::Intrinsic {
                dest: Some(sum13),
                op: add_op,
                dest_ptr: None,
                args: vec![Operand::Value(chain.phi), Operand::Value(c1)],
            });
            ctx.body_insts.push(Instruction::Intrinsic {
                dest: Some(chain.next),
                op: add_op,
                dest_ptr: None,
                args: vec![Operand::Value(sum13), Operand::Value(c3)],
            });
        } else {
            let [c0, c1] = copy_vals[..] else {
                debug_assert!(false, "unroll 2 emitted {} copies", copy_vals.len());
                return 0;
            };
            ctx.body_insts.push(Instruction::Intrinsic {
                dest: Some(acc0.next),
                op: add_op,
                dest_ptr: None,
                args: vec![Operand::Value(acc0.phi), Operand::Value(c0)],
            });
            ctx.body_insts.push(Instruction::Intrinsic {
                dest: Some(chain.next),
                op: add_op,
                dest_ptr: None,
                args: vec![Operand::Value(chain.phi), Operand::Value(c1)],
            });
        }
    } else {
        ctx.byte_disp = 0;
        let Some(v0) = ctx.emit(&p.expr) else {
            return 0;
        };
        let Some(op) = width.bin_op(IrBinOp::Add) else {
            return 0;
        };
        ctx.body_insts.push(Instruction::Intrinsic {
            dest: Some(acc0.next),
            op,
            dest_ptr: None,
            args: vec![Operand::Value(acc0.phi), Operand::Value(v0)],
        });
    }

    if let Some(stream) = iv.as_ref() {
        let Some(op) = width.bin_op(IrBinOp::Add) else {
            return 0;
        };
        if multi_iv {
            for (phi, next) in std::iter::once((stream.phi, stream.next))
                .chain(stream.extra.iter().map(|chain| (chain.phi, chain.next)))
            {
                ctx.body_insts.push(Instruction::Intrinsic {
                    dest: Some(next),
                    op,
                    dest_ptr: None,
                    args: vec![Operand::Value(phi), Operand::Value(stream.step)],
                });
            }
        } else {
            ctx.body_insts.push(Instruction::Intrinsic {
                dest: Some(stream.next),
                op,
                dest_ptr: None,
                args: vec![Operand::Value(stream.phi), Operand::Value(stream.step)],
            });
        }
    }
    ctx.body_insts.push(Instruction::BinOp {
        dest: byte_iv_next,
        op: IrBinOp::Add,
        lhs: Operand::Value(byte_iv_phi),
        rhs: Operand::Const(IrConst::I64(step_bytes)),
        ty: IrType::U64,
    });

    let mut header_insts = vec![
        Instruction::Phi {
            dest: byte_iv_phi,
            ty: IrType::U64,
            incoming: vec![
                (Operand::Const(IrConst::I64(0)), preheader_label),
                (Operand::Value(byte_iv_next), vec_body_label),
            ],
        },
        Instruction::Phi {
            dest: acc0.phi,
            ty: IrType::I32,
            incoming: vec![
                (Operand::Value(acc0.init), preheader_label),
                (Operand::Value(acc0.next), vec_body_label),
            ],
        },
    ];
    if let Some(chain) = acc1 {
        header_insts.push(Instruction::Phi {
            dest: chain.phi,
            ty: IrType::I32,
            incoming: vec![
                (Operand::Value(chain.init), preheader_label),
                (Operand::Value(chain.next), vec_body_label),
            ],
        });
    }
    if let Some(stream) = iv.as_ref() {
        header_insts.push(Instruction::Phi {
            dest: stream.phi,
            ty: IrType::I32,
            incoming: vec![
                (Operand::Value(stream.init), preheader_label),
                (Operand::Value(stream.next), vec_body_label),
            ],
        });
        for chain in &stream.extra {
            header_insts.push(Instruction::Phi {
                dest: chain.phi,
                ty: IrType::I32,
                incoming: vec![
                    (Operand::Value(chain.init), preheader_label),
                    (Operand::Value(chain.next), vec_body_label),
                ],
            });
        }
    }
    header_insts.push(Instruction::Cmp {
        dest: vec_cmp_dest,
        op: IrCmpOp::Ult,
        lhs: Operand::Value(byte_iv_phi),
        rhs: Operand::Const(IrConst::I64(total_vec_bytes)),
        ty: IrType::U64,
    });

    // Reduce block: sum the accumulator chains, mask only an even-lane
    // representation, horizontally add, add `acc_init`, and hand off to the
    // scalar header at `total_vec_bytes`.
    let mut reduce_insts = Vec::new();
    let Some(combined_acc) = emit_reduce_epilogue(
        &mut ctx,
        width,
        acc0.phi,
        acc1.map(|chain| chain.phi),
        ones,
        &p.acc_init,
        p.acc_ty,
        &mut reduce_insts,
    ) else {
        return 0;
    };

    let scalar_header_label = func.blocks[p.header_idx].label;
    for inst in &mut func.blocks[p.header_idx].instructions {
        if let Instruction::Phi { dest, incoming, .. } = inst {
            if *dest == p.iv_phi {
                for (val, lbl) in incoming.iter_mut() {
                    if *lbl == preheader_label {
                        *val = Operand::Const(rem_iv_const);
                        *lbl = vec_reduce_label;
                    }
                }
            } else if *dest == p.acc_phi {
                for (val, lbl) in incoming.iter_mut() {
                    if *lbl == preheader_label {
                        *val = Operand::Value(combined_acc);
                        *lbl = vec_reduce_label;
                    }
                }
            }
        }
    }

    let preheader_insts = std::mem::take(&mut ctx.preheader_insts);
    let body_insts = std::mem::take(&mut ctx.body_insts);
    let final_next_val = ctx.next_val;

    func.blocks[p.preheader_idx]
        .instructions
        .extend(preheader_insts);
    func.blocks[p.preheader_idx].terminator = Terminator::Branch(vec_header_label);

    let change_count = header_insts.len() + body_insts.len() + reduce_insts.len();
    func.blocks.push(BasicBlock {
        label: vec_header_label,
        instructions: header_insts,
        terminator: Terminator::CondBranch {
            cond: Operand::Value(vec_cmp_dest),
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
    change_count
}

#[cfg(test)]
mod tests {
    use super::*;

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
                vec_iv: Value(2),
                bcast_cache: vec![],
                preheader_insts: vec![],
                body_insts: vec![],
                next_val: 3,
            };
            ctx.emit(&StridedExpr::LoadEven32(0));
            let Instruction::Intrinsic { op, args, .. } = &ctx.body_insts[0] else {
                panic!("expected vector load");
            };
            let expected = match (width, private) {
                (StridedWidth::X8, true) => IntrinsicOp::VecLoadI32x8,
                (StridedWidth::X8, false) => IntrinsicOp::VecLoadStrided2I32x8,
                (StridedWidth::X4, true) => IntrinsicOp::VecLoadI32x4,
                (StridedWidth::X4, false) => IntrinsicOp::VecLoadStrided2I32x4,
                (StridedWidth::X4Compact, true) => IntrinsicOp::VecLoadStrided2CompactI32x4,
                (StridedWidth::X4Compact, false) => unreachable!("tested above only with proof"),
            };
            assert_eq!(*op, expected, "{width:?} private={private}");
            assert_eq!(args[2], Operand::Const(IrConst::I64(32)));
        }
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
