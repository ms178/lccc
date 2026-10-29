//! AVX2 strided-2 32-bit integer sum-reduction vectorizer.
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
//! count is known when the transform runs. A run-time bound -- including the
//! `(i + 8) <= len` spelling this module used to document -- is declined: the
//! guard-page contract below needs the scalar trip count `T` up front. Admitting
//! one would need the grouped-bound machinery the map and matmul arms use (a
//! `(bound - 1) >> 2` trip count and a per-vector in-range test in the
//! preheader) plus a re-proof of the dereferenceability argument against a
//! non-constant prefix; that is not implemented here.
//!
//! The corpus gate `tests/regression/check_vector_remainder_codegen.sh` pins
//! both dereferenceability paths below on
//! `tests/regression/vectorize_strided_i32_reduce.c`.
//!
//! ### Even-lane SIMD homomorphism (zero shuffles inside the loop)
//!
//! A single 256-bit `VecLoadI32x8` at `base + byte_iv` places four consecutive
//! stride-8B dwords (`+0, +8, +16, +24`) in the **even** 32-bit lanes `0, 2,
//! 4, 6` of a YMM register while lanes `1, 3, 5, 7` hold the intervening odd
//! dwords (`+4, +12, +20, +28`). Because modular 32-bit lane operations
//! (`vpaddd`, `vpsubd`, `vpmulld`, `vpand`, `vpor`, `vpxor`, `vpslld`,
//! `vpsrld`, `vpsrad`) never propagate carries or traps across lane
//! boundaries, evaluating the entire per-element DAG across all 8 lanes in
//! parallel computes the exact scalar values in lanes `0, 2, 4, 6` with zero
//! in-loop shuffles or permutes. Once the vector loop exits, masking the vector
//! accumulator with `0x0000_0000_FFFF_FFFF` (`VecBroadcastI64x4`) zeroes the
//! odd lanes before `VecHorizontalAddI32x8`.
//!
//! ### Dereferenceability / guard-page contract
//!
//! Iteration `t` (`0 <= t < T`) of the scalar loop dereferences bytes
//! `[t*8 .. t*8 + 3]`, proving `[0 .. (T - 1)*8 + 3]` readable on entry to the
//! last iteration. A 32-byte `VecLoadI32x8` covering scalar iterations
//! `4*v .. 4*v + 3` reads `[32*v .. 32*v + 31]`:
//!
//! * When every loaded base pointer names a locally binding `GlobalAddr` (via
//!   [`ModuleSymbolFacts::global_min_bytes`]) or stack `Alloca` of at least
//!   `T * 8` bytes, all `T / 4` vectors lie strictly within the allocated
//!   object (`safe_vecs = T / 4`).
//! * Otherwise, `32*v + 31 <= (T - 1)*8 + 3` holds unconditionally for all
//!   `v < (T - 1) / 4` (`safe_vecs = (T - 1) / 4`), so the vector prefix never
//!   reads a single byte past the scalar loop's own highest dereferenced byte;
//!   the untouched original scalar loop executes the remaining `T - 4*used`
//!   iterations.

use super::*;

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
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LCCC_VEC_STRIDED_TRACE").is_some())
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

/// Vector iterations the guard-page contract allows for `scalar_trips`.
///
/// Iteration `t` reads `[8t, 8t+3]`, so the source program itself guarantees
/// `[0, 8(T-1)+3]` is readable. A 32-byte vector covering iterations
/// `4v..4v+3` reads `[32v, 32v+31]`, and `32v+31 <= 8(T-1)+3` holds for every
/// `v < (T-1)/4`, so the vector prefix never touches a byte the scalar loop
/// would not have touched. When every base is a proven object of at least
/// `8T` bytes the tighter `T/4` is used instead.
fn safe_vec_count(scalar_trips: u64, full_cover: bool) -> u64 {
    if full_cover {
        scalar_trips / VECTORS_PER_SCALAR_ITER
    } else {
        scalar_trips.saturating_sub(1) / VECTORS_PER_SCALAR_ITER
    }
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

fn proven_object_bytes(
    func: &IrFunction,
    base: Value,
    symbol_facts: &ModuleSymbolFacts,
) -> Option<u64> {
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::GlobalAddr { dest, name } if *dest == base => {
                    return symbol_facts
                        .global_min_bytes(name)
                        .and_then(|n| u64::try_from(n).ok());
                }
                Instruction::Alloca { dest, size, .. } if *dest == base => {
                    return u64::try_from(*size).ok();
                }
                _ => {}
            }
        }
    }
    None
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
    let body_idx = *loop_info.body.iter().find(|&&b| b != header_idx)?;
    let header_label = func.blocks[header_idx].label;
    let body_label = func.blocks[body_idx].label;
    if !matches!(func.blocks[body_idx].terminator, Terminator::Branch(lbl) if lbl == header_label) {
        decline!();
    }

    // Unique external preheader jumping unconditionally to `header_idx`.
    let preheader_idx = loop_info.find_preheader(&cfg.preds)?;
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
    let exit_idx = func.blocks.iter().position(|b| b.label == false_label)?;
    if loop_info.body.contains(&exit_idx) {
        decline!();
    }

    // Header must contain only two 2-way Phis (IV and accumulator) plus the exit Cmp.
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
                let pre = incoming
                    .iter()
                    .find(|(_, l)| *l == preheader_label)?
                    .0
                    .clone();
                let back = incoming.iter().find(|(_, l)| *l == body_label)?.0.clone();
                phis.push((*dest, *ty, pre, back));
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
            _ => return None,
        }
    }
    if phis.len() != 2 {
        decline!();
    }
    let (cmp_op, Operand::Value(cmp_lhs), Operand::Const(cmp_rhs_c), cmp_ty) = exit_cmp? else {
        decline!();
    };
    let bound_raw = cmp_rhs_c.to_i64()?;
    if bound_raw <= 0 {
        decline!();
    }

    // Identify which phi is `iv_phi` (compared in `exit_cmp`, starts at 0, steps by +8).
    let iv_pos = phis.iter().position(|(d, _, _, _)| *d == cmp_lhs)?;
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
    let scalar_trips = scalar_trip_count(cmp_op, iv_ty, bound_raw)?;
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

    // Escape closure: only `acc_phi` may be observed outside `loop_info.body`.
    let mut loop_defs = FxHashSet::default();
    for &b in &loop_info.body {
        for inst in &func.blocks[b].instructions {
            if let Some(d) = inst.dest() {
                loop_defs.insert(d);
            }
        }
    }
    for (b_idx, block) in func.blocks.iter().enumerate() {
        if loop_info.body.contains(&b_idx) {
            continue;
        }
        let mut bad_escape = false;
        for inst in &block.instructions {
            crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    if loop_defs.contains(v) && *v != acc_phi {
                        bad_escape = true;
                    }
                }
            });
            if let Instruction::GetElementPtr { base, .. } = inst {
                if loop_defs.contains(base) && *base != acc_phi {
                    bad_escape = true;
                }
            }
        }
        crate::backend::liveness::for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                if loop_defs.contains(v) && *v != acc_phi {
                    bad_escape = true;
                }
            }
        });
        if bad_escape {
            decline!();
        }
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
                let base = match_base_plus_iv(func, &loop_info.body, &loop_defs, *ptr, iv_phi)?;
                let stream_idx = register_base(base, &mut load_bases)?;
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
                                    let stream_idx = register_base(base, &mut load_bases)?;
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
            _ => return None,
        }
    }
    if consumed_u8_loads.len() != u8_loads.len() || acc_uses_in_body != 1 || load_bases.is_empty() {
        decline!();
    }
    let contrib_op = contrib_op?;
    let expr = parse_expr(
        func,
        &loop_info.body,
        iv_phi,
        acc_phi,
        &load_map,
        &contrib_op,
        0,
    )?;
    if !expr_has_load(&expr) {
        decline!();
    }

    let needed_bytes = scalar_trips.checked_mul(SCALAR_STEP_BYTES)?;
    let all_bases_cover_full = load_bases.iter().all(|&base| {
        proven_object_bytes(func, base, symbol_facts).is_some_and(|sz| sz >= needed_bytes)
    });
    let safe_vecs = safe_vec_count(scalar_trips, all_bases_cover_full);
    if safe_vecs < 4 {
        decline!();
    }

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
        expr,
    })
}

struct EmitCtx<'a> {
    load_bases: &'a [Value],
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

    fn emit(&mut self, expr: &StridedExpr) -> Value {
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
                    op: IntrinsicOp::VecLoadI32x8,
                    dest_ptr: None,
                    args,
                });
                dest
            }
            StridedExpr::IvLow32 => self.vec_iv,
            StridedExpr::Invariant(op) => {
                if let Some(&(_, cached)) = self.bcast_cache.iter().find(|(k, _)| k == op) {
                    return cached;
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
                    op: IntrinsicOp::VecBroadcastI32x8,
                    dest_ptr: None,
                    args: vec![bcast_src],
                });
                self.bcast_cache.push((op.clone(), dest));
                dest
            }
            StridedExpr::BinOp(op, l, r) => {
                let lv = self.emit(l);
                if matches!(op, IrBinOp::Shl | IrBinOp::LShr | IrBinOp::AShr) {
                    let StridedExpr::Invariant(sh_op) = r.as_ref() else {
                        unreachable!("validated constant shift")
                    };
                    let vec_op = match op {
                        IrBinOp::Shl => IntrinsicOp::VecShlI32x8,
                        IrBinOp::LShr => IntrinsicOp::VecLShrI32x8,
                        IrBinOp::AShr => IntrinsicOp::VecAShrI32x8,
                        _ => unreachable!(),
                    };
                    let dest = self.fresh();
                    self.body_insts.push(Instruction::Intrinsic {
                        dest: Some(dest),
                        op: vec_op,
                        dest_ptr: None,
                        args: vec![Operand::Value(lv), sh_op.clone()],
                    });
                    return dest;
                }
                let rv = self.emit(r);
                let vec_op = match op {
                    IrBinOp::Add => IntrinsicOp::VecAddI32x8,
                    IrBinOp::Sub => IntrinsicOp::VecSubI32x8,
                    IrBinOp::Mul => IntrinsicOp::VecMulI32x8,
                    IrBinOp::And => IntrinsicOp::VecAndI32x8,
                    IrBinOp::Or => IntrinsicOp::VecOrI32x8,
                    IrBinOp::Xor => IntrinsicOp::VecXorI32x8,
                    _ => unreachable!(),
                };
                let dest = self.fresh();
                self.body_insts.push(Instruction::Intrinsic {
                    dest: Some(dest),
                    op: vec_op,
                    dest_ptr: None,
                    args: vec![Operand::Value(lv), Operand::Value(rv)],
                });
                dest
            }
        }
    }
}

fn transform(func: &mut IrFunction, p: &StridedI32ReducePattern) -> usize {
    // Two independent accumulator chains when there are enough vectors to
    // keep both `vpaddd` pipelines busy.
    let unroll: u64 = if p.safe_vecs >= 8 { 4 } else { 1 };
    let vec_turns = p.safe_vecs / unroll;
    if vec_turns == 0 {
        return 0;
    }
    let step_bytes = (unroll * VECTOR_BYTES) as i64;
    let total_vec_bytes = (vec_turns * unroll * VECTOR_BYTES) as i64;
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

    let acc0_init = fresh();
    preheader_insts.push(Instruction::Intrinsic {
        dest: Some(acc0_init),
        op: IntrinsicOp::VecZeroI32x8,
        dest_ptr: None,
        args: vec![],
    });
    let acc0_phi = fresh();
    let acc0_next = fresh();

    let (acc1_init, acc1_phi, acc1_next) = if unroll == 4 {
        let init = fresh();
        preheader_insts.push(Instruction::Intrinsic {
            dest: Some(init),
            op: IntrinsicOp::VecZeroI32x8,
            dest_ptr: None,
            args: vec![],
        });
        (Some(init), Some(fresh()), Some(fresh()))
    } else {
        (None, None, None)
    };

    let ones_tbl = {
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
        preheader_insts.push(Instruction::Intrinsic {
            dest: Some(tbl),
            op: IntrinsicOp::VecConstI8x32,
            dest_ptr: None,
            args: bytes,
        });
        tbl
    };

    let (iv_vec_init, iv_vec_phi, iv_vec_next, iv_step_vec, iv_off_vecs) = if needs_iv_vec {
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
        preheader_insts.push(Instruction::Intrinsic {
            dest: Some(raw_tbl),
            op: IntrinsicOp::VecConstI8x32,
            dest_ptr: None,
            args: init_bytes,
        });
        let init = fresh();
        preheader_insts.push(Instruction::Intrinsic {
            dest: Some(init),
            op: IntrinsicOp::VecMaddwdI16x16,
            dest_ptr: None,
            args: vec![Operand::Value(raw_tbl), Operand::Value(ones_tbl)],
        });
        let step_v = fresh();
        preheader_insts.push(Instruction::Intrinsic {
            dest: Some(step_v),
            op: IntrinsicOp::VecBroadcastI32x8,
            dest_ptr: None,
            args: vec![Operand::Const(IrConst::I32(step_bytes as i32))],
        });
        let mut offs = Vec::new();
        if unroll == 4 {
            for copy_idx in 1..4i32 {
                let ov = fresh();
                preheader_insts.push(Instruction::Intrinsic {
                    dest: Some(ov),
                    op: IntrinsicOp::VecBroadcastI32x8,
                    dest_ptr: None,
                    args: vec![Operand::Const(IrConst::I32(copy_idx * 32))],
                });
                offs.push(ov);
            }
        }
        (Some(init), Some(fresh()), Some(fresh()), Some(step_v), offs)
    } else {
        (None, None, None, None, Vec::new())
    };

    let byte_iv_phi = fresh();
    let byte_iv_next = fresh();
    let vec_cmp_dest = fresh();

    let mut ctx = EmitCtx {
        load_bases: &p.load_bases,
        byte_iv: byte_iv_phi,
        byte_disp: 0,
        vec_iv: iv_vec_phi.unwrap_or(Value(0)),
        bcast_cache: Vec::new(),
        preheader_insts,
        body_insts: Vec::new(),
        next_val,
    };

    if unroll == 4 {
        let mut copy_vals = Vec::with_capacity(4);
        for copy_idx in 0..4usize {
            ctx.byte_disp = (copy_idx as i64) * 32;
            if needs_iv_vec {
                if copy_idx == 0 {
                    ctx.vec_iv = iv_vec_phi.unwrap();
                } else {
                    let cur_iv = ctx.fresh();
                    ctx.body_insts.push(Instruction::Intrinsic {
                        dest: Some(cur_iv),
                        op: IntrinsicOp::VecAddI32x8,
                        dest_ptr: None,
                        args: vec![
                            Operand::Value(iv_vec_phi.unwrap()),
                            Operand::Value(iv_off_vecs[copy_idx - 1]),
                        ],
                    });
                    ctx.vec_iv = cur_iv;
                }
            }
            copy_vals.push(ctx.emit(&p.expr));
        }
        let sum02 = ctx.fresh();
        ctx.body_insts.push(Instruction::Intrinsic {
            dest: Some(sum02),
            op: IntrinsicOp::VecAddI32x8,
            dest_ptr: None,
            args: vec![Operand::Value(acc0_phi), Operand::Value(copy_vals[0])],
        });
        ctx.body_insts.push(Instruction::Intrinsic {
            dest: Some(acc0_next),
            op: IntrinsicOp::VecAddI32x8,
            dest_ptr: None,
            args: vec![Operand::Value(sum02), Operand::Value(copy_vals[2])],
        });
        let sum13 = ctx.fresh();
        ctx.body_insts.push(Instruction::Intrinsic {
            dest: Some(sum13),
            op: IntrinsicOp::VecAddI32x8,
            dest_ptr: None,
            args: vec![
                Operand::Value(acc1_phi.unwrap()),
                Operand::Value(copy_vals[1]),
            ],
        });
        ctx.body_insts.push(Instruction::Intrinsic {
            dest: Some(acc1_next.unwrap()),
            op: IntrinsicOp::VecAddI32x8,
            dest_ptr: None,
            args: vec![Operand::Value(sum13), Operand::Value(copy_vals[3])],
        });
    } else {
        ctx.byte_disp = 0;
        let v0 = ctx.emit(&p.expr);
        ctx.body_insts.push(Instruction::Intrinsic {
            dest: Some(acc0_next),
            op: IntrinsicOp::VecAddI32x8,
            dest_ptr: None,
            args: vec![Operand::Value(acc0_phi), Operand::Value(v0)],
        });
    }

    if let (Some(iv_phi_v), Some(iv_next_v), Some(step_v)) = (iv_vec_phi, iv_vec_next, iv_step_vec)
    {
        ctx.body_insts.push(Instruction::Intrinsic {
            dest: Some(iv_next_v),
            op: IntrinsicOp::VecAddI32x8,
            dest_ptr: None,
            args: vec![Operand::Value(iv_phi_v), Operand::Value(step_v)],
        });
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
            dest: acc0_phi,
            ty: IrType::I32,
            incoming: vec![
                (Operand::Value(acc0_init), preheader_label),
                (Operand::Value(acc0_next), vec_body_label),
            ],
        },
    ];
    if let (Some(init), Some(phi), Some(next)) = (acc1_init, acc1_phi, acc1_next) {
        header_insts.push(Instruction::Phi {
            dest: phi,
            ty: IrType::I32,
            incoming: vec![
                (Operand::Value(init), preheader_label),
                (Operand::Value(next), vec_body_label),
            ],
        });
    }
    if let (Some(init), Some(phi), Some(next)) = (iv_vec_init, iv_vec_phi, iv_vec_next) {
        header_insts.push(Instruction::Phi {
            dest: phi,
            ty: IrType::I32,
            incoming: vec![
                (Operand::Value(init), preheader_label),
                (Operand::Value(next), vec_body_label),
            ],
        });
    }
    header_insts.push(Instruction::Cmp {
        dest: vec_cmp_dest,
        op: IrCmpOp::Ult,
        lhs: Operand::Value(byte_iv_phi),
        rhs: Operand::Const(IrConst::I64(total_vec_bytes)),
        ty: IrType::U64,
    });

    // Reduce block: sum accumulators, mask even lanes 0,2,4,6, horizontal add,
    // add `acc_init`, and hand off to the scalar header at `total_vec_bytes`.
    let mut reduce_insts = Vec::new();
    let merged_acc = if let Some(acc1_v) = acc1_phi {
        let m = ctx.fresh();
        reduce_insts.push(Instruction::Intrinsic {
            dest: Some(m),
            op: IntrinsicOp::VecAddI32x8,
            dest_ptr: None,
            args: vec![Operand::Value(acc0_phi), Operand::Value(acc1_v)],
        });
        m
    } else {
        acc0_phi
    };
    let mask_tbl = ctx.fresh();
    let mask_i16: [i16; 16] = [-1, 0, 0, 0, -1, 0, 0, 0, -1, 0, 0, 0, -1, 0, 0, 0];
    let mut mask_bytes = Vec::with_capacity(32);
    for v in mask_i16 {
        for b in v.to_le_bytes() {
            mask_bytes.push(Operand::Const(IrConst::I8(b as i8)));
        }
    }
    reduce_insts.push(Instruction::Intrinsic {
        dest: Some(mask_tbl),
        op: IntrinsicOp::VecConstI8x32,
        dest_ptr: None,
        args: mask_bytes,
    });
    let even_mask = ctx.fresh();
    reduce_insts.push(Instruction::Intrinsic {
        dest: Some(even_mask),
        op: IntrinsicOp::VecMaddwdI16x16,
        dest_ptr: None,
        args: vec![Operand::Value(mask_tbl), Operand::Value(ones_tbl)],
    });
    let masked_acc = ctx.fresh();
    reduce_insts.push(Instruction::Intrinsic {
        dest: Some(masked_acc),
        op: IntrinsicOp::VecAndI32x8,
        dest_ptr: None,
        args: vec![Operand::Value(merged_acc), Operand::Value(even_mask)],
    });
    let hsum = ctx.fresh();
    reduce_insts.push(Instruction::Intrinsic {
        dest: Some(hsum),
        op: IntrinsicOp::VecHorizontalAddI32x8,
        dest_ptr: None,
        args: vec![Operand::Value(masked_acc)],
    });
    let combined_acc = ctx.fresh();
    reduce_insts.push(Instruction::BinOp {
        dest: combined_acc,
        op: IrBinOp::Add,
        lhs: p.acc_init.clone(),
        rhs: Operand::Value(hsum),
        ty: p.acc_ty,
    });

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

    /// The whole transform rests on one arithmetic fact: every vector the pass
    /// emits stays inside the byte footprint the scalar loop itself
    /// dereferences. Check it for both admission arms over a dense sweep, using
    /// the production bound function (not a copy of its formula).
    #[test]
    fn guard_page_safe_vec_count_never_overreads_scalar_footprint() {
        for scalar_trips in MIN_SCALAR_TRIPS..=200_000 {
            let max_scalar_byte = (scalar_trips - 1) * SCALAR_STEP_BYTES + 3;
            // Unproven objects: the vector prefix must stay inside the bytes the
            // scalar loop itself dereferences, so a guard page or an
            // exactly-sized heap block cannot be crossed.
            let vecs = safe_vec_count(scalar_trips, false);
            if vecs > 0 {
                let max_vec_byte = (vecs - 1) * VECTOR_BYTES + (VECTOR_BYTES - 1);
                assert!(
                    max_vec_byte <= max_scalar_byte,
                    "trips={scalar_trips}: vector reads {max_vec_byte} > scalar \
                     footprint {max_scalar_byte}"
                );
            }
            // Proven objects of >= 8T bytes: the stronger bound may use the
            // whole object, which is exactly `8T` bytes.
            let vecs = safe_vec_count(scalar_trips, true);
            if vecs > 0 {
                let max_vec_byte = (vecs - 1) * VECTOR_BYTES + (VECTOR_BYTES - 1);
                assert!(
                    max_vec_byte < scalar_trips * SCALAR_STEP_BYTES,
                    "trips={scalar_trips} full-cover: vector reads {max_vec_byte} \
                     past the proven object of {} bytes",
                    scalar_trips * SCALAR_STEP_BYTES
                );
            }
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
