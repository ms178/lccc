//! Algebraic simplification and strength reduction pass.
//!
//! Applies algebraic identities to simplify instructions (integer types):
//! - x + 0 => x, 0 + x => x
//! - x - 0 => x, x - x => 0
//! - x * 0 => 0, x * 1 => x
//! - x / 1 => x, x / x => 1
//! - x % 1 => 0, x % x => 0
//! - x & 0 => 0, x & x => x, x & all_ones => x
//! - x | 0 => x, x | x => x, x | all_ones => all_ones
//! - x ^ 0 => x, x ^ x => 0
//! - x << 0 => x, x >> 0 => x
//!
//! Float-unsafe simplifications (x + 0, 0 + x, x * 0, x - x, x / x) are
//! restricted to integer types to preserve IEEE 754 semantics (signed zeros,
//! NaN propagation, infinity arithmetic).
//!
//! Strength reduction (integer types only):
//! - x * 2^k => x << k  (multiply by power-of-2 to shift)
//! - x * 2 => x + x     (slightly cheaper on some uarches)
//! - x * (-1) => neg(x)  (single neg instruction vs imul)
//! - x / 2^k => x >> k  (unsigned divide by power-of-2 to logical shift)
//!
//! Constant reassociation (requires def lookup):
//! - (x + C1) + C2 => x + (C1 + C2)
//! - (x - C1) - C2 => x - (C1 + C2)
//! - (x * C1) * C2 => x * (C1 * C2)
//! - (x & C1) & C2 => x & (C1 & C2)
//! - (x | C1) | C2 => x | (C1 | C2)
//! - (x ^ C1) ^ C2 => x ^ (C1 ^ C2)
//! - (x << C1) << C2 => x << (C1 + C2)
//!
//! Negation elimination (requires def lookup):
//! - x - (neg y) => x + y
//!
//! Redundant instruction elimination:
//! - Cast where from_ty == to_ty => Copy
//! - GetElementPtr chain: GEP(GEP(base, c1), c2) => GEP(base, c1+c2)
//! - GetElementPtr with constant zero offset => Copy of base
//!
//! Cast chain optimization (requires def lookup):
//! - Cast(Cast(x, A->B), B->A) where A is unsigned and fits in B => Copy of x
//! - Cast(Cast(x, A->B), B->C) => Cast(x, A->C) (double widen/narrow)
//! - Cast of constant => constant (fold at compile time)
//!
//! Comparison simplification (requires def lookup):
//! - Cmp(Ne, cmp_result, 0) => Copy(cmp_result) -- redundant boolean test
//! - Cmp(Eq, cmp_result, 0) => Cmp(inverted_op, orig_lhs, orig_rhs) -- negated comparison
//! - Cmp(Ne, cmp_result, 1) => Cmp(inverted_op, orig_lhs, orig_rhs) -- negated boolean
//! - Cmp(Eq, cmp_result, 1) => Copy(cmp_result) -- redundant boolean test
//! - Cmp(op, x, x) => constant -- self-comparison
//! - Cmp(Ule, x, 0) => Cmp(Eq, x, 0) -- unsigned x <= 0 is x == 0
//! - Cmp(Ugt, x, 0) => Cmp(Ne, x, 0) -- unsigned x > 0 is x != 0
//!
//! Operand canonicalization (for better codegen):
//! - Cmp(op, Const, Value) => Cmp(swapped_op, Value, Const)
//! - BinOp(commutative_op, Const, Value) => BinOp(op, Value, Const)
//!
//! Select simplification:
//! - select cond, x, x => x (both arms same)
//! - select Const(0), a, b => b (constant false condition)
//! - select Const(nonzero), a, b => a (constant true condition)

use crate::common::types::IrType;
use crate::ir::instruction::Terminator;
use crate::ir::reexports::{
    Instruction, IntrinsicOp, IrBinOp, IrCmpOp, IrConst, IrFunction, IrModule, IrUnaryOp, Operand,
    Value,
};

/// Run algebraic simplification on the module.
/// Returns the number of instructions simplified.
pub fn run(module: &mut IrModule) -> usize {
    run_with_config(module, false)
}

/// `fill_use_counts` enables the reassociation single-use profitability
/// guard. It is requested by the -O1 tier, where the guard is REQUIRED for
/// correctness (va-arg-21: the inlined glibc extern-inline vprintf body kept
/// a stale register home after a multi-use reassociation re-based its def;
/// 20050124-1 on i686: the k++ arm's leal landed in %edx while the
/// phi-eliminated merge read %ecx). The -O2+ tiers keep the historical
/// always-reassociate behavior: there the reassociations also serve as
/// canonicalization for downstream vectorizer pattern matching, and blocking
/// them changes which shape reaches the vectorizer (affine_map_vectorization
/// regressed when the guard was active at -O2).
pub fn run_with_config(module: &mut IrModule, fill_use_counts: bool) -> usize {
    module.for_each_function(|f| simplify_function_with_config(f, fill_use_counts))
}

pub(crate) fn simplify_function(func: &mut IrFunction) -> usize {
    simplify_function_with_config(func, false)
}

fn simplify_function_with_config(func: &mut IrFunction, fill_use_counts: bool) -> usize {
    if super::fp_environment_blocks_reordering(func) {
        return 0;
    }
    let mut total = 0;
    // Captured before the block loops borrow the function; the iabs
    // compare folds consult it per `Cmp` site (see `SelectFoldCtx`).
    let wrap_signed_overflow = func.pass_policy.wrap_signed_overflow;

    // Build def maps for chain optimizations: Value -> defining instruction
    // We use flat Vecs indexed by Value ID for O(1) lookup.
    let max_id = func.max_value_id() as usize;
    let mut def_counts = vec![0u8; max_id + 1];
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some(d) = inst.dest() {
                let idx = d.0 as usize;
                if idx < def_counts.len() {
                    def_counts[idx] = def_counts[idx].saturating_add(1);
                }
            }
        }
    }
    let mut cast_defs: Vec<Option<CastDef>> = vec![None; max_id + 1];
    let mut gep_defs: Vec<Option<GepDef>> = vec![None; max_id + 1];
    let mut cmp_defs: Vec<Option<CmpDef>> = vec![None; max_id + 1];
    let mut select_defs: Vec<Option<SelectDef>> = vec![None; max_id + 1];
    let mut const_defs: Vec<Option<IrConst>> = vec![None; max_id + 1];
    let mut binop_defs: Vec<Option<BinOpDef>> = vec![None; max_id + 1];
    let mut neg_defs: Vec<Option<NegDef>> = vec![None; max_id + 1];
    let mut unary_defs: Vec<Option<UnaryDef>> = vec![None; max_id + 1];
    // `Not` producers, kept apart from `unary_defs` (which the clz/ctz folds
    // own) so the complement fold cannot perturb their matching.
    let mut not_defs: Vec<Option<Operand>> = vec![None; max_id + 1];
    let mut value_types: Vec<Option<IrType>> = vec![None; max_id + 1];
    // Track values known to be boolean (0 or 1). This includes Cmp results
    // and bitwise And/Or/Xor of boolean values.
    let mut is_boolean = vec![false; max_id + 1];

    // Track values produced by fabs/fabsf (call or intrinsic) so
    // `fabs(x) < 0.0` can fold to false, including through Copies.
    // gcc.c-torture 20020720-1: `if (fabs(x) < 0.0) link_error();`
    let mut is_fabs = vec![false; max_id + 1];

    // Use counts for the reassociation profitability guards. A reassociation
    // such as (x + C1) + C2 => x + (C1 + C2) only *removes work* when the
    // folded intermediate has no other consumers; with multiple uses the
    // rewrite merely re-bases the def onto a different value web without
    // deleting anything, which on i686 splits coalesced register homes
    // (gcc.c-torture 20050124-1: the k++ arm's leal landed in %edx while the
    // phi-eliminated merge expected %ecx). Count every value operand read by
    // instructions and terminators; over-counting is safe (the guard only
    // becomes more conservative).
    let mut use_counts = vec![0u32; if fill_use_counts { max_id + 1 } else { 0 }];
    {
        let bump = |op: &Operand, uc: &mut Vec<u32>| {
            if let Operand::Value(v) = op {
                let id = v.0 as usize;
                if id < uc.len() {
                    uc[id] += 1;
                }
            }
        };
        for block in &func.blocks {
            for inst in &block.instructions {
                match inst {
                    Instruction::Copy { src, .. }
                    | Instruction::Cast { src, .. }
                    | Instruction::UnaryOp { src, .. } => {
                        bump(src, &mut use_counts);
                    }
                    Instruction::Load { ptr, .. } => {
                        bump(&Operand::Value(*ptr), &mut use_counts);
                    }
                    Instruction::BinOp { lhs, rhs, .. } | Instruction::Cmp { lhs, rhs, .. } => {
                        bump(lhs, &mut use_counts);
                        bump(rhs, &mut use_counts);
                    }
                    Instruction::Store { ptr, val, .. } => {
                        bump(&Operand::Value(*ptr), &mut use_counts);
                        bump(val, &mut use_counts);
                    }
                    Instruction::Select {
                        true_val,
                        false_val,
                        cond,
                        ..
                    } => {
                        bump(true_val, &mut use_counts);
                        bump(false_val, &mut use_counts);
                        bump(cond, &mut use_counts);
                    }
                    Instruction::Phi { incoming, .. } => {
                        for (op, _) in incoming {
                            bump(op, &mut use_counts);
                        }
                    }
                    Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
                        for arg in &info.args {
                            bump(arg, &mut use_counts);
                        }
                    }
                    _ => {}
                }
            }
            match &block.terminator {
                Terminator::CondBranch { cond, .. } => bump(cond, &mut use_counts),
                Terminator::Return(Some(v)) => bump(v, &mut use_counts),
                _ => {}
            }
        }
    }

    // Collect definitions - first pass: mark Cmp results as boolean
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::BinOp { dest, ty, .. }
                | Instruction::UnaryOp { dest, ty, .. }
                | Instruction::Load { dest, ty, .. }
                | Instruction::ParamRef { dest, ty, .. } => {
                    set_def(&mut value_types, dest.0, *ty);
                }
                Instruction::Cast { dest, to_ty, .. } => {
                    set_def(&mut value_types, dest.0, *to_ty);
                }
                _ => {}
            }
            match inst {
                Instruction::Cast {
                    dest,
                    src,
                    from_ty,
                    to_ty,
                } => {
                    set_def(
                        &mut cast_defs,
                        dest.0,
                        CastDef {
                            src: *src,
                            from_ty: *from_ty,
                            to_ty: *to_ty,
                        },
                    );
                }
                Instruction::GetElementPtr {
                    dest, base, offset, ..
                } => {
                    set_def(
                        &mut gep_defs,
                        dest.0,
                        GepDef {
                            base: *base,
                            offset: *offset,
                        },
                    );
                }
                Instruction::Cmp {
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty,
                } => {
                    set_def(
                        &mut cmp_defs,
                        dest.0,
                        CmpDef {
                            op: *op,
                            lhs: *lhs,
                            rhs: *rhs,
                            ty: *ty,
                        },
                    );
                    let id = dest.0 as usize;
                    if id < is_boolean.len() {
                        is_boolean[id] = true;
                    }
                }
                Instruction::BinOp {
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty,
                } => {
                    // Track all binary definitions. Constant reassociation needs
                    // the constant-operand subset, but canonical BitTest
                    // recognition must see variable-index shifts such as
                    // `(base >> index) & 1`.
                    set_def(
                        &mut binop_defs,
                        dest.0,
                        BinOpDef {
                            op: *op,
                            lhs: *lhs,
                            rhs: *rhs,
                            ty: *ty,
                        },
                    );
                }
                Instruction::UnaryOp { dest, op, src, ty } => match op {
                    IrUnaryOp::Not => {
                        set_def(&mut not_defs, dest.0, *src);
                    }
                    IrUnaryOp::Neg => {
                        set_def(&mut neg_defs, dest.0, NegDef { src: *src });
                        if def_counts.get(dest.0 as usize).copied() == Some(1) {
                            if let Operand::Const(c) = src {
                                if let Some(folded) = super::constant_fold::eval_unaryop_const(
                                    IrUnaryOp::Neg,
                                    *c,
                                    None,
                                    *ty,
                                ) {
                                    set_def(&mut const_defs, dest.0, folded);
                                }
                            }
                        }
                    }
                    IrUnaryOp::Clz
                    | IrUnaryOp::ClzNonZero
                    | IrUnaryOp::Ctz
                    | IrUnaryOp::CtzNonZero => {
                        set_def(
                            &mut unary_defs,
                            dest.0,
                            UnaryDef {
                                op: *op,
                                src: *src,
                                ty: *ty,
                            },
                        );
                    }
                    _ => {}
                },
                Instruction::Copy {
                    dest,
                    src: Operand::Const(c),
                } => {
                    if def_counts.get(dest.0 as usize).copied() == Some(1) {
                        set_def(&mut const_defs, dest.0, *c);
                    }
                }
                Instruction::Select {
                    dest,
                    cond,
                    true_val,
                    false_val,
                    ty,
                } => {
                    if def_counts.get(dest.0 as usize).copied() == Some(1) {
                        set_def(
                            &mut select_defs,
                            dest.0,
                            SelectDef {
                                cond: *cond,
                                true_val: *true_val,
                                false_val: *false_val,
                                ty: *ty,
                            },
                        );
                    }
                }
                Instruction::Call { func, info } if info.assume_library => {
                    if matches!(
                        func.as_str(),
                        "fabs"
                            | "fabsf"
                            | "fabsl"
                            | "__builtin_fabs"
                            | "__builtin_fabsf"
                            | "__builtin_fabsl"
                    ) {
                        if let Some(d) = info.dest {
                            let id = d.0 as usize;
                            if id < is_fabs.len() {
                                is_fabs[id] = true;
                            }
                        }
                    }
                }
                Instruction::Intrinsic { dest, op, .. } => {
                    if matches!(
                        op,
                        IntrinsicOp::FabsF32
                            | IntrinsicOp::FabsF64
                            | IntrinsicOp::LDFabs
                            | IntrinsicOp::F128Fabs
                    ) {
                        if let Some(d) = dest {
                            let id = d.0 as usize;
                            if id < is_fabs.len() {
                                is_fabs[id] = true;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Exact use counts for widening-cast consumers.  The generic
    // reassociation counter above is intentionally optional at -O2+ because
    // it controls historical canonicalisation, but these two rewrites are a
    // profitability transform: require the cast to die after its only use.
    // Replacing a one-use widened carrier with its narrow source is then
    // live-range-neutral in cardinality (one live value before and after),
    // and never leaves an additional long-lived raw source alongside a still
    // needed wide result.  `for_each_used_value` covers every opaque/asm/
    // atomic operand too, so an unfamiliar instruction can only veto a fold.
    let mut cast_use_counts = vec![0u32; max_id + 1];
    for block in &func.blocks {
        for inst in &block.instructions {
            inst.for_each_used_value(|id| {
                let idx = id as usize;
                if idx < cast_use_counts.len() && cast_defs[idx].is_some() {
                    cast_use_counts[idx] = cast_use_counts[idx].saturating_add(1);
                }
            });
        }
        block.terminator.for_each_used_value(|id| {
            let idx = id as usize;
            if idx < cast_use_counts.len() && cast_defs[idx].is_some() {
                cast_use_counts[idx] = cast_use_counts[idx].saturating_add(1);
            }
        });
    }

    // Which widening casts die once their readers are narrowed: the compare
    // narrowing and the widened-condition rewrite share this one census, so
    // neither can leave the other's cast alive beside its narrow source.
    let widened_cond_live =
        !simplify_skip("widened_cond") && std::env::var_os("CCC_NO_WIDENED_COND").is_none();
    let cast_dies_if_narrowed = casts_dying_under_narrowing(func, &cast_defs, widened_cond_live);

    // Second pass: propagate boolean-ness through And/Or/Xor of boolean values.
    // A single pass is sufficient for most patterns since And/Or/Xor of Cmp results
    // are the dominant case. For deeply nested boolean expressions, this misses some
    // cases, but those are rare in practice.
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::BinOp {
                dest, op, lhs, rhs, ..
            } = inst
            {
                if matches!(op, IrBinOp::And | IrBinOp::Or | IrBinOp::Xor)
                    && operand_is_boolean(lhs, &is_boolean)
                    && operand_is_boolean(rhs, &is_boolean)
                {
                    let id = dest.0 as usize;
                    if id < is_boolean.len() {
                        is_boolean[id] = true;
                    }
                }
            }
        }
    }

    // Propagate fabs-ness through Copy (p = fabs(x); if (p < 0.0)).
    let mut fabs_changed = true;
    while fabs_changed {
        fabs_changed = false;
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Instruction::Copy {
                    dest,
                    src: Operand::Value(v),
                } = inst
                {
                    let src_id = v.0 as usize;
                    let dest_id = dest.0 as usize;
                    if src_id < is_fabs.len()
                        && dest_id < is_fabs.len()
                        && is_fabs[src_id]
                        && !is_fabs[dest_id]
                    {
                        is_fabs[dest_id] = true;
                        fabs_changed = true;
                    }
                }
            }
        }
    }

    for block in &mut func.blocks {
        for inst in &mut block.instructions {
            if let Instruction::Cmp {
                dest,
                op,
                lhs,
                rhs,
                ty,
            } = inst
            {
                if let Some(folded) = fold_fabs_lt_zero(*dest, *op, lhs, rhs, *ty, &is_fabs) {
                    *inst = folded;
                    total += 1;
                    continue;
                }
            }
            let mut simplified_here = false;
            if let Some(simplified) = try_simplify_with_types(
                inst,
                &cast_defs,
                &gep_defs,
                &cmp_defs,
                &binop_defs,
                &neg_defs,
                &unary_defs,
                &not_defs,
                &is_boolean,
                &value_types,
                &use_counts,
                &cast_use_counts,
                &cast_dies_if_narrowed,
            ) {
                *inst = simplified;
                total += 1;
                simplified_here = true;
            }
            if !simplify_skip("cmp") {
                if let Instruction::Cmp {
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty,
                } = inst
                {
                    let fold_ctx = SelectFoldCtx {
                        select_defs: &select_defs,
                        cmp_defs: &cmp_defs,
                        cast_defs: &cast_defs,
                        const_defs: &const_defs,
                        neg_defs: &neg_defs,
                        is_boolean: &is_boolean,
                        def_counts: &def_counts,
                        wrap_signed_overflow,
                    };
                    if let Some(folded) = fold_cmp_of_select(*dest, *op, lhs, rhs, *ty, &fold_ctx)
                        .or_else(|| fold_cmp_of_iabs(*dest, *op, lhs, rhs, *ty, &fold_ctx))
                    {
                        *inst = folded;
                        if !simplified_here {
                            total += 1;
                        }
                    }
                }
            }
        }
    }

    // The `k > 0` half of the iabs compare folds: a separate phase
    // because it inserts the biasing add (see `rewrite_iabs_range_cmps`).
    // Runs after the in-place sweep so it rebuilds its def facts from the
    // post-sweep function rather than trusting the entry snapshot across
    // in-place replacements.
    if !simplify_skip("cmp") {
        total += rewrite_iabs_range_cmps(func);
    }
    if !simplify_skip("rotate_chain") {
        total += rewrite_rotate_and_popcount_chains(func);
    }

    // A widening integer conversion preserves exactly the zero/nonzero
    // partition: for every source value x, widen(x) == 0 iff x == 0.  A
    // CondBranch consumes only that partition, not the converted numerical
    // value, so branch directly on the narrow source.  Besides deleting a
    // dead extension this lets x86 use `testb`/`testw` rather than making a
    // 32/64-bit temporary.  The helper rejects truncations (e.g. 0x100 -> u8
    // is zero) and float/pointer conversions with target-specific exceptional
    // semantics.
    if !simplify_skip("widened_cond") && std::env::var_os("CCC_NO_WIDENED_COND").is_none() {
        for block in &mut func.blocks {
            let Terminator::CondBranch { cond, .. } = &mut block.terminator else {
                continue;
            };
            if let Some(narrow) =
                widened_cond_replacement(*cond, &cast_defs, &cast_dies_if_narrowed)
            {
                *cond = narrow;
                total += 1;
            }
        }
    }
    total
}

/// Store a definition in a flat Vec, bounds-checked.
fn set_def<T>(defs: &mut [Option<T>], id: u32, val: T) {
    let idx = id as usize;
    if idx < defs.len() {
        defs[idx] = Some(val);
    }
}

/// Check whether an operand is known to be boolean (0 or 1).
fn operand_is_boolean(op: &Operand, is_boolean: &[bool]) -> bool {
    match op {
        Operand::Value(v) => {
            let id = v.0 as usize;
            id < is_boolean.len() && is_boolean[id]
        }
        Operand::Const(c) => matches!(c.to_i64(), Some(0) | Some(1)),
    }
}

/// Cached information about a Cast instruction for chain elimination.
#[derive(Clone, Copy)]
struct CastDef {
    src: Operand,
    from_ty: IrType,
    to_ty: IrType,
}

/// Cached information about a GEP instruction for chain folding.
#[derive(Clone, Copy)]
struct GepDef {
    base: Value,
    offset: Operand,
}

/// Cached information about a Cmp instruction for comparison simplification.
#[derive(Clone, Copy)]
struct CmpDef {
    op: IrCmpOp,
    lhs: Operand,
    rhs: Operand,
    ty: IrType,
}

/// Cached information about a Select instruction for comparison-of-select folding.
#[derive(Clone, Copy)]
struct SelectDef {
    cond: Operand,
    true_val: Operand,
    false_val: Operand,
    ty: IrType,
}

/// Cached information about a UnaryOp::Neg instruction.
/// Used to simplify `x - (neg y)` => `x + y`.
#[derive(Clone, Copy)]
struct NegDef {
    src: Operand,
}

/// Cached information about a UnaryOp::Clz / UnaryOp::Ctz instruction.
/// The IR defines Clz(0)/Ctz(0) == operand width, which lets a guarded
/// ternary `x ? clz(x) : width` collapse onto the bare intrinsic.
#[derive(Clone, Copy)]
struct UnaryDef {
    op: IrUnaryOp,
    src: Operand,
    ty: IrType,
}

/// Peel integer->integer Cast chains (optionally only same-carrier-size
/// casts — used on the condition/clz-operand side, where only a same-size
/// cast is guaranteed to preserve zero-ness) off an operand.
fn peel_int_cast_chain(
    mut op: Operand,
    cast_defs: &[Option<CastDef>],
    same_size_only: bool,
) -> Operand {
    for _ in 0..32 {
        let Operand::Value(v) = op else { break };
        match cast_defs.get(v.0 as usize).and_then(|d| d.as_ref()) {
            Some(CastDef {
                src,
                from_ty,
                to_ty,
                ..
            }) if from_ty.is_integer()
                && to_ty.is_integer()
                && (!same_size_only || from_ty.size() == to_ty.size()) =>
            {
                op = *src;
            }
            _ => break,
        }
    }
    op
}

/// Redundant zero-guard select elimination for the bit intrinsics.
///
/// `select(cond != 0 → t, else W)` where `t` is a Clz/Ctz result chain over
/// `x`, the condition selects the intrinsic arm exactly when `x` is nonzero,
/// and `W` equals the intrinsic's operand width collapses onto `t`: the IR
/// defines Clz(0)/Ctz(0) == width, so `t` already yields `W` in the zero
/// case. This is the shape C sources write as `x ? __builtin_clz(x) : 32`
/// (and the bsr/bsf clz lowering already provides the zero answer, so the
/// emitted codegen otherwise duplicates the zero fix-up as a dead cmov).
///
/// Public entry point for the dedicated pass placed right after if-conversion
/// (which *creates* the Select form from a branchy ternary only after the
/// generic simplify sweep has already run in the same pipeline iteration).
pub(crate) fn remove_redundant_bitop_guards(func: &mut IrFunction) -> usize {
    let max = func.max_value_id() as usize + 1;
    let mut def_counts = vec![0u8; max];
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some(d) = inst.dest() {
                let idx = d.0 as usize;
                if idx < def_counts.len() {
                    def_counts[idx] = def_counts[idx].saturating_add(1);
                }
            }
        }
    }
    let mut cast_defs: Vec<Option<CastDef>> = vec![None; max];
    let mut cmp_defs: Vec<Option<CmpDef>> = vec![None; max];
    let mut select_defs: Vec<Option<SelectDef>> = vec![None; max];
    let mut const_defs: Vec<Option<IrConst>> = vec![None; max];
    let mut neg_defs: Vec<Option<NegDef>> = vec![None; max];
    let mut is_boolean = vec![false; max];
    // Captured before the block loops borrow the function; consumed by
    // the iabs compare folds (see `SelectFoldCtx`).
    let wrap_signed_overflow = func.pass_policy.wrap_signed_overflow;
    let mut binop_defs: Vec<Option<BinOpDef>> = vec![None; max];
    let mut unary_defs: Vec<Option<UnaryDef>> = vec![None; max];
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Cast {
                    dest,
                    src,
                    from_ty,
                    to_ty,
                } => {
                    if (dest.0 as usize) < cast_defs.len() {
                        cast_defs[dest.0 as usize] = Some(CastDef {
                            src: *src,
                            from_ty: *from_ty,
                            to_ty: *to_ty,
                        });
                    }
                }
                Instruction::Cmp {
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty,
                } => {
                    let idx = dest.0 as usize;
                    if idx < cmp_defs.len() {
                        cmp_defs[idx] = Some(CmpDef {
                            op: *op,
                            lhs: *lhs,
                            rhs: *rhs,
                            ty: *ty,
                        });
                        is_boolean[idx] = true;
                    }
                }
                Instruction::Select {
                    dest,
                    cond,
                    true_val,
                    false_val,
                    ty,
                } => {
                    let idx = dest.0 as usize;
                    if idx < select_defs.len() && def_counts.get(idx).copied() == Some(1) {
                        select_defs[idx] = Some(SelectDef {
                            cond: *cond,
                            true_val: *true_val,
                            false_val: *false_val,
                            ty: *ty,
                        });
                    }
                }
                Instruction::Copy {
                    dest,
                    src: Operand::Const(c),
                } => {
                    let idx = dest.0 as usize;
                    if idx < const_defs.len() && def_counts.get(idx).copied() == Some(1) {
                        const_defs[idx] = Some(*c);
                    }
                }
                Instruction::BinOp {
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty,
                } => {
                    binop_defs[dest.0 as usize] = Some(BinOpDef {
                        op: *op,
                        lhs: *lhs,
                        rhs: *rhs,
                        ty: *ty,
                    });
                }
                Instruction::UnaryOp { dest, op, src, ty } => match op {
                    IrUnaryOp::Neg => {
                        set_def(&mut neg_defs, dest.0, NegDef { src: *src });
                        let idx = dest.0 as usize;
                        if idx < const_defs.len() && def_counts.get(idx).copied() == Some(1) {
                            if let Operand::Const(c) = src {
                                if let Some(folded) = super::constant_fold::eval_unaryop_const(
                                    IrUnaryOp::Neg,
                                    *c,
                                    None,
                                    *ty,
                                ) {
                                    const_defs[idx] = Some(folded);
                                }
                            }
                        }
                    }
                    IrUnaryOp::Clz
                    | IrUnaryOp::ClzNonZero
                    | IrUnaryOp::Ctz
                    | IrUnaryOp::CtzNonZero => {
                        if (dest.0 as usize) < unary_defs.len() {
                            unary_defs[dest.0 as usize] = Some(UnaryDef {
                                op: *op,
                                src: *src,
                                ty: *ty,
                            });
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
    let uses = value_use_counts(func, max);
    let mut refined: Vec<Value> = Vec::new();
    let mut changes = 0;
    for block in &mut func.blocks {
        for inst in &mut block.instructions {
            match inst {
                Instruction::Cmp {
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty,
                } if !simplify_skip("cmp") => {
                    let fold_ctx = SelectFoldCtx {
                        select_defs: &select_defs,
                        cmp_defs: &cmp_defs,
                        cast_defs: &cast_defs,
                        const_defs: &const_defs,
                        neg_defs: &neg_defs,
                        is_boolean: &is_boolean,
                        def_counts: &def_counts,
                        wrap_signed_overflow,
                    };
                    if let Some(folded) = fold_cmp_of_select(*dest, *op, lhs, rhs, *ty, &fold_ctx)
                        .or_else(|| fold_cmp_of_iabs(*dest, *op, lhs, rhs, *ty, &fold_ctx))
                    {
                        *inst = folded;
                        changes += 1;
                    }
                }
                Instruction::Select {
                    dest,
                    cond,
                    true_val,
                    false_val,
                    ty,
                } => {
                    if let Some(replacement) = safe_clz_guard_replacement(
                        *dest,
                        *cond,
                        *true_val,
                        *false_val,
                        *ty,
                        &cast_defs,
                        &unary_defs,
                        &binop_defs,
                    ) {
                        *inst = replacement;
                        changes += 1;
                    } else if let Some(src) = clz_ctz_zero_guard_replacement(
                        cond,
                        true_val,
                        false_val,
                        &cast_defs,
                        &unary_defs,
                    )
                    .or_else(|| {
                        zero_value_replacement(
                            cond,
                            true_val,
                            false_val,
                            *ty,
                            &cast_defs,
                            &unary_defs,
                            &binop_defs,
                        )
                    })
                    .or_else(|| {
                        let root = nonzero_guard_refinement(
                            cond,
                            true_val,
                            false_val,
                            &cast_defs,
                            &unary_defs,
                            &uses,
                        )?;
                        refined.push(root);
                        Some(*true_val)
                    }) {
                        *inst = Instruction::Copy { dest: *dest, src };
                        changes += 1;
                    }
                }
                // `(W-1) - ClzNonZero(x)` is the index of the highest set bit
                // (zstd's `ZSTD_highbit32`, `BIT_highbit32`, Linux `__fls`).
                // The count lies in [0, W-1] and W-1 is all ones over those
                // bits, so the subtraction never borrows and equals
                // `count ^ (W-1)` in any carrier at least as wide; integer
                // casts between them preserve a value <= 63. On baseline x86
                // the count itself is `bsr ^ (W-1)`, so the two XORs cancel
                // in the peephole and the idiom becomes one `bsr`, as with
                // GCC; on LZCNT, AArch64 and RISC-V `eor` costs what `sub`
                // did. Defined-zero `Clz` reaches W and is excluded.
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::Sub,
                    lhs: Operand::Const(k),
                    rhs,
                    ty,
                } if ty.is_integer() => {
                    if is_highbit_of_clz(k, *rhs, &cast_defs, &unary_defs) {
                        *inst = Instruction::BinOp {
                            dest: *dest,
                            op: IrBinOp::Xor,
                            lhs: *rhs,
                            rhs: Operand::Const(*k),
                            ty: *ty,
                        };
                        changes += 1;
                    }
                }
                // The select collapse above exposes the pre-existing round
                // trip that wrapped the intrinsic arm to the select's (wider)
                // carrier type: sext(clz(x)) i32->i64 followed by a truncate
                // back to i32. Truncate(sext(v)) == v for every v, so the
                // round trip is exact; for a Clz/Ctz root the intermediate
                // sign-extension is additionally never needed (results are
                // non-negative). Fold it onto the intrinsic result, keeping
                // the types consistent (the outer trunc's destination type
                // equals the intrinsic result type).
                Instruction::Cast {
                    dest,
                    src,
                    from_ty,
                    to_ty,
                } if from_ty.is_integer()
                    && to_ty.is_integer()
                    && from_ty.size() > to_ty.size() =>
                {
                    if let Operand::Value(s) = src {
                        if let Some(Some(inner)) = cast_defs.get(s.0 as usize) {
                            let is_round_trip = inner.to_ty == *from_ty && inner.from_ty == *to_ty;
                            let root = peel_int_cast_chain(inner.src, &cast_defs, true);
                            let bitop_root = match root {
                                Operand::Value(rv) => matches!(
                                    unary_defs.get(rv.0 as usize).and_then(|d| d.as_ref()),
                                    Some(UnaryDef {
                                        op: IrUnaryOp::Clz
                                            | IrUnaryOp::ClzNonZero
                                            | IrUnaryOp::Ctz
                                            | IrUnaryOp::CtzNonZero,
                                        ..
                                    })
                                ),
                                _ => false,
                            };
                            if is_round_trip && bitop_root {
                                *inst = Instruction::Copy {
                                    dest: *dest,
                                    src: inner.src,
                                };
                                changes += 1;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    if !refined.is_empty() {
        refine_nonzero_counts(func, &refined);
    }
    // If-conversion creates the Select-form abs this late in the pipeline,
    // so the `k > 0` range rewrite runs here as well as in the main sweep.
    if !simplify_skip("cmp") {
        changes += rewrite_iabs_range_cmps(func);
    }
    changes
}

/// Readers of every value id below `max` (instructions and terminators).
fn value_use_counts(func: &IrFunction, max: usize) -> Vec<u32> {
    let mut uses = vec![0u32; max];
    for block in &func.blocks {
        let mut count = |v: u32| {
            if let Some(n) = uses.get_mut(v as usize) {
                *n += 1;
            }
        };
        for inst in &block.instructions {
            inst.for_each_used_value(&mut count);
        }
        block.terminator.for_each_used_value(&mut count);
    }
    uses
}

/// `k - rhs` is `(W-1) - ClzNonZero(x)` through integer casts: the index of
/// the highest set bit, equal to `rhs ^ k` (see the caller).
fn is_highbit_of_clz(
    k: &IrConst,
    rhs: Operand,
    cast_defs: &[Option<CastDef>],
    unary_defs: &[Option<UnaryDef>],
) -> bool {
    match peel_int_cast_chain(rhs, cast_defs, false) {
        Operand::Value(rv) => matches!(
            unary_defs.get(rv.0 as usize).and_then(|d| d.as_ref()),
            Some(UnaryDef { op: IrUnaryOp::ClzNonZero, ty: cty, .. })
                if cty.is_integer() && k.to_i128() == Some((cty.size() * 8) as i128 - 1)
        ),
        _ => false,
    }
}

/// Turn the NonZero counts `nonzero_guard_refinement` selected into their
/// defined-zero forms (a refinement: they agree wherever NonZero is defined).
fn refine_nonzero_counts(func: &mut IrFunction, refined: &[Value]) {
    for inst in func
        .blocks
        .iter_mut()
        .flat_map(|b| b.instructions.iter_mut())
    {
        if let Instruction::UnaryOp { dest, op, .. } = inst
            && refined.contains(dest)
        {
            *op = match *op {
                IrUnaryOp::ClzNonZero => IrUnaryOp::Clz,
                IrUnaryOp::CtzNonZero => IrUnaryOp::Ctz,
                other => other,
            };
        }
    }
}

fn eval_int_cmp_bool(op: IrCmpOp, lhs: IrConst, rhs: IrConst, ty: IrType) -> Option<bool> {
    if !ty.is_integer() {
        return None;
    }
    match super::constant_fold::eval_cmp_const(op, lhs, rhs, ty)? {
        IrConst::I32(v) => Some(v != 0),
        IrConst::I8(v) => Some(v != 0),
        _ => None,
    }
}

/// Hard bound on the single-definition cast chain `resolve_select_const`
/// will follow.  Real frontend output nests two or three casts; the bound
/// exists so pathological or adversarial IR costs O(1) per operand instead
/// of O(chain), and failing means "missed fold", never "wrong fold".  The
/// boundary is pinned by `select_constant_cast_walk_is_bounded_and_folds_valid_chains`.
const MAX_SELECT_CONST_DEPTH: u8 = 8;

/// Hard bound on the cast layers `peel_bool_cond` will strip while looking
/// for a provably-boolean condition.  Same fail-closed rationale as
/// [`MAX_SELECT_CONST_DEPTH`]: beyond the bound the original operand is
/// returned unchanged.
const MAX_BOOL_CAST_PEEL: usize = 16;

/// The single-def facts the select/compare folds consult, bundled so the
/// folds take one evidence parameter instead of seven parallel slices that
/// could be transposed at a call site without any type error (five of the
/// seven share an element type shape).  Constructed once per `Cmp` site
/// from the per-function fact tables; the table fields are shared borrows,
/// so the bundle is `Copy`-sized (seven fat pointers and a `bool`) and
/// free to pass.  `wrap_signed_overflow` rides along because the integer
/// absolute-value folds are valid only when signed overflow is undefined
/// behavior the fold may exploit (under `-fwrapv`, `abs(INT_MIN)` is the
/// representable, negative `INT_MIN`, which inverts the folded answers).
struct SelectFoldCtx<'a> {
    select_defs: &'a [Option<SelectDef>],
    cmp_defs: &'a [Option<CmpDef>],
    cast_defs: &'a [Option<CastDef>],
    const_defs: &'a [Option<IrConst>],
    neg_defs: &'a [Option<NegDef>],
    is_boolean: &'a [bool],
    def_counts: &'a [u8],
    wrap_signed_overflow: bool,
}

fn resolve_select_const(
    op: Operand,
    const_defs: &[Option<IrConst>],
    cast_defs: &[Option<CastDef>],
    def_counts: &[u8],
) -> Option<IrConst> {
    resolve_select_const_bounded(op, const_defs, cast_defs, def_counts, 0)
}

fn resolve_select_const_bounded(
    op: Operand,
    const_defs: &[Option<IrConst>],
    cast_defs: &[Option<CastDef>],
    def_counts: &[u8],
    depth: u8,
) -> Option<IrConst> {
    if depth > MAX_SELECT_CONST_DEPTH {
        return None;
    }
    match op {
        Operand::Const(c) => Some(c),
        Operand::Value(v) => {
            let idx = v.0 as usize;
            if def_counts.get(idx).copied() != Some(1) {
                return None;
            }
            if let Some(Some(c)) = const_defs.get(idx) {
                return Some(*c);
            }
            if let Some(Some(cdef)) = cast_defs.get(idx) {
                if cdef.from_ty.is_integer() && cdef.to_ty.is_integer() {
                    let inner = resolve_select_const_bounded(
                        cdef.src,
                        const_defs,
                        cast_defs,
                        def_counts,
                        depth + 1,
                    )?;
                    return super::constant_fold::eval_cast_const(inner, cdef.from_ty, cdef.to_ty);
                }
            }
            None
        }
    }
}

fn peel_bool_cond(
    op: Operand,
    cast_defs: &[Option<CastDef>],
    is_boolean: &[bool],
    cmp_defs: &[Option<CmpDef>],
    def_counts: &[u8],
) -> Operand {
    let mut candidate = op;
    let mut saw_narrowing = false;
    for _ in 0..MAX_BOOL_CAST_PEEL {
        let Operand::Value(v) = candidate else { break };
        let idx = v.0 as usize;
        if def_counts.get(idx).copied() != Some(1) {
            break;
        }
        if let Some(Some(cdef)) = cast_defs.get(idx) {
            if cdef.from_ty.is_integer() && cdef.to_ty.is_integer() {
                if cdef.to_ty.size() < cdef.from_ty.size() {
                    saw_narrowing = true;
                }
                candidate = cdef.src;
                continue;
            }
        }
        break;
    }
    if !saw_narrowing {
        return candidate;
    }
    if let Operand::Value(v) = candidate {
        let idx = v.0 as usize;
        let is_bool = def_counts.get(idx).copied() == Some(1)
            && (is_boolean.get(idx).copied().unwrap_or(false)
                || cmp_defs.get(idx).and_then(|d| *d).is_some());
        if is_bool {
            return candidate;
        }
    }
    op
}

fn fold_bool_select_arms(
    dest: Value,
    cond: Operand,
    true_branch: bool,
    false_branch: bool,
    is_boolean: &[bool],
    cmp_defs: &[Option<CmpDef>],
    def_counts: &[u8],
) -> Option<Instruction> {
    if true_branch == false_branch {
        return Some(Instruction::Copy {
            dest,
            src: Operand::Const(IrConst::I8(i8::from(true_branch))),
        });
    }
    let Operand::Value(cv) = cond else {
        return None;
    };
    let idx = cv.0 as usize;
    if def_counts.get(idx).copied() != Some(1) {
        return None;
    }
    if true_branch && !false_branch {
        if is_boolean.get(idx).copied().unwrap_or(false)
            || cmp_defs.get(idx).and_then(|d| *d).is_some()
        {
            return Some(Instruction::Copy {
                dest,
                src: Operand::Value(cv),
            });
        }
        None
    } else {
        let ic = (*cmp_defs.get(idx)?)?;
        if ic.ty.is_float() && !matches!(ic.op, IrCmpOp::Eq | IrCmpOp::Ne) {
            return None;
        }
        Some(Instruction::Cmp {
            dest,
            op: invert_cmp_op(ic.op),
            lhs: ic.lhs,
            rhs: ic.rhs,
            ty: ic.ty,
        })
    }
}

fn int_cmp_trichotomy_mask(op: IrCmpOp) -> u8 {
    match op {
        IrCmpOp::Eq => 0b010,
        IrCmpOp::Ne => 0b101,
        IrCmpOp::Slt | IrCmpOp::Ult => 0b001,
        IrCmpOp::Sle | IrCmpOp::Ule => 0b011,
        IrCmpOp::Sgt | IrCmpOp::Ugt => 0b100,
        IrCmpOp::Sge | IrCmpOp::Uge => 0b110,
    }
}

fn int_cmp_from_trichotomy_mask(mask: u8, unsigned: bool) -> Option<IrCmpOp> {
    Some(match (mask & 0b111, unsigned) {
        (0b001, true) => IrCmpOp::Ult,
        (0b001, false) => IrCmpOp::Slt,
        (0b010, _) => IrCmpOp::Eq,
        (0b011, true) => IrCmpOp::Ule,
        (0b011, false) => IrCmpOp::Sle,
        (0b100, true) => IrCmpOp::Ugt,
        (0b100, false) => IrCmpOp::Sgt,
        (0b101, _) => IrCmpOp::Ne,
        (0b110, true) => IrCmpOp::Uge,
        (0b110, false) => IrCmpOp::Sge,
        _ => return None,
    })
}

/// Normalize `Cmp(op, lhs, rhs)` to `(value, constant, effective_op)` with
/// the constant moved to the right (swapping the predicate when it started
/// on the left), resolving a constant-valued `Value` operand through
/// single-def const/cast chains when neither side is a literal constant.
fn normalize_cmp_value_const(
    op: IrCmpOp,
    lhs: &Operand,
    rhs: &Operand,
    const_defs: &[Option<IrConst>],
    cast_defs: &[Option<CastDef>],
    def_counts: &[u8],
) -> Option<(Value, IrConst, IrCmpOp)> {
    match (lhs, rhs) {
        (Operand::Value(v), Operand::Const(c)) => Some((*v, *c, op)),
        (Operand::Const(c), Operand::Value(v)) => Some((*v, *c, swap_cmp_op(op))),
        (Operand::Value(v), Operand::Value(cv)) => {
            if let Some(c) =
                resolve_select_const(Operand::Value(*cv), const_defs, cast_defs, def_counts)
            {
                Some((*v, c, op))
            } else {
                resolve_select_const(Operand::Value(*v), const_defs, cast_defs, def_counts)
                    .map(|c| (*cv, c, swap_cmp_op(op)))
            }
        }
        _ => None,
    }
}

/// The signed mathematical value of an integer comparison constant at
/// `ty`, or `None` when the constant is not an integer. Sub-64-bit values
/// are sign-extended from the type's width (so an `I8` constant `0xFF`
/// reads as -1); 128-bit constants keep their full range.
fn signed_cmp_const_value(c: IrConst, ty: IrType) -> Option<i128> {
    if ty.is_128bit() {
        c.to_i128()
    } else {
        Some(ty.truncate_i64(c.to_i64()?) as i128)
    }
}

/// Fold comparisons of 1-level and 2-level constant-leaf `Select` trees against
/// an integer constant. Inlined 3-way comparators (`(a < b) ? -1 : ((a > b) ? 1 : 0)`)
/// compared against `0` after `if_convert` collapse to a single `Cmp(a, b)`.
fn fold_cmp_of_select(
    dest: Value,
    op: IrCmpOp,
    lhs: &Operand,
    rhs: &Operand,
    ty: IrType,
    ctx: &SelectFoldCtx<'_>,
) -> Option<Instruction> {
    let SelectFoldCtx {
        select_defs,
        cmp_defs,
        cast_defs,
        const_defs,
        is_boolean,
        def_counts,
        ..
    } = *ctx;
    if !ty.is_integer() {
        return None;
    }
    let (val, cval, effective_op) =
        normalize_cmp_value_const(op, lhs, rhs, const_defs, cast_defs, def_counts)?;
    let val_idx = val.0 as usize;
    if def_counts.get(val_idx).copied() != Some(1) {
        return None;
    }
    let sdef = (*select_defs.get(val_idx)?)?;
    if !sdef.ty.is_integer() || sdef.ty.size() != ty.size() {
        return None;
    }
    let c1 = peel_bool_cond(sdef.cond, cast_defs, is_boolean, cmp_defs, def_counts);
    let true_const = resolve_select_const(sdef.true_val, const_defs, cast_defs, def_counts);
    let false_const = resolve_select_const(sdef.false_val, const_defs, cast_defs, def_counts);

    // 1-level Select(c1, k1, k2) compared against cval.
    if let (Some(k1), Some(k2)) = (true_const, false_const) {
        let p1 = eval_int_cmp_bool(effective_op, k1, cval, ty)?;
        let p2 = eval_int_cmp_bool(effective_op, k2, cval, ty)?;
        return fold_bool_select_arms(dest, c1, p1, p2, is_boolean, cmp_defs, def_counts);
    }

    // 2-level Select(c1, k1, Select(c2, k2, k3)) or Select(c1, Select(c2, k2, k3), k1).
    let (k1, inner_op, inner_on_false) = match (true_const, false_const) {
        (Some(k1), None) => (k1, sdef.false_val, true),
        (None, Some(k1)) => (k1, sdef.true_val, false),
        _ => return None,
    };
    let Operand::Value(inner_v) = peel_int_cast_chain(inner_op, cast_defs, true) else {
        return None;
    };
    let inner_idx = inner_v.0 as usize;
    if def_counts.get(inner_idx).copied() != Some(1) {
        return None;
    }
    let sdef2 = (*select_defs.get(inner_idx)?)?;
    if !sdef2.ty.is_integer() || sdef2.ty.size() != ty.size() {
        return None;
    }
    let k2 = resolve_select_const(sdef2.true_val, const_defs, cast_defs, def_counts)?;
    let k3 = resolve_select_const(sdef2.false_val, const_defs, cast_defs, def_counts)?;
    let p1 = eval_int_cmp_bool(effective_op, k1, cval, ty)?;
    let p2 = eval_int_cmp_bool(effective_op, k2, cval, ty)?;
    let p3 = eval_int_cmp_bool(effective_op, k3, cval, ty)?;

    // Unconditional reduction when both arms of the inner select compare identically.
    if p2 == p3 {
        let (tb, fb) = if inner_on_false { (p1, p2) } else { (p2, p1) };
        return fold_bool_select_arms(dest, c1, tb, fb, is_boolean, cmp_defs, def_counts);
    }

    // Trichotomy truth-table reduction when c1 and c2 compare the same integer operands.
    let c2 = peel_bool_cond(sdef2.cond, cast_defs, is_boolean, cmp_defs, def_counts);
    let (Operand::Value(cv1), Operand::Value(cv2)) = (c1, c2) else {
        return None;
    };
    let (idx1, idx2) = (cv1.0 as usize, cv2.0 as usize);
    if def_counts.get(idx1).copied() != Some(1) || def_counts.get(idx2).copied() != Some(1) {
        return None;
    }
    let ic1 = (*cmp_defs.get(idx1)?)?;
    let ic2 = (*cmp_defs.get(idx2)?)?;
    if !ic1.ty.is_integer() || ic1.ty != ic2.ty {
        return None;
    }
    let op2_norm = if same_operand(&ic1.lhs, &ic2.lhs) && same_operand(&ic1.rhs, &ic2.rhs) {
        ic2.op
    } else if same_operand(&ic1.lhs, &ic2.rhs) && same_operand(&ic1.rhs, &ic2.lhs) {
        swap_cmp_op(ic2.op)
    } else {
        return None;
    };
    let is_unsigned_rel =
        |o: IrCmpOp| matches!(o, IrCmpOp::Ult | IrCmpOp::Ule | IrCmpOp::Ugt | IrCmpOp::Uge);
    let is_signed_rel =
        |o: IrCmpOp| matches!(o, IrCmpOp::Slt | IrCmpOp::Sle | IrCmpOp::Sgt | IrCmpOp::Sge);
    if (is_unsigned_rel(ic1.op) && is_signed_rel(op2_norm))
        || (is_signed_rel(ic1.op) && is_unsigned_rel(op2_norm))
    {
        return None;
    }
    let use_unsigned = is_unsigned_rel(ic1.op) || is_unsigned_rel(op2_norm);
    let m1 = int_cmp_trichotomy_mask(ic1.op);
    let m2 = int_cmp_trichotomy_mask(op2_norm);
    let mut result_mask = 0u8;
    for s_bit in [1u8, 2u8, 4u8] {
        let b1 = (m1 & s_bit) != 0;
        let b2 = (m2 & s_bit) != 0;
        let inner_p = if b2 { p2 } else { p3 };
        let outer_p = if inner_on_false {
            if b1 { p1 } else { inner_p }
        } else if b1 {
            inner_p
        } else {
            p1
        };
        if outer_p {
            result_mask |= s_bit;
        }
    }
    match result_mask {
        0b000 => Some(Instruction::Copy {
            dest,
            src: Operand::Const(IrConst::I8(0)),
        }),
        0b111 => Some(Instruction::Copy {
            dest,
            src: Operand::Const(IrConst::I8(1)),
        }),
        _ => {
            let merged_op = int_cmp_from_trichotomy_mask(result_mask, use_unsigned)?;
            if merged_op == ic1.op {
                Some(Instruction::Copy {
                    dest,
                    src: Operand::Value(cv1),
                })
            } else if same_operand(&ic1.lhs, &ic2.lhs)
                && same_operand(&ic1.rhs, &ic2.rhs)
                && merged_op == ic2.op
            {
                Some(Instruction::Copy {
                    dest,
                    src: Operand::Value(cv2),
                })
            } else {
                Some(Instruction::Cmp {
                    dest,
                    op: merged_op,
                    lhs: ic1.lhs,
                    rhs: ic1.rhs,
                    ty: ic1.ty,
                })
            }
        }
    }
}

/// Recognize the integer absolute-value idiom `a < 0 ? -a : a` (and the
/// `a >= 0 ? a : -a` spelling) in a `Select` definition, returning `a`.
///
/// `cond` is the select's condition operand (callers with cast tables pass
/// it through `peel_bool_cond` first). The condition must be a single-def
/// signed comparison of `a` against literal zero at the select's own type,
/// the arm on the negative side must be a single-def `Neg(a)`, and the
/// other arm must be `a` itself. Anything else is a miss, never a guess.
fn recognize_iabs_select(
    sdef: &SelectDef,
    cond: Operand,
    cmp_defs: &[Option<CmpDef>],
    neg_defs: &[Option<NegDef>],
    def_counts: &[u8],
) -> Option<Operand> {
    if !sdef.ty.is_signed() {
        return None;
    }
    let Operand::Value(cond_v) = cond else {
        return None;
    };
    let cond_idx = cond_v.0 as usize;
    if def_counts.get(cond_idx).copied() != Some(1) {
        return None;
    }
    let ic = (*cmp_defs.get(cond_idx)?)?;
    if ic.ty != sdef.ty {
        return None;
    }
    let is_zero_op = |o: &Operand| matches!(o, Operand::Const(c) if c.is_zero());
    let (a, neg_on_true) = match ic.op {
        IrCmpOp::Slt if is_zero_op(&ic.rhs) => (ic.lhs, true),
        IrCmpOp::Sgt if is_zero_op(&ic.lhs) => (ic.rhs, true),
        IrCmpOp::Sge if is_zero_op(&ic.rhs) => (ic.lhs, false),
        IrCmpOp::Sle if is_zero_op(&ic.lhs) => (ic.rhs, false),
        _ => return None,
    };
    let (neg_arm, plain_arm) = if neg_on_true {
        (sdef.true_val, sdef.false_val)
    } else {
        (sdef.false_val, sdef.true_val)
    };
    if !same_operand(&plain_arm, &a) {
        return None;
    }
    let Operand::Value(neg_v) = neg_arm else {
        return None;
    };
    let neg_idx = neg_v.0 as usize;
    if def_counts.get(neg_idx).copied() != Some(1) {
        return None;
    }
    let ndef = (*neg_defs.get(neg_idx)?)?;
    if !same_operand(&ndef.src, &a) {
        return None;
    }
    Some(a)
}

/// Fold `abs(a) <cmp> k` for `k <= 0`, where `abs(a)` is the recognized
/// select idiom. With signed overflow undefined (`-fwrapv` off), `abs(a)`
/// ranges over `[0, MAX]`, so `abs(a) <= 0` is `a == 0`, `abs(a) < 0` is
/// false, and so on — the same reductions GCC and Clang apply through
/// value-range propagation on their abs expressions. Under `-fwrapv` the
/// premise fails (`abs(INT_MIN)` wraps to `INT_MIN`), so the fold is
/// suppressed and the wrapping computation stands. The `k > 0` forms need
/// a new value for the biased addend and live in
/// `rewrite_iabs_range_cmps` instead.
fn fold_cmp_of_iabs(
    dest: Value,
    op: IrCmpOp,
    lhs: &Operand,
    rhs: &Operand,
    ty: IrType,
    ctx: &SelectFoldCtx<'_>,
) -> Option<Instruction> {
    let SelectFoldCtx {
        select_defs,
        cmp_defs,
        cast_defs,
        const_defs,
        neg_defs,
        is_boolean,
        def_counts,
        wrap_signed_overflow,
    } = *ctx;
    if wrap_signed_overflow || !ty.is_signed() {
        return None;
    }
    let (val, cval, effective_op) =
        normalize_cmp_value_const(op, lhs, rhs, const_defs, cast_defs, def_counts)?;
    let k = signed_cmp_const_value(cval, ty)?;
    let val_idx = val.0 as usize;
    if def_counts.get(val_idx).copied() != Some(1) {
        return None;
    }
    let sdef = (*select_defs.get(val_idx)?)?;
    if sdef.ty != ty {
        return None;
    }
    let cond = peel_bool_cond(sdef.cond, cast_defs, is_boolean, cmp_defs, def_counts);
    let a = recognize_iabs_select(&sdef, cond, cmp_defs, neg_defs, def_counts)?;
    let const_bool = |b: bool| Instruction::Copy {
        dest,
        src: Operand::Const(IrConst::I8(i8::from(b))),
    };
    let cmp_zero = |op: IrCmpOp| Instruction::Cmp {
        dest,
        op,
        lhs: a,
        rhs: Operand::Const(IrConst::from_i64(0, ty)),
        ty,
    };
    Some(match (effective_op, k) {
        (IrCmpOp::Slt, _) if k <= 0 => const_bool(false),
        (IrCmpOp::Sge, _) if k <= 0 => const_bool(true),
        (IrCmpOp::Eq, _) if k < 0 => const_bool(false),
        (IrCmpOp::Ne, _) if k < 0 => const_bool(true),
        (IrCmpOp::Sle, 0) | (IrCmpOp::Eq, 0) => cmp_zero(IrCmpOp::Eq),
        (IrCmpOp::Sgt, 0) | (IrCmpOp::Ne, 0) => cmp_zero(IrCmpOp::Ne),
        (IrCmpOp::Sle, _) if k < 0 => const_bool(false),
        (IrCmpOp::Sgt, _) if k < 0 => const_bool(true),
        _ => return None,
    })
}

/// `|a| < k` / `|a| <= k` range-test constants: the addend that biases `a`
/// so the two-sided range starts at zero, and the inclusive unsigned bound
/// of the biased range (`2k - 2` for strict, `2k` for inclusive).
fn iabs_addend_const(v: i128, ty: IrType) -> IrConst {
    if ty.is_128bit() {
        IrConst::I128(v)
    } else {
        IrConst::from_i64(v as i64, ty)
    }
}

fn iabs_bound_const(bound: u128, ty: IrType) -> IrConst {
    if ty.is_128bit() {
        IrConst::I128(bound as i128)
    } else {
        IrConst::from_i64(bound as u64 as i64, ty.to_unsigned())
    }
}

/// Rewrite `abs(a) <cmp> k` for `k > 0` into the single unsigned range
/// test both GCC and Clang emit: `|a| < k` is `(unsigned)(a + k - 1) <=
/// 2k - 2`, `|a| <= k` is `(unsigned)(a + k) <= 2k`, and the `>`/`>=`
/// forms are the same tests inverted. The biased add wraps in two's
/// complement exactly over the accepted range, so one add and one
/// unsigned compare replace the negate, the select, and the signed
/// compare. Runs as its own phase because it mints a fresh value and
/// inserts the add ahead of the comparison, which the in-place simplify
/// sweep cannot do; it rebuilds the minimal def facts it consults from
/// the current function state. Suppressed under `-fwrapv` for the same
/// reason as `fold_cmp_of_iabs`.
fn rewrite_iabs_range_cmps(func: &mut IrFunction) -> usize {
    if func.pass_policy.wrap_signed_overflow {
        return 0;
    }
    let max = func.max_value_id() as usize + 1;
    let mut def_counts = vec![0u8; max];
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some(d) = inst.dest() {
                let idx = d.0 as usize;
                if idx < def_counts.len() {
                    def_counts[idx] = def_counts[idx].saturating_add(1);
                }
            }
        }
    }
    let mut select_defs: Vec<Option<SelectDef>> = vec![None; max];
    let mut cmp_defs: Vec<Option<CmpDef>> = vec![None; max];
    let mut neg_defs: Vec<Option<NegDef>> = vec![None; max];
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Select {
                    dest,
                    cond,
                    true_val,
                    false_val,
                    ty,
                } => {
                    if def_counts.get(dest.0 as usize).copied() == Some(1) {
                        set_def(
                            &mut select_defs,
                            dest.0,
                            SelectDef {
                                cond: *cond,
                                true_val: *true_val,
                                false_val: *false_val,
                                ty: *ty,
                            },
                        );
                    }
                }
                Instruction::Cmp {
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty,
                } => {
                    set_def(
                        &mut cmp_defs,
                        dest.0,
                        CmpDef {
                            op: *op,
                            lhs: *lhs,
                            rhs: *rhs,
                            ty: *ty,
                        },
                    );
                }
                Instruction::UnaryOp {
                    dest,
                    op: IrUnaryOp::Neg,
                    src,
                    ..
                } => {
                    set_def(&mut neg_defs, dest.0, NegDef { src: *src });
                }
                _ => {}
            }
        }
    }

    let mut changes = 0;
    // Disjoint field borrows: the rewrite mints fresh value IDs while
    // holding each block mutably.
    let IrFunction {
        blocks,
        next_value_id,
        ..
    } = func;
    for block in blocks.iter_mut() {
        // Inserting an instruction must keep `source_spans` parallel; when
        // it is neither empty nor parallel the block's bookkeeping is not
        // in a state this phase can extend safely (same fail-closed guard
        // as `split_errno_sqrt`).
        if !(block.source_spans.is_empty() || block.source_spans.len() == block.instructions.len())
        {
            continue;
        }
        let mut i = 0;
        while i < block.instructions.len() {
            let rewritten = (|| {
                let Instruction::Cmp {
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty,
                } = block.instructions[i]
                else {
                    return None;
                };
                if !ty.is_signed() {
                    return None;
                }
                let (val, cval, effective_op) = match (lhs, rhs) {
                    (Operand::Value(v), Operand::Const(c)) => (v, c, op),
                    (Operand::Const(c), Operand::Value(v)) => (v, c, swap_cmp_op(op)),
                    _ => return None,
                };
                let k = signed_cmp_const_value(cval, ty)?;
                if k <= 0 {
                    return None;
                }
                let (addend, bound, new_op) = match effective_op {
                    IrCmpOp::Slt => (k - 1, 2 * (k as u128) - 2, IrCmpOp::Ule),
                    IrCmpOp::Sle => (k, 2 * (k as u128), IrCmpOp::Ule),
                    IrCmpOp::Sgt => (k, 2 * (k as u128), IrCmpOp::Ugt),
                    IrCmpOp::Sge => (k - 1, 2 * (k as u128) - 2, IrCmpOp::Ugt),
                    _ => return None,
                };
                let val_idx = val.0 as usize;
                if def_counts.get(val_idx).copied() != Some(1) {
                    return None;
                }
                let sdef = (*select_defs.get(val_idx)?)?;
                if sdef.ty != ty {
                    return None;
                }
                let a = recognize_iabs_select(&sdef, sdef.cond, &cmp_defs, &neg_defs, &def_counts)?;
                Some((dest, ty, a, addend, bound, new_op))
            })();
            match rewritten {
                Some((dest, ty, a, addend, bound, new_op)) => {
                    let biased = Value(*next_value_id);
                    *next_value_id += 1;
                    block.instructions.insert(
                        i,
                        Instruction::BinOp {
                            dest: biased,
                            op: IrBinOp::Add,
                            lhs: a,
                            rhs: Operand::Const(iabs_addend_const(addend, ty)),
                            ty,
                        },
                    );
                    if !block.source_spans.is_empty() {
                        let span = block.source_spans[i].clone();
                        block.source_spans.insert(i, span);
                    }
                    block.instructions[i + 1] = Instruction::Cmp {
                        dest,
                        op: new_op,
                        lhs: Operand::Value(biased),
                        rhs: Operand::Const(iabs_bound_const(bound, ty)),
                        ty,
                    };
                    changes += 1;
                    i += 2;
                }
                None => i += 1,
            }
        }
    }
    changes
}

/// Fuse chained rotates and canonicalize `Popcount(Not(x))` -> `W - Popcount(x)`:
///
/// 1. `rotl(rotr(x, a), a) => x` and `rotr(rotl(x, a), a) => x` (exact inverse
///    on every power-of-two width `W in {8, 16, 32, 64}`).
/// 2. `rot{l,r}(rot{l,r}(x, a), b)` (when the inner rotate has a single use or
///    both counts are constants) => single rotate by `a + b` / `b - a`: because
///    `W | 2^K` for every integer count width `K >= 8`, modular addition and
///    subtraction commute with `(count mod W)` with zero edge cases.
/// Rewrite two algebraic rotate identities in `func`:
/// 1. `Rot(Rot(x, a), b)` (same direction, single-use inner rotate) =>
///    `Rot(x, a + b)`.
/// 2. `RotL(RotR(x, a), b)` => `RotL(x, b - a)` and
///    `RotR(RotL(x, a), b)` => `RotR(x, b - a)` (single-use inner rotate).
///    Because `W = 8 / 16 / 32 / 64` is a power of two, modular addition and
///    subtraction commute with `(count mod W)` with zero edge cases.
fn collect_rotate_facts(
    func: &IrFunction,
    max: usize,
) -> (
    Vec<u32>,
    Vec<Option<IrType>>,
    Vec<Option<BinOpDef>>,
    Vec<Option<CastDef>>,
) {
    let mut def_counts = vec![0u8; max];
    let mut use_counts = vec![0u32; max];
    let mut value_types: Vec<Option<IrType>> = vec![None; max];
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some(d) = inst.dest() {
                let idx = d.0 as usize;
                if idx < max {
                    def_counts[idx] = def_counts[idx].saturating_add(1);
                }
            }
            match inst {
                Instruction::BinOp { dest, ty, .. }
                | Instruction::UnaryOp { dest, ty, .. }
                | Instruction::Load { dest, ty, .. }
                | Instruction::ParamRef { dest, ty, .. }
                | Instruction::Select { dest, ty, .. }
                | Instruction::Phi { dest, ty, .. } => set_def(&mut value_types, dest.0, *ty),
                Instruction::Cast { dest, to_ty, .. } => set_def(&mut value_types, dest.0, *to_ty),
                Instruction::Cmp { dest, .. } => set_def(&mut value_types, dest.0, IrType::I32),
                _ => {}
            }
            inst.for_each_used_value(|v| {
                let idx = v as usize;
                if idx < max {
                    use_counts[idx] = use_counts[idx].saturating_add(1);
                }
            });
        }
        block.terminator.for_each_used_value(|v| {
            let idx = v as usize;
            if idx < max {
                use_counts[idx] = use_counts[idx].saturating_add(1);
            }
        });
    }
    let mut rot_defs: Vec<Option<BinOpDef>> = vec![None; max];
    let mut cast_defs: Vec<Option<CastDef>> = vec![None; max];
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::BinOp {
                    dest,
                    op: op @ (IrBinOp::RotateLeft | IrBinOp::RotateRight),
                    lhs,
                    rhs,
                    ty,
                } if def_counts.get(dest.0 as usize).copied() == Some(1) => {
                    set_def(
                        &mut rot_defs,
                        dest.0,
                        BinOpDef {
                            op: *op,
                            lhs: *lhs,
                            rhs: *rhs,
                            ty: *ty,
                        },
                    );
                }
                Instruction::Cast {
                    dest,
                    src,
                    from_ty,
                    to_ty,
                } if def_counts.get(dest.0 as usize).copied() == Some(1) => {
                    set_def(
                        &mut cast_defs,
                        dest.0,
                        CastDef {
                            src: *src,
                            from_ty: *from_ty,
                            to_ty: *to_ty,
                        },
                    );
                }
                _ => {}
            }
        }
    }
    (use_counts, value_types, rot_defs, cast_defs)
}

fn rewrite_rotate_and_popcount_chains(func: &mut IrFunction) -> usize {
    let max = func.max_value_id() as usize + 1;
    let (use_counts, value_types, rot_defs, cast_defs) = collect_rotate_facts(func, max);

    let peel_rot_cnt = |mut op: Operand| -> Operand {
        for _ in 0..4 {
            if let Operand::Value(v) = op
                && let Some(Some(cd)) = cast_defs.get(v.0 as usize).copied()
                && cd.from_ty.is_integer()
                && cd.to_ty.is_integer()
                && cd.from_ty.size() >= 1
                && (cd.to_ty.size() >= cd.from_ty.size() || cd.to_ty.size() >= 1)
                && (cd.from_ty.size() == cd.to_ty.size() || cd.to_ty.size() >= cd.from_ty.size())
            {
                op = cd.src;
            } else {
                break;
            }
        }
        op
    };

    let operand_ty = |op: &Operand, fallback: IrType| -> Option<IrType> {
        match op {
            Operand::Value(v) => value_types.get(v.0 as usize).copied().flatten(),
            Operand::Const(_) => Some(fallback),
        }
    };
    let typed_int_const = |ty: IrType, v: i128| -> IrConst {
        match ty {
            IrType::I8 | IrType::U8 => IrConst::I8(v as i8),
            IrType::I16 | IrType::U16 => IrConst::I16(v as i16),
            IrType::I32 | IrType::U32 => IrConst::I32(v as i32),
            _ => IrConst::I64(v as i64),
        }
    };

    let mut next_id = func.next_value_id.max(max as u32);
    let mut changes = 0usize;

    for block in &mut func.blocks {
        let mut i = 0;
        while i < block.instructions.len() {
            match block.instructions[i] {
                Instruction::BinOp {
                    dest,
                    op: outer_op @ (IrBinOp::RotateLeft | IrBinOp::RotateRight),
                    lhs: Operand::Value(inner_v),
                    rhs: outer_cnt,
                    ty,
                } => {
                    let bits = (ty.size() * 8) as i128;
                    if matches!(bits, 8 | 16 | 32 | 64)
                        && let Some(Some(inner)) = rot_defs.get(inner_v.0 as usize).copied()
                        && inner.ty == ty
                    {
                        // 1. Exact inverse cancellation: rotl(rotr(x, a), a) == x.
                        if inner.op != outer_op
                            && peel_rot_cnt(inner.rhs) == peel_rot_cnt(outer_cnt)
                        {
                            block.instructions[i] = Instruction::Copy {
                                dest,
                                src: inner.lhs,
                            };
                            changes += 1;
                            i += 1;
                            continue;
                        }
                        // 2. Constant-count fusion (profitable regardless of use count).
                        if let (Operand::Const(c1), Operand::Const(c2)) = (inner.rhs, outer_cnt)
                            && let (Some(a), Some(b)) = (c1.to_i128(), c2.to_i128())
                        {
                            let combined = if inner.op == outer_op {
                                (a + b).rem_euclid(bits)
                            } else {
                                (b - a).rem_euclid(bits)
                            };
                            block.instructions[i] = if combined == 0 {
                                Instruction::Copy {
                                    dest,
                                    src: inner.lhs,
                                }
                            } else {
                                Instruction::BinOp {
                                    dest,
                                    op: outer_op,
                                    lhs: inner.lhs,
                                    rhs: Operand::Const(typed_int_const(ty, combined)),
                                    ty,
                                }
                            };
                            changes += 1;
                            i += 1;
                            continue;
                        }
                        // 3. Variable-count fusion when inner rotate is single-use.
                        if use_counts.get(inner_v.0 as usize).copied() == Some(1) {
                            let t_a = operand_ty(&inner.rhs, ty);
                            let t_b = operand_ty(&outer_cnt, ty);
                            if let (Some(ta), Some(tb)) = (t_a, t_b)
                                && ta == tb
                                && ta.is_integer()
                                && ta.size() >= 1
                                && ta.size() <= 8
                            {
                                let cnt_dest = Value(next_id);
                                next_id += 1;
                                let (comb_op, comb_lhs, comb_rhs) = if inner.op == outer_op {
                                    (IrBinOp::Add, inner.rhs, outer_cnt)
                                } else {
                                    (IrBinOp::Sub, outer_cnt, inner.rhs)
                                };
                                block.instructions.insert(
                                    i,
                                    Instruction::BinOp {
                                        dest: cnt_dest,
                                        op: comb_op,
                                        lhs: comb_lhs,
                                        rhs: comb_rhs,
                                        ty: ta,
                                    },
                                );
                                if !block.source_spans.is_empty() {
                                    let span = block.source_spans[i].clone();
                                    block.source_spans.insert(i, span);
                                }
                                block.instructions[i + 1] = Instruction::BinOp {
                                    dest,
                                    op: outer_op,
                                    lhs: inner.lhs,
                                    rhs: Operand::Value(cnt_dest),
                                    ty,
                                };
                                changes += 1;
                                i += 2;
                                continue;
                            }
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }
    func.next_value_id = next_id;
    changes
}

/// `x ? count_nz(x) : W` whose count (and every cast between it and the
/// select) has the select as its ONLY reader: the root to refine.
///
/// `__builtin_clz`/`__builtin_ctz` lower to the NonZero counts, so the
/// guarded spelling `x ? __builtin_ctz(x) : 32` (and the bundled
/// `_tzcnt_u32`/`_lzcnt_u32`) carries a NonZero arm, which
/// `clz_ctz_zero_guard_replacement` must refuse to collapse onto. Replacing
/// a NonZero count by the defined-zero one is a refinement, sound for every
/// reader: it agrees wherever the NonZero count is defined. Once defined,
/// `select(x, count(x), W) == count(x)` holds exactly (the IR's
/// `Clz(0) == Ctz(0) == W`, preserved by every integer cast of a value
/// <= 64). The sole-reader condition is a cost rule, not a soundness one:
/// another reader would pay the zero fix-up on targets without
/// LZCNT/TZCNT; with the guard as the only reader the fix-up replaces the
/// guard, and LZCNT/BMI1 then emit a single `lzcnt`/`tzcnt`.
fn nonzero_guard_refinement(
    cond: &Operand,
    true_val: &Operand,
    false_val: &Operand,
    cast_defs: &[Option<CastDef>],
    unary_defs: &[Option<UnaryDef>],
    uses: &[u32],
) -> Option<Value> {
    let Operand::Value(_) = cond else { return None };
    let mut cur = *true_val;
    let root = loop {
        let Operand::Value(v) = cur else { return None };
        if uses.get(v.0 as usize) != Some(&1) {
            return None;
        }
        if let Some(Some(UnaryDef {
            op: IrUnaryOp::ClzNonZero | IrUnaryOp::CtzNonZero,
            ..
        })) = unary_defs.get(v.0 as usize)
        {
            break v;
        }
        match cast_defs.get(v.0 as usize).and_then(|d| d.as_ref()) {
            Some(CastDef {
                src,
                from_ty,
                to_ty,
                ..
            }) if from_ty.is_integer() && to_ty.is_integer() => cur = *src,
            _ => return None,
        }
    };
    let UnaryDef { src, ty, .. } = *unary_defs[root.0 as usize].as_ref()?;
    if !ty.is_integer() {
        return None;
    }
    let width = ty.size() as i128 * 8;
    let Operand::Const(zero_case) = false_val else {
        return None;
    };
    if zero_case.to_i128() != Some(width) {
        return None;
    }
    if peel_int_cast_chain(*cond, cast_defs, true) != peel_int_cast_chain(src, cast_defs, true) {
        return None;
    }
    Some(root)
}

/// Fold a guard around the *safe* portable CLZ, not a speculated bare
/// NonZero intrinsic. At x=0 the OR arm produces W-1: fallback W-1 selects
/// that existing arm, whereas fallback W requires a new DEFINED-zero Clz(x).
/// The new unary keeps the operand width; a wider/narrower result carrier is
/// deliberately rejected instead of accidentally changing the count width.
#[allow(clippy::too_many_arguments)]
fn safe_clz_guard_replacement(
    dest: Value,
    cond: Operand,
    true_val: Operand,
    false_val: Operand,
    select_ty: IrType,
    cast_defs: &[Option<CastDef>],
    unary_defs: &[Option<UnaryDef>],
    binop_defs: &[Option<BinOpDef>],
) -> Option<Instruction> {
    // Result casts, unlike INPUT casts, are transparent here: safe CLZ's
    // range is 0..=W-1 (at most 63), representable even in I8/U8. Thus every
    // integer narrowing/widening/sign change preserves the count. Copy keeps
    // the original result cast chain; the W fallback still requires a
    // width-matched result before constructing a new Clz with that type.
    let Operand::Value(root) = peel_int_cast_chain(true_val, cast_defs, false) else {
        return None;
    };
    let unary = unary_defs.get(root.0 as usize)?.as_ref()?;
    if !matches!(unary.op, IrUnaryOp::Clz | IrUnaryOp::ClzNonZero)
        || !matches!(
            unary.ty,
            IrType::I32 | IrType::U32 | IrType::I64 | IrType::U64
        )
    {
        return None;
    }
    let Operand::Value(safe) = peel_int_cast_chain(unary.src, cast_defs, true) else {
        return None;
    };
    let or = binop_defs.get(safe.0 as usize)?.as_ref()?;
    if or.op != IrBinOp::Or || !or.ty.is_integer() || or.ty.size() != unary.ty.size() {
        return None;
    }
    let one = |o: Operand| matches!(o, Operand::Const(c) if c.to_i128() == Some(1));
    let x = if one(or.rhs) {
        or.lhs
    } else if one(or.lhs) {
        or.rhs
    } else {
        return None;
    };
    if peel_int_cast_chain(cond, cast_defs, true) != peel_int_cast_chain(x, cast_defs, true) {
        return None;
    }
    let Operand::Const(zero) = false_val else {
        return None;
    };
    let width = (unary.ty.size() * 8) as i128;
    if zero.to_i128() == Some(width - 1) {
        Some(Instruction::Copy {
            dest,
            src: true_val,
        })
    } else if zero.to_i128() == Some(width)
        && select_ty.is_integer()
        && select_ty.size() == unary.ty.size()
    {
        Some(Instruction::UnaryOp {
            dest,
            op: IrUnaryOp::Clz,
            src: x,
            ty: select_ty,
        })
    } else {
        None
    }
}

/// Core predicate + rewrite of the redundant zero-guard pattern. Returns the
/// operand the select should collapse onto (the intrinsic arm), or None when
/// the select is not provably redundant.
fn clz_ctz_zero_guard_replacement(
    cond: &Operand,
    true_val: &Operand,
    false_val: &Operand,
    cast_defs: &[Option<CastDef>],
    unary_defs: &[Option<UnaryDef>],
) -> Option<Operand> {
    let Operand::Value(_) = cond else { return None };
    // Walk the true arm through its integer cast chain (value-preserving for
    // the small clz/ctz results) to the intrinsic.
    let mut cur = *true_val;
    let found = loop {
        let Operand::Value(v) = cur else { break None };
        if let Some(Some(UnaryDef {
            op: IrUnaryOp::Clz | IrUnaryOp::Ctz,
            ..
        })) = unary_defs.get(v.0 as usize)
        {
            break Some(v);
        }
        match cast_defs.get(v.0 as usize).and_then(|d| d.as_ref()) {
            Some(CastDef {
                src,
                from_ty,
                to_ty,
                ..
            }) if from_ty.is_integer() && to_ty.is_integer() => cur = *src,
            _ => break None,
        }
    };
    let root = found?;
    let UnaryDef { op, src, ty } = *unary_defs[root.0 as usize].as_ref()?;
    if !ty.is_integer() {
        return None;
    }
    let bits = ty.size() as u128 * 8;
    // The zero-case constant must be exactly the intrinsic width.
    let false_imm = match false_val {
        Operand::Const(c) => c.to_i128(),
        _ => None,
    };
    if false_imm != Some(bits as i128) {
        return None;
    }
    // The condition must select the intrinsic arm exactly when its operand is
    // nonzero (same value up to zero-ness-preserving casts).
    if peel_int_cast_chain(*cond, cast_defs, true) != peel_int_cast_chain(src, cast_defs, true) {
        return None;
    }
    // Defined-zero intrinsics only. `select(x, clz(x), W) == clz(x)` is an
    // algebraic identity BECAUSE the IR defines Clz(0)/Ctz(0) == W. The
    // NonZero spellings have no value at zero, so the identity does not
    // apply to them: their guard-select is the only thing standing between
    // a speculated BSR/BSF and a garbage return for x == 0. A NonZero arm
    // reaches this shape when CVP specialized the intrinsic under a branch
    // proof and if-conversion then speculated the arm ABOVE that proof
    // (i686 has no pre-CVP diamond conversion; on x86-64 the vectorizer's
    // pre-conversion merely hides the shape). Collapsing such a select
    // returns BSR's undefined destination for x == 0 — measured
    // `clz_if(0) == -846929913` before this rejection. When the nonzero
    // fact DOES dominate the select, CVP's own fact-stack select folding
    // performs the identical rewrite soundly (it runs both before and
    // immediately after this pass), so nothing is lost here.
    match op {
        IrUnaryOp::Clz | IrUnaryOp::Ctz => Some(*true_val),
        _ => None,
    }
}

/// Zero-value replacement (GCC phiopt's `value_replacement`, generalised to
/// an expression arm): `select(x, T, C)` is `T` when `T` evaluated at
/// `x == 0` is exactly `C`. The select picks `T` for every nonzero `x`, and
/// for `x == 0` both arms agree. `select(x, 31 - clz(x), -1)` (zstd's and the
/// kernel's highest-set-bit index with -1 for zero) collapses because the IR
/// defines `Clz(0) == 32`; so does `x ? 32 - clz(x) : 0` (`fls`), while
/// `x ? clz(x) ^ 31 : -1` does not (`32 ^ 31 == 63`) and keeps its select.
/// The bare `x ? clz(x) : W` is the special case that
/// `clz_ctz_zero_guard_replacement` already handles.
///
/// Nothing is speculated that was not already: a `Select` evaluates `T`
/// unconditionally. The evaluation (`eval_at_zero`) follows the IR semantics
/// width-exactly and refuses everything it does not model: the NonZero
/// intrinsics (no value at zero — their guard is what makes them safe),
/// shift amounts outside the type, division, any value other than `x` and
/// its integer casts, and trees over a small node budget.
fn zero_value_replacement(
    cond: &Operand,
    true_val: &Operand,
    false_val: &Operand,
    ty: IrType,
    cast_defs: &[Option<CastDef>],
    unary_defs: &[Option<UnaryDef>],
    binop_defs: &[Option<BinOpDef>],
) -> Option<Operand> {
    let Operand::Value(_) = cond else { return None };
    let Operand::Value(_) = true_val else {
        return None;
    };
    let Operand::Const(c) = false_val else {
        return None;
    };
    if !ty.is_integer() || ty.size() == 0 || ty.size() > 8 {
        return None;
    }
    // Same-size casts only: a truncation could zero a nonzero condition.
    let x = peel_int_cast_chain(*cond, cast_defs, true);
    let Operand::Value(_) = x else { return None };
    let mut budget = 16u32;
    let at_zero = eval_at_zero(
        *true_val,
        ty,
        x,
        cast_defs,
        unary_defs,
        binop_defs,
        &mut budget,
    )?;
    let mask = width_mask(ty.size() as u32 * 8);
    let c = (c.to_i128()? as u128) & mask;
    (at_zero == c).then_some(*true_val)
}

fn width_mask(bits: u32) -> u128 {
    if bits >= 128 {
        u128::MAX
    } else {
        (1u128 << bits) - 1
    }
}

/// Value of `op` (an operand of IR type `ty`) when `x == 0`, as the low
/// `ty`-width bits; see `zero_value_replacement`. `x` and every integer cast
/// of it are 0 there, whatever the widths.
fn eval_at_zero(
    op: Operand,
    ty: IrType,
    x: Operand,
    cast_defs: &[Option<CastDef>],
    unary_defs: &[Option<UnaryDef>],
    binop_defs: &[Option<BinOpDef>],
    budget: &mut u32,
) -> Option<u128> {
    *budget = budget.checked_sub(1)?;
    if !ty.is_integer() || ty.size() == 0 || ty.size() > 8 {
        return None;
    }
    let bits = ty.size() as u32 * 8;
    let mask = width_mask(bits);
    let v = match op {
        Operand::Const(c) => return Some((c.to_i128()? as u128) & mask),
        Operand::Value(v) => v,
    };
    if peel_int_cast_chain(op, cast_defs, false) == x {
        return Some(0);
    }
    let i = v.0 as usize;
    if let Some(Some(u)) = unary_defs.get(i) {
        let w = u.ty.size() as u32 * 8;
        if !u.ty.is_integer() || w == 0 || w > 64 {
            return None;
        }
        let a = eval_at_zero(u.src, u.ty, x, cast_defs, unary_defs, binop_defs, budget)?;
        let a = a as u64 & (width_mask(w) as u64);
        let r = match u.op {
            IrUnaryOp::Clz if a == 0 => w,
            IrUnaryOp::Clz => a.leading_zeros() - (64 - w),
            IrUnaryOp::Ctz if a == 0 => w,
            IrUnaryOp::Ctz => a.trailing_zeros(),
            _ => return None,
        };
        return Some(u128::from(r) & mask);
    }
    if let Some(Some(cd)) = cast_defs.get(i) {
        let (from, to) = (cd.from_ty, cd.to_ty);
        if !from.is_integer() || !to.is_integer() || from.size() == 0 || from.size() > 8 {
            return None;
        }
        let a = eval_at_zero(cd.src, from, x, cast_defs, unary_defs, binop_defs, budget)?;
        let fbits = from.size() as u32 * 8;
        let widened = if from.is_signed() && fbits < 128 && (a >> (fbits - 1)) & 1 == 1 {
            a | !width_mask(fbits)
        } else {
            a
        };
        return Some(widened & width_mask(to.size() as u32 * 8) & mask);
    }
    if let Some(Some(b)) = binop_defs.get(i) {
        let bty = b.ty;
        let bbits = bty.size() as u32 * 8;
        if !bty.is_integer() || bbits == 0 || bbits > 64 {
            return None;
        }
        let bmask = width_mask(bbits);
        let l = eval_at_zero(b.lhs, bty, x, cast_defs, unary_defs, binop_defs, budget)?;
        let r = eval_at_zero(b.rhs, bty, x, cast_defs, unary_defs, binop_defs, budget)?;
        let sext = |a: u128| -> i128 {
            if (a >> (bbits - 1)) & 1 == 1 {
                (a | !bmask) as i128
            } else {
                a as i128
            }
        };
        let res = match b.op {
            IrBinOp::Add => l.wrapping_add(r),
            IrBinOp::Sub => l.wrapping_sub(r),
            IrBinOp::Mul => l.wrapping_mul(r),
            IrBinOp::And => l & r,
            IrBinOp::Or => l | r,
            IrBinOp::Xor => l ^ r,
            IrBinOp::Shl if r < u128::from(bbits) => l << r,
            IrBinOp::LShr if r < u128::from(bbits) => l >> r,
            IrBinOp::AShr if r < u128::from(bbits) => (sext(l) >> r) as u128,
            _ => return None,
        };
        return Some(res & bmask & mask);
    }
    None
}

/// Cached information about a BinOp instruction for constant reassociation.
/// We only store BinOps where at least one operand is a constant, since
/// reassociation requires folding two constants together.
#[derive(Clone, Copy)]
struct BinOpDef {
    op: IrBinOp,
    lhs: Operand,
    rhs: Operand,
    ty: IrType,
}

/// Try to simplify an instruction using algebraic identities and strength reduction.
/// Bisection helper for the preboot-ZSTD miscompile. `CCC_SIMPLIFY_SKIP` is a
/// comma-separated list of fold families to disable:
/// `bittest`, `reassoc`, `cast`, `cast_ident`, `gep`, `cmp`, `select`, `ident`,
/// `negneg`, `addneg`, `cast_narrow`, `cast_widen`, `cast_same`, `cast_mix`,
/// `pack_extract`, `rotate_chain`, `widened_cond`, `zeroneg`, `negcast`,
/// `notcast`.
/// `zeroneg` (`0 - x` => `Neg`), `negcast` (a truncation distributes over a
/// negate) and `notcast` (a truncation distributes over a complement) are the
/// pieces of a narrow unary op; each is measured on its own by
/// `check_narrow_negate_fold.sh`.
fn simplify_skip(family: &str) -> bool {
    static SKIP: std::sync::LazyLock<String> =
        std::sync::LazyLock::new(|| std::env::var("CCC_SIMPLIFY_SKIP").unwrap_or_default());
    if SKIP.is_empty() {
        return false;
    }
    SKIP.split(',').any(|s| s.trim() == family)
}

fn try_simplify_with_types(
    inst: &Instruction,
    cast_defs: &[Option<CastDef>],
    gep_defs: &[Option<GepDef>],
    cmp_defs: &[Option<CmpDef>],
    binop_defs: &[Option<BinOpDef>],
    neg_defs: &[Option<NegDef>],
    unary_defs: &[Option<UnaryDef>],
    not_defs: &[Option<Operand>],
    is_boolean: &[bool],
    value_types: &[Option<IrType>],
    use_counts: &[u32],
    cast_use_counts: &[u32],
    cast_dies_if_narrowed: &[bool],
) -> Option<Instruction> {
    match inst {
        Instruction::BinOp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        } => simplify_binop(
            *dest, *op, lhs, rhs, *ty, cast_defs, binop_defs, neg_defs, use_counts,
        ),
        Instruction::Cast {
            dest,
            src,
            from_ty,
            to_ty,
        } => simplify_cast(
            *dest,
            src,
            *from_ty,
            *to_ty,
            cast_defs,
            binop_defs,
            neg_defs,
            not_defs,
            value_types,
        ),
        Instruction::GetElementPtr {
            dest,
            base,
            offset,
            ty,
        } => simplify_gep(*dest, *base, offset, *ty, gep_defs),
        Instruction::Cmp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        } => fold_sign_region_mask_test(*dest, *op, lhs, rhs, *ty, binop_defs, cast_defs).or_else(
            || {
                simplify_cmp(
                    *dest,
                    *op,
                    lhs,
                    rhs,
                    *ty,
                    cmp_defs,
                    cast_defs,
                    is_boolean,
                    cast_use_counts,
                    cast_dies_if_narrowed,
                )
            },
        ),
        Instruction::Load {
            dest,
            ptr,
            ty,
            seg_override,
            volatile,
        } => {
            // Load through GEP+0 is a load of the base. Complements the
            // address-of lowering that no longer emits GEP+0, and also
            // catches offset-0 field / helper-inlined stores that still
            // produce a zero GEP.
            let ptr_idx = ptr.0 as usize;
            if let Some(Some(gep)) = gep_defs.get(ptr_idx) {
                if is_zero(&gep.offset) {
                    return Some(Instruction::Load {
                        dest: *dest,
                        ptr: gep.base,
                        ty: *ty,
                        seg_override: *seg_override,
                        volatile: *volatile,
                    });
                }
            }
            None
        }
        Instruction::Store {
            val,
            ptr,
            ty,
            seg_override,
            volatile,
        } => {
            let ptr_idx = ptr.0 as usize;
            if let Some(Some(gep)) = gep_defs.get(ptr_idx) {
                if is_zero(&gep.offset) {
                    return Some(Instruction::Store {
                        val: *val,
                        ptr: gep.base,
                        ty: *ty,
                        seg_override: *seg_override,
                        volatile: *volatile,
                    });
                }
            }
            None
        }
        Instruction::Select {
            dest,
            cond,
            true_val,
            false_val,
            ty,
        } => {
            // select cond, x, x => x (both arms are the same)
            if same_operand(true_val, false_val) {
                return Some(Instruction::Copy {
                    dest: *dest,
                    src: *true_val,
                });
            }
            // select Const(0), a, b => b (constant false condition)
            // select Const(nonzero), a, b => a (constant true condition)
            if let Operand::Const(c) = cond {
                if c.is_zero() {
                    return Some(Instruction::Copy {
                        dest: *dest,
                        src: *false_val,
                    });
                } else {
                    return Some(Instruction::Copy {
                        dest: *dest,
                        src: *true_val,
                    });
                }
            }
            // select(cond != 0 -> clz/ctz(x), else W) => clz/ctz(x): the IR
            // defines Clz(0)/Ctz(0) == width, so the intrinsic arm already
            // produces W for zero and the guarded ternary is redundant.
            if let Some(replacement) = safe_clz_guard_replacement(
                *dest, *cond, *true_val, *false_val, *ty, cast_defs, unary_defs, binop_defs,
            ) {
                return Some(replacement);
            }
            if let Some(t) =
                clz_ctz_zero_guard_replacement(cond, true_val, false_val, cast_defs, unary_defs)
            {
                return Some(Instruction::Copy {
                    dest: *dest,
                    src: t,
                });
            }
            None
        }
        // Only a call that reaches the C library function has its semantics
        // (`CallInfo::assume_library`; false under `-fno-builtin[-<fn>]`).
        Instruction::Call { func, info } if info.assume_library => {
            if let Some(d) = info.dest {
                simplify_math_call(d, func, &info.args, info.return_type)
            } else {
                None
            }
        }
        Instruction::UnaryOp { dest, op, src, .. } => {
            // neg(neg(x)) => x. Exact in both domains: float Neg is the
            // sign-bit XOR, so the double flip is a BITWISE identity —
            // NaN payloads, NaN signs and ±0 all survive exactly; integer
            // Neg is its own inverse in the two's-complement ring (INT_MIN
            // included). The rewrite is a pure work deletion (the folded
            // Neg chain loses its only reader or keeps exactly the readers
            // it had), so no use-count profitability guard applies.
            // Types line up by construction: the outer Neg reads the inner
            // Neg's value at the inner's SSA type (the same invariant the
            // `x - (neg y) => x + y` fold below already relies on).
            // Chains collapse in ONE sweep: the def map links every Neg to
            // its original source, so neg^3 folds to neg(x) + copies and
            // neg^4 to x + copies without re-running the pass.
            if *op == IrUnaryOp::Neg && !simplify_skip("negneg") {
                if let Some(inner) = get_neg_def(src, neg_defs) {
                    return Some(Instruction::Copy {
                        dest: *dest,
                        src: inner,
                    });
                }
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
fn try_simplify(
    inst: &Instruction,
    cast_defs: &[Option<CastDef>],
    gep_defs: &[Option<GepDef>],
    cmp_defs: &[Option<CmpDef>],
    binop_defs: &[Option<BinOpDef>],
    neg_defs: &[Option<NegDef>],
    is_boolean: &[bool],
) -> Option<Instruction> {
    try_simplify_with_types(
        inst,
        cast_defs,
        gep_defs,
        cmp_defs,
        binop_defs,
        neg_defs,
        &[],
        &[],
        is_boolean,
        &[],
        &[],
        &[],
        &[],
    )
}

/// Test wrapper for the Neg-def-sensitive folds (neg-of-neg,
/// add/sub-of-neg): threads the NegDef map with everything else empty.
#[cfg(test)]
fn try_simplify_neg(inst: &Instruction, neg_defs: &[Option<NegDef>]) -> Option<Instruction> {
    try_simplify_with_types(
        inst,
        &[],
        &[],
        &[],
        &[],
        neg_defs,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
    )
}

/// Simplify a Cast instruction.
///
/// Handles:
/// - Cast from type T to same type T => Copy
/// - Cast chain: Cast(Cast(x, A->B), B->C) optimizations
/// - Cast of constant => constant (fold at compile time)
fn simplify_cast(
    dest: Value,
    src: &Operand,
    from_ty: IrType,
    to_ty: IrType,
    def_map: &[Option<CastDef>],
    binop_defs: &[Option<BinOpDef>],
    neg_defs: &[Option<NegDef>],
    not_defs: &[Option<Operand>],
    value_types: &[Option<IrType>],
) -> Option<Instruction> {
    // Identity cast: same type
    if from_ty == to_ty {
        return Some(Instruction::Copy { dest, src: *src });
    }

    // Two-register aggregate carrier: a half extracted from the I128 pack it
    // was just built from is the narrow value that went in.
    if !simplify_skip("pack_extract") {
        if let Some((half, half_ty)) =
            packed_range_source(src, from_ty, to_ty, def_map, binop_defs, value_types)
        {
            if half_ty == to_ty {
                return Some(Instruction::Copy { dest, src: half });
            }
            return Some(Instruction::Cast {
                dest,
                src: half,
                from_ty: half_ty,
                to_ty,
            });
        }
    }

    // Some lowering paths conservatively annotate an integer expression as
    // coming from the target's evaluation width (typically I64), even though
    // its defining IR instruction already produced the requested narrow type.
    // In that case the truncation is semantically an identity and retaining it
    // creates needless extend/truncate moves and hides fusion opportunities.
    if !simplify_skip("cast_ident") {
        if let Operand::Value(v) = src {
            if value_types.get(v.0 as usize).copied().flatten() == Some(to_ty) {
                return Some(Instruction::Copy { dest, src: *src });
            }
        }
    }

    // A negation sandwiched between an extension and a truncation:
    // `(T)(- (W)(T)v)` is `-(T)v`.  The extension's high bits die in the
    // truncation and negation is a ring operation, so it commutes with both;
    // what is left is the negate at the width the result is observed at.
    //
    // This is not cosmetic.  A 64-bit negate is never cheaper than a 32-bit
    // one, so `trunc32(neg64(ext64(v)))` is pure overhead: `-(x != 0)` used to
    // be `setne; movzbl; movslq; negq; movl` where `testl; setne; negl` is the
    // whole answer (GCC: 3, LCCC 6 either way around the intermediate).
    //
    // The fold is deliberately restricted to a 64-bit source and a 32-bit
    // destination, which is the ONE width pair where it measured as a pure
    // win.  The two shapes it does not touch were measured too, and both were
    // net pessimisations on the 2,791-file sweep:
    //
    //   * a 32-bit source (`trunc16(neg32(ext32(v)))`, the packed-mask idiom
    //     `(u16)(0 - (u16)(a < b))`) removes two instructions per site but
    //     flipped the vectorizer's decision in `packed_mask_shifts.c` to a
    //     worse 128-bit form: +60 instructions over four functions, versus
    //     -1 or -2 at each of the scalar sites;
    //   * a 16-bit destination costs a partial-register negate whose result the
    //     consumers must re-extend, and register allocation paid for it
    //     (`bb_slp_unary_demote.c` +3, `sib_index_narrow_sext.c` +1, both
    //     spills rather than folds).
    //
    // Narrow destinations are therefore left alone; a narrow negation reaches
    // them as the wide negate plus the truncation the backend already had.
    if !simplify_skip("negcast")
        && from_ty.is_integer()
        && to_ty.is_integer()
        && from_ty.size() == 8
        && to_ty.size() == 4
        && let Some(neg_src) = get_neg_def(src, neg_defs)
    {
        // Either the negate already reads a `to_ty`-wide value (the extension
        // above it is then the one the truncation undoes), or it reads a cast
        // that extends `to_ty` to the negate's width.  Both leave `-(T)v`.
        let narrowed = match neg_src {
            Operand::Value(v)
                if value_types.get(v.0 as usize).copied().flatten() == Some(to_ty) =>
            {
                Some(neg_src)
            }
            Operand::Value(v) => def_map
                .get(v.0 as usize)
                .and_then(|d| d.as_ref())
                .filter(|d| d.from_ty == to_ty && d.to_ty == from_ty)
                .map(|d| d.src),
            Operand::Const(_) => None,
        };
        if let Some(inner) = narrowed {
            return Some(Instruction::UnaryOp {
                dest,
                op: IrUnaryOp::Neg,
                src: inner,
                ty: to_ty,
            });
        }
    }

    // The complement's twin: `(T)(~(W)(T)v)` is `~(T)v`.
    //
    // `Not` is bitwise, so it commutes with truncation outright -- the low
    // bits of `~(W)(ext v)` ARE `~v`, whatever the extension put in the high
    // ones -- which is why, unlike the negate above, this fold needs no width
    // pair restriction and no signedness argument.  What it does need is a
    // NARROWING outer cast: `ext(~v)` and `~ext(v)` differ in the high bits,
    // so a widening step cannot be dropped.
    //
    // Measured: `x & ~(1 << n)` used to widen the `~` to 64 bits for no
    // reason (`shll; cltq; notq; andl`, 4 instructions for the complement)
    // because the extension was materialized around the `Not`; the truncated
    // complement is `notl` (one instruction).
    if !simplify_skip("notcast")
        && from_ty.is_integer()
        && to_ty.is_integer()
        && to_ty.size() < from_ty.size()
        && let Some(not_src) = get_not_def(src, not_defs)
    {
        let narrowed = match not_src {
            Operand::Value(v)
                if value_types.get(v.0 as usize).copied().flatten() == Some(to_ty) =>
            {
                Some(not_src)
            }
            Operand::Value(v) => def_map
                .get(v.0 as usize)
                .and_then(|d| d.as_ref())
                .filter(|d| d.from_ty == to_ty && d.to_ty == from_ty)
                .map(|d| d.src),
            Operand::Const(_) => None,
        };
        if let Some(inner) = narrowed {
            return Some(Instruction::UnaryOp {
                dest,
                op: IrUnaryOp::Not,
                src: inner,
                ty: to_ty,
            });
        }
    }

    // Constant folding: cast of a constant => new constant
    if let Operand::Const(c) = src {
        if let Some(folded) = fold_const_cast(c, from_ty, to_ty) {
            return Some(Instruction::Copy {
                dest,
                src: Operand::Const(folded),
            });
        }
    }

    // Cast chain optimization: if src is defined by another Cast, try to fold.
    // Only handle the widen-then-narrow-back case which is safe and common.
    if simplify_skip("cast") {
        return None;
    }
    if let Operand::Value(v) = src {
        let idx = v.0 as usize;
        if let Some(Some(inner_cast)) = def_map.get(idx) {
            let inner_from = inner_cast.from_ty;
            let inner_to = inner_cast.to_ty;
            let inner_src = inner_cast.src;

            // Chain consistency: the inner cast must produce the bits this
            // cast reads. Two integer types of one width are the same bits
            // (`Cast(Cast(x, U32->I64), U64->I16)`: aggregate SROA's covering
            // loads build U64 views of I64 values and vice versa), and every
            // rule below is phrased in THIS cast's `from_ty` -- the only
            // place B's signedness matters is a widening outer step, whose
            // sext/zext choice is exactly `from_ty`'s.
            let same_bits =
                inner_to.is_integer() && from_ty.is_integer() && inner_to.size() == from_ty.size();
            if inner_to == from_ty || same_bits {
                // Widen then narrow back to exact same type.
                //   Cast(Cast(x, A->B), B->A) => Copy of x
                // ONLY for unsigned A. Signed I32/I16/I8 homes are frequently
                // zero-extended in the 64-bit register (`movl` / 32-bit ALU);
                // the round-trip sign-extends (`movslq`). Collapsing to Copy
                // keeps the zext high bits, so a later 64-bit use of that
                // "I32" (pointer += length, size_t mix, shift count) sees
                // 0x00000000FFFFFFFF instead of -1. That miscompiled the
                // kernel preboot ZSTD decoder at -O1+ (`ZSTD-compressed
                // data is corrupt` / SEGV on patterned payloads ≥ 320 B).
                // Unsigned round-trips are already zext-canonical, so Copy
                // is a strict win there.
                if inner_from == to_ty
                    && inner_from.is_unsigned()
                    && from_ty.is_integer()
                    && inner_from.size() <= from_ty.size()
                    && !simplify_skip("cast_same")
                {
                    return Some(Instruction::Copy {
                        dest,
                        src: inner_src,
                    });
                }

                // Double widen: Cast(Cast(x, A->B), B->C) where A < B < C (all ints)
                // => Cast(x, A->C) - skip intermediate.
                // ONLY safe when A and B have the same signedness, because the
                // extension from B to C uses B's signedness. If we skip B and go
                // directly A->C, the extension uses A's signedness instead. When
                // A and B differ in signedness (e.g., I8->U16->I32), the
                // intermediate unsigned type changes the extension behavior and
                // skipping it would be incorrect.
                if inner_from.is_integer() && from_ty.is_integer() && to_ty.is_integer() {
                    let a = inner_from.size();
                    let b = from_ty.size();
                    let c = to_ty.size();
                    if a < b && b < c && !simplify_skip("cast_widen") {
                        // Only safe when inner source and intermediate have same signedness
                        let same_sign = inner_from.is_signed() == from_ty.is_signed()
                            || inner_from.is_unsigned() == from_ty.is_unsigned();
                        if same_sign {
                            return Some(Instruction::Cast {
                                dest,
                                src: inner_src,
                                from_ty: inner_from,
                                to_ty,
                            });
                        }
                    }

                    // Double narrow: Cast(Cast(x, A->B), B->C) where A > B > C (all ints)
                    // => Cast(x, A->C) - skip intermediate narrow.
                    // Safe because narrowing only keeps the low bits, so
                    // narrow(narrow(x, A->B), B->C) = narrow(x, A->C) regardless
                    // of signedness (both just truncate to C bits).
                    //
                    // Not from a 128-bit A: no target has a single-register
                    // home for one (x86-64 pairs/slots and the wide-hazard
                    // points of `regalloc`, i686's 64-bit pair model, AArch64
                    // and RISC-V register pairs), so composing moves a read
                    // ONTO the I128 and stretches its web to this point, while
                    // the intermediate B (<= 64 bits) it saves is a free
                    // subregister read. Two-register carriers hit this on
                    // every member narrower than its eightbyte: the low half
                    // `trunc64(v)` already exists, and `trunc32(trunc64(v))`
                    // must not become a second I128 reader.
                    if a > b && b > c && a < 16 && !simplify_skip("cast_narrow") {
                        return Some(Instruction::Cast {
                            dest,
                            src: inner_src,
                            from_ty: inner_from,
                            to_ty,
                        });
                    }

                    // Widen then narrow, B strictly widest: Cast(Cast(x, A->B), B->C)
                    // with B > A and B > C (all ints) => Cast(x, A->C).
                    // The widening A->B preserves every A bit (zero- or
                    // sign-extended), and narrowing B->C discards only high bits,
                    // so the result is exactly extend-then-truncate(x) which
                    // equals Cast(x, A->C) for any signedness and any C.
                    // (Covers the C-promotion idiom the range-check fold emits:
                    // bool I8 -> I64 -> I32 collapses to bool I8 -> I32.)
                    // A->B->A is exclusively `cast_same` (unsigned Copy). Folding
                    // it here to Cast(A->A) identity-Copies on the next pass
                    // and reintroduces the signed-home miscompile.
                    if b > a && b > c && inner_from != to_ty && !simplify_skip("cast_mix") {
                        let same_sign = inner_from.is_signed() == to_ty.is_signed()
                            || inner_from.is_unsigned() == to_ty.is_unsigned();
                        // Unsigned A is always zext-canonical, so A->B->C
                        // matches Cast(A->C) for any C (bool U8->I64->I32).
                        if c <= a || same_sign || inner_from.is_unsigned() {
                            return Some(Instruction::Cast {
                                dest,
                                src: inner_src,
                                from_ty: inner_from,
                                to_ty,
                            });
                        }
                    }
                }
            }
        }
    }

    None
}

/// `trunc_T(x)` where the low `|T|` bits of `x` provably come from one
/// narrower value: return that value and its type, so the truncation can read
/// it directly (`cast_{A->T}(src)`).
///
/// The producers are bit packs that are taken apart again in the same
/// function. The two-register aggregate carrier (`unpack_two_reg_return`: a
/// 9..=16-byte struct returned in RAX:RDX, x0:x1 or a0:a1 travels as I128)
/// leaves the callee's pack and the caller's unpack side by side once the
/// callee is inlined; aggregate SROA's covering accesses (rule 3b, `I64` over
/// `{u32, u32}`) build `or(zext(f_i) << 8*rel_i)` packs that the members'
/// readers take apart. The pack is pure overhead either way, and an I128 one
/// is worse: any I128 value makes the x86-64 backend withdraw
/// r8/r9/rdi/rsi/rdx from the general caller-saved pool for the whole
/// function (`x86_body_has_i128_ops`). In zstd's `ZSTD_compressBlock_opt2`
/// six inlined `ZSTD_newRep` carriers left a 2,700-value function two
/// caller-saved registers. Once every extract reads its source, DCE deletes
/// the pack.
///
/// `BitDefs::range_source` walks the def chain of `x` for the bit range `[0, |T|)`
/// (W-bit integers, W in 32/64/128, depth-bounded):
///
/// * `or(a, b)`: the range comes from `b` when `a` is provably zero there
///   (`BitDefs::bits_known_zero`), and vice versa;
/// * `shl(y, S)`: bits `[lo, lo+t)` are bits `[lo-S, lo-S+t)` of `y` when
///   `lo >= S`;
/// * `lshr`/`ashr(y, S)`: bits `[lo+S, lo+S+t)` of `y`, provided they lie
///   inside `y` (so an arithmetic fill never enters the range);
/// * `and(y, M)`: transparent when `M` is all ones on the range;
/// * `ext_A(y)` (a widening cast): at `lo = 0`, `trunc_t(ext_A(y)) =
///   cast_{A->T}(y)` for every `t <= W` (extension semantics stay A's); a
///   narrowing cast is transparent for in-range bits, except out of a
///   128-bit value (that would add an I128 reader, see `cast_narrow`);
/// * any value at `lo = 0` is itself the answer (a plain truncation), unless
///   it is 128 bits wide (no new I128 readers; see the end of
///   `packed_range_source`).
///
/// The walk must start at a bit operation (plain `trunc(ext(x))` chains keep
/// `simplify_cast`'s signed round-trip guard), so a rewrite always reads a
/// different value than before. 128-bit packs need a
/// 64-bit target: i686 models I128 as its low 64-bit pair, where shifts by
/// 64 and bits above 64 do not mean what this algebra assumes.
fn packed_range_source(
    src: &Operand,
    from_ty: IrType,
    to_ty: IrType,
    cast_defs: &[Option<CastDef>],
    binop_defs: &[Option<BinOpDef>],
    value_types: &[Option<IrType>],
) -> Option<(Operand, IrType)> {
    let w = bits_of(from_ty);
    if !matches!(w, 32 | 64 | 128) || !from_ty.is_integer() || !to_ty.is_integer() {
        return None;
    }
    if w == 128 && crate::common::types::target_is_32bit() {
        return None;
    }
    let t = bits_of(to_ty);
    if t >= w {
        return None;
    }
    let defs = BitDefs {
        cast_defs,
        binop_defs,
        value_types,
    };
    // Plain cast chains (`trunc(ext(x))`) stay with `simplify_cast`'s own
    // rules, whose signed round-trip guard (`cast_same`) is deliberate: the
    // walk only starts at a bit operation, i.e. at a pack.
    defs.binop(src)?;
    let (found, ty) = defs.range_source(src, from_ty, 0, t, 0)?;
    // Never hand back a 128-bit value: that adds an I128 reader (the
    // `cast_narrow` rule), and 128-bit SSA values include `__m128`-style
    // vectors whose XMM homes leave the 16-byte slot an I128 read uses
    // unwritten (`trunc64(and(v, m))` -> `trunc64(v)` read a stale slot in
    // the SSE wrapper tests). Carrier members are at most 64 bits wide.
    if bits_of(ty) >= 128 {
        return None;
    }
    (found != *src).then_some((found, ty))
}

fn bits_of(ty: IrType) -> i64 {
    (ty.size() * 8) as i64
}

/// Def lookups for the bit-range walk.
struct BitDefs<'a> {
    cast_defs: &'a [Option<CastDef>],
    binop_defs: &'a [Option<BinOpDef>],
    value_types: &'a [Option<IrType>],
}

/// Depth bound of the bit-range walk: a 16-byte carrier over byte fields
/// nests eight `or`s; anything deeper is not a pack worth taking apart.
const BIT_WALK_DEPTH: u32 = 12;

impl BitDefs<'_> {
    fn binop(&self, op: &Operand) -> Option<BinOpDef> {
        let Operand::Value(v) = op else { return None };
        self.binop_defs.get(v.0 as usize).copied().flatten()
    }

    fn cast(&self, op: &Operand) -> Option<CastDef> {
        let Operand::Value(v) = op else { return None };
        self.cast_defs.get(v.0 as usize).copied().flatten()
    }

    fn shift(op: &Operand, w: i64) -> Option<i64> {
        let Operand::Const(c) = op else { return None };
        c.to_i64().filter(|&k| 0 < k && k < w)
    }

    /// Whether bits `[lo, lo+t)` of `x` (a `w`-bit integer) are provably 0.
    fn bits_known_zero(&self, x: &Operand, w: i64, lo: i64, t: i64, depth: u32) -> bool {
        if depth > BIT_WALK_DEPTH || lo < 0 || t <= 0 {
            return false;
        }
        if lo + t > w {
            return false;
        }
        if let Operand::Const(c) = x {
            return const_range_is(c, lo, t, false);
        }
        if let Some(b) = self.binop(x) {
            if bits_of(b.ty) != w {
                return false;
            }
            return match b.op {
                IrBinOp::Shl => Self::shift(&b.rhs, w).is_some_and(|s| {
                    lo + t <= s
                        || (lo >= s && self.bits_known_zero(&b.lhs, w, lo - s, t, depth + 1))
                }),
                IrBinOp::LShr => Self::shift(&b.rhs, w).is_some_and(|s| {
                    lo >= w - s
                        || (lo + t <= w - s
                            && self.bits_known_zero(&b.lhs, w, lo + s, t, depth + 1))
                }),
                IrBinOp::And => {
                    let zero = |o: &Operand| matches!(o, Operand::Const(c) if const_range_is(c, lo, t, false));
                    zero(&b.rhs)
                        || zero(&b.lhs)
                        || self.bits_known_zero(&b.lhs, w, lo, t, depth + 1)
                        || self.bits_known_zero(&b.rhs, w, lo, t, depth + 1)
                }
                IrBinOp::Or => {
                    self.bits_known_zero(&b.lhs, w, lo, t, depth + 1)
                        && self.bits_known_zero(&b.rhs, w, lo, t, depth + 1)
                }
                _ => false,
            };
        }
        if let Some(c) = self.cast(x) {
            if bits_of(c.to_ty) != w || !c.from_ty.is_integer() {
                return false;
            }
            let a = bits_of(c.from_ty);
            if a < w {
                // Zero extension: everything above the source is zero.
                if c.from_ty.is_unsigned() && lo >= a {
                    return true;
                }
                return lo + t <= a && self.bits_known_zero(&c.src, a, lo, t, depth + 1);
            }
            return self.bits_known_zero(&c.src, a, lo, t, depth + 1);
        }
        false
    }

    /// The value whose low `t` bits are bits `[lo, lo+t)` of `x` (a `ty`-typed
    /// integer), reached only at `lo == 0`.
    fn range_source(
        &self,
        x: &Operand,
        ty: IrType,
        lo: i64,
        t: i64,
        depth: u32,
    ) -> Option<(Operand, IrType)> {
        let w = bits_of(ty);
        if depth > BIT_WALK_DEPTH || lo < 0 || lo + t > w || !ty.is_integer() {
            return None;
        }
        if let Some(b) = self.binop(x) {
            if bits_of(b.ty) != w {
                return None;
            }
            let step = match b.op {
                IrBinOp::Or => {
                    if self.bits_known_zero(&b.lhs, w, lo, t, depth + 1) {
                        self.range_source(&b.rhs, b.ty, lo, t, depth + 1)
                    } else if self.bits_known_zero(&b.rhs, w, lo, t, depth + 1) {
                        self.range_source(&b.lhs, b.ty, lo, t, depth + 1)
                    } else {
                        None
                    }
                }
                IrBinOp::Shl => Self::shift(&b.rhs, w)
                    .filter(|&s| lo >= s)
                    .and_then(|s| self.range_source(&b.lhs, b.ty, lo - s, t, depth + 1)),
                IrBinOp::LShr | IrBinOp::AShr => Self::shift(&b.rhs, w)
                    .filter(|&s| lo + s + t <= w)
                    .and_then(|s| self.range_source(&b.lhs, b.ty, lo + s, t, depth + 1)),
                IrBinOp::And => {
                    let ones = |o: &Operand| matches!(o, Operand::Const(c) if const_range_is(c, lo, t, true));
                    if ones(&b.rhs) {
                        self.range_source(&b.lhs, b.ty, lo, t, depth + 1)
                    } else if ones(&b.lhs) {
                        self.range_source(&b.rhs, b.ty, lo, t, depth + 1)
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if step.is_some() {
                return step;
            }
        }
        if let Some(c) = self.cast(x) {
            if bits_of(c.to_ty) == w && c.from_ty.is_integer() {
                let a = bits_of(c.from_ty);
                if a < w && lo == 0 {
                    return Some((c.src, c.from_ty));
                }
                // A narrowing cast is transparent for in-range bits, but not
                // out of a 128-bit value: that would add an I128 reader
                // (see `cast_narrow` in `simplify_cast`).
                if lo + t <= a && a < 128 {
                    if let Some(r) = self.range_source(&c.src, c.from_ty, lo, t, depth + 1) {
                        return Some(r);
                    }
                }
            }
        }
        if lo == 0 {
            let vty = match x {
                Operand::Value(v) => self.value_types.get(v.0 as usize).copied().flatten(),
                Operand::Const(_) => None,
            };
            if vty.is_some_and(|vt| vt.is_integer() && bits_of(vt) == w) {
                return Some((*x, ty));
            }
        }
        None
    }
}

/// Whether bits `[lo, lo+t)` of an integer constant operand are all ones
/// (`ones`) or all zeros, counting only bits every backend agrees on.
///
/// A constant narrower than its operation is extended by the backend, and
/// not uniformly: x86-64 zero-extends a non-I128 constant into the high half
/// of a 128-bit operand (`operand_to_rax_rdx`: `xorl %edx, %edx`), while a
/// 64-bit `imm32` sign-extends. Bits above the constant's own width are
/// therefore known only when both readings agree -- a non-negative constant
/// (zeros either way). Reading `and(x, I64(-1))` in an I128 as an all-ones
/// mask passed `x`'s high half through a mask that clears it (the SSE
/// wrapper regressions `agg_fwd_param_home`, `simd_sse_float`,
/// `builtin_ia32_vector_value`).
fn const_range_is(c: &IrConst, lo: i64, t: i64, ones: bool) -> bool {
    let (v, width): (i128, i64) = match *c {
        IrConst::I8(v) => (v.into(), 8),
        IrConst::I16(v) => (v.into(), 16),
        IrConst::I32(v) => (v.into(), 32),
        IrConst::I64(v) => (v.into(), 64),
        IrConst::I128(v) => (v, 128),
        IrConst::Zero => (0, 128),
        _ => return false,
    };
    if lo + t > width && v < 0 {
        return false;
    }
    let bits = (v as u128 >> lo) & range_mask(t);
    if ones {
        bits == range_mask(t)
    } else {
        bits == 0
    }
}

/// `t` low bits set (`t <= 128`).
fn range_mask(t: i64) -> u128 {
    if t >= 128 {
        u128::MAX
    } else {
        (1u128 << t) - 1
    }
}

/// Fold a cast of a constant at compile time.
/// Returns the new constant if the cast can be folded, None otherwise.
///
/// `from_ty` is needed because IrConst::to_i64() always sign-extends from
/// the storage type (I8/I16/I32/I64), but unsigned source types require
/// zero-extension. For example, IrConst::I8(-1) with from_ty=U8 represents
/// the value 255, not -1.
fn fold_const_cast(c: &IrConst, from_ty: IrType, to_ty: IrType) -> Option<IrConst> {
    // Handle I128 source directly to avoid truncation through to_i64().
    // to_i64() truncates the upper 64 bits, which loses data for values like (U)1 << 127.
    if let IrConst::I128(v128) = c {
        return fold_i128_cast(*v128, from_ty, to_ty);
    }

    // Integer-to-integer/float constant cast
    if let Some(raw_val) = c.to_i64() {
        // Normalize source value according to from_ty signedness.
        // Signed types sign-extend; unsigned types zero-extend.
        let val = from_ty.truncate_i64(raw_val);
        return fold_int_cast(val, from_ty, to_ty);
    }

    // Float-to-other constant cast
    // For LongDouble, use full x87 precision for integer targets
    if let IrConst::LongDouble(_, bytes) = c {
        return IrConst::cast_long_double_to_target(bytes, to_ty);
    }
    if let Some(fval) = c.to_f64() {
        return IrConst::cast_float_to_target(fval, to_ty);
    }

    None
}

/// Convert an i128 constant to the target type.
fn fold_i128_cast(v128: i128, from_ty: IrType, to_ty: IrType) -> Option<IrConst> {
    let bits_lo = v128 as u64;
    Some(match to_ty {
        IrType::I8 | IrType::U8 => IrConst::I8(bits_lo as i8),
        IrType::I16 | IrType::U16 => IrConst::I16(bits_lo as i16),
        IrType::I32 => IrConst::I32(bits_lo as i32),
        IrType::U32 => IrConst::I64(bits_lo as u32 as i64),
        IrType::I64 | IrType::U64 | IrType::Ptr => IrConst::I64(bits_lo as i64),
        IrType::I128 | IrType::U128 => IrConst::I128(v128),
        IrType::F32 => {
            if from_ty.is_unsigned() {
                IrConst::F32((v128 as u128) as f32)
            } else {
                IrConst::F32(v128 as f32)
            }
        }
        IrType::F64 => {
            let fv = if from_ty.is_unsigned() {
                (v128 as u128) as f64
            } else {
                v128 as f64
            };
            IrConst::F64(fv)
        }
        IrType::F128 => {
            // Use direct integer-to-x87 conversion to preserve full 64-bit
            // mantissa precision (x87 has 64-bit mantissa, unlike f64's 52-bit).
            if from_ty.is_unsigned() {
                IrConst::long_double_from_u128(v128 as u128)
            } else {
                IrConst::long_double_from_i128(v128)
            }
        }
        _ => return None,
    })
}

/// Convert a normalized i64 value to the target type.
fn fold_int_cast(val: i64, from_ty: IrType, to_ty: IrType) -> Option<IrConst> {
    Some(match to_ty {
        IrType::I8 | IrType::U8 => IrConst::I8(val as i8),
        IrType::I16 | IrType::U16 => IrConst::I16(val as i16),
        IrType::I32 => IrConst::I32(val as i32),
        // U32: keep as I64 with zero-extended value so that loading into
        // a 64-bit register preserves the unsigned 32-bit value. If we
        // stored as I32(val as i32), the constant load would sign-extend
        // (e.g. -13 -> 0xFFFFFFFFFFFFFFF3) which corrupts 64-bit unsigned
        // operations like divq.
        IrType::U32 => IrConst::I64(val as u32 as i64),
        IrType::I64 | IrType::U64 | IrType::Ptr => IrConst::I64(val),
        IrType::I128 | IrType::U128 => {
            if from_ty.is_unsigned() {
                IrConst::I128(val as u64 as u128 as i128)
            } else {
                IrConst::I128(val as i128)
            }
        }
        IrType::F32 => IrConst::F32(if from_ty.is_unsigned() {
            val as u64 as f32
        } else {
            val as f32
        }),
        IrType::F64 => IrConst::F64(if from_ty.is_unsigned() {
            val as u64 as f64
        } else {
            val as f64
        }),
        IrType::F128 => {
            // Use direct integer-to-x87 conversion for full 64-bit precision
            if from_ty.is_unsigned() {
                IrConst::long_double_from_u64(val as u64)
            } else {
                IrConst::long_double_from_i64(val)
            }
        }
        _ => return None,
    })
}

/// Invert a comparison operator (negate the condition).
/// Rebuild an integer constant of type `ty` from an i64 value (bit-pattern
/// semantics for unsigned types, like the range-check fold's int_const).
/// Returns None if the value does not fit the type's width.
fn int_const_of_type(ty: IrType, v: i64) -> Option<IrConst> {
    match ty {
        IrType::I8 => i8::try_from(v).ok().map(IrConst::I8),
        IrType::U8 => u8::try_from(v).ok().map(|x| IrConst::I8(x as i8)),
        IrType::I16 => i16::try_from(v).ok().map(IrConst::I16),
        IrType::U16 => u16::try_from(v).ok().map(|x| IrConst::I16(x as i16)),
        IrType::I32 => i32::try_from(v).ok().map(IrConst::I32),
        IrType::U32 => u32::try_from(v).ok().map(|x| IrConst::I32(x as i32)),
        IrType::I64 | IrType::U64 => Some(IrConst::I64(v)),
        _ => None,
    }
}

fn invert_cmp_op(op: IrCmpOp) -> IrCmpOp {
    match op {
        IrCmpOp::Eq => IrCmpOp::Ne,
        IrCmpOp::Ne => IrCmpOp::Eq,
        IrCmpOp::Slt => IrCmpOp::Sge,
        IrCmpOp::Sle => IrCmpOp::Sgt,
        IrCmpOp::Sgt => IrCmpOp::Sle,
        IrCmpOp::Sge => IrCmpOp::Slt,
        IrCmpOp::Ult => IrCmpOp::Uge,
        IrCmpOp::Ule => IrCmpOp::Ugt,
        IrCmpOp::Ugt => IrCmpOp::Ule,
        IrCmpOp::Uge => IrCmpOp::Ult,
    }
}

/// Swap a comparison operator (reverse operand order).
fn swap_cmp_op(op: IrCmpOp) -> IrCmpOp {
    match op {
        IrCmpOp::Eq | IrCmpOp::Ne => op,
        IrCmpOp::Slt => IrCmpOp::Sgt,
        IrCmpOp::Sle => IrCmpOp::Sge,
        IrCmpOp::Sgt => IrCmpOp::Slt,
        IrCmpOp::Sge => IrCmpOp::Sle,
        IrCmpOp::Ult => IrCmpOp::Ugt,
        IrCmpOp::Ule => IrCmpOp::Uge,
        IrCmpOp::Ugt => IrCmpOp::Ult,
        IrCmpOp::Uge => IrCmpOp::Ule,
    }
}

/// Return the narrow source for a condition that only observes whether a
/// widened integer result is zero.  See the caller in `simplify_function` for
/// the zero-ness proof and why truncating/non-integer casts are refused.
///
/// Only when the rewrite deletes the cast (`cast_dies_if_narrowed`, see
/// [`casts_dying_under_narrowing`]): a cast that stays live for another reader
/// would gain its narrow source as a second live value.
fn widened_cond_replacement(
    cond: Operand,
    cast_defs: &[Option<CastDef>],
    cast_dies_if_narrowed: &[bool],
) -> Option<Operand> {
    let Operand::Value(v) = cond else {
        return None;
    };
    // An empty slice is the direct unit-test API: production always supplies
    // the census, while the pure proof tests intentionally exercise the type
    // predicate independently of an enclosing function.
    if !cast_dies_if_narrowed.is_empty()
        && cast_dies_if_narrowed.get(v.0 as usize).copied() != Some(true)
    {
        return None;
    }
    let cast = cast_defs.get(v.0 as usize).and_then(|d| d.as_ref())?;
    is_integer_widening(cast).then_some(cast.src)
}

/// A conversion that preserves the zero/nonzero partition of every source
/// value: integer to a strictly wider integer.
fn is_integer_widening(cast: &CastDef) -> bool {
    cast.from_ty.is_integer() && cast.to_ty.is_integer() && cast.from_ty.size() < cast.to_ty.size()
}

/// `(value, constant, op)` of a compare against a constant, with the operator
/// swapped when the constant is on the left.
fn cmp_against_const(
    lhs: &Operand,
    rhs: &Operand,
    op: IrCmpOp,
) -> Option<(Value, IrConst, IrCmpOp)> {
    match (lhs, rhs) {
        (Operand::Value(v), Operand::Const(c)) => Some((*v, *c, op)),
        (Operand::Const(c), Operand::Value(v)) => Some((*v, *c, swap_cmp_op(op))),
        _ => None,
    }
}

/// `Cmp(op, cast(x: T -> W), C)` at width W as `Cmp(op, x, C')` at T, when the
/// extension preserves the comparison's order and C fits in T:
///   - unsigned T (zero-extend): Ult/Ule/Ugt/Uge + Eq/Ne, C in [0, 2^n);
///   - signed   T (sign-extend): Slt/Sle/Sgt/Sge + Eq/Ne, C in signed range.
///
/// The rewrite in `simplify_cmp` and the reader census in
/// [`casts_dying_under_narrowing`] both decide through this one predicate.
fn narrow_cast_cmp(op: IrCmpOp, c: i64, cast: &CastDef, ty: IrType) -> Option<(IrConst, IrType)> {
    let narrow_ty = cast.from_ty;
    if cast.to_ty != ty || !narrow_ty.is_integer() || narrow_ty.size() >= ty.size() {
        return None;
    }
    let bits = narrow_ty.size() * 8;
    let fits = if narrow_ty.is_unsigned() {
        c >= 0 && (c as u64) <= (1u64 << bits) - 1
    } else {
        c >= -(1i64 << (bits - 1)) && c <= (1i64 << (bits - 1)) - 1
    };
    let order_ok = match op {
        IrCmpOp::Eq | IrCmpOp::Ne => true,
        IrCmpOp::Ult | IrCmpOp::Ule | IrCmpOp::Ugt | IrCmpOp::Uge => narrow_ty.is_unsigned(),
        IrCmpOp::Slt | IrCmpOp::Sle | IrCmpOp::Sgt | IrCmpOp::Sge => narrow_ty.is_signed(),
    };
    if !(fits && order_ok) {
        return None;
    }
    Some((int_const_of_type(narrow_ty, c)?, narrow_ty))
}

/// For each value, whether it is a widening cast EVERY live reader of which is
/// a rewrite that reads the cast's narrow source instead: a compare against a
/// constant that [`narrow_cast_cmp`] accepts, or -- when the widened-condition
/// rewrite is enabled -- a `CondBranch` on an integer widening. Narrowing all
/// of them deletes the cast, so the narrow source replaces it one for one.
///
/// One live reader of any other kind keeps the cast live, and narrowing the
/// rest would only add the narrow source as a second live value (sqlite's
/// `serial_type >= 128` beside `vdbe_serial_get(body, serial_type, ..)` kept
/// both the byte and its extension live across the call and spilled the
/// byte). The sole-reader rule this replaces was the one-reader case of the
/// same argument; it refused a byte classified by several compares -- Expat's
/// `c < 0x80`, `c >= 0xc2`, `c <= 0xdf`, ... -- whose extension dies once
/// every compare reads the byte.
///
/// Readers DCE will delete do not count ([`dead_pure_values`]): inlining
/// leaves `Copy` instructions of a promoted byte that nothing reads, and such
/// a copy vetoing the narrowing would make the result depend on pass order.
/// Every operand read counts (`for_each_used_value`), so a compare reading the
/// cast on both sides counts twice and narrows nowhere.
fn casts_dying_under_narrowing(
    func: &IrFunction,
    cast_defs: &[Option<CastDef>],
    widened_cond_live: bool,
) -> Vec<bool> {
    let dead = dead_pure_values(func);
    let is_dead = |inst: &Instruction| {
        inst.dest()
            .is_some_and(|d| dead.get(d.0 as usize).copied().unwrap_or(false))
    };
    let is_cast = |id: u32| cast_defs.get(id as usize).is_some_and(|d| d.is_some());
    let mut live_reads = vec![0u32; cast_defs.len()];
    let mut narrowable = vec![0u32; cast_defs.len()];
    let cmp_live = !simplify_skip("cmp");
    let cast_of = |v: Value| cast_defs.get(v.0 as usize).and_then(|d| d.as_ref());
    for block in &func.blocks {
        for inst in block.instructions.iter().filter(|i| !is_dead(i)) {
            inst.for_each_used_value(|id| {
                if is_cast(id) {
                    live_reads[id as usize] += 1;
                }
            });
            let Instruction::Cmp {
                op, lhs, rhs, ty, ..
            } = inst
            else {
                continue;
            };
            let Some((v, c, op)) = cmp_against_const(lhs, rhs, *op).filter(|_| cmp_live) else {
                continue;
            };
            let narrows = cast_of(v)
                .zip(c.to_i64())
                .is_some_and(|(cast, c)| narrow_cast_cmp(op, c, cast, *ty).is_some());
            if narrows {
                narrowable[v.0 as usize] += 1;
            }
        }
        block.terminator.for_each_used_value(|id| {
            if is_cast(id) {
                live_reads[id as usize] += 1;
            }
        });
        if let Terminator::CondBranch {
            cond: Operand::Value(v),
            ..
        } = &block.terminator
            && widened_cond_live
            && cast_of(*v).is_some_and(is_integer_widening)
        {
            narrowable[v.0 as usize] += 1;
        }
    }
    live_reads
        .iter()
        .zip(&narrowable)
        .map(|(&reads, &n)| reads != 0 && reads == n)
        .collect()
}

/// Values whose definitions dead-code elimination deletes: a definition with
/// no side effects ([`super::dce::has_side_effects`]) all of whose readers are
/// themselves such definitions. A worklist from the unread values, so a chain
/// of dead copies dies whole. `Alloca` is excluded (DCE roots it
/// conditionally), and a value with no `dest()` definition -- an inline-asm
/// output, a parameter -- is never dead.
fn dead_pure_values(func: &IrFunction) -> Vec<bool> {
    let len = func.max_value_id() as usize + 1;
    let mut uses = vec![0u32; len];
    let mut def: Vec<Option<(usize, usize)>> = vec![None; len];
    let mut count = |id: u32| {
        if let Some(n) = uses.get_mut(id as usize) {
            *n += 1;
        }
    };
    for (b, block) in func.blocks.iter().enumerate() {
        for (i, inst) in block.instructions.iter().enumerate() {
            inst.for_each_used_value(&mut count);
            if let Some(slot) = inst.dest().and_then(|d| def.get_mut(d.0 as usize)) {
                *slot = Some((b, i));
            }
        }
        block.terminator.for_each_used_value(&mut count);
    }
    let removable = |at: Option<(usize, usize)>| {
        at.is_some_and(|(b, i)| {
            let inst = &func.blocks[b].instructions[i];
            !matches!(inst, Instruction::Alloca { .. }) && !super::dce::has_side_effects(inst)
        })
    };
    let mut dead = vec![false; len];
    let mut work: Vec<usize> = (0..len)
        .filter(|&v| uses[v] == 0 && removable(def[v]))
        .collect();
    while let Some(v) = work.pop() {
        if std::mem::replace(&mut dead[v], true) {
            continue;
        }
        let Some((b, i)) = def[v] else { continue };
        func.blocks[b].instructions[i].for_each_used_value(|id| {
            let u = id as usize;
            if let Some(n) = uses.get_mut(u) {
                *n = n.saturating_sub(1);
                if *n == 0 && !dead[u] && removable(def[u]) {
                    work.push(u);
                }
            }
        });
    }
    dead
}

/// Narrow a comparison whose two operands are identically widened integer
/// values back to their original carrier type.
///
/// Sign extension preserves signed ordering, so signed relational predicates
/// (plus equality) may use the narrow carrier.  The two casts must agree on
/// both source and destination types; mixed extensions are intentionally left
/// alone because e.g. sext(0xff) and zext(0xff) are different values.
///
/// This IR rewrite is deliberately limited to signed roots.  The existing
/// backend PF-15 path still folds an *adjacent* zero-extend pair, where no
/// liveness/RA reshaping is needed.  Extending that transformation across an
/// intervening instruction rewrites the value web; a 101-round interleaved
/// Expat measurement found the U8 quote-scan form slower despite a static
/// instruction saving, due to its changed hot-loop layout.  Keep unsigned
/// non-adjacent pairs on the mature backend path until a profitability model
/// includes layout/placement costs.
///
/// Unlike a backend peephole this rewrites the SSA uses themselves.  The
/// original narrow values then remain live until the compare, so register
/// allocation sees the real lifetime rather than relying on a deferred move
/// surviving an intervening load or branch.
fn narrow_widened_cmp_pair(
    dest: Value,
    op: IrCmpOp,
    lhs: &Operand,
    rhs: &Operand,
    ty: IrType,
    cast_defs: &[Option<CastDef>],
    cast_use_counts: &[u32],
) -> Option<Instruction> {
    let (Operand::Value(lv), Operand::Value(rv)) = (lhs, rhs) else {
        return None;
    };
    if !cast_use_counts.is_empty()
        && (cast_use_counts.get(lv.0 as usize).copied() != Some(1)
            || cast_use_counts.get(rv.0 as usize).copied() != Some(1))
    {
        return None;
    }
    let left = cast_defs.get(lv.0 as usize).and_then(|d| d.as_ref())?;
    let right = cast_defs.get(rv.0 as usize).and_then(|d| d.as_ref())?;
    let narrow_ty = left.from_ty;
    if !narrow_ty.is_integer()
        || !narrow_ty.is_signed()
        || !left.to_ty.is_integer()
        || left.to_ty != ty
        || right.from_ty != narrow_ty
        || right.to_ty != left.to_ty
        || narrow_ty.size() >= ty.size()
    {
        return None;
    }

    let narrow_op = match op {
        IrCmpOp::Eq | IrCmpOp::Ne | IrCmpOp::Slt | IrCmpOp::Sle | IrCmpOp::Sgt | IrCmpOp::Sge => op,
        // An unsigned compare of sign-extended negatives observes their high
        // extension bits and cannot be expressed as a narrow signed compare
        // without a different proof.
        IrCmpOp::Ult | IrCmpOp::Ule | IrCmpOp::Ugt | IrCmpOp::Uge => return None,
    };

    Some(Instruction::Cmp {
        dest,
        op: narrow_op,
        lhs: left.src,
        rhs: right.src,
        ty: narrow_ty,
    })
}

/// `(x & C) ==/!= 0` where every bit `C` selects is a copy of one sign bit
/// becomes the sign test `x' >=s 0` / `x' <s 0`.
///
/// Two shapes put a sign bit under the mask:
/// * `sext(x: iN) & C`: bits `N-1 .. W` of the extension all copy `x`'s sign,
///   so any nonzero `C` confined to them tests exactly that bit
///   (`((long)*p & 0x100) != 0` on a `signed char`);
/// * `x & 2^(W-1)` on a signed `x` at the compare's own width.
///
/// Both keep the tested value's own signed type. The unsigned analogues
/// (`zext(x: uN) & 2^(N-1)`, `x & 2^(W-1)` on an unsigned `x`) would need a
/// signed compare of an unsigned-typed value, and a backend that compares
/// full registers relies on each value being extended per its declared type
/// (RV64, AArch64 `cmp w`), so those stay mask tests here; the x86 peephole
/// materializes them as a shift instead.
///
/// The sign test is the form every backend materializes best (GCC 16.2,
/// Clang 23.1.4 and ICX 2026.2 all emit `shr $31`/`movsbl; shr` where the
/// mask test costs `and; test; setne` or `test; setne; movzbl`), and it frees
/// the widening cast when the compare was its only reader.
fn fold_sign_region_mask_test(
    dest: Value,
    op: IrCmpOp,
    lhs: &Operand,
    rhs: &Operand,
    ty: IrType,
    binop_defs: &[Option<BinOpDef>],
    cast_defs: &[Option<CastDef>],
) -> Option<Instruction> {
    if simplify_skip("sign_mask") || !matches!(op, IrCmpOp::Eq | IrCmpOp::Ne) {
        return None;
    }
    let v = match (lhs, rhs) {
        (Operand::Value(v), Operand::Const(c)) | (Operand::Const(c), Operand::Value(v))
            if c.to_i64() == Some(0) =>
        {
            *v
        }
        _ => return None,
    };
    let and = binop_defs.get(v.0 as usize).and_then(|d| d.as_ref())?;
    if and.op != IrBinOp::And || and.ty != ty {
        return None;
    }
    let (masked, mask) = match (and.lhs, and.rhs) {
        (Operand::Value(m), Operand::Const(c)) | (Operand::Const(c), Operand::Value(m)) => {
            (m, c.to_i64()?)
        }
        _ => return None,
    };
    let width = u32::try_from(ty.size()).ok()? * 8;
    if !ty.is_integer() || width > 64 {
        return None;
    }
    let mask = (mask as u64) & (u64::MAX >> (64 - width));
    if mask == 0 {
        return None;
    }
    let cast = cast_defs
        .get(masked.0 as usize)
        .and_then(|d| d.as_ref())
        .filter(|c| c.to_ty == ty && c.from_ty.is_integer() && c.from_ty.size() < ty.size());
    let (tested, narrow) = match cast {
        Some(c) => {
            let n = u32::try_from(c.from_ty.size()).ok()? * 8;
            // Every bit at or above N-1 of a sign extension copies the sign.
            if !c.from_ty.is_signed() || mask & ((1u64 << (n - 1)) - 1) != 0 {
                return None;
            }
            (c.src, c.from_ty)
        }
        None if ty.is_signed() && mask == 1u64 << (width - 1) => (Operand::Value(masked), ty),
        None => return None,
    };
    Some(Instruction::Cmp {
        dest,
        op: if op == IrCmpOp::Ne {
            IrCmpOp::Slt
        } else {
            IrCmpOp::Sge
        },
        lhs: tested,
        rhs: Operand::Const(int_const_of_type(narrow, 0)?),
        ty: narrow,
    })
}

/// Simplify a Cmp instruction.
///
/// Handles:
/// - Cmp(op, x, x) => constant (self-comparison: Eq/Sle/Sge/Ule/Uge => 1, others => 0)
/// - Cmp(Ne, cmp_result, 0) => Copy(cmp_result) when cmp_result is from another Cmp
///   (a Cmp already produces 0/1, so comparing != 0 is a no-op)
/// - Cmp(Eq, cmp_result, 0) => Cmp(inverted_op, orig_lhs, orig_rhs)
///   (negating a boolean comparison result)
/// - Cmp(Ne, cmp_result, 1) => Cmp(inverted_op, orig_lhs, orig_rhs)
///   (checking if comparison result is not-true = inverted)
/// - Cmp(Eq, cmp_result, 1) => Copy(cmp_result) when cmp_result is from another Cmp
///   (comparing a 0/1 value against 1 is the same as the original comparison)
/// - Comparison narrowing: Cmp(op, Cast(x, T->W), C, W) => Cmp(op, x, C', T)
///   when the extension T->W preserves op's ordering and C fits in T.
fn simplify_cmp(
    dest: Value,
    op: IrCmpOp,
    lhs: &Operand,
    rhs: &Operand,
    ty: IrType,
    cmp_defs: &[Option<CmpDef>],
    cast_defs: &[Option<CastDef>],
    is_boolean: &[bool],
    cast_use_counts: &[u32],
    cast_dies_if_narrowed: &[bool],
) -> Option<Instruction> {
    if simplify_skip("cmp") {
        return None;
    }
    if std::env::var_os("CCC_NO_WIDENED_CMP_PAIR").is_none() {
        if let Some(narrowed) =
            narrow_widened_cmp_pair(dest, op, lhs, rhs, ty, cast_defs, cast_use_counts)
        {
            return Some(narrowed);
        }
    }
    // Self-comparison: Cmp(op, x, x) => constant
    if same_value_operands(lhs, rhs) && ty.is_integer() {
        let result: i8 = match op {
            // x == x, x <= x, x >= x are always true
            IrCmpOp::Eq | IrCmpOp::Sle | IrCmpOp::Sge | IrCmpOp::Ule | IrCmpOp::Uge => 1,
            // x != x, x < x, x > x are always false
            IrCmpOp::Ne | IrCmpOp::Slt | IrCmpOp::Sgt | IrCmpOp::Ult | IrCmpOp::Ugt => 0,
        };
        return Some(Instruction::Copy {
            dest,
            src: Operand::Const(IrConst::I8(result)),
        });
    }

    // Canonicalize to (value, const) form by swapping the comparison operator.
    // This eliminates the need for separate swapped/non-swapped handling below.
    let (val, cval, effective_op) = match (lhs, rhs) {
        (Operand::Value(v), Operand::Const(c)) => (v, c, op),
        (Operand::Const(c), Operand::Value(v)) => (v, c, swap_cmp_op(op)),
        _ => return None,
    };

    let const_i64 = cval.to_i64();
    let is_zero_const = const_i64 == Some(0);
    let is_one_const = const_i64 == Some(1);

    if simplify_skip("cmp") {
        return None;
    }

    // Comparison narrowing: Cmp(op, Cast(x, T->W), C, W) => Cmp(op, x, C', T).
    //
    // A value widened for C's integer promotions (e.g. a `movzbl`-loaded byte
    // promoted to int for `c < 0x80`) is compared AT ITS NATIVE WIDTH, so the
    // widening cast -- and its register copy -- disappears and the backend emits
    // `cmpb`/`testb` directly (GCC's `test dl,dl; jns`, ICX's `cmp cl,-62`).
    // `narrow_cast_cmp` holds the order and range conditions. Only when every
    // reader of the cast narrows (`casts_dying_under_narrowing`): a cast that
    // stays live saves nothing and gives its narrow source a second reader.
    let cast_dies = cast_dies_if_narrowed.is_empty()
        || cast_dies_if_narrowed.get(val.0 as usize).copied() == Some(true);
    if let Some(const_i64) = const_i64
        && cast_dies
        && let Some(Some(cdef)) = cast_defs.get(val.0 as usize)
        && let Some((narrowed_const, narrow_ty)) =
            narrow_cast_cmp(effective_op, const_i64, cdef, ty)
    {
        return Some(Instruction::Cmp {
            dest,
            op: effective_op,
            lhs: cdef.src,
            rhs: Operand::Const(narrowed_const),
            ty: narrow_ty,
        });
    }

    // Check if the value is known boolean (0 or 1). This includes:
    // - Values defined by Cmp instructions
    // - Values from And/Or/Xor of boolean values
    let idx = val.0 as usize;
    let val_is_boolean = idx < is_boolean.len() && is_boolean[idx];

    // If the value is from a Cmp, we can do precise inversion
    let inner_cmp = if let Some(Some(cdef)) = cmp_defs.get(idx) {
        Some(cdef)
    } else {
        None
    };

    if val_is_boolean {
        match effective_op {
            IrCmpOp::Eq => {
                if is_zero_const {
                    // Cmp(Eq, boolean_val, 0) => logical NOT of boolean_val
                    // If from a Cmp, we can invert the comparison directly
                    if let Some(ic) = inner_cmp {
                        // For float ordered comparisons (Slt, Sle, Sgt, Sge),
                        // inversion is NOT valid because of NaN: !(a <= b) is
                        // NOT the same as (a > b) when either operand is NaN.
                        // !(NaN <= x) should be true (since NaN <= x is false),
                        // but (NaN > x) is also false. Only Eq<->Ne inversion
                        // is safe for floats since both handle NaN consistently.
                        let is_float_ordered =
                            ic.ty.is_float() && !matches!(ic.op, IrCmpOp::Eq | IrCmpOp::Ne);
                        if !is_float_ordered {
                            return Some(Instruction::Cmp {
                                dest,
                                op: invert_cmp_op(ic.op),
                                lhs: ic.lhs,
                                rhs: ic.rhs,
                                ty: ic.ty,
                            });
                        }
                    }
                    // For non-Cmp boolean values (e.g., And/Or of booleans),
                    // Cmp(Eq, val, 0) = Xor(val, 1) - but we can't easily
                    // express this without creating new instructions, so skip.
                }
                if is_one_const {
                    // Cmp(Eq, boolean_val, 1) => Copy(boolean_val)
                    // For a boolean value, "== 1" is identity
                    return Some(Instruction::Copy {
                        dest,
                        src: Operand::Value(*val),
                    });
                }
            }
            IrCmpOp::Ne => {
                if is_zero_const {
                    // Cmp(Ne, boolean_val, 0) => Copy(boolean_val)
                    // For a boolean value, "!= 0" is identity
                    return Some(Instruction::Copy {
                        dest,
                        src: Operand::Value(*val),
                    });
                }
                if is_one_const {
                    // Cmp(Ne, boolean_val, 1) => logical NOT
                    if let Some(ic) = inner_cmp {
                        // Skip inversion for float ordered comparisons (NaN safety).
                        // See the Eq/is_zero_const case above for the full explanation.
                        let is_float_ordered =
                            ic.ty.is_float() && !matches!(ic.op, IrCmpOp::Eq | IrCmpOp::Ne);
                        if !is_float_ordered {
                            return Some(Instruction::Cmp {
                                dest,
                                op: invert_cmp_op(ic.op),
                                lhs: ic.lhs,
                                rhs: ic.rhs,
                                ty: ic.ty,
                            });
                        }
                    }
                }
            }
            _ => {
                // For ordered comparisons (Slt, Sgt, etc.) with a boolean value:
                // Boolean values are 0 or 1, so we can simplify some patterns.
                // However, these are rare enough that we skip them for now.
            }
        }
    }

    // Unsigned comparisons with zero (already canonicalized to value op const form).
    if is_zero_const {
        match effective_op {
            // Cmp(Ult, x, 0) is always false (nothing is less than 0 unsigned)
            IrCmpOp::Ult => {
                return Some(Instruction::Copy {
                    dest,
                    src: Operand::Const(IrConst::I8(0)),
                });
            }
            // Cmp(Uge, x, 0) is always true (everything is >= 0 unsigned)
            IrCmpOp::Uge => {
                return Some(Instruction::Copy {
                    dest,
                    src: Operand::Const(IrConst::I8(1)),
                });
            }
            // Cmp(Ule, x, 0) => Cmp(Eq, x, 0) (unsigned x <= 0 means x == 0)
            IrCmpOp::Ule => {
                return Some(Instruction::Cmp {
                    dest,
                    op: IrCmpOp::Eq,
                    lhs: Operand::Value(*val),
                    rhs: Operand::Const(*cval),
                    ty,
                });
            }
            // Cmp(Ugt, x, 0) => Cmp(Ne, x, 0) (unsigned x > 0 means x != 0)
            IrCmpOp::Ugt => {
                return Some(Instruction::Cmp {
                    dest,
                    op: IrCmpOp::Ne,
                    lhs: Operand::Value(*val),
                    rhs: Operand::Const(*cval),
                    ty,
                });
            }
            _ => {}
        }
    }

    // Operand canonicalization: if the constant is on the LHS, rewrite to place it
    // on the RHS with the comparison operator swapped. This enables the backend to
    // use immediate-form compare instructions (e.g., `cmp $imm, %reg` or
    // `test %reg, %reg` for zero) instead of loading the constant into a register.
    // Only rewrite when the instruction actually has the constant on the LHS.
    if matches!(lhs, Operand::Const(_)) && matches!(rhs, Operand::Value(_)) {
        let swapped_op = swap_cmp_op(op);
        return Some(Instruction::Cmp {
            dest,
            op: swapped_op,
            lhs: *rhs,
            rhs: *lhs,
            ty,
        });
    }

    None
}

/// Fold `fabs(x) < 0` (and the swapped `0 > fabs(x)`) to false.
///
/// IEEE 754: `fabs` never returns a negative number, and a comparison
/// against NaN is unordered (false for ordered `<`). This is exactly the
/// identity gcc.c-torture 20020720-1 checks: the `link_error()` call in
/// `if (fabs(x) < 0.0)` must be DCE'd.
fn fold_fabs_lt_zero(
    dest: Value,
    op: IrCmpOp,
    lhs: &Operand,
    rhs: &Operand,
    ty: IrType,
    is_fabs: &[bool],
) -> Option<Instruction> {
    if !ty.is_float() {
        return None;
    }
    let (val, cval, effective_op) = match (lhs, rhs) {
        (Operand::Value(v), Operand::Const(c)) => (v, c, op),
        (Operand::Const(c), Operand::Value(v)) => (v, c, swap_cmp_op(op)),
        _ => return None,
    };
    if !cval.is_zero() {
        return None;
    }
    let idx = val.0 as usize;
    if idx >= is_fabs.len() || !is_fabs[idx] {
        return None;
    }
    if effective_op == IrCmpOp::Slt {
        return Some(Instruction::Copy {
            dest,
            src: Operand::Const(IrConst::I8(0)),
        });
    }
    None
}

/// Simplify a GetElementPtr instruction.
///
/// - GEP with constant zero offset => Copy of base pointer
/// - GEP chain: GEP(GEP(base, c1), c2) => GEP(base, c1 + c2) when both offsets are constants
fn simplify_gep(
    dest: Value,
    base: Value,
    offset: &Operand,
    ty: IrType,
    gep_defs: &[Option<GepDef>],
) -> Option<Instruction> {
    if is_zero(offset) {
        return Some(Instruction::Copy {
            dest,
            src: Operand::Value(base),
        });
    }

    // GEP chain folding: GEP(GEP(inner_base, c1), c2) => GEP(inner_base, c1 + c2)
    // Only safe when both offsets are constants (we can compute the sum at compile time).
    if let Operand::Const(outer_c) = offset {
        if let Some(outer_val) = outer_c.to_i64() {
            let base_idx = base.0 as usize;
            if let Some(Some(inner_gep)) = gep_defs.get(base_idx) {
                if let Operand::Const(inner_c) = &inner_gep.offset {
                    if let Some(inner_val) = inner_c.to_i64() {
                        let combined = inner_val.wrapping_add(outer_val);
                        if combined == 0 {
                            // Combined offset is zero => just copy the original base
                            return Some(Instruction::Copy {
                                dest,
                                src: Operand::Value(inner_gep.base),
                            });
                        }
                        return Some(Instruction::GetElementPtr {
                            dest,
                            base: inner_gep.base,
                            offset: Operand::Const(IrConst::I64(combined)),
                            ty,
                        });
                    }
                }
            }
        }
    }

    None
}

/// Return the log2 of a positive i64 value if it is a power of 2, or None otherwise.
/// Recognise a constant power of two and return its exponent.
///
/// The constant must be read at the operation's width. Going through
/// `to_i64()` silently truncates `IrConst::I128`, so the multiplier
/// `0xffffffffffffffff0000000000000001` came back as `1`, was recognised as
/// 2^0, and rewrote `x * K` into `x << 0` -- that is, into `x`, a silent
/// miscompile of every `__int128` multiply by a constant whose low 64 bits
/// happen to be a power of two. The same signed read also hid the single set
/// bit of 2^63 in an unsigned 64-bit multiplier, so `x * 0x8000000000000000u`
/// missed the shift entirely.
///
/// Both are fixed by taking the constant's bit pattern and masking it to the
/// type's width before the power-of-two test.
fn const_power_of_two(op: &Operand, ty: IrType) -> Option<u32> {
    let c = match op {
        Operand::Const(c) => *c,
        _ => return None,
    };
    let raw: u128 = match c {
        IrConst::I8(v) => v as u8 as u128,
        IrConst::I16(v) => v as u16 as u128,
        IrConst::I32(v) => v as u32 as u128,
        IrConst::I64(v) => v as u64 as u128,
        IrConst::I128(v) => v as u128,
        _ => return None,
    };
    // Bits above the operation's width are not part of the value and must not
    // be mistaken for the single set bit of a power of two.
    let width = ty.size().saturating_mul(8) as u32;
    let val = if width == 0 || width >= 128 {
        raw
    } else {
        raw & ((1u128 << width) - 1)
    };
    if val != 0 && (val & (val - 1)) == 0 {
        Some(val.trailing_zeros())
    } else {
        None
    }
}

/// Try to strength-reduce a multiply by a power of 2 on either side.
/// Returns the shift instruction if successful.
fn try_mul_power_of_two(
    dest: Value,
    val: &Operand,
    pow2: &Operand,
    ty: IrType,
) -> Option<Instruction> {
    let shift = const_power_of_two(pow2, ty)?;
    if shift == 1 {
        // x * 2 => x + x
        Some(Instruction::BinOp {
            dest,
            op: IrBinOp::Add,
            lhs: *val,
            rhs: *val,
            ty,
        })
    } else {
        // x * 2^k => x << k
        Some(Instruction::BinOp {
            dest,
            op: IrBinOp::Shl,
            lhs: *val,
            rhs: Operand::Const(IrConst::from_i64(shift as i64, ty)),
            ty,
        })
    }
}

/// Remove the rounding-toward-zero bias from an exact signed division by 2.
/// div_by_const runs before GVN, so related product operands often become
/// identical only AFTER division has expanded. Recognize that expanded form.
/// For an even two's-complement p, (p + signbit(p)) >>s 1 == p >>s 1,
/// including wrapped products and MIN. No positivity or no-overflow assumption
/// is needed. x and x +/- odd (also x XOR odd) have opposite low bits, hence
/// their product is even in EVERY integer ring. Do not cross width changes.
fn unbiased_even_product_half(
    biased: Operand,
    ty: IrType,
    casts: &[Option<CastDef>],
    bins: &[Option<BinOpDef>],
) -> Option<Operand> {
    if !matches!(ty, IrType::I32 | IrType::U32 | IrType::I64 | IrType::U64) {
        return None;
    }
    let width = ty.size() * 8;
    let literal = |o: Operand, n: i128| matches!(o, Operand::Const(c) if c.to_i128() == Some(n));
    let bin =
        |o: Operand, op: IrBinOp| get_binop_def(&o, bins).filter(|d| d.op == op && d.ty == ty);
    let add = bin(biased, IrBinOp::Add)?;
    for (product, correction) in [(add.lhs, add.rhs), (add.rhs, add.lhs)] {
        let Some(bias) = bin(correction, IrBinOp::LShr) else {
            continue;
        };
        if !literal(bias.rhs, (width - 1) as i128) {
            continue;
        }
        let Some(sign) = bin(bias.lhs, IrBinOp::AShr) else {
            continue;
        };
        if !literal(sign.rhs, (width - 1) as i128) || sign.lhs != product {
            continue;
        }
        let root = peel_int_cast_chain(product, casts, true);
        let Some(mul) = get_binop_def(&root, bins) else {
            continue;
        };
        if mul.op != IrBinOp::Mul || !mul.ty.is_integer() || mul.ty.size() != ty.size() {
            continue;
        }
        for (x, adjacent) in [(mul.lhs, mul.rhs), (mul.rhs, mul.lhs)] {
            let Some(pair) = get_binop_def(&adjacent, bins) else {
                continue;
            };
            if pair.ty != mul.ty || !matches!(pair.op, IrBinOp::Add | IrBinOp::Sub | IrBinOp::Xor) {
                continue;
            }
            // Sub is symmetric modulo 2 too: odd-x and x-odd both flip x's LSB.
            for (base, odd) in [(pair.lhs, pair.rhs), (pair.rhs, pair.lhs)] {
                if base == x
                    && matches!(odd, Operand::Const(c) if c.to_i128().is_some_and(|n| n & 1 != 0))
                {
                    return Some(product);
                }
            }
        }
    }
    None
}

/// Exact reciprocal for a normal binary power of two, with a normal reciprocal.
/// Multiplication and division then have the same exact real result and the
/// same single rounding. Excluding subnormal constants on BOTH sides also
/// preserves DAZ behavior: flushing a newly subnormal reciprocal to zero would
/// not be equivalent to division by a large normal constant. Construct bits;
/// never approximate a reciprocal using host floating arithmetic.
fn normal_power_of_two_reciprocal(rhs: &Operand, ty: IrType) -> Option<IrConst> {
    match (ty, rhs) {
        (IrType::F64, Operand::Const(IrConst::F64(value))) => {
            let bits = value.to_bits();
            let exponent = (bits >> 52) & 0x7ff;
            if bits & 0x000f_ffff_ffff_ffff != 0 || !(1..=2045).contains(&exponent) {
                return None;
            }
            Some(IrConst::F64(f64::from_bits(
                (bits & (1 << 63)) | ((2046 - exponent) << 52),
            )))
        }
        (IrType::F32, Operand::Const(IrConst::F32(value))) => {
            let bits = value.to_bits();
            let exponent = (bits >> 23) & 0xff;
            if bits & 0x007f_ffff != 0 || !(1..=253).contains(&exponent) {
                return None;
            }
            Some(IrConst::F32(f32::from_bits(
                (bits & (1 << 31)) | ((254 - exponent) << 23),
            )))
        }
        _ => None,
    }
}

/// High half of the 2W-bit product by a constant power of two 2^k is
/// the operand shifted down by W - k: logically for the unsigned form,
/// arithmetically for the signed one (2^k must then be positive, so
/// k < W - 1). A zero operand gives zero, and 2^0 is the k = 0 case:
/// zero (unsigned) or the sign fill `x >> (W - 1)` (signed).
/// `None` keeps the operation.
fn simplify_mul_high(
    dest: Value,
    op: IrBinOp,
    lhs: &Operand,
    rhs: &Operand,
    ty: IrType,
    any_zero: bool,
) -> Option<Instruction> {
    if any_zero {
        return Some(Instruction::Copy {
            dest,
            src: Operand::Const(IrConst::zero(ty)),
        });
    }
    let bits = (ty.size() * 8) as i64;
    let signed = op == IrBinOp::SMulHigh;
    for (x, c) in [(lhs, rhs), (rhs, lhs)] {
        let Operand::Const(c) = c else { continue };
        let Some(v) = c.to_i64() else { continue };
        // The operand's bit pattern at width W: 2^(W-1) is a power of
        // two for the unsigned form (and negative for the signed one,
        // which the k < W - 1 test below refuses).
        let raw = if bits < 64 {
            (v as u64) & ((1u64 << bits) - 1)
        } else {
            v as u64
        };
        if raw.count_ones() != 1 {
            continue;
        }
        let k = i64::from(raw.trailing_zeros());
        if signed && k >= bits - 1 {
            continue;
        }
        if k == 0 && !signed {
            return Some(Instruction::Copy {
                dest,
                src: Operand::Const(IrConst::zero(ty)),
            });
        }
        return Some(Instruction::BinOp {
            dest,
            op: if signed { IrBinOp::AShr } else { IrBinOp::LShr },
            lhs: *x,
            // Signed k = 0 is the sign fill: shift by W - 1, not W.
            rhs: Operand::Const(IrConst::I64(if k == 0 { bits - 1 } else { bits - k })),
            ty,
        });
    }
    None
}

/// `x /s -1 => -x`: exact for every defined input (`INT_MIN / -1`
/// overflows, which C leaves undefined).  It reached the backend as
/// `cqto; movq $-1, %rcx; idivq %rcx` (GCC: `negq`).
fn sdiv_by_minus_one(dest: Value, lhs: &Operand, ty: IrType) -> Instruction {
    Instruction::UnaryOp {
        dest,
        op: IrUnaryOp::Neg,
        src: *lhs,
        ty,
    }
}

/// `Bzhi(0, n)` keeps bits of zero.  A constant index is a fixed mask: only
/// its low eight bits count, `k >= W` is the identity and `k == 0` keeps
/// nothing; anything else is the plain `and` every later pass (known bits,
/// demanded bits, immediates) already understands.  `None` keeps the
/// operation (opaque index on a nonzero source).
fn simplify_bzhi(
    dest: Value,
    lhs: &Operand,
    rhs: &Operand,
    ty: IrType,
    lhs_zero: bool,
) -> Option<Instruction> {
    if lhs_zero {
        return Some(Instruction::Copy {
            dest,
            src: Operand::Const(IrConst::zero(ty)),
        });
    }
    let Operand::Const(c) = rhs else {
        return None;
    };
    let index = c.to_i128()?;
    let k = (index & 0xFF) as u32;
    let bits = (ty.size() * 8) as u32;
    if bits == 0 || bits > 64 {
        return None;
    }
    if k >= bits {
        return Some(Instruction::Copy { dest, src: *lhs });
    }
    if k == 0 {
        return Some(Instruction::Copy {
            dest,
            src: Operand::Const(IrConst::zero(ty)),
        });
    }
    Some(Instruction::BinOp {
        dest,
        op: IrBinOp::And,
        lhs: *lhs,
        rhs: Operand::Const(IrConst::from_i64(((1u64 << k) - 1) as i64, ty)),
        ty,
    })
}

fn simplify_binop(
    dest: Value,
    op: IrBinOp,
    lhs: &Operand,
    rhs: &Operand,
    ty: IrType,
    cast_defs: &[Option<CastDef>],
    binop_defs: &[Option<BinOpDef>],
    neg_defs: &[Option<NegDef>],
    use_counts: &[u32],
) -> Option<Instruction> {
    let lhs_zero = is_zero(lhs);
    let rhs_zero = is_zero(rhs);
    let lhs_one = is_one(lhs);
    let rhs_one = is_one(rhs);
    let same_value = same_value_operands(lhs, rhs);
    let is_float = ty.is_float();

    match op {
        IrBinOp::Add => {
            // For floats, x + 0 => x is invalid because -0.0 + 0.0 = +0.0, not -0.0.
            // IEEE 754: the sign of a sum differs from at most one addend.
            if !is_float {
                if rhs_zero {
                    // x + 0 => x (integers only)
                    return Some(Instruction::Copy { dest, src: *lhs });
                }
                if lhs_zero {
                    // 0 + x => x (integers only)
                    return Some(Instruction::Copy { dest, src: *rhs });
                }
                // Modular ring cancellation: `x + (y - x) => y`, `(y - x) + x => y`,
                // `x + (-x) => 0`, `(-x) + x => 0`.
                if let Some(inst) = try_cancel_add_sub(
                    dest,
                    IrBinOp::Add,
                    lhs,
                    rhs,
                    ty,
                    cast_defs,
                    binop_defs,
                    neg_defs,
                ) {
                    return Some(inst);
                }
                // Reassociation: (x + C1) + C2 => x + (C1 + C2)
                if let Some(inst) = try_reassociate_add(dest, lhs, rhs, ty, binop_defs, use_counts)
                {
                    return Some(inst);
                }
            }
            if is_float && !simplify_skip("addneg") {
                if let Some(inst) = simplify_float_add_neg(dest, lhs, rhs, ty, neg_defs) {
                    return Some(inst);
                }
            }
        }
        IrBinOp::Sub => {
            if rhs_zero && (!is_float || is_positive_zero(rhs)) {
                // x - 0 => x (for floats, only valid when rhs is +0.0; -0.0 - (-0.0) = +0.0)
                return Some(Instruction::Copy { dest, src: *lhs });
            }
            if !is_float && same_value {
                // x - x => 0 (integers only; for floats, Inf - Inf = NaN)
                return Some(Instruction::Copy {
                    dest,
                    src: Operand::Const(IrConst::zero(ty)),
                });
            }
            if !is_float && lhs_zero && !simplify_skip("zeroneg") {
                // `0 - x => -x` (integers only: -0.0 - x is not -x for x = 0.0).
                // The ring identity makes this a pure canonicalisation, but it
                // is worth stating because the two forms do NOT lower alike:
                // the subtraction has to materialise the zero first (`xor;sub`
                // on x86) and every consumer that understands a negate -- the
                // `x & -x` lowest-set-bit recognition, `try_cancel_add_sub`, the
                // backends' negate/flag folds -- only speaks `Neg`. Source
                // negation reaches the IR as `Neg` anyway; this is the signed-
                // zero and macro-expanded spelling of the same value (e.g. the
                // `x & -x` idiom, whose `0u - x` reaches the IR as this Sub).
                return Some(Instruction::UnaryOp {
                    dest,
                    op: IrUnaryOp::Neg,
                    src: *rhs,
                    ty,
                });
            }
            if !is_float {
                // Modular ring cancellation: `(x + y) - x => y`, `(y + x) - x => y`,
                // `x - (x - y) => y`.
                if let Some(inst) = try_cancel_add_sub(
                    dest,
                    IrBinOp::Sub,
                    lhs,
                    rhs,
                    ty,
                    cast_defs,
                    binop_defs,
                    neg_defs,
                ) {
                    return Some(inst);
                }
                // Reassociation: (x + C1) - C2 => x + (C1 - C2)
                // Also: (x - C1) - C2 => x - (C1 + C2)
                if let Some(inst) = try_reassociate_sub(dest, lhs, rhs, ty, binop_defs, use_counts)
                {
                    return Some(inst);
                }
                // x - (neg y) => x + y (eliminate negation)
                if let Some(neg_src) = get_neg_def(rhs, neg_defs) {
                    return Some(Instruction::BinOp {
                        dest,
                        op: IrBinOp::Add,
                        lhs: *lhs,
                        rhs: neg_src,
                        ty,
                    });
                }
            }
            // Floats: x - (neg y) => x + y — the same four-oracle
            // consensus fold as the Add mirror above, unconditional for
            // the same bit-parity reason (see the adjudication there).
            if is_float && !simplify_skip("addneg") {
                if let Some(neg_src) = get_neg_def(rhs, neg_defs) {
                    return Some(Instruction::BinOp {
                        dest,
                        op: IrBinOp::Add,
                        lhs: *lhs,
                        rhs: neg_src,
                        ty,
                    });
                }
            }
        }
        IrBinOp::Mul => {
            if !is_float && (rhs_zero || lhs_zero) {
                // x * 0 or 0 * x => 0 (integers only; for floats, NaN*0=NaN, -x*0=-0)
                return Some(Instruction::Copy {
                    dest,
                    src: Operand::Const(IrConst::zero(ty)),
                });
            }
            if rhs_one {
                // x * 1 => x (valid for both int and float)
                return Some(Instruction::Copy { dest, src: *lhs });
            }
            if lhs_one {
                // 1 * x => x (valid for both int and float)
                return Some(Instruction::Copy { dest, src: *rhs });
            }
            // Strength reduction: multiply by power-of-2 => shift left (integers only)
            if ty.is_integer() || ty == IrType::Ptr {
                if let Some(inst) = try_mul_power_of_two(dest, lhs, rhs, ty) {
                    return Some(inst);
                }
                if let Some(inst) = try_mul_power_of_two(dest, rhs, lhs, ty) {
                    return Some(inst);
                }
            }
            // x * (-1) => neg(x), (-1) * x => neg(x) (integers only)
            // A single neg instruction is cheaper than imul by -1.
            if !is_float {
                if is_neg_one(rhs, ty) {
                    return Some(Instruction::UnaryOp {
                        dest,
                        op: IrUnaryOp::Neg,
                        src: *lhs,
                        ty,
                    });
                }
                if is_neg_one(lhs, ty) {
                    return Some(Instruction::UnaryOp {
                        dest,
                        op: IrUnaryOp::Neg,
                        src: *rhs,
                        ty,
                    });
                }
            }
            // Reassociation: (x * C1) * C2 => x * (C1 * C2) (integers only)
            if !is_float {
                if let Some(inst) = try_reassociate_mul(dest, lhs, rhs, ty, binop_defs, use_counts)
                {
                    return Some(inst);
                }
            }
        }
        IrBinOp::SDiv | IrBinOp::UDiv => {
            if op == IrBinOp::SDiv
                && let Some(reciprocal) = normal_power_of_two_reciprocal(rhs, ty)
            {
                return Some(Instruction::BinOp {
                    dest,
                    op: IrBinOp::Mul,
                    lhs: *lhs,
                    rhs: Operand::Const(reciprocal),
                    ty,
                });
            }
            if rhs_one {
                // x / 1 => x (valid for both int and float)
                return Some(Instruction::Copy { dest, src: *lhs });
            }
            if op == IrBinOp::SDiv && !is_float && is_all_ones(rhs, ty) {
                return Some(sdiv_by_minus_one(dest, lhs, ty));
            }
            if !is_float && same_value {
                // x / x => 1 (integers only; for floats, 0/0=NaN, Inf/Inf=NaN)
                return Some(Instruction::Copy {
                    dest,
                    src: Operand::Const(IrConst::one(ty)),
                });
            }
            // Strength reduction: unsigned divide by power-of-2 => logical shift right
            // Note: We check the opcode (UDiv) for unsigned semantics, not the type.
            // On 64-bit targets, C integer promotion widens unsigned int ops to I64,
            // but UDiv still has unsigned semantics regardless of type signedness.
            if op == IrBinOp::UDiv && (ty.is_integer() || ty == IrType::Ptr) {
                if let Some(shift) = const_power_of_two(rhs, ty) {
                    return Some(Instruction::BinOp {
                        dest,
                        op: IrBinOp::LShr,
                        lhs: *lhs,
                        rhs: Operand::Const(IrConst::from_i64(shift as i64, ty)),
                        ty,
                    });
                }
            }
        }
        IrBinOp::SRem | IrBinOp::URem => {
            // x %s -1 => 0 for every defined input (INT_MIN % -1 overflows the
            // quotient, undefined in C), like x % 1.
            let signed_minus_one = op == IrBinOp::SRem && !is_float && is_all_ones(rhs, ty);
            if rhs_one || same_value || signed_minus_one {
                // x % 1 => 0, x % x => 0
                return Some(Instruction::Copy {
                    dest,
                    src: Operand::Const(IrConst::zero(ty)),
                });
            }
            // Strength reduction: unsigned rem by power-of-2 => bitwise AND with mask
            // Note: We check the opcode (URem) for unsigned semantics, not the type.
            // On 64-bit targets, C integer promotion widens unsigned int ops to I64,
            // but URem still has unsigned semantics regardless of type signedness.
            if op == IrBinOp::URem && (ty.is_integer() || ty == IrType::Ptr) {
                if let Some(shift) = const_power_of_two(rhs, ty) {
                    // x % 2^k => x & (2^k - 1).  The mask is built at the
                    // OPERATION's width, not i64's: `const_power_of_two`
                    // reports shifts up to 127 for the 128-bit types, and an
                    // i64 construction of the mask is wrong in two ways --
                    // `1i64 << 64` is `1` in a release build (the shift amount
                    // is masked modulo 64), so a 2^64 mask became 0 and
                    // `x % 2^64` compiled to `x & 0`; and `(1i64 << 63) - 1`
                    // panics in a debug build.  See `IrConst::low_mask`.
                    return Some(Instruction::BinOp {
                        dest,
                        op: IrBinOp::And,
                        lhs: *lhs,
                        rhs: Operand::Const(IrConst::low_mask(ty, shift)),
                        ty,
                    });
                }
            }
        }
        IrBinOp::And => {
            if rhs_zero || lhs_zero {
                // x & 0 => 0
                return Some(Instruction::Copy {
                    dest,
                    src: Operand::Const(IrConst::zero(ty)),
                });
            }
            if is_all_ones(rhs, ty) {
                // x & all_ones => x (identity element for AND)
                return Some(Instruction::Copy { dest, src: *lhs });
            }
            if is_all_ones(lhs, ty) {
                // all_ones & x => x
                return Some(Instruction::Copy { dest, src: *rhs });
            }
            if same_value {
                // x & x => x
                return Some(Instruction::Copy { dest, src: *lhs });
            }
            // Canonicalize `(base >> index) & 1` to a cross-target BitTest.
            // Both operand orders are accepted; for a signed right shift the
            // isolated low bit is identical to a logical shift, so AShr is
            // safe here too.
            let mask_is_one = is_one(lhs) || is_one(rhs);
            if mask_is_one && ty.is_integer() {
                let candidate = if is_one(lhs) { rhs } else { lhs };
                // Peek through same-width integer casts. They are often
                // inserted by C integer promotion (`(int)(unsigned)x`) but do
                // change the isolated low bit.
                let peeled = match candidate {
                    Operand::Value(v) => match cast_defs.get(v.0 as usize).copied().flatten() {
                        Some(CastDef {
                            from_ty,
                            to_ty,
                            src,
                        }) if from_ty.is_integer()
                            && to_ty.is_integer()
                            && from_ty.size() == to_ty.size() =>
                        {
                            src
                        }
                        _ => *candidate,
                    },
                    _ => *candidate,
                };
                if let Some(def) = get_binop_def(&peeled, binop_defs) {
                    if matches!(def.op, IrBinOp::LShr | IrBinOp::AShr) {
                        // Isolating the low bit is independent of whether the
                        // shift was arithmetic or logical; only that bit can
                        // reach `& 1`. Preserve the source width. A typed
                        // narrow/wide mismatch uses the natural 32-bit width so
                        // the backend receives a legal i32 BitTest.
                        let bt_ty = if def.ty == ty { ty } else { IrType::I32 };
                        // LEGALITY: BitTest must fit the native BT width. On a
                        // 32-bit target an I64/U64 BitTest reaches the wide-int
                        // pair path, which has no BT lowering — upstream this
                        // was an ICE ("unhandled i128 binary op: BitTest") for
                        // `(u64 >> i) & 1` at -m32. Keep the shift+and form
                        // there; the pair shift lowering handles it correctly.
                        let native_ok = !crate::common::types::target_is_32bit()
                            || matches!(
                                bt_ty,
                                IrType::I8
                                    | IrType::U8
                                    | IrType::I16
                                    | IrType::U16
                                    | IrType::I32
                                    | IrType::U32
                            );
                        if native_ok && !bt_ty.is_128bit() {
                            return Some(Instruction::BinOp {
                                dest,
                                op: IrBinOp::BitTest,
                                lhs: def.lhs,
                                rhs: def.rhs,
                                ty: bt_ty,
                            });
                        }
                    }
                }
            }
            // Reassociation: (x & C1) & C2 => x & (C1 & C2)
            if let Some(inst) =
                try_reassociate_bitwise(dest, IrBinOp::And, lhs, rhs, ty, binop_defs, use_counts)
            {
                return Some(inst);
            }
        }
        IrBinOp::Or => {
            if rhs_zero {
                // x | 0 => x
                return Some(Instruction::Copy { dest, src: *lhs });
            }
            if lhs_zero {
                // 0 | x => x
                return Some(Instruction::Copy { dest, src: *rhs });
            }
            if is_all_ones(rhs, ty) || is_all_ones(lhs, ty) {
                // x | all_ones => all_ones (annihilator for OR)
                return Some(Instruction::Copy {
                    dest,
                    src: Operand::Const(all_ones_const(ty)),
                });
            }
            if same_value {
                // x | x => x
                return Some(Instruction::Copy { dest, src: *lhs });
            }
            // Reassociation: (x | C1) | C2 => x | (C1 | C2)
            if let Some(inst) =
                try_reassociate_bitwise(dest, IrBinOp::Or, lhs, rhs, ty, binop_defs, use_counts)
            {
                return Some(inst);
            }
        }
        IrBinOp::Xor => {
            if rhs_zero {
                // x ^ 0 => x
                return Some(Instruction::Copy { dest, src: *lhs });
            }
            if lhs_zero {
                // 0 ^ x => x
                return Some(Instruction::Copy { dest, src: *rhs });
            }
            if same_value {
                // x ^ x => 0
                return Some(Instruction::Copy {
                    dest,
                    src: Operand::Const(IrConst::zero(ty)),
                });
            }
            // Reassociation: (x ^ C1) ^ C2 => x ^ (C1 ^ C2)
            if let Some(inst) =
                try_reassociate_bitwise(dest, IrBinOp::Xor, lhs, rhs, ty, binop_defs, use_counts)
            {
                return Some(inst);
            }
        }
        IrBinOp::Shl | IrBinOp::AShr | IrBinOp::LShr => {
            if op == IrBinOp::AShr && rhs_one {
                if let Some(product) = unbiased_even_product_half(*lhs, ty, cast_defs, binop_defs) {
                    return Some(Instruction::BinOp {
                        dest,
                        op,
                        lhs: product,
                        rhs: *rhs,
                        ty,
                    });
                }
            }
            if rhs_zero {
                // x << 0 or x >> 0 => x
                return Some(Instruction::Copy { dest, src: *lhs });
            }
            if lhs_zero {
                // 0 << x or 0 >> x => 0
                return Some(Instruction::Copy {
                    dest,
                    src: Operand::Const(IrConst::zero(ty)),
                });
            }
            if op == IrBinOp::AShr && is_all_ones(lhs, ty) {
                // Arithmetic right shift replicates the sign bit, so -1 stays
                // all-ones for every defined shift count.
                return Some(Instruction::Copy {
                    dest,
                    src: Operand::Const(IrConst::from_i64(-1, ty)),
                });
            }
            // Reassociation: (x << C1) << C2 => x << (C1 + C2)
            // Also for >>: (x >> C1) >> C2 => x >> (C1 + C2)
            if let Some(inst) =
                try_reassociate_shift(dest, op, lhs, rhs, ty, binop_defs, use_counts)
            {
                return Some(inst);
            }
            // High-bit logical shift over bitwise Xor/Or with a zero-extended
            // narrower operand: `(a ^ zext_W(b)) >> s => a >> s` (and `|`) when
            // `s >= W`. Zero-extension sets bits `W..` to 0, and bitwise Xor/Or
            // has no carry, so bits `s..` of `a ^ zext_W(b)` equal bits `s..`
            // of `a`. Breaks the loop-carried `crc` serial dependency chain in
            // slice-by-8 CRC32 (`gzip_crc32_slice8`) and exposes 32-bit `xorl`
            // narrowing on the remaining low half. Gated to SSA mode
            // (`use_counts.is_empty()`) so post-SSA `-O1` i686 webs stay intact.
            if op == IrBinOp::LShr
                && use_counts.is_empty()
                && !simplify_skip("reassoc")
                && matches!(ty, IrType::I64 | IrType::U64 | IrType::I32 | IrType::U32)
                && let Operand::Const(c) = rhs
                && let Some(shift) = c.to_i64()
            {
                let width = (ty.size() * 8) as i64;
                if shift > 0
                    && shift < width
                    && let Some(def) = get_binop_def(lhs, binop_defs)
                    && matches!(def.op, IrBinOp::Xor | IrBinOp::Or)
                    && def.ty == ty
                {
                    let high_bits_zero = |opnd: &Operand| -> bool {
                        let Operand::Value(v) = opnd else {
                            return false;
                        };
                        let Some(Some(cd)) = cast_defs.get(v.0 as usize) else {
                            return false;
                        };
                        cd.to_ty == ty
                            && cd.from_ty.is_unsigned()
                            && cd.from_ty.is_integer()
                            && ((cd.from_ty.size() * 8) as i64) <= shift
                    };
                    if high_bits_zero(&def.rhs) {
                        return Some(Instruction::BinOp {
                            dest,
                            op,
                            lhs: def.lhs,
                            rhs: *rhs,
                            ty,
                        });
                    }
                    if high_bits_zero(&def.lhs) {
                        return Some(Instruction::BinOp {
                            dest,
                            op,
                            lhs: def.rhs,
                            rhs: *rhs,
                            ty,
                        });
                    }
                }
            }
        }
        IrBinOp::BitTest => {
            // `BitTest(base, index)` is `(base >> index) & 1` — see
            // `fold_binop` in constant_fold.rs and `IrBinOp::eval` in
            // ir/ops.rs. Only a zero *base* folds to zero.
            //
            // A zero *index* folds to NOTHING: `(base >> 0) & 1` is `base & 1`,
            // i.e. bit zero of the base, which is 1 for every odd base.
            // Folding it to zero was a miscompile (found by
            // scripts/gen_slot_stress.py: an `if (((x >> 0) & 1) ^ 1)` was
            // folded to a constant, taking the wrong arm of a diamond). Keep
            // the BitTest so x86 can still select BT.
            if lhs_zero {
                return Some(Instruction::Copy {
                    dest,
                    src: Operand::Const(IrConst::zero(ty)),
                });
            }
        }
        IrBinOp::RotateLeft | IrBinOp::RotateRight => {
            // A rotate permutes the operand's bits, which makes it the one
            // shift-family operation whose zero *amount* genuinely is an
            // identity -- contrast the BitTest arm above, where a zero index
            // still selects bit zero and folding it was a miscompile.  The
            // amount reduces modulo the operation width (exactly what the
            // x86/AArch64/RISC-V count masks do), so `rotl(x, 32)` on an I32
            // folds to `x` instead of surviving as a rotation the backend
            // would still spend an instruction on.
            if lhs_zero {
                return Some(Instruction::Copy {
                    dest,
                    src: Operand::Const(IrConst::zero(ty)),
                });
            }
            if let Operand::Const(c) = rhs
                && let Some(amount) = c.to_i128()
            {
                let bits = (ty.size() * 8) as i128;
                if bits > 0 && amount.rem_euclid(bits) == 0 {
                    return Some(Instruction::Copy { dest, src: *lhs });
                }
            }
        }
        IrBinOp::Bzhi => {
            if let Some(rewritten) = simplify_bzhi(dest, lhs, rhs, ty, lhs_zero) {
                return Some(rewritten);
            }
        }
        IrBinOp::UMulHigh | IrBinOp::SMulHigh => {
            if let Some(rewritten) = simplify_mul_high(dest, op, lhs, rhs, ty, lhs_zero || rhs_zero)
            {
                return Some(rewritten);
            }
        }
        IrBinOp::BitSet | IrBinOp::BitClear | IrBinOp::BitToggle => {
            // Bit set/clear/toggle are produced by `bit_idioms` from the
            // shift/OR, shift/NOT/AND and shift/XOR shapes (see
            // `bit_idioms::bit_select`), so there is no producer here.  Only
            // one identity holds without knowing the index: clearing any bit
            // of zero leaves zero.  Setting or toggling a bit of zero is
            // `1 << (n mod W)`, never a constant, so there is no mirrored fold
            // -- folding it to zero (or to one) was the same trap as the
            // zero-index `BitTest` above.
            //
            // The other two do have a defining identity that needs no index:
            // toggling a bit twice -- or clearing a bit an already-clear mask
            // has -- is idempotent/involutive.  Both need the *producer* to be
            // the same operation on the same index, which this pass cannot see
            // from one instruction, so they stay in the corpus's "not done"
            // list rather than being guessed at here.
            if op == IrBinOp::BitClear && lhs_zero {
                return Some(Instruction::Copy {
                    dest,
                    src: Operand::Const(IrConst::zero(ty)),
                });
            }
        }
    }

    // Commutative operand canonicalization: for commutative operations
    // (Add, Mul, And, Or, Xor), place the constant on the RHS so the backend
    // can use immediate-form instructions (e.g., `addq $imm, %reg` instead of
    // loading the constant into a register first).
    if op.is_commutative() && matches!(lhs, Operand::Const(_)) && matches!(rhs, Operand::Value(_)) {
        return Some(Instruction::BinOp {
            dest,
            op,
            lhs: *rhs,
            rhs: *lhs,
            ty,
        });
    }

    None
}

/// Check if an operand is the all-ones bit pattern for the given integer type.
/// All-ones is the identity element for AND and the annihilator for OR.
/// Examples: 0xFF for I8/U8, 0xFFFFFFFF for I32/U32, -1 for I64/U64.
fn is_all_ones(op: &Operand, ty: IrType) -> bool {
    if !ty.is_integer() {
        return false;
    }
    match op {
        Operand::Const(c) => {
            // For 128-bit types, check the full 128-bit value directly.
            // We must NOT use to_i64() for I128 constants because it truncates
            // the upper 64 bits, causing false positives (e.g., I128(0xFFFFFFFFFFFFFFFF)
            // would truncate to i64 -1, which would incorrectly match "all ones" for U128).
            if ty.is_128bit() {
                if let IrConst::I128(v) = c {
                    *v == -1i128
                } else {
                    false
                }
            } else {
                match c.to_i64() {
                    Some(val) => {
                        // After truncation to the type's width, all-ones is -1.
                        ty.truncate_i64(val) == ty.truncate_i64(-1)
                    }
                    None => false,
                }
            }
        }
        _ => false,
    }
}

/// Create the all-ones constant for a given integer type.
fn all_ones_const(ty: IrType) -> IrConst {
    if ty.is_128bit() {
        IrConst::I128(-1i128)
    } else {
        IrConst::from_i64(-1, ty)
    }
}

/// Check if an operand is zero (including both +0.0 and -0.0 for floats).
fn is_zero(op: &Operand) -> bool {
    matches!(op, Operand::Const(c) if c.is_zero())
}

/// Check if a float operand is positive zero (+0.0), using the bit pattern.
/// Returns false for -0.0, non-zero values, and non-float types.
fn is_positive_zero(op: &Operand) -> bool {
    match op {
        Operand::Const(IrConst::F32(v)) => *v == 0.0 && !v.is_sign_negative(),
        Operand::Const(IrConst::F64(v)) => *v == 0.0 && !v.is_sign_negative(),
        Operand::Const(IrConst::LongDouble(v, _)) => *v == 0.0 && !v.is_sign_negative(),
        _ => false,
    }
}

/// Check if an operand is one.
fn is_one(op: &Operand) -> bool {
    matches!(op, Operand::Const(c) if c.is_one())
}

/// Check if an operand is -1 for the given integer type.
/// This is the two's complement representation: all bits set.
/// Used for x * (-1) => neg(x) optimization.
fn is_neg_one(op: &Operand, ty: IrType) -> bool {
    if !ty.is_integer() {
        return false;
    }
    match op {
        Operand::Const(c) => {
            // For 128-bit types, check the full 128-bit value directly to avoid
            // truncation through to_i64() (same issue as is_all_ones).
            if ty.is_128bit() {
                if let IrConst::I128(v) = c {
                    *v == -1i128
                } else {
                    false
                }
            } else {
                match c.to_i64() {
                    Some(val) => ty.truncate_i64(val) == ty.truncate_i64(-1),
                    None => false,
                }
            }
        }
        _ => false,
    }
}

/// The source of a `Not` defining `op`, if any.
fn get_not_def(op: &Operand, not_defs: &[Option<Operand>]) -> Option<Operand> {
    let Operand::Value(v) = op else {
        return None;
    };
    not_defs.get(v.0 as usize).copied().flatten()
}

/// Look up a NegDef for a Value operand.
/// Returns the source operand of the negation if the operand is defined by UnaryOp::Neg.
fn get_neg_def(op: &Operand, neg_defs: &[Option<NegDef>]) -> Option<Operand> {
    if let Operand::Value(v) = op {
        let idx = v.0 as usize;
        if let Some(Some(def)) = neg_defs.get(idx) {
            return Some(def.src);
        }
    }
    None
}

/// Check if two operands refer to the same value.
fn same_value_operands(lhs: &Operand, rhs: &Operand) -> bool {
    match (lhs, rhs) {
        (Operand::Value(a), Operand::Value(b)) => a.0 == b.0,
        _ => false,
    }
}

/// Check if two operands are identical (same value or same constant).
fn same_operand(a: &Operand, b: &Operand) -> bool {
    match (a, b) {
        (Operand::Value(va), Operand::Value(vb)) => va.0 == vb.0,
        (Operand::Const(ca), Operand::Const(cb)) => ca.to_hash_key() == cb.to_hash_key(),
        _ => false,
    }
}

/// Look up a BinOpDef for a Value operand.
fn get_binop_def<'a>(op: &Operand, binop_defs: &'a [Option<BinOpDef>]) -> Option<&'a BinOpDef> {
    if let Operand::Value(v) = op {
        let idx = v.0 as usize;
        if let Some(Some(def)) = binop_defs.get(idx) {
            return Some(def);
        }
    }
    None
}

/// Combine two i64 constants with wrapping addition, truncated to the type's width.
fn combine_add_consts(c1: i64, c2: i64, ty: IrType) -> i64 {
    ty.truncate_i64(c1.wrapping_add(c2))
}

/// Try reassociating addition: (x + C1) + C2 => x + (C1 + C2)
/// Also handles: C2 + (x + C1) => x + (C1 + C2)
/// Reassociation profitability guard: folding through an intermediate whose
/// value has other consumers re-bases the def onto a different register web
/// without deleting any instruction (work-neutral at best). On i686 that web
/// split changed the def's home register while the phi-eliminated merge kept
/// the old expectation, miscompiling gcc.c-torture 20050124-1 (the k++ arm's
/// `leal` landed in %edx while the merge read %ecx). Only fold through
/// single-use intermediates so the intermediate's def actually dies and the
/// rewrite is a strict win. An empty use_counts slice (test entry points)
/// keeps the historical behavior.
fn reassociation_unprofitable(intermediate: &Operand, use_counts: &[u32]) -> bool {
    if simplify_skip("reassoc") {
        return true;
    }
    if use_counts.is_empty() {
        return false;
    }
    // Bisection kill-switch (mirrors the other CCC_NO_* hooks): forces the
    // historical always-reassociate behavior.
    if std::env::var("CCC_NO_REASSOC_GUARD").is_ok() {
        return false;
    }
    match intermediate {
        Operand::Value(v) => use_counts.get(v.0 as usize).copied().unwrap_or(0) > 1,
        _ => false,
    }
}

/// Floats: `x + (-y) => x - y` and `(-x) + y => y - x` (four-oracle consensus:
/// GCC 16.2, Clang 23.1, ICX).
fn simplify_float_add_neg(
    dest: Value,
    lhs: &Operand,
    rhs: &Operand,
    ty: IrType,
    neg_defs: &[Option<NegDef>],
) -> Option<Instruction> {
    if let Some(neg_src) = get_neg_def(rhs, neg_defs) {
        Some(Instruction::BinOp {
            dest,
            op: IrBinOp::Sub,
            lhs: *lhs,
            rhs: neg_src,
            ty,
        })
    } else {
        get_neg_def(lhs, neg_defs).map(|neg_src| Instruction::BinOp {
            dest,
            op: IrBinOp::Sub,
            lhs: *rhs,
            rhs: neg_src,
            ty,
        })
    }
}

/// Peel same-width integer casts (e.g., `I32 <-> U32`, `I64 <-> U64`), which
/// preserve the full bit pattern under modular `Add` / `Sub`.
fn peel_same_width_int_casts(
    mut op: Operand,
    width: usize,
    cast_defs: &[Option<CastDef>],
) -> Operand {
    for _ in 0..4 {
        if let Operand::Value(v) = op
            && let Some(Some(cd)) = cast_defs.get(v.0 as usize).copied()
            && cd.from_ty.is_integer()
            && cd.to_ty.is_integer()
            && cd.from_ty.size() == width
            && cd.to_ty.size() == width
        {
            op = cd.src;
        } else {
            break;
        }
    }
    op
}

fn coerce_same_width_int_result(
    dest: Value,
    src: Operand,
    src_ty: IrType,
    dest_ty: IrType,
) -> Option<Instruction> {
    if src_ty == dest_ty {
        return Some(Instruction::Copy { dest, src });
    }
    if !src_ty.is_integer() || !dest_ty.is_integer() || src_ty.size() != dest_ty.size() {
        return None;
    }
    match src {
        Operand::Const(c) => {
            let v = c.to_i64()?;
            Some(Instruction::Copy {
                dest,
                src: Operand::Const(IrConst::from_i64(v, dest_ty)),
            })
        }
        Operand::Value(_) => Some(Instruction::Cast {
            dest,
            src,
            from_ty: src_ty,
            to_ty: dest_ty,
        }),
    }
}

/// Modular integer ring cancellations across `Add` and `Sub` (peeling same-width
/// signed/unsigned casts):
/// - `x + (y - x) => y` and `(y - x) + x => y`
/// - `x + (-x) => 0` and `(-x) + x => 0`
/// - `(x + y) - x => y` and `(y + x) - x => y`
/// - `x - (x - y) => y`
fn try_cancel_add_sub(
    dest: Value,
    op: IrBinOp,
    lhs: &Operand,
    rhs: &Operand,
    ty: IrType,
    cast_defs: &[Option<CastDef>],
    binop_defs: &[Option<BinOpDef>],
    neg_defs: &[Option<NegDef>],
) -> Option<Instruction> {
    if !ty.is_integer() {
        return None;
    }
    let w = ty.size();
    if w == 0 || w > 8 {
        return None;
    }
    let pl = peel_same_width_int_casts(*lhs, w, cast_defs);
    let pr = peel_same_width_int_casts(*rhs, w, cast_defs);

    let bin_of = |o: Operand| -> Option<BinOpDef> {
        let Operand::Value(v) = o else {
            return None;
        };
        let bd = (*binop_defs.get(v.0 as usize)?)?;
        if bd.ty.is_integer() && bd.ty.size() == w {
            Some(bd)
        } else {
            None
        }
    };

    match op {
        IrBinOp::Add => {
            if let Some(neg_src) = get_neg_def(&pr, neg_defs)
                && peel_same_width_int_casts(neg_src, w, cast_defs) == pl
            {
                return Some(Instruction::Copy {
                    dest,
                    src: Operand::Const(IrConst::zero(ty)),
                });
            }
            if let Some(neg_src) = get_neg_def(&pl, neg_defs)
                && peel_same_width_int_casts(neg_src, w, cast_defs) == pr
            {
                return Some(Instruction::Copy {
                    dest,
                    src: Operand::Const(IrConst::zero(ty)),
                });
            }
            if let Some(bd) = bin_of(pr)
                && bd.op == IrBinOp::Sub
                && peel_same_width_int_casts(bd.rhs, w, cast_defs) == pl
            {
                return coerce_same_width_int_result(dest, bd.lhs, bd.ty, ty);
            }
            if let Some(bd) = bin_of(pl)
                && bd.op == IrBinOp::Sub
                && peel_same_width_int_casts(bd.rhs, w, cast_defs) == pr
            {
                return coerce_same_width_int_result(dest, bd.lhs, bd.ty, ty);
            }
            None
        }
        IrBinOp::Sub => {
            if let Some(bd) = bin_of(pl)
                && bd.op == IrBinOp::Add
            {
                if peel_same_width_int_casts(bd.lhs, w, cast_defs) == pr {
                    return coerce_same_width_int_result(dest, bd.rhs, bd.ty, ty);
                }
                if peel_same_width_int_casts(bd.rhs, w, cast_defs) == pr {
                    return coerce_same_width_int_result(dest, bd.lhs, bd.ty, ty);
                }
            }
            if let Some(bd) = bin_of(pr)
                && bd.op == IrBinOp::Sub
                && peel_same_width_int_casts(bd.lhs, w, cast_defs) == pl
            {
                return coerce_same_width_int_result(dest, bd.rhs, bd.ty, ty);
            }
            None
        }
        _ => None,
    }
}

fn try_reassociate_add(
    dest: Value,
    lhs: &Operand,
    rhs: &Operand,
    ty: IrType,
    binop_defs: &[Option<BinOpDef>],
    use_counts: &[u32],
) -> Option<Instruction> {
    // Skip I128/U128: to_i64() truncates, which would silently corrupt 128-bit constants.
    if matches!(ty, IrType::I128 | IrType::U128) {
        return None;
    }
    if reassociation_unprofitable(lhs, use_counts) {
        return None;
    }
    // Pattern: (x + C1) + C2 or (x - C1) + C2
    if let Operand::Const(c2) = rhs {
        let c2_val = c2.to_i64()?;
        if let Some(def) = get_binop_def(lhs, binop_defs) {
            if def.ty == ty {
                match def.op {
                    IrBinOp::Add => {
                        // (x + C1) + C2 => x + (C1 + C2)
                        if let Operand::Const(c1) = &def.rhs {
                            let c1_val = c1.to_i64()?;
                            let combined = combine_add_consts(c1_val, c2_val, ty);
                            if combined == 0 {
                                return Some(Instruction::Copy { dest, src: def.lhs });
                            }
                            return Some(Instruction::BinOp {
                                dest,
                                op: IrBinOp::Add,
                                lhs: def.lhs,
                                rhs: Operand::Const(IrConst::from_i64(combined, ty)),
                                ty,
                            });
                        }
                        // (C1 + x) + C2 => x + (C1 + C2)
                        if let Operand::Const(c1) = &def.lhs {
                            let c1_val = c1.to_i64()?;
                            let combined = combine_add_consts(c1_val, c2_val, ty);
                            if combined == 0 {
                                return Some(Instruction::Copy { dest, src: def.rhs });
                            }
                            return Some(Instruction::BinOp {
                                dest,
                                op: IrBinOp::Add,
                                lhs: def.rhs,
                                rhs: Operand::Const(IrConst::from_i64(combined, ty)),
                                ty,
                            });
                        }
                    }
                    IrBinOp::Sub => {
                        // (x - C1) + C2 => x + (C2 - C1)
                        if let Operand::Const(c1) = &def.rhs {
                            let c1_val = c1.to_i64()?;
                            let combined = ty.truncate_i64(c2_val.wrapping_sub(c1_val));
                            if combined == 0 {
                                return Some(Instruction::Copy { dest, src: def.lhs });
                            }
                            return Some(Instruction::BinOp {
                                dest,
                                op: IrBinOp::Add,
                                lhs: def.lhs,
                                rhs: Operand::Const(IrConst::from_i64(combined, ty)),
                                ty,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    // Pattern: C2 + (x + C1) -- commuted case
    if let Operand::Const(c2) = lhs {
        let c2_val = c2.to_i64()?;
        if let Some(def) = get_binop_def(rhs, binop_defs) {
            if def.ty == ty && def.op == IrBinOp::Add {
                if let Operand::Const(c1) = &def.rhs {
                    let c1_val = c1.to_i64()?;
                    let combined = combine_add_consts(c1_val, c2_val, ty);
                    if combined == 0 {
                        return Some(Instruction::Copy { dest, src: def.lhs });
                    }
                    return Some(Instruction::BinOp {
                        dest,
                        op: IrBinOp::Add,
                        lhs: def.lhs,
                        rhs: Operand::Const(IrConst::from_i64(combined, ty)),
                        ty,
                    });
                }
                if let Operand::Const(c1) = &def.lhs {
                    let c1_val = c1.to_i64()?;
                    let combined = combine_add_consts(c1_val, c2_val, ty);
                    if combined == 0 {
                        return Some(Instruction::Copy { dest, src: def.rhs });
                    }
                    return Some(Instruction::BinOp {
                        dest,
                        op: IrBinOp::Add,
                        lhs: def.rhs,
                        rhs: Operand::Const(IrConst::from_i64(combined, ty)),
                        ty,
                    });
                }
            }
        }
    }
    None
}

/// Try reassociating subtraction:
/// - (x + C1) - C2 => x + (C1 - C2)
/// - (x - C1) - C2 => x - (C1 + C2)
fn try_reassociate_sub(
    dest: Value,
    lhs: &Operand,
    rhs: &Operand,
    ty: IrType,
    binop_defs: &[Option<BinOpDef>],
    use_counts: &[u32],
) -> Option<Instruction> {
    // Skip I128/U128: to_i64() truncates, which would silently corrupt 128-bit constants.
    if matches!(ty, IrType::I128 | IrType::U128) {
        return None;
    }
    if reassociation_unprofitable(lhs, use_counts) {
        return None;
    }
    let c2_val = match rhs {
        Operand::Const(c) => c.to_i64()?,
        _ => return None,
    };
    let def = get_binop_def(lhs, binop_defs)?;
    if def.ty != ty {
        return None;
    }
    match def.op {
        IrBinOp::Add => {
            // (x + C1) - C2 => x + (C1 - C2)
            if let Operand::Const(c1) = &def.rhs {
                let c1_val = c1.to_i64()?;
                let combined = ty.truncate_i64(c1_val.wrapping_sub(c2_val));
                if combined == 0 {
                    return Some(Instruction::Copy { dest, src: def.lhs });
                }
                return Some(Instruction::BinOp {
                    dest,
                    op: IrBinOp::Add,
                    lhs: def.lhs,
                    rhs: Operand::Const(IrConst::from_i64(combined, ty)),
                    ty,
                });
            }
            // (C1 + x) - C2 => x + (C1 - C2)
            if let Operand::Const(c1) = &def.lhs {
                let c1_val = c1.to_i64()?;
                let combined = ty.truncate_i64(c1_val.wrapping_sub(c2_val));
                if combined == 0 {
                    return Some(Instruction::Copy { dest, src: def.rhs });
                }
                return Some(Instruction::BinOp {
                    dest,
                    op: IrBinOp::Add,
                    lhs: def.rhs,
                    rhs: Operand::Const(IrConst::from_i64(combined, ty)),
                    ty,
                });
            }
        }
        IrBinOp::Sub => {
            // (x - C1) - C2 => x - (C1 + C2)
            if let Operand::Const(c1) = &def.rhs {
                let c1_val = c1.to_i64()?;
                let combined = combine_add_consts(c1_val, c2_val, ty);
                if combined == 0 {
                    return Some(Instruction::Copy { dest, src: def.lhs });
                }
                return Some(Instruction::BinOp {
                    dest,
                    op: IrBinOp::Sub,
                    lhs: def.lhs,
                    rhs: Operand::Const(IrConst::from_i64(combined, ty)),
                    ty,
                });
            }
        }
        _ => {}
    }
    None
}

/// Try reassociating bitwise operations: (x & C1) & C2 => x & (C1 & C2)
/// Works for And, Or, and Xor (all associative and commutative).
fn try_reassociate_bitwise(
    dest: Value,
    op: IrBinOp,
    lhs: &Operand,
    rhs: &Operand,
    ty: IrType,
    binop_defs: &[Option<BinOpDef>],
    use_counts: &[u32],
) -> Option<Instruction> {
    // Skip I128/U128: to_i64() truncates, which would silently corrupt 128-bit constants.
    if matches!(ty, IrType::I128 | IrType::U128) {
        return None;
    }
    if reassociation_unprofitable(lhs, use_counts) {
        return None;
    }
    // Pattern: (x op C1) op C2
    let c2_val = match rhs {
        Operand::Const(c) => c.to_i64()?,
        _ => return None,
    };
    let def = get_binop_def(lhs, binop_defs)?;
    if def.op != op || def.ty != ty {
        return None;
    }
    // (x op C1) op C2 => x op (C1 op C2)
    let (non_const_operand, c1_val) = if let Operand::Const(c1) = &def.rhs {
        (def.lhs, c1.to_i64()?)
    } else if let Operand::Const(c1) = &def.lhs {
        (def.rhs, c1.to_i64()?)
    } else {
        return None;
    };

    let combined = match op {
        IrBinOp::And => c1_val & c2_val,
        IrBinOp::Or => c1_val | c2_val,
        IrBinOp::Xor => c1_val ^ c2_val,
        _ => return None,
    };
    let combined = ty.truncate_i64(combined);

    // Check if combined result is identity and can eliminate the instruction
    let is_identity = match op {
        IrBinOp::And => is_all_ones(&Operand::Const(IrConst::from_i64(combined, ty)), ty),
        IrBinOp::Or | IrBinOp::Xor => combined == 0,
        _ => false,
    };
    if is_identity {
        return Some(Instruction::Copy {
            dest,
            src: non_const_operand,
        });
    }

    // Check if combined result is annihilator
    let is_annihilator = match op {
        IrBinOp::And => combined == 0,
        IrBinOp::Or => is_all_ones(&Operand::Const(IrConst::from_i64(combined, ty)), ty),
        _ => false,
    };
    if is_annihilator {
        return Some(Instruction::Copy {
            dest,
            src: Operand::Const(IrConst::from_i64(combined, ty)),
        });
    }

    Some(Instruction::BinOp {
        dest,
        op,
        lhs: non_const_operand,
        rhs: Operand::Const(IrConst::from_i64(combined, ty)),
        ty,
    })
}

/// Try reassociating shift operations: (x << C1) << C2 => x << (C1 + C2)
/// Works for Shl, AShr, and LShr (but only when same shift direction).
fn try_reassociate_shift(
    dest: Value,
    op: IrBinOp,
    lhs: &Operand,
    rhs: &Operand,
    ty: IrType,
    binop_defs: &[Option<BinOpDef>],
    use_counts: &[u32],
) -> Option<Instruction> {
    // Skip I128/U128: to_i64() truncates, which would silently corrupt 128-bit constants.
    if matches!(ty, IrType::I128 | IrType::U128) {
        return None;
    }
    if reassociation_unprofitable(lhs, use_counts) {
        return None;
    }
    let c2_val = match rhs {
        Operand::Const(c) => c.to_i64()?,
        _ => return None,
    };
    if c2_val < 0 {
        return None;
    }
    let def = get_binop_def(lhs, binop_defs)?;
    // Must be the same shift operation
    if def.op != op || def.ty != ty {
        return None;
    }
    // (x shift C1) shift C2 => x shift (C1 + C2)
    let c1_val = match &def.rhs {
        Operand::Const(c) => c.to_i64()?,
        _ => return None,
    };
    if c1_val < 0 {
        return None;
    }

    let combined = c1_val + c2_val;
    let bit_width = (ty.size() * 8) as i64;

    // If combined shift exceeds bit width, result is 0 (for Shl/LShr)
    // or sign-extended (for AShr). Conservatively only fold to 0 for Shl/LShr.
    if combined >= bit_width {
        if matches!(op, IrBinOp::Shl | IrBinOp::LShr) {
            return Some(Instruction::Copy {
                dest,
                src: Operand::Const(IrConst::zero(ty)),
            });
        }
        // For AShr, don't fold -- the result depends on the sign bit
        return None;
    }

    Some(Instruction::BinOp {
        dest,
        op,
        lhs: def.lhs,
        rhs: Operand::Const(IrConst::from_i64(combined, ty)),
        ty,
    })
}

/// Try reassociating multiplication: (x * C1) * C2 => x * (C1 * C2)
/// Also handles: C2 * (x * C1) => x * (C1 * C2) (commuted)
/// Integer multiplication is associative, so folding two constant factors into
/// one eliminates an instruction.
fn try_reassociate_mul(
    dest: Value,
    lhs: &Operand,
    rhs: &Operand,
    ty: IrType,
    binop_defs: &[Option<BinOpDef>],
    use_counts: &[u32],
) -> Option<Instruction> {
    // Skip I128/U128: to_i64() truncates, which would silently corrupt 128-bit constants.
    if matches!(ty, IrType::I128 | IrType::U128) {
        return None;
    }
    if reassociation_unprofitable(lhs, use_counts) || reassociation_unprofitable(rhs, use_counts) {
        return None;
    }

    // Helper: try pattern (expr * C1) * C2 where expr is on one side and C2 on the other.
    let try_pattern = |expr_side: &Operand, const_side: &Operand| -> Option<Instruction> {
        let c2_val = match const_side {
            Operand::Const(c) => c.to_i64()?,
            _ => return None,
        };
        let def = get_binop_def(expr_side, binop_defs)?;
        if def.op != IrBinOp::Mul || def.ty != ty {
            return None;
        }
        // Find the constant in the inner multiply: (x * C1) or (C1 * x)
        let (non_const, c1_val) = if let Operand::Const(c1) = &def.rhs {
            (def.lhs, c1.to_i64()?)
        } else if let Operand::Const(c1) = &def.lhs {
            (def.rhs, c1.to_i64()?)
        } else {
            return None;
        };
        let combined = ty.truncate_i64(c1_val.wrapping_mul(c2_val));
        if combined == 0 {
            return Some(Instruction::Copy {
                dest,
                src: Operand::Const(IrConst::zero(ty)),
            });
        }
        if combined == 1 || ty.truncate_i64(combined) == ty.truncate_i64(1) {
            return Some(Instruction::Copy {
                dest,
                src: non_const,
            });
        }
        Some(Instruction::BinOp {
            dest,
            op: IrBinOp::Mul,
            lhs: non_const,
            rhs: Operand::Const(IrConst::from_i64(combined, ty)),
            ty,
        })
    };

    // Try both orderings: (x*C1)*C2 and C2*(x*C1)
    if let Some(inst) = try_pattern(lhs, rhs) {
        return Some(inst);
    }
    try_pattern(rhs, lhs)
}

/// Table of unary math functions that map directly to intrinsic instructions.
const UNARY_INTRINSICS: &[(&str, IntrinsicOp)] = &[
    ("sqrt", IntrinsicOp::SqrtF64),
    ("sqrtf", IntrinsicOp::SqrtF32),
    ("fabs", IntrinsicOp::FabsF64),
    ("fabsf", IntrinsicOp::FabsF32),
    // _Float128 / long-double bit ops: inline beats any libgcc/libm call.
    ("__fabstf2", IntrinsicOp::F128Fabs),
    ("fabsl", IntrinsicOp::LDFabs),
    // SSE4.1 directed rounding (project baseline x86-64-v3; same policy as
    // the FMA table below). Inline expansion is REQUIRED for glibc
    // self-hosting: its s_floor.c/s_trunc.c define floor()/trunc() as the
    // corresponding builtin, so a libm call here is infinite recursion.
    // Immediates are GCC-verified: floor 9, ceil 10, trunc 11, rint 4,
    // nearbyint 12, roundeven 8. round()/roundf() are deliberately ABSENT:
    // GCC 14 keeps them as libm calls, and glibc's s_round.c uses its C
    // bit-twiddling path, never __builtin_round, on x86-64.
    ("floor", IntrinsicOp::RoundScalarF64(9)),
    ("floorf", IntrinsicOp::RoundScalarF32(9)),
    ("ceil", IntrinsicOp::RoundScalarF64(10)),
    ("ceilf", IntrinsicOp::RoundScalarF32(10)),
    ("trunc", IntrinsicOp::RoundScalarF64(11)),
    ("truncf", IntrinsicOp::RoundScalarF32(11)),
    ("rint", IntrinsicOp::RoundScalarF64(4)),
    ("rintf", IntrinsicOp::RoundScalarF32(4)),
    ("nearbyint", IntrinsicOp::RoundScalarF64(12)),
    ("nearbyintf", IntrinsicOp::RoundScalarF32(12)),
    ("roundeven", IntrinsicOp::RoundScalarF64(8)),
    ("roundevenf", IntrinsicOp::RoundScalarF32(8)),
];

/// Table of binary math functions that map directly to intrinsic instructions.
const BINARY_INTRINSICS: &[(&str, IntrinsicOp)] = &[
    ("__copysigntf3", IntrinsicOp::F128Copysign),
    ("copysignl", IntrinsicOp::LDCopysign),
    // Pure SSE bit ops; a libm call would recurse inside glibc's own
    // s_copysign.c (copysign() IS __builtin_copysign there).
    ("copysign", IntrinsicOp::CopysignF64),
    ("copysignf", IntrinsicOp::CopysignF32),
];

thread_local! {
    /// C's default `-fmath-errno` (GCC 16.2 and Clang 23.1.3 on glibc, whose
    /// `math_errhandling` includes `MATH_ERRNO`): `sqrt` of a negative operand
    /// must set `errno = EDOM`, which the bare instruction cannot do.  Cleared by
    /// `-fno-math-errno` / `-ffast-math`; set once per compilation by
    /// `run_passes` on the compiling thread (passes never leave it), so
    /// concurrent compilations -- and unit tests -- cannot observe each other.
    static MATH_ERRNO: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

pub(crate) fn set_math_errno(enabled: bool) {
    MATH_ERRNO.with(|m| m.set(enabled));
}

fn math_errno() -> bool {
    MATH_ERRNO.with(std::cell::Cell::get)
}

/// `sqrt`/`sqrtf` under `-fmath-errno` (O2+): keep the instruction for every
/// operand that cannot raise a domain error and call the library only for
/// the one that does -- GCC's and Clang's shape:
///
/// ```text
///   B:  r_fast = SqrtF64 x ; neg = r_fast != r_fast ; br neg, S, J
///   S:  r_slow = call sqrt(x)        ; assume_library = false: never refolded
///   J:  r = phi [r_fast, B], [r_slow, S] ; rest of B ; B's terminator
/// ```
///
/// The instruction returns the correctly rounded result (and FE_INVALID)
/// for every operand, and NaN exactly for C17 7.12.7.5's domain error
/// (`x < 0`; `-0` and `+inf` raise none) or a NaN operand, for which the
/// library call is a harmless no-op on errno.  A constant operand folds
/// the compare and the slow block dies.  The slow
/// block is appended after the function body, off the fall-through path,
/// and its call carries `CallInfo::cold`, so the x86-64 allocator keeps FP
/// values live across it in XMM registers (saved around the call only).
/// Operands proven never `< 0` (sum of squares, `fabs`, ...) skip the
/// guard entirely: [`unguard_provably_nonnegative_sqrt`].
pub fn guard_errno_sqrt_calls(module: &mut IrModule) {
    if !math_errno() {
        return;
    }
    for func in &mut module.functions {
        unguard_provably_nonnegative_sqrt(func);
        let mut bi = 0;
        while bi < func.blocks.len() {
            let found = func.blocks[bi].instructions.iter().position(|inst| {
                matches!(inst, Instruction::Call { func: callee, info }
                    if info.assume_library
                        && info.dest.is_some()
                        && info.args.len() == 1
                        && matches!(callee.as_str(), "sqrt" | "sqrtf"))
            });
            match found {
                Some(at) => split_errno_sqrt(func, bi, at),
                None => bi += 1,
            }
        }
    }
}

/// Fold `sqrt(x)` straight to the instruction when `x < 0` is impossible --
/// no domain error, so no guard (GCC's frange proof; the dominant real-world
/// operand, a sum of squares, then vectorizes like any arithmetic).
fn unguard_provably_nonnegative_sqrt(func: &mut IrFunction) {
    let mut defs: crate::common::fx_hash::FxHashMap<u32, Instruction> =
        crate::common::fx_hash::FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some(d) = inst.dest() {
                defs.insert(d.0, inst.clone());
            }
        }
    }
    for block in &mut func.blocks {
        for inst in &mut block.instructions {
            let Instruction::Call { func: callee, info } = inst else {
                continue;
            };
            let op = match callee.as_str() {
                "sqrt" => IntrinsicOp::SqrtF64,
                "sqrtf" => IntrinsicOp::SqrtF32,
                _ => continue,
            };
            if !info.assume_library || info.dest.is_none() || info.args.len() != 1 {
                continue;
            }
            let mut visiting = Vec::new();
            if never_below_zero(&defs, info.args[0], &mut visiting, 0) {
                *inst = Instruction::Intrinsic {
                    dest: info.dest,
                    op,
                    dest_ptr: None,
                    args: vec![info.args[0]],
                };
            }
        }
    }
}

/// Conservative proof that the floating-point value `op` is never `< 0`
/// (it may be `-0`, `+0`, positive, `+inf` or NaN).  IEEE 754 facts used:
/// `a*a` is `+0`, positive, `+inf` or NaN; a sum or product of such values
/// is again never `< 0` (`+0 * -0 = -0`, `-0 + -0 = -0`, `inf * 0 = NaN`);
/// `fabs`/`sqrt` results, unsigned-integer conversions and FP width changes
/// (rounding preserves the sign) keep the property.  Division is excluded:
/// `1 / -0 = -inf`.  Phis need every incoming value; a cycle fails closed.
fn never_below_zero(
    defs: &crate::common::fx_hash::FxHashMap<u32, Instruction>,
    op: Operand,
    visiting: &mut Vec<u32>,
    depth: u32,
) -> bool {
    const MAX_DEPTH: u32 = 16;
    let v = match op {
        Operand::Const(IrConst::F64(c)) => return c >= 0.0 || c.is_nan(),
        Operand::Const(IrConst::F32(c)) => return c >= 0.0 || c.is_nan(),
        Operand::Const(_) => return false,
        Operand::Value(v) => v,
    };
    if depth > MAX_DEPTH || visiting.contains(&v.0) {
        return false;
    }
    let Some(inst) = defs.get(&v.0) else {
        return false;
    };
    let is_fp = |ty: &IrType| matches!(ty, IrType::F32 | IrType::F64);
    visiting.push(v.0);
    let mut sub = |o: Operand| never_below_zero(defs, o, visiting, depth + 1);
    let proven = match inst {
        Instruction::Copy { src, .. } => sub(*src),
        Instruction::BinOp {
            op: IrBinOp::Mul,
            lhs,
            rhs,
            ty,
            ..
        } if is_fp(ty) => lhs == rhs || (sub(*lhs) && sub(*rhs)),
        Instruction::BinOp {
            op: IrBinOp::Add,
            lhs,
            rhs,
            ty,
            ..
        } if is_fp(ty) => sub(*lhs) && sub(*rhs),
        Instruction::Intrinsic {
            op:
                IntrinsicOp::SqrtF64
                | IntrinsicOp::SqrtF32
                | IntrinsicOp::FabsF64
                | IntrinsicOp::FabsF32,
            ..
        } => true,
        Instruction::Call { func, info }
            if info.assume_library && matches!(func.as_str(), "fabs" | "fabsf") =>
        {
            true
        }
        Instruction::Cast {
            src,
            from_ty,
            to_ty,
            ..
        } if is_fp(to_ty) => {
            if is_fp(from_ty) {
                sub(*src)
            } else {
                matches!(
                    from_ty,
                    IrType::U8 | IrType::U16 | IrType::U32 | IrType::U64
                )
            }
        }
        Instruction::Select {
            true_val,
            false_val,
            ..
        } => sub(*true_val) && sub(*false_val),
        Instruction::Phi { incoming, .. } => incoming.iter().all(|(o, _)| sub(*o)),
        _ => false,
    };
    visiting.pop();
    proven
}

fn split_errno_sqrt(func: &mut IrFunction, bi: usize, at: usize) {
    use crate::ir::reexports::{BasicBlock, BlockId};
    // The guard located a result-producing call at `at`; re-check here,
    // BEFORE any mutation, so a stale index leaves the function untouched
    // instead of aborting the compiler.
    let Some(block) = func.blocks.get(bi) else {
        return;
    };
    let call_at = matches!(
        block.instructions.get(at),
        Some(Instruction::Call { info, .. }) if info.dest.is_some() && !info.args.is_empty()
    );
    let spans_parallel =
        block.source_spans.is_empty() || block.source_spans.len() == block.instructions.len();
    if !call_at || !spans_parallel {
        return;
    }
    let live_max = func.blocks.iter().map(|b| b.label.0).max().unwrap_or(0);
    let base = func.next_label.max(live_max.saturating_add(1));
    func.next_label = base.saturating_add(2);
    let (slow_label, join_label) = (BlockId(base), BlockId(base.saturating_add(1)));
    let mut fresh = || {
        let v = Value(func.next_value_id);
        func.next_value_id += 1;
        v
    };
    let (r_fast, r_slow, neg) = (fresh(), fresh(), fresh());

    let block = &mut func.blocks[bi];
    let b_label = block.label;
    let rest = block.instructions.split_off(at + 1);
    // Both shapes were checked on entry; the `else` arms only keep this
    // total (they restore the block exactly).
    let (callee, mut info, dest) = match block.instructions.pop() {
        Some(Instruction::Call {
            func: callee,
            mut info,
        }) => match info.dest.replace(r_slow) {
            Some(dest) => (callee, info, dest),
            None => {
                info.dest = None;
                block
                    .instructions
                    .push(Instruction::Call { func: callee, info });
                block.instructions.extend(rest);
                return;
            }
        },
        other => {
            block.instructions.extend(other);
            block.instructions.extend(rest);
            return;
        }
    };
    let spans = if block.source_spans.is_empty() {
        None
    } else {
        let rest_spans = block.source_spans.split_off(at + 1);
        block
            .source_spans
            .pop()
            .map(|call_span| (call_span, rest_spans))
    };
    let (ty, op) = if callee == "sqrtf" {
        (IrType::F32, IntrinsicOp::SqrtF32)
    } else {
        (IrType::F64, IntrinsicOp::SqrtF64)
    };
    let x = info.args[0];
    info.assume_library = false;
    info.cold = true;
    block.instructions.push(Instruction::Intrinsic {
        dest: Some(r_fast),
        op,
        dest_ptr: None,
        args: vec![x],
    });
    // `r_fast` is NaN exactly when `x < 0` or `x` is NaN: testing the result
    // needs no 0.0 constant in the loop (x86: `ucomis r,r; jp`).  A NaN
    // operand then also takes the call, which returns the same NaN and
    // leaves errno alone (Clang's condition; GCC tests `x < 0`).  GCC's
    // operand test was built and measured (2026-10-20): x86-64 still copies
    // `x` out of %xmm0 (the expression scratch) and the result still needs
    // its own register, so it only adds the 0.0 materialisation
    // (`xorpd`) to the hot path; its earlier branch resolution pays off
    // only on a misprediction, which a domain error is not.
    block.instructions.push(Instruction::Cmp {
        dest: neg,
        op: IrCmpOp::Ne,
        lhs: Operand::Value(r_fast),
        rhs: Operand::Value(r_fast),
        ty,
    });
    let old_term = std::mem::replace(
        &mut block.terminator,
        Terminator::CondBranch {
            cond: Operand::Value(neg),
            true_label: slow_label,
            false_label: join_label,
        },
    );
    let mut join_insts = vec![Instruction::Phi {
        dest,
        ty,
        incoming: vec![
            (Operand::Value(r_fast), b_label),
            (Operand::Value(r_slow), slow_label),
        ],
    }];
    join_insts.extend(rest);
    // Debug spans stay parallel to the instructions: every new instruction
    // takes the call's span.
    let (slow_spans, join_spans) = match spans {
        Some((span, rest_spans)) => {
            block.source_spans.extend([span, span]);
            (
                vec![span],
                std::iter::once(span).chain(rest_spans).collect(),
            )
        }
        None => (Vec::new(), Vec::new()),
    };
    // B's old successors now come from J.
    for other in &mut func.blocks {
        for inst in &mut other.instructions {
            if let Instruction::Phi { incoming, .. } = inst {
                for (_, label) in incoming.iter_mut() {
                    if *label == b_label {
                        *label = join_label;
                    }
                }
            }
        }
    }
    func.blocks.insert(
        bi + 1,
        BasicBlock {
            label: join_label,
            instructions: join_insts,
            terminator: old_term,
            source_spans: join_spans,
        },
    );
    func.blocks.push(BasicBlock {
        label: slow_label,
        instructions: vec![Instruction::Call { func: callee, info }],
        terminator: Terminator::Branch(join_label),
        source_spans: slow_spans,
    });
}

/// Whether the compilation target has FMA3 (set once by run_passes from the
/// target + -mfma flags, same pattern as vectorize::set_x86_fma_enabled).
/// The fma/fmaf libcall fold requires it: the fused form IS the C99
/// single-rounding semantics, so folding without a fused lowering would
/// silently miscompile (mul+add double-rounds).
static HAS_FMA3: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn set_has_fma3(enabled: bool) {
    HAS_FMA3.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

/// Also read by the FMA operand-negation peel (`fma_neg_peel.rs`): the same
/// target set that materialises `FmaScalarF{32,64}` (all via this gate)
/// carries the four signed family spellings, so the peel rides the signal
/// instead of growing a parallel static.
pub(crate) fn has_fma3() -> bool {
    HAS_FMA3.load(std::sync::atomic::Ordering::Relaxed)
}

/// Whether the target can lower `RoundScalarF{32,64}` inline (x86-64:
/// SSE4.1 `roundsd`; i686: x87 `frndint`; AArch64: `frint*`).  Without it
/// the floor/ceil/trunc/rint/nearbyint calls must stay libcalls — the
/// x86-64 emitter has no SSE2-only rounding sequence, and inventing one
/// (2^52 magic-number add/sub) changes results for |x| ≥ 2^52 and for
/// signalling NaNs, which the libm implementation gets right.
static HAS_ROUND_INSN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub(crate) fn set_has_round_insn(enabled: bool) {
    HAS_ROUND_INSN.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

fn has_round_insn() -> bool {
    HAS_ROUND_INSN.load(std::sync::atomic::Ordering::Relaxed)
}

/// Ternary math functions mapping to single-instruction intrinsics.
/// fma/fmaf REQUIRE the fused form (single rounding, C99 F.10.10.1);
/// converting the libcall is not just faster, it is what makes glibc's own
/// s_fma.c/s_fmaf.c (built with math-use-builtins-fma.h) link at all.
/// The x86-64 backend emits VEX FMA3 whenever the TU's resolved ISA permission
/// grants it, matching the existing mul+add fusion policy.
const TERNARY_INTRINSICS: &[(&str, IntrinsicOp)] = &[
    ("fma", IntrinsicOp::FmaScalarF64),
    ("fmaf", IntrinsicOp::FmaScalarF32),
];

/// Create a float constant (F32 or F64) appropriate for the return type.
fn float_const(return_type: IrType, val: f64) -> IrConst {
    if return_type == IrType::F32 {
        IrConst::F32(val as f32)
    } else {
        IrConst::F64(val)
    }
}

/// Simplify calls to known math library functions.
///
/// Optimizations:
/// - pow(x, 2.0) / powf(x, 2.0f) => x * x  (avoids expensive libm call)
/// - pow(x, 0.0) => 1.0, pow(x, 1.0) => x
/// - pow(x, -1.0) => 1.0 / x
/// - sqrt(x) / sqrtf(x) => SqrtF64/SqrtF32 intrinsic (inline instruction)
/// - fabs(x) / fabsf(x) => FabsF64/FabsF32 intrinsic (inline instruction)
fn simplify_math_call(
    dest: Value,
    func: &str,
    args: &[Operand],
    return_type: IrType,
) -> Option<Instruction> {
    // Under -fmath-errno a bare instruction would lose `errno = EDOM`; the
    // O2+ pipeline builds the guarded form instead (guard_errno_sqrt_calls),
    // and the cheap tiers keep the call, as GCC and Clang do at -O0.
    if math_errno() && matches!(func, "sqrt" | "sqrtf") {
        return None;
    }
    // Check unary intrinsic table first (sqrt, sqrtf, fabs, fabsf).
    for &(name, intrinsic_op) in UNARY_INTRINSICS {
        if func == name {
            if args.len() != 1 {
                return None;
            }
            // Directed rounding needs an inline lowering on the target
            // (see set_has_round_insn); otherwise keep the libm call.
            if matches!(
                intrinsic_op,
                IntrinsicOp::RoundScalarF64(_) | IntrinsicOp::RoundScalarF32(_)
            ) && !has_round_insn()
            {
                return None;
            }
            return Some(Instruction::Intrinsic {
                dest: Some(dest),
                op: intrinsic_op,
                dest_ptr: None,
                args: args.to_vec(),
            });
        }
    }

    // Ternary intrinsic table (fma/fmaf -> single vfmadd, single rounding).
    // Gated on the TARGET having FMA3: without it the backend has no fused
    // lowering, so the libcall must survive (fmaf/fma in libm provide the
    // C99-mandated single rounding; mul+add does not).  x86-64's project
    // baseline is v3 (always FMA3); i686 folds only under -mfma.
    for &(name, intrinsic_op) in TERNARY_INTRINSICS {
        if func == name {
            if !has_fma3() {
                return None;
            }
            if args.len() != 3 {
                return None;
            }
            return Some(Instruction::Intrinsic {
                dest: Some(dest),
                op: intrinsic_op,
                dest_ptr: None,
                args: args.to_vec(),
            });
        }
    }

    // Binary intrinsic table (copysignf128/copysignl -> inline bit ops).
    for &(name, intrinsic_op) in BINARY_INTRINSICS {
        if func == name {
            if args.len() != 2 {
                return None;
            }
            return Some(Instruction::Intrinsic {
                dest: Some(dest),
                op: intrinsic_op,
                dest_ptr: None,
                args: args.to_vec(),
            });
        }
    }

    // pow/powf with constant exponent
    if func == "pow" || func == "powf" {
        if args.len() != 2 {
            return None;
        }
        let exp = match &args[1] {
            Operand::Const(IrConst::F64(v)) => *v,
            Operand::Const(IrConst::F32(v)) => *v as f64,
            Operand::Const(IrConst::LongDouble(v, _)) => *v,
            _ => return None,
        };
        let base = args[0];

        if exp == 0.0 {
            // pow(x, 0.0) => 1.0 (C11 7.12.7.4: pow(x,+-0) returns 1 for all x, even NaN)
            return Some(Instruction::Copy {
                dest,
                src: Operand::Const(float_const(return_type, 1.0)),
            });
        }
        if exp == 1.0 {
            // pow(x, 1.0) => x
            return Some(Instruction::Copy { dest, src: base });
        }
        if exp == 2.0 {
            // pow(x, 2.0) => x * x
            return Some(Instruction::BinOp {
                dest,
                op: IrBinOp::Mul,
                lhs: base,
                rhs: base,
                ty: return_type,
            });
        }
        if exp == -1.0 {
            // pow(x, -1.0) => 1.0 / x
            return Some(Instruction::BinOp {
                dest,
                op: IrBinOp::SDiv, // float division
                lhs: Operand::Const(float_const(return_type, 1.0)),
                rhs: base,
                ty: return_type,
            });
        }
        // pow(x, 0.5) is NOT sqrt(x): pow(-0, 0.5) = +0 and
        // pow(-inf, 0.5) = +inf (C17 F.10.4.4), where sqrt gives -0 and NaN.
        // GCC 16.2 and Clang 23.1 keep the call at -O2, even with
        // -fno-math-errno; only unsafe-math licenses the rewrite.
    }

    None
}

/// Fold calls to math functions that map to inline intrinsics (sqrt, fabs,
/// `__fabstf2`, `__copysigntf3`, `copysignl`, ...) into `Instruction::Intrinsic`.
///
/// The full -O2/-O3 pipeline does this inside `try_simplify`; the cheap -O0/-O1
/// tiers skip `simplify` entirely, so a call to `__copysigntf3` (the lowering
/// of `__builtin_copysignf128`) would otherwise survive to the backend as an
/// external reference and fail to link (LCCC links no libgcc). Running the
/// same folding here makes `_Float128` math builtins work at every opt level.
/// Semantics-preserving: the fold is a rename to the backend's own intrinsic.
pub fn fold_math_intrinsic_calls(module: &mut IrModule) {
    for func in &mut module.functions {
        let strict_fp_environment = super::fp_environment_blocks_reordering(func);
        for block in &mut func.blocks {
            for inst in &mut block.instructions {
                if let Instruction::Call { func: callee, info } = inst {
                    if !info.assume_library {
                        continue;
                    }
                    if let Some(d) = info.dest {
                        if let Some(new_inst) =
                            simplify_math_call(d, callee, &info.args, info.return_type)
                        {
                            // fabs/copysign are pure sign-bit operations. FMA
                            // and hardware sqrt are also valid under strict FP:
                            // their instructions obey MXCSR rounding and raise
                            // arithmetic FP exceptions while preserving the
                            // builtin operation's rounding. GCC emits these
                            // instructions with both -frounding-math and
                            // -ftrapping-math. Keeping libcalls here makes
                            // glibc's builtin-backed implementations recurse
                            // through their own public aliases. Do not extend
                            // this exemption to directed-rounding functions or
                            // pow, whose replacement constraints differ.
                            let environment_safe = matches!(
                                new_inst,
                                Instruction::Intrinsic {
                                    op: IntrinsicOp::FabsF32
                                        | IntrinsicOp::FabsF64
                                        | IntrinsicOp::F128Fabs
                                        | IntrinsicOp::LDFabs
                                        | IntrinsicOp::CopysignF32
                                        | IntrinsicOp::CopysignF64
                                        | IntrinsicOp::F128Copysign
                                        | IntrinsicOp::LDCopysign
                                        | IntrinsicOp::FmaScalarF32
                                        | IntrinsicOp::FmaScalarF64
                                        | IntrinsicOp::SqrtF32
                                        | IntrinsicOp::SqrtF64,
                                    ..
                                }
                            );
                            if !strict_fp_environment || environment_safe {
                                *inst = new_inst;
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::reexports::CallInfo;

    // === Test helpers ===

    /// Shorthand: try_simplify with empty def maps (no chain optimization context).
    fn simplify_default(inst: &Instruction) -> Option<Instruction> {
        try_simplify(inst, &[], &[], &[], &[], &[], &[])
    }

    /// Create a BinOp instruction with standard test values.
    fn binop(op: IrBinOp, lhs: Operand, rhs: Operand, ty: IrType) -> Instruction {
        Instruction::BinOp {
            dest: Value(0),
            op,
            lhs,
            rhs,
            ty,
        }
    }

    /// Create a BinOp with Value(1) as lhs and the given const rhs.
    fn binop_val_const(op: IrBinOp, rhs: IrConst, ty: IrType) -> Instruction {
        binop(op, Operand::Value(Value(1)), Operand::Const(rhs), ty)
    }

    /// Create a BinOp with both operands being Value(1).
    fn binop_self(op: IrBinOp, ty: IrType) -> Instruction {
        Instruction::BinOp {
            dest: Value(2),
            op,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Value(Value(1)),
            ty,
        }
    }

    /// Assert result is a Copy of the given value id.
    fn assert_copy_value(result: &Instruction, expected_id: u32) {
        match result {
            Instruction::Copy {
                src: Operand::Value(v),
                ..
            } => assert_eq!(v.0, expected_id),
            _ => panic!("Expected Copy of Value({}), got {:?}", expected_id, result),
        }
    }

    /// Assert result is a Copy of the given constant.
    fn assert_copy_const_i32(result: &Instruction, expected: i32) {
        match result {
            Instruction::Copy {
                src: Operand::Const(IrConst::I32(v)),
                ..
            } => assert_eq!(*v, expected),
            _ => panic!("Expected Copy with I32({}), got {:?}", expected, result),
        }
    }

    /// Assert result is a Copy of I8 constant (used for boolean results).
    fn assert_copy_bool(result: &Instruction, expected: i8) {
        match result {
            Instruction::Copy {
                src: Operand::Const(IrConst::I8(v)),
                ..
            } => assert_eq!(*v, expected),
            _ => panic!("Expected Copy with I8({}), got {:?}", expected, result),
        }
    }

    fn make_call(func_name: &str, args: Vec<Operand>, return_type: IrType) -> Instruction {
        Instruction::Call {
            func: func_name.to_string(),
            info: CallInfo {
                dest: Some(Value(10)),
                args,
                arg_types: vec![],
                return_type,
                is_variadic: false,
                num_fixed_args: 0,
                struct_arg_sizes: vec![],
                struct_arg_aligns: vec![],
                struct_arg_classes: Vec::new(),
                struct_arg_riscv_float_classes: Vec::new(),
                struct_arg_is_f128_sse: Vec::new(),
                is_sret: false,
                calling_convention: crate::common::types::CallingConvention::Default,
                is_fastcall: false,
                regparm: None,
                is_pure: false,
                is_const: false,
                // A call to the C library function (what lowering produces for a
                // hosted TU without -fno-builtin).
                assume_library: true,
                ret_eightbyte_classes: Vec::new(),
                ret_is_f128_sse: false,
                cold: false,
            },
        }
    }

    // === clz/ctz guarded-ternary zero-guard elimination ===

    #[test]
    fn test_select_clz_guard_elimination() {
        // `x ? __builtin_clz(x) : 32` shape after the ternary lowers:
        //   v2 = cast v1 (u32 -> i32, same size)
        //   v3 = Clz(v2)                    // IR defines Clz(0) == 32
        //   v4 = sext v3 (i32 -> i64)
        //   v5 = select cond=v1 ? v4 : 32   // redundant zero guard
        let mut cast_defs: Vec<Option<CastDef>> = vec![None; 6];
        cast_defs[2] = Some(CastDef {
            src: Operand::Value(Value(1)),
            from_ty: IrType::U32,
            to_ty: IrType::I32,
        });
        cast_defs[4] = Some(CastDef {
            src: Operand::Value(Value(3)),
            from_ty: IrType::I32,
            to_ty: IrType::I64,
        });
        let mut unary_defs: Vec<Option<UnaryDef>> = vec![None; 6];
        unary_defs[3] = Some(UnaryDef {
            op: IrUnaryOp::Clz,
            src: Operand::Value(Value(2)),
            ty: IrType::I32,
        });
        let r = clz_ctz_zero_guard_replacement(
            &Operand::Value(Value(1)),
            &Operand::Value(Value(4)),
            &Operand::Const(IrConst::I64(32)),
            &cast_defs,
            &unary_defs,
        );
        assert!(
            matches!(r, Some(Operand::Value(v)) if v.0 == 4),
            "select(x ? clz(x) : 32) should collapse onto the clz arm"
        );
        // The condition may itself be a same-size cast of x (same zero-ness).
        let r2 = clz_ctz_zero_guard_replacement(
            &Operand::Value(Value(2)),
            &Operand::Value(Value(4)),
            &Operand::Const(IrConst::I64(32)),
            &cast_defs,
            &unary_defs,
        );
        assert!(matches!(r2, Some(Operand::Value(v)) if v.0 == 4));
        // Ctz variant with its width constant folds the same way.
        unary_defs[3] = Some(UnaryDef {
            op: IrUnaryOp::Ctz,
            src: Operand::Value(Value(2)),
            ty: IrType::I32,
        });
        let r3 = clz_ctz_zero_guard_replacement(
            &Operand::Value(Value(1)),
            &Operand::Value(Value(4)),
            &Operand::Const(IrConst::I64(32)),
            &cast_defs,
            &unary_defs,
        );
        assert!(matches!(r3, Some(Operand::Value(v)) if v.0 == 4));
    }

    /// `v2 = Clz(v1); v3 = K op v2` over a U32 `v1`; zero-value replacement
    /// of `select(cond, v3, c)`.
    fn zero_value_case(
        unary: IrUnaryOp,
        op: IrBinOp,
        k: i32,
        cond: u32,
        c: IrConst,
        cast_defs: &[Option<CastDef>],
    ) -> Option<Operand> {
        let mut unary_defs: Vec<Option<UnaryDef>> = vec![None; 8];
        unary_defs[2] = Some(UnaryDef {
            op: unary,
            src: Operand::Value(Value(1)),
            ty: IrType::U32,
        });
        let mut binop_defs: Vec<Option<BinOpDef>> = vec![None; 8];
        binop_defs[3] = Some(BinOpDef {
            op,
            lhs: Operand::Const(IrConst::I32(k)),
            rhs: Operand::Value(Value(2)),
            ty: IrType::I32,
        });
        zero_value_replacement(
            &Operand::Value(Value(cond)),
            &Operand::Value(Value(3)),
            &Operand::Const(c),
            IrType::I32,
            cast_defs,
            &unary_defs,
            &binop_defs,
        )
    }

    /// `(v3 = v2 & mask) OP 0` with `v2 = cast(v1: from -> to)`.
    fn sign_mask_case(op: IrCmpOp, from: IrType, to: IrType, mask: i64) -> Option<Instruction> {
        let mut cast_defs: Vec<Option<CastDef>> = vec![None; 8];
        cast_defs[2] = Some(CastDef {
            src: Operand::Value(Value(1)),
            from_ty: from,
            to_ty: to,
        });
        let mut binop_defs: Vec<Option<BinOpDef>> = vec![None; 8];
        binop_defs[3] = Some(BinOpDef {
            op: IrBinOp::And,
            lhs: Operand::Value(Value(2)),
            rhs: Operand::Const(IrConst::I64(mask)),
            ty: to,
        });
        fold_sign_region_mask_test(
            Value(4),
            op,
            &Operand::Value(Value(3)),
            &Operand::Const(IrConst::I64(0)),
            to,
            &binop_defs,
            &cast_defs,
        )
    }

    #[test]
    fn sign_region_mask_tests_become_sign_tests() {
        // ((long)*p & 0x100) != 0 on a signed char: bit 8 copies the sign.
        for (mask, op, want) in [
            (0x100, IrCmpOp::Ne, IrCmpOp::Slt),
            (0x80, IrCmpOp::Ne, IrCmpOp::Slt),
            (-128, IrCmpOp::Eq, IrCmpOp::Sge),
            (i64::MIN, IrCmpOp::Ne, IrCmpOp::Slt),
        ] {
            let got = sign_mask_case(op, IrType::I8, IrType::I64, mask);
            assert!(
                matches!(
                    got,
                    Some(Instruction::Cmp {
                        op: o,
                        lhs: Operand::Value(Value(1)),
                        rhs: Operand::Const(IrConst::I8(0)),
                        ty: IrType::I8,
                        ..
                    }) if o == want
                ),
                "mask {mask:#x}: {got:?}"
            );
        }
        // Same width, signed: x & INT_MIN.
        let mut binop_defs: Vec<Option<BinOpDef>> = vec![None; 8];
        binop_defs[3] = Some(BinOpDef {
            op: IrBinOp::And,
            lhs: Operand::Const(IrConst::I32(i32::MIN)),
            rhs: Operand::Value(Value(1)),
            ty: IrType::I32,
        });
        let got = fold_sign_region_mask_test(
            Value(4),
            IrCmpOp::Ne,
            &Operand::Const(IrConst::I32(0)),
            &Operand::Value(Value(3)),
            IrType::I32,
            &binop_defs,
            &[],
        );
        assert!(
            matches!(
                got,
                Some(Instruction::Cmp {
                    op: IrCmpOp::Slt,
                    ty: IrType::I32,
                    ..
                })
            ),
            "{got:?}"
        );
    }

    #[test]
    fn sign_region_mask_test_refusals() {
        // A mask bit below the sign region reads a value bit.
        assert!(sign_mask_case(IrCmpOp::Ne, IrType::I8, IrType::I64, 0x140).is_none());
        assert!(sign_mask_case(IrCmpOp::Ne, IrType::I8, IrType::I64, 0x40).is_none());
        // Zero-extended sources: the upper bits are zero, and the narrow sign
        // bit would need a signed compare of an unsigned-typed value.
        assert!(sign_mask_case(IrCmpOp::Ne, IrType::U8, IrType::I32, 0x80).is_none());
        assert!(sign_mask_case(IrCmpOp::Ne, IrType::U8, IrType::I32, 0x100).is_none());
        // Ordered compares and an empty mask are not mask tests.
        assert!(sign_mask_case(IrCmpOp::Slt, IrType::I8, IrType::I64, 0x100).is_none());
        assert!(sign_mask_case(IrCmpOp::Ne, IrType::I8, IrType::I64, 0).is_none());
        // A mask that lives entirely above the compare width is empty there.
        assert!(sign_mask_case(IrCmpOp::Ne, IrType::I8, IrType::I32, 1 << 40).is_none());
        // Same width, unsigned: no reinterpretation.
        let mut binop_defs: Vec<Option<BinOpDef>> = vec![None; 8];
        binop_defs[3] = Some(BinOpDef {
            op: IrBinOp::And,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I32(i32::MIN)),
            ty: IrType::U32,
        });
        assert!(
            fold_sign_region_mask_test(
                Value(4),
                IrCmpOp::Ne,
                &Operand::Value(Value(3)),
                &Operand::Const(IrConst::I32(0)),
                IrType::U32,
                &binop_defs,
                &[],
            )
            .is_none()
        );
    }

    #[test]
    fn test_zero_value_replacement_collapses_highbit_and_fls() {
        let mut cast_defs: Vec<Option<CastDef>> = vec![None; 8];
        // v5 = same-size cast of v1 (zero-ness preserved), v6 = truncation
        // of a wider v7 (zero-ness NOT preserved).
        cast_defs[5] = Some(CastDef {
            src: Operand::Value(Value(1)),
            from_ty: IrType::U32,
            to_ty: IrType::I32,
        });
        cast_defs[6] = Some(CastDef {
            src: Operand::Value(Value(7)),
            from_ty: IrType::I64,
            to_ty: IrType::U32,
        });
        let hit = Some(Operand::Value(Value(3)));
        use IrBinOp::{Add, Sub, Xor};
        use IrUnaryOp::{Clz, ClzNonZero, Ctz};
        // x ? 31 - clz(x) : -1 — the -1 may be typed wider than the select.
        assert_eq!(
            zero_value_case(Clz, Sub, 31, 1, IrConst::I64(-1), &cast_defs),
            hit
        );
        assert_eq!(
            zero_value_case(Clz, Sub, 31, 1, IrConst::I32(-1), &cast_defs),
            hit
        );
        assert_eq!(
            zero_value_case(Clz, Sub, 31, 5, IrConst::I32(-1), &cast_defs),
            hit
        );
        // x ? 32 - clz(x) : 0 (fls), x ? 1 + ctz(x) : 33.
        assert_eq!(
            zero_value_case(Clz, Sub, 32, 1, IrConst::I32(0), &cast_defs),
            hit
        );
        // 31 ^ clz(0) is 31 ^ 32 = 63: collapses against 63, never -1.
        assert_eq!(
            zero_value_case(Clz, Xor, 31, 1, IrConst::I32(63), &cast_defs),
            hit
        );
        assert_eq!(
            zero_value_case(Clz, Xor, 31, 1, IrConst::I64(-1), &cast_defs),
            None
        );
        assert_eq!(
            zero_value_case(Ctz, Add, 1, 1, IrConst::I32(33), &cast_defs),
            hit
        );
        // Wrong zero value, NonZero intrinsic, truncated condition, and a
        // condition unrelated to the intrinsic's operand all refuse.
        assert_eq!(
            zero_value_case(Clz, Sub, 31, 1, IrConst::I32(0), &cast_defs),
            None
        );
        // 31 - ctz(0) == -1 as well, but never 0.
        assert_eq!(
            zero_value_case(Ctz, Sub, 31, 1, IrConst::I32(-1), &cast_defs),
            hit
        );
        assert_eq!(
            zero_value_case(Ctz, Sub, 31, 1, IrConst::I32(0), &cast_defs),
            None
        );
        assert_eq!(
            zero_value_case(ClzNonZero, Sub, 31, 1, IrConst::I32(-1), &cast_defs),
            None
        );
        assert_eq!(
            zero_value_case(Clz, Sub, 31, 6, IrConst::I32(-1), &cast_defs),
            None
        );
        assert_eq!(
            zero_value_case(Clz, Sub, 31, 4, IrConst::I32(-1), &cast_defs),
            None
        );
    }

    #[test]
    fn test_eval_at_zero_is_width_exact() {
        // v1: U32 x; v2 = sext(i32 -1 const via binop) ... build directly:
        // v3 = LShr(I32 v2, 1) where v2 = Sub(I32 0, 1) = 0xFFFF_FFFF.
        let mut binop_defs: Vec<Option<BinOpDef>> = vec![None; 8];
        binop_defs[2] = Some(BinOpDef {
            op: IrBinOp::Sub,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I32(1)),
            ty: IrType::I32,
        });
        binop_defs[3] = Some(BinOpDef {
            op: IrBinOp::LShr,
            lhs: Operand::Value(Value(2)),
            rhs: Operand::Const(IrConst::I32(1)),
            ty: IrType::I32,
        });
        binop_defs[4] = Some(BinOpDef {
            op: IrBinOp::AShr,
            lhs: Operand::Value(Value(2)),
            rhs: Operand::Const(IrConst::I32(4)),
            ty: IrType::I32,
        });
        binop_defs[5] = Some(BinOpDef {
            op: IrBinOp::Shl,
            lhs: Operand::Value(Value(2)),
            rhs: Operand::Const(IrConst::I32(32)),
            ty: IrType::I32,
        });
        let mut cast_defs: Vec<Option<CastDef>> = vec![None; 8];
        cast_defs[6] = Some(CastDef {
            src: Operand::Value(Value(2)),
            from_ty: IrType::I32,
            to_ty: IrType::I64,
        });
        cast_defs[7] = Some(CastDef {
            src: Operand::Value(Value(2)),
            from_ty: IrType::U32,
            to_ty: IrType::I64,
        });
        let unary_defs: Vec<Option<UnaryDef>> = vec![None; 8];
        let x = Operand::Value(Value(1));
        let ev = |v: u32, ty: IrType| {
            let mut budget = 16;
            eval_at_zero(
                Operand::Value(Value(v)),
                ty,
                x,
                &cast_defs,
                &unary_defs,
                &binop_defs,
                &mut budget,
            )
        };
        assert_eq!(ev(2, IrType::I32), Some(0xFFFF_FFFF));
        // A 64-bit container would give 0x7FFF_FFFF_FFFF_FFFF here.
        assert_eq!(ev(3, IrType::I32), Some(0x7FFF_FFFF));
        assert_eq!(ev(4, IrType::I32), Some(0xFFFF_FFFF));
        // Shift by the full width is not modelled.
        assert_eq!(ev(5, IrType::I32), None);
        // Signed source sign-extends, unsigned zero-extends.
        assert_eq!(ev(6, IrType::I64), Some(u64::MAX as u128));
        assert_eq!(ev(7, IrType::I64), Some(0xFFFF_FFFF));
        // Unknown values refuse.
        let mut budget = 16;
        assert_eq!(
            eval_at_zero(
                Operand::Value(Value(9)),
                IrType::I32,
                x,
                &cast_defs,
                &unary_defs,
                &binop_defs,
                &mut budget
            ),
            None
        );
    }

    #[test]
    fn test_select_clz_guard_rejects_bad_width_and_unrelated_cond() {
        let mut cast_defs: Vec<Option<CastDef>> = vec![None; 7];
        cast_defs[2] = Some(CastDef {
            src: Operand::Value(Value(1)),
            from_ty: IrType::U32,
            to_ty: IrType::I32,
        });
        cast_defs[4] = Some(CastDef {
            src: Operand::Value(Value(3)),
            from_ty: IrType::I32,
            to_ty: IrType::I64,
        });
        // A truncating cast on the CONDITION side changes zero-ness (x ==
        // 0x100 has low byte 0), so it must never fold.
        cast_defs[5] = Some(CastDef {
            src: Operand::Value(Value(1)),
            from_ty: IrType::I64,
            to_ty: IrType::U32,
        });
        let mut unary_defs: Vec<Option<UnaryDef>> = vec![None; 7];
        unary_defs[3] = Some(UnaryDef {
            op: IrUnaryOp::Clz,
            src: Operand::Value(Value(2)),
            ty: IrType::I32,
        });
        let near = |cond: Operand| {
            clz_ctz_zero_guard_replacement(
                &cond,
                &Operand::Value(Value(4)),
                &Operand::Const(IrConst::I64(32)),
                &cast_defs,
                &unary_defs,
            )
        };
        // Width mismatch: zero-case constant 31 != clz width 32.
        assert_eq!(
            clz_ctz_zero_guard_replacement(
                &Operand::Value(Value(1)),
                &Operand::Value(Value(4)),
                &Operand::Const(IrConst::I64(31)),
                &cast_defs,
                &unary_defs,
            ),
            None
        );
        // Unrelated condition value.
        assert_eq!(near(Operand::Value(Value(6))), None);
        // Truncated-condition value id 5 peels to nothing (same-size only).
        assert_eq!(near(Operand::Value(Value(5))), None);
    }

    #[test]
    fn test_select_clz_guard_rejects_nonzero_variants() {
        // The algebraic identity `select(x, clz(x), W) == clz(x)` rests on
        // the IR's defined-zero extension. The NonZero spellings have NO
        // value at zero, so a guard-select over a NonZero arm must survive:
        // it is the only consumer discipline keeping a speculated BSR/BSF
        // result out of the return value for x == 0. When the nonzero fact
        // dominates the select, CVP's fact-stack select folding performs
        // the same rewrite soundly instead.
        let mut cast_defs: Vec<Option<CastDef>> = vec![None; 7];
        cast_defs[4] = Some(CastDef {
            src: Operand::Value(Value(3)),
            from_ty: IrType::I32,
            to_ty: IrType::I64,
        });
        let mut unary_defs: Vec<Option<UnaryDef>> = vec![None; 7];
        for op in [IrUnaryOp::ClzNonZero, IrUnaryOp::CtzNonZero] {
            unary_defs[3] = Some(UnaryDef {
                op,
                src: Operand::Value(Value(1)),
                ty: IrType::I32,
            });
            let arm = Operand::Value(Value(4));
            assert_eq!(
                clz_ctz_zero_guard_replacement(
                    &Operand::Value(Value(1)),
                    &arm,
                    &Operand::Const(IrConst::I64(32)),
                    &cast_defs,
                    &unary_defs,
                ),
                None,
                "{op:?} guard-select must not collapse without a dominating proof"
            );
        }
    }

    // === BinOp identity tests ===

    #[test]
    fn test_add_zero() {
        let result =
            simplify_default(&binop_val_const(IrBinOp::Add, IrConst::I32(0), IrType::I32)).unwrap();
        assert_copy_value(&result, 1);
    }

    #[test]
    fn test_mul_zero() {
        let result =
            simplify_default(&binop_val_const(IrBinOp::Mul, IrConst::I32(0), IrType::I32)).unwrap();
        assert_copy_const_i32(&result, 0);
    }

    #[test]
    fn test_mul_one() {
        let result =
            simplify_default(&binop_val_const(IrBinOp::Mul, IrConst::I32(1), IrType::I32)).unwrap();
        assert_copy_value(&result, 1);
    }

    #[test]
    fn test_sub_self() {
        let result = simplify_default(&binop_self(IrBinOp::Sub, IrType::I32)).unwrap();
        assert_copy_const_i32(&result, 0);
    }

    #[test]
    fn test_xor_self() {
        let result = simplify_default(&binop_self(IrBinOp::Xor, IrType::I32)).unwrap();
        assert_copy_const_i32(&result, 0);
    }

    #[test]
    fn test_and_self() {
        let result = simplify_default(&binop_self(IrBinOp::And, IrType::I32)).unwrap();
        assert_copy_value(&result, 1);
    }

    #[test]
    fn test_no_simplify() {
        // x + y (non-trivial) should not simplify
        let inst = binop(
            IrBinOp::Add,
            Operand::Value(Value(0)),
            Operand::Value(Value(1)),
            IrType::I32,
        );
        assert!(simplify_default(&inst).is_none());
    }

    // === Strength reduction tests ===

    #[test]
    fn test_mul_power_of_two_to_shift() {
        // x * 4 => x << 2
        let result =
            simplify_default(&binop_val_const(IrBinOp::Mul, IrConst::I64(4), IrType::I64)).unwrap();
        match result {
            Instruction::BinOp {
                op: IrBinOp::Shl,
                rhs: Operand::Const(IrConst::I64(2)),
                ..
            } => {}
            _ => panic!("Expected Shl by 2, got {:?}", result),
        }
    }

    #[test]
    fn test_mul_two_to_add() {
        // x * 2 => x + x
        let result =
            simplify_default(&binop_val_const(IrBinOp::Mul, IrConst::I64(2), IrType::I64)).unwrap();
        match result {
            Instruction::BinOp {
                op: IrBinOp::Add,
                lhs: Operand::Value(a),
                rhs: Operand::Value(b),
                ..
            } => {
                assert_eq!(a.0, 1);
                assert_eq!(b.0, 1);
            }
            _ => panic!("Expected Add x,x, got {:?}", result),
        }
    }

    #[test]
    fn test_mul_power_of_two_i32() {
        // x * 8 => x << 3 (I32)
        let result =
            simplify_default(&binop_val_const(IrBinOp::Mul, IrConst::I32(8), IrType::I32)).unwrap();
        match result {
            Instruction::BinOp {
                op: IrBinOp::Shl,
                rhs: Operand::Const(IrConst::I32(3)),
                ..
            } => {}
            _ => panic!("Expected Shl by 3, got {:?}", result),
        }
    }

    #[test]
    fn test_mul_non_power_of_two_no_change() {
        // x * 3 should NOT be simplified to shift
        assert!(
            simplify_default(&binop_val_const(IrBinOp::Mul, IrConst::I64(3), IrType::I64))
                .is_none()
        );
    }

    #[test]
    fn test_mul_float_no_strength_reduction() {
        // x * 2.0 should NOT be simplified (float type)
        assert!(
            simplify_default(&binop_val_const(IrBinOp::Mul, IrConst::I64(2), IrType::F64))
                .is_none()
        );
    }

    #[test]
    fn test_udiv_power_of_two() {
        // x /u 8 => x >> 3 (unsigned)
        let inst = binop_val_const(IrBinOp::UDiv, IrConst::I64(8), IrType::U64);
        let result = simplify_default(&inst).unwrap();
        match result {
            Instruction::BinOp {
                op: IrBinOp::LShr,
                rhs: Operand::Const(IrConst::I64(3)),
                ..
            } => {}
            _ => panic!("Expected LShr by 3, got {:?}", result),
        }
    }

    #[test]
    fn test_urem_power_of_two() {
        // x %u 8 => x & 7 (unsigned)
        let inst = binop_val_const(IrBinOp::URem, IrConst::I64(8), IrType::U64);
        let result = simplify_default(&inst).unwrap();
        match result {
            Instruction::BinOp {
                op: IrBinOp::And,
                rhs: Operand::Const(IrConst::I64(7)),
                ..
            } => {}
            _ => panic!("Expected And with 7, got {:?}", result),
        }
    }

    // === Cast tests ===

    #[test]
    fn test_cast_same_type() {
        let inst = Instruction::Cast {
            dest: Value(0),
            src: Operand::Value(Value(1)),
            from_ty: IrType::I32,
            to_ty: IrType::I32,
        };
        let result = simplify_default(&inst).unwrap();
        assert_copy_value(&result, 1);
    }

    #[test]
    fn test_cast_chain_widen_narrow_unsigned() {
        // Cast(Cast(x, U32->I64), I64->U32) => Copy of x (zext-canonical)
        let mut defs: Vec<Option<CastDef>> = vec![None; 3];
        defs[1] = Some(CastDef {
            src: Operand::Value(Value(0)),
            from_ty: IrType::U32,
            to_ty: IrType::I64,
        });
        let inst = Instruction::Cast {
            dest: Value(2),
            src: Operand::Value(Value(1)),
            from_ty: IrType::I64,
            to_ty: IrType::U32,
        };
        let result = try_simplify(&inst, &defs, &[], &[], &[], &[], &[]).unwrap();
        assert_copy_value(&result, 0);
    }

    #[test]
    fn test_cast_chain_widen_narrow_signed_not_copy() {
        // Cast(Cast(x, I32->I64), I64->I32) must NOT collapse to Copy:
        // signed 32-bit homes are often zext in the 64-bit register.
        let mut defs: Vec<Option<CastDef>> = vec![None; 3];
        defs[1] = Some(CastDef {
            src: Operand::Value(Value(0)),
            from_ty: IrType::I32,
            to_ty: IrType::I64,
        });
        let inst = Instruction::Cast {
            dest: Value(2),
            src: Operand::Value(Value(1)),
            from_ty: IrType::I64,
            to_ty: IrType::I32,
        };
        assert!(try_simplify(&inst, &defs, &[], &[], &[], &[], &[]).is_none());
    }

    #[test]
    fn test_cast_const_fold() {
        // Cast const I32(42) to I64 => const I64(42)
        let inst = Instruction::Cast {
            dest: Value(0),
            src: Operand::Const(IrConst::I32(42)),
            from_ty: IrType::I32,
            to_ty: IrType::I64,
        };
        let result = simplify_default(&inst).unwrap();
        match result {
            Instruction::Copy {
                src: Operand::Const(IrConst::I64(42)),
                ..
            } => {}
            _ => panic!("Expected Copy with I64(42)"),
        }
    }

    #[test]
    fn test_cast_different_type_no_change() {
        let inst = Instruction::Cast {
            dest: Value(0),
            src: Operand::Value(Value(1)),
            from_ty: IrType::I32,
            to_ty: IrType::I64,
        };
        assert!(simplify_default(&inst).is_none());
    }

    // === GEP tests ===

    #[test]
    fn test_gep_zero_offset() {
        let inst = Instruction::GetElementPtr {
            dest: Value(0),
            base: Value(1),
            offset: Operand::Const(IrConst::I64(0)),
            ty: IrType::Ptr,
        };
        let result = simplify_default(&inst).unwrap();
        assert_copy_value(&result, 1);
    }

    #[test]
    fn test_gep_nonzero_no_change() {
        let inst = Instruction::GetElementPtr {
            dest: Value(0),
            base: Value(1),
            offset: Operand::Const(IrConst::I64(4)),
            ty: IrType::Ptr,
        };
        assert!(simplify_default(&inst).is_none());
    }

    #[test]
    fn test_load_through_gep_zero() {
        let mut gep_defs: Vec<Option<GepDef>> = vec![None; 3];
        gep_defs[1] = Some(GepDef {
            base: Value(0),
            offset: Operand::Const(IrConst::I64(0)),
        });
        let inst = Instruction::Load {
            dest: Value(2),
            ptr: Value(1),
            ty: IrType::I32,
            seg_override: crate::common::types::AddressSpace::Default,
            volatile: false,
        };
        let result = try_simplify(&inst, &[], &gep_defs, &[], &[], &[], &[]).unwrap();
        match result {
            Instruction::Load { ptr, dest, ty, .. } => {
                assert_eq!(ptr.0, 0);
                assert_eq!(dest.0, 2);
                assert_eq!(ty, IrType::I32);
            }
            other => panic!("expected Load of GEP base, got {other:?}"),
        }
    }

    #[test]
    fn test_store_through_gep_zero() {
        let mut gep_defs: Vec<Option<GepDef>> = vec![None; 3];
        gep_defs[1] = Some(GepDef {
            base: Value(0),
            offset: Operand::Const(IrConst::I64(0)),
        });
        let inst = Instruction::Store {
            val: Operand::Value(Value(3)),
            ptr: Value(1),
            ty: IrType::I32,
            seg_override: crate::common::types::AddressSpace::Default,
            volatile: false,
        };
        let result = try_simplify(&inst, &[], &gep_defs, &[], &[], &[], &[]).unwrap();
        match result {
            Instruction::Store { ptr, val, .. } => {
                assert_eq!(ptr.0, 0);
                assert!(matches!(val, Operand::Value(Value(3))));
            }
            other => panic!("expected Store of GEP base, got {other:?}"),
        }
    }

    #[test]
    fn test_gep_chain_fold() {
        // GEP(GEP(base, 8), 4) => GEP(base, 12)
        let mut gep_defs: Vec<Option<GepDef>> = vec![None; 3];
        gep_defs[1] = Some(GepDef {
            base: Value(0),
            offset: Operand::Const(IrConst::I64(8)),
        });
        let inst = Instruction::GetElementPtr {
            dest: Value(2),
            base: Value(1),
            offset: Operand::Const(IrConst::I64(4)),
            ty: IrType::Ptr,
        };
        let result = try_simplify(&inst, &[], &gep_defs, &[], &[], &[], &[]).unwrap();
        match result {
            Instruction::GetElementPtr {
                base,
                offset: Operand::Const(IrConst::I64(12)),
                ..
            } => {
                assert_eq!(base.0, 0, "Should use original base");
            }
            _ => panic!("Expected GEP with combined offset 12, got {:?}", result),
        }
    }

    #[test]
    fn test_gep_chain_fold_to_zero() {
        // GEP(GEP(base, 4), -4) => Copy of base (offsets cancel)
        let mut gep_defs: Vec<Option<GepDef>> = vec![None; 3];
        gep_defs[1] = Some(GepDef {
            base: Value(0),
            offset: Operand::Const(IrConst::I64(4)),
        });
        let inst = Instruction::GetElementPtr {
            dest: Value(2),
            base: Value(1),
            offset: Operand::Const(IrConst::I64(-4)),
            ty: IrType::Ptr,
        };
        let result = try_simplify(&inst, &[], &gep_defs, &[], &[], &[], &[]).unwrap();
        assert_copy_value(&result, 0);
    }

    // === IEEE 754 float safety tests ===

    #[test]
    fn test_float_add_zero_not_simplified() {
        // -0.0 + 0.0 must NOT be simplified to -0.0 (result should be +0.0)
        let inst = binop(
            IrBinOp::Add,
            Operand::Const(IrConst::F64(-0.0)),
            Operand::Const(IrConst::F64(0.0)),
            IrType::F64,
        );
        assert!(simplify_default(&inst).is_none());
    }

    #[test]
    fn test_float_add_zero_lhs_not_simplified() {
        // 0.0 + (-0.0) must NOT simplify to -0.0
        let inst = binop(
            IrBinOp::Add,
            Operand::Const(IrConst::F64(0.0)),
            Operand::Const(IrConst::F64(-0.0)),
            IrType::F64,
        );
        assert!(simplify_default(&inst).is_none());
    }

    #[test]
    fn test_float_mul_zero_not_simplified() {
        // x * 0.0 must NOT simplify for floats (NaN*0=NaN, -5.0*0=-0.0)
        let inst = binop_val_const(IrBinOp::Mul, IrConst::F64(0.0), IrType::F64);
        assert!(simplify_default(&inst).is_none());
    }

    #[test]
    fn test_float_sub_self_not_simplified() {
        // x - x must NOT simplify for floats (Inf - Inf = NaN)
        assert!(simplify_default(&binop_self(IrBinOp::Sub, IrType::F64)).is_none());
    }

    #[test]
    fn test_float_div_self_not_simplified() {
        // x / x must NOT simplify for floats (0/0=NaN, Inf/Inf=NaN)
        assert!(simplify_default(&binop_self(IrBinOp::SDiv, IrType::F64)).is_none());
    }

    #[test]
    fn test_float_sub_positive_zero_ok() {
        // x - (+0.0) is safe (subtracting +0 preserves value)
        let inst = binop_val_const(IrBinOp::Sub, IrConst::F64(0.0), IrType::F64);
        let result = simplify_default(&inst).unwrap();
        assert_copy_value(&result, 1);
    }

    #[test]
    fn test_float_sub_negative_zero_not_simplified() {
        // x - (-0.0) is NOT safe (equivalent to x + 0.0, changes -0.0 to +0.0)
        let inst = binop_val_const(IrBinOp::Sub, IrConst::F64(-0.0), IrType::F64);
        assert!(simplify_default(&inst).is_none());
    }

    #[test]
    fn test_float_mul_one_not_simplified() {
        // x * F64(1.0) is not simplified because is_one() only matches integer 1.
        let inst = binop_val_const(IrBinOp::Mul, IrConst::F64(1.0), IrType::F64);
        assert!(simplify_default(&inst).is_none());
    }

    #[test]
    fn test_float_div_one_keeps_an_fp_operation() {
        // A copy would lose signaling-NaN quieting/invalid. Multiplication by
        // exact normal 1 retains that operation; this is not an x/1 -> x fold.
        let inst = binop_val_const(IrBinOp::SDiv, IrConst::F64(1.0), IrType::F64);
        assert!(matches!(simplify_default(&inst), Some(Instruction::BinOp {
            op: IrBinOp::Mul, rhs: Operand::Const(IrConst::F64(v)), ..
        }) if v == 1.0));
    }

    #[test]
    fn exact_reciprocals_cover_both_signs_and_every_normal_exponent() {
        for sign in [0u64, 1 << 63] {
            for exponent in 1u64..=2045 {
                let divisor = f64::from_bits(sign | (exponent << 52));
                let result = normal_power_of_two_reciprocal(
                    &Operand::Const(IrConst::F64(divisor)),
                    IrType::F64,
                );
                let Some(IrConst::F64(inverse)) = result else {
                    panic!("normal reciprocal missing")
                };
                assert!(inverse.is_normal());
                assert_eq!(divisor * inverse, 1.0);
                assert_eq!(inverse.is_sign_negative(), divisor.is_sign_negative());
            }
        }
        for sign in [0u32, 1 << 31] {
            for exponent in 1u32..=253 {
                let divisor = f32::from_bits(sign | (exponent << 23));
                let result = normal_power_of_two_reciprocal(
                    &Operand::Const(IrConst::F32(divisor)),
                    IrType::F32,
                );
                let Some(IrConst::F32(inverse)) = result else {
                    panic!("normal reciprocal missing")
                };
                assert!(inverse.is_normal());
                assert_eq!(divisor * inverse, 1.0);
                assert_eq!(inverse.is_sign_negative(), divisor.is_sign_negative());
            }
        }
    }

    #[test]
    fn reciprocal_strength_reduction_rejects_approximation_and_daz_traps() {
        for value in [
            0.0,
            -0.0,
            3.0,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
            f64::from_bits(1),
            f64::from_bits(2046 << 52),
            f64::from_bits(0x3ff0000000000001),
        ] {
            assert!(
                normal_power_of_two_reciprocal(&Operand::Const(IrConst::F64(value)), IrType::F64)
                    .is_none()
            );
        }
        for value in [
            0.0,
            -0.0,
            3.0,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
            f32::from_bits(1),
            f32::from_bits(254 << 23),
            f32::from_bits(0x3f800001),
        ] {
            assert!(
                normal_power_of_two_reciprocal(&Operand::Const(IrConst::F32(value)), IrType::F32)
                    .is_none()
            );
        }
        assert!(
            normal_power_of_two_reciprocal(&Operand::Const(IrConst::F32(2.0)), IrType::F64)
                .is_none()
        );
        assert!(
            normal_power_of_two_reciprocal(&Operand::Const(IrConst::I64(2)), IrType::F64).is_none()
        );
        assert!(normal_power_of_two_reciprocal(&Operand::Value(Value(1)), IrType::F32).is_none());
    }

    // === Math call simplification tests ===

    #[test]
    fn test_sqrt_to_intrinsic() {
        let inst = make_call("sqrt", vec![Operand::Value(Value(1))], IrType::F64);
        // -fmath-errno (the default): the call stays for the errno guard.
        assert!(simplify_default(&inst).is_none());
        set_math_errno(false);
        let result = simplify_default(&inst).unwrap();
        match result {
            Instruction::Intrinsic {
                op: IntrinsicOp::SqrtF64,
                args,
                ..
            } => {
                assert_eq!(args.len(), 1);
            }
            _ => panic!("Expected SqrtF64 intrinsic, got {:?}", result),
        }
    }

    #[test]
    fn test_sqrtf_to_intrinsic() {
        let inst = make_call("sqrtf", vec![Operand::Value(Value(1))], IrType::F32);
        assert!(simplify_default(&inst).is_none());
        set_math_errno(false);
        let result = simplify_default(&inst).unwrap();
        match result {
            Instruction::Intrinsic {
                op: IntrinsicOp::SqrtF32,
                args,
                ..
            } => {
                assert_eq!(args.len(), 1);
            }
            _ => panic!("Expected SqrtF32 intrinsic, got {:?}", result),
        }
    }

    #[test]
    fn test_fabs_to_intrinsic() {
        let inst = make_call("fabs", vec![Operand::Value(Value(1))], IrType::F64);
        let result = simplify_default(&inst).unwrap();
        match result {
            Instruction::Intrinsic {
                op: IntrinsicOp::FabsF64,
                args,
                ..
            } => {
                assert_eq!(args.len(), 1);
            }
            _ => panic!("Expected FabsF64 intrinsic, got {:?}", result),
        }
    }

    #[test]
    fn test_fabs_lt_zero_folds_false() {
        let mut is_fabs = vec![false; 4];
        is_fabs[1] = true;
        let inst = fold_fabs_lt_zero(
            Value(2),
            IrCmpOp::Slt,
            &Operand::Value(Value(1)),
            &Operand::Const(IrConst::F64(0.0)),
            IrType::F64,
            &is_fabs,
        )
        .unwrap();
        match inst {
            Instruction::Copy {
                src: Operand::Const(c),
                ..
            } => assert!(c.is_zero()),
            other => panic!("expected Copy(0), got {other:?}"),
        }
    }

    #[test]
    fn test_fabs_lt_zero_swapped_gt_folds_false() {
        let mut is_fabs = vec![false; 4];
        is_fabs[1] = true;
        // 0.0 > fabs(x)  <=>  fabs(x) < 0.0
        let inst = fold_fabs_lt_zero(
            Value(2),
            IrCmpOp::Sgt,
            &Operand::Const(IrConst::F64(0.0)),
            &Operand::Value(Value(1)),
            IrType::F64,
            &is_fabs,
        )
        .unwrap();
        match inst {
            Instruction::Copy {
                src: Operand::Const(c),
                ..
            } => assert!(c.is_zero()),
            other => panic!("expected Copy(0), got {other:?}"),
        }
    }

    #[test]
    fn test_fabsf_to_intrinsic() {
        let inst = make_call("fabsf", vec![Operand::Value(Value(1))], IrType::F32);
        let result = simplify_default(&inst).unwrap();
        match result {
            Instruction::Intrinsic {
                op: IntrinsicOp::FabsF32,
                args,
                ..
            } => {
                assert_eq!(args.len(), 1);
            }
            _ => panic!("Expected FabsF32 intrinsic, got {:?}", result),
        }
    }

    #[test]
    fn test_pow_x_zero() {
        let inst = make_call(
            "pow",
            vec![Operand::Value(Value(1)), Operand::Const(IrConst::F64(0.0))],
            IrType::F64,
        );
        let result = simplify_default(&inst).unwrap();
        match result {
            Instruction::Copy {
                src: Operand::Const(IrConst::F64(v)),
                ..
            } => assert_eq!(v, 1.0),
            _ => panic!("Expected Copy with 1.0, got {:?}", result),
        }
    }

    #[test]
    fn test_pow_x_one() {
        let inst = make_call(
            "pow",
            vec![Operand::Value(Value(1)), Operand::Const(IrConst::F64(1.0))],
            IrType::F64,
        );
        let result = simplify_default(&inst).unwrap();
        assert_copy_value(&result, 1);
    }

    #[test]
    fn test_pow_x_two() {
        let inst = make_call(
            "pow",
            vec![Operand::Value(Value(1)), Operand::Const(IrConst::F64(2.0))],
            IrType::F64,
        );
        let result = simplify_default(&inst).unwrap();
        match result {
            Instruction::BinOp {
                op: IrBinOp::Mul,
                lhs: Operand::Value(a),
                rhs: Operand::Value(b),
                ..
            } => {
                assert_eq!(a.0, 1);
                assert_eq!(b.0, 1);
            }
            _ => panic!("Expected Mul x*x, got {:?}", result),
        }
    }

    #[test]
    fn test_pow_x_neg_one() {
        let inst = make_call(
            "pow",
            vec![Operand::Value(Value(1)), Operand::Const(IrConst::F64(-1.0))],
            IrType::F64,
        );
        let result = simplify_default(&inst).unwrap();
        match result {
            Instruction::BinOp {
                op: IrBinOp::SDiv,
                lhs: Operand::Const(IrConst::F64(v)),
                ..
            } => {
                assert_eq!(v, 1.0);
            }
            _ => panic!("Expected 1.0/x, got {:?}", result),
        }
    }

    /// pow(-0, 0.5) = +0 and pow(-inf, 0.5) = +inf, but sqrt gives -0 and
    /// NaN: the call must stay (GCC 16.2 / Clang 23.1 agree at -O2).
    #[test]
    fn test_pow_x_half_is_not_sqrt() {
        let inst = make_call(
            "pow",
            vec![Operand::Value(Value(1)), Operand::Const(IrConst::F64(0.5))],
            IrType::F64,
        );
        assert!(simplify_default(&inst).is_none());
    }

    /// `-fno-builtin[-sqrt]`: a call that does not reach the library
    /// function has no known semantics and is never rewritten.
    #[test]
    fn withdrawn_math_calls_are_not_folded() {
        // sqrt folds only without errno semantics (thread-local flag).
        set_math_errno(false);
        for (name, args) in [
            ("sqrt", vec![Operand::Value(Value(1))]),
            ("fabs", vec![Operand::Value(Value(1))]),
            (
                "pow",
                vec![Operand::Value(Value(1)), Operand::Const(IrConst::F64(2.0))],
            ),
        ] {
            let mut inst = make_call(name, args, IrType::F64);
            assert!(simplify_default(&inst).is_some(), "{name}");
            if let Instruction::Call { info, .. } = &mut inst {
                info.assume_library = false;
            }
            assert!(simplify_default(&inst).is_none(), "{name}");
        }
    }

    #[test]
    fn test_pow_non_special_exponent() {
        let inst = make_call(
            "pow",
            vec![Operand::Value(Value(1)), Operand::Const(IrConst::F64(3.0))],
            IrType::F64,
        );
        assert!(simplify_default(&inst).is_none());
    }

    #[test]
    fn test_math_call_wrong_arg_count() {
        // sqrt with 0 args should not simplify
        let inst = make_call("sqrt", vec![], IrType::F64);
        assert!(simplify_default(&inst).is_none());
    }

    #[test]
    fn test_unknown_func_no_simplify() {
        let inst = make_call("sin", vec![Operand::Value(Value(1))], IrType::F64);
        assert!(simplify_default(&inst).is_none());
    }

    #[test]
    fn test_pow_variable_exponent_no_simplify() {
        let inst = make_call(
            "pow",
            vec![Operand::Value(Value(1)), Operand::Value(Value(2))],
            IrType::F64,
        );
        assert!(simplify_default(&inst).is_none());
    }

    #[test]
    fn test_powf_x_two_f32() {
        let inst = make_call(
            "powf",
            vec![Operand::Value(Value(1)), Operand::Const(IrConst::F32(2.0))],
            IrType::F32,
        );
        let result = simplify_default(&inst).unwrap();
        match result {
            Instruction::BinOp {
                op: IrBinOp::Mul,
                ty: IrType::F32,
                ..
            } => {}
            _ => panic!("Expected F32 Mul, got {:?}", result),
        }
    }

    // === Cmp simplification tests ===

    fn widening_pair_defs(from_ty: IrType, to_ty: IrType) -> Vec<Option<CastDef>> {
        let mut defs = vec![None; 8];
        defs[1] = Some(CastDef {
            src: Operand::Value(Value(5)),
            from_ty,
            to_ty,
        });
        defs[2] = Some(CastDef {
            src: Operand::Value(Value(6)),
            from_ty,
            to_ty,
        });
        defs
    }

    #[test]
    fn test_cmp_widen_pair_signed_keeps_signed_predicates() {
        let defs = widening_pair_defs(IrType::I8, IrType::I32);
        for op in [
            IrCmpOp::Eq,
            IrCmpOp::Ne,
            IrCmpOp::Slt,
            IrCmpOp::Sle,
            IrCmpOp::Sgt,
            IrCmpOp::Sge,
        ] {
            let result = narrow_widened_cmp_pair(
                Value(7),
                op,
                &Operand::Value(Value(1)),
                &Operand::Value(Value(2)),
                IrType::I32,
                &defs,
                &[],
            )
            .expect("signed widened byte pair should narrow");
            match result {
                Instruction::Cmp {
                    dest,
                    op: got_op,
                    lhs: Operand::Value(lhs),
                    rhs: Operand::Value(rhs),
                    ty,
                } => {
                    assert_eq!(dest, Value(7));
                    assert_eq!(got_op, op);
                    assert_eq!(lhs, Value(5));
                    assert_eq!(rhs, Value(6));
                    assert_eq!(ty, IrType::I8);
                }
                other => panic!("expected narrowed Cmp, got {other:?}"),
            }
        }
        // Sign-extended negative bytes compare differently under an unsigned
        // wide predicate, so this must remain wide.
        assert!(
            narrow_widened_cmp_pair(
                Value(7),
                IrCmpOp::Ult,
                &Operand::Value(Value(1)),
                &Operand::Value(Value(2)),
                IrType::I32,
                &defs,
                &[],
            )
            .is_none()
        );
    }

    #[test]
    fn test_cmp_widen_pair_unsigned_is_left_to_adjacent_backend_fold() {
        // The IR rewrite may cross an intervening instruction and reshape RA.
        // Keep U8/U16 pairs for the backend's already-proven adjacent PF-15
        // fold; see the helper's profitability contract.
        let defs = widening_pair_defs(IrType::U16, IrType::I32);
        for op in [
            IrCmpOp::Eq,
            IrCmpOp::Ne,
            IrCmpOp::Slt,
            IrCmpOp::Sle,
            IrCmpOp::Sgt,
            IrCmpOp::Sge,
            IrCmpOp::Ult,
            IrCmpOp::Ule,
            IrCmpOp::Ugt,
            IrCmpOp::Uge,
        ] {
            assert!(
                narrow_widened_cmp_pair(
                    Value(7),
                    op,
                    &Operand::Value(Value(1)),
                    &Operand::Value(Value(2)),
                    IrType::I32,
                    &defs,
                    &[],
                )
                .is_none()
            );
        }
    }

    #[test]
    fn test_widened_rewrites_require_the_dead_cast_carrier() {
        let defs = widening_pair_defs(IrType::I8, IrType::I32);
        let mut uses = vec![0; 8];
        uses[1] = 1;
        uses[2] = 2; // An additional consumer keeps this widened value live.
        assert!(
            narrow_widened_cmp_pair(
                Value(7),
                IrCmpOp::Eq,
                &Operand::Value(Value(1)),
                &Operand::Value(Value(2)),
                IrType::I32,
                &defs,
                &uses,
            )
            .is_none()
        );

        uses[2] = 1;
        assert!(
            narrow_widened_cmp_pair(
                Value(7),
                IrCmpOp::Eq,
                &Operand::Value(Value(1)),
                &Operand::Value(Value(2)),
                IrType::I32,
                &defs,
                &uses,
            )
            .is_some()
        );

        // A cast that survives the rewrite (another reader) keeps its condition.
        let mut dies = vec![true; uses.len()];
        dies[1] = false;
        assert!(widened_cond_replacement(Operand::Value(Value(1)), &defs, &dies).is_none());
    }

    #[test]
    fn test_cmp_widen_pair_refuses_mixed_or_nonwidening_types() {
        let mut mixed = widening_pair_defs(IrType::I8, IrType::I32);
        mixed[2] = Some(CastDef {
            src: Operand::Value(Value(6)),
            from_ty: IrType::U8,
            to_ty: IrType::I32,
        });
        assert!(
            narrow_widened_cmp_pair(
                Value(7),
                IrCmpOp::Eq,
                &Operand::Value(Value(1)),
                &Operand::Value(Value(2)),
                IrType::I32,
                &mixed,
                &[],
            )
            .is_none()
        );

        let same_width = widening_pair_defs(IrType::I32, IrType::U32);
        assert!(
            narrow_widened_cmp_pair(
                Value(7),
                IrCmpOp::Eq,
                &Operand::Value(Value(1)),
                &Operand::Value(Value(2)),
                IrType::U32,
                &same_width,
                &[],
            )
            .is_none()
        );
    }

    #[test]
    fn test_widened_cond_replacement_preserves_only_widening_integer_zero_test() {
        let signed = widening_pair_defs(IrType::I8, IrType::I32);
        assert!(matches!(
            widened_cond_replacement(Operand::Value(Value(1)), &signed, &[]),
            Some(Operand::Value(Value(5)))
        ));

        // 0x100 truncates to zero in U8, so replacing that condition with its
        // source would be wrong.
        let trunc = widening_pair_defs(IrType::I32, IrType::U8);
        assert!(widened_cond_replacement(Operand::Value(Value(1)), &trunc, &[]).is_none());
        assert!(widened_cond_replacement(Operand::Const(IrConst::I32(1)), &signed, &[]).is_none());
    }

    #[test]
    fn test_cmp_widen_pair_enters_generic_simplify_dispatch() {
        let defs = widening_pair_defs(IrType::I8, IrType::I32);
        let inst = Instruction::Cmp {
            dest: Value(7),
            op: IrCmpOp::Eq,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Value(Value(2)),
            ty: IrType::I32,
        };
        let result = try_simplify(&inst, &defs, &[], &[], &[], &[], &[])
            .expect("generic simplify should dispatch the widened pair rule");
        assert!(matches!(
            result,
            Instruction::Cmp {
                ty: IrType::I8,
                lhs: Operand::Value(Value(5)),
                rhs: Operand::Value(Value(6)),
                ..
            }
        ));
    }

    #[test]
    fn test_cmp_self_eq() {
        // Cmp(Eq, x, x) => 1 (always true)
        let inst = Instruction::Cmp {
            dest: Value(2),
            op: IrCmpOp::Eq,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Value(Value(1)),
            ty: IrType::I32,
        };
        let result = simplify_default(&inst).unwrap();
        assert_copy_bool(&result, 1);
    }

    #[test]
    fn test_cmp_self_ne() {
        // Cmp(Ne, x, x) => 0 (always false)
        let inst = Instruction::Cmp {
            dest: Value(2),
            op: IrCmpOp::Ne,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Value(Value(1)),
            ty: IrType::I64,
        };
        let result = simplify_default(&inst).unwrap();
        assert_copy_bool(&result, 0);
    }

    #[test]
    fn test_cmp_self_sle() {
        // Cmp(Sle, x, x) => 1 (always true)
        let inst = Instruction::Cmp {
            dest: Value(2),
            op: IrCmpOp::Sle,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Value(Value(1)),
            ty: IrType::I32,
        };
        let result = simplify_default(&inst).unwrap();
        assert_copy_bool(&result, 1);
    }

    #[test]
    fn test_cmp_self_slt() {
        // Cmp(Slt, x, x) => 0 (always false)
        let inst = Instruction::Cmp {
            dest: Value(2),
            op: IrCmpOp::Slt,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Value(Value(1)),
            ty: IrType::I32,
        };
        let result = simplify_default(&inst).unwrap();
        assert_copy_bool(&result, 0);
    }

    #[test]
    fn test_cmp_ne_cmp_result_zero() {
        // Cmp(Ne, cmp_result, 0) => Copy(cmp_result)
        let mut cmp_defs: Vec<Option<CmpDef>> = vec![None; 3];
        cmp_defs[1] = Some(CmpDef {
            op: IrCmpOp::Eq,
            lhs: Operand::Value(Value(10)),
            rhs: Operand::Value(Value(11)),
            ty: IrType::I64,
        });
        let mut booleans = vec![false; 3];
        booleans[1] = true;

        let inst = Instruction::Cmp {
            dest: Value(2),
            op: IrCmpOp::Ne,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I64(0)),
            ty: IrType::I64,
        };
        let result = try_simplify(&inst, &[], &[], &cmp_defs, &[], &[], &booleans).unwrap();
        assert_copy_value(&result, 1);
    }

    #[test]
    fn test_cmp_eq_cmp_result_zero() {
        // Cmp(Eq, cmp_result, 0) => Cmp(inverted, orig_lhs, orig_rhs)
        let mut cmp_defs: Vec<Option<CmpDef>> = vec![None; 3];
        cmp_defs[1] = Some(CmpDef {
            op: IrCmpOp::Slt,
            lhs: Operand::Value(Value(10)),
            rhs: Operand::Value(Value(11)),
            ty: IrType::I64,
        });
        let mut booleans = vec![false; 3];
        booleans[1] = true;

        let inst = Instruction::Cmp {
            dest: Value(2),
            op: IrCmpOp::Eq,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I64(0)),
            ty: IrType::I64,
        };
        let result = try_simplify(&inst, &[], &[], &cmp_defs, &[], &[], &booleans).unwrap();
        match result {
            Instruction::Cmp {
                op: IrCmpOp::Sge,
                lhs: Operand::Value(a),
                rhs: Operand::Value(b),
                ..
            } => {
                assert_eq!(a.0, 10);
                assert_eq!(b.0, 11);
            }
            _ => panic!("Expected Cmp(Sge, V10, V11), got {:?}", result),
        }
    }

    #[test]
    fn test_cmp_eq_cmp_result_one() {
        // Cmp(Eq, cmp_result, 1) => Copy(cmp_result)
        let mut cmp_defs: Vec<Option<CmpDef>> = vec![None; 3];
        cmp_defs[1] = Some(CmpDef {
            op: IrCmpOp::Eq,
            lhs: Operand::Value(Value(10)),
            rhs: Operand::Value(Value(11)),
            ty: IrType::I64,
        });
        let mut booleans = vec![false; 3];
        booleans[1] = true;

        let inst = Instruction::Cmp {
            dest: Value(2),
            op: IrCmpOp::Eq,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I64(1)),
            ty: IrType::I64,
        };
        let result = try_simplify(&inst, &[], &[], &cmp_defs, &[], &[], &booleans).unwrap();
        assert_copy_value(&result, 1);
    }

    #[test]
    fn test_cmp_ne_cmp_result_one() {
        // Cmp(Ne, cmp_result, 1) => Cmp(inverted)
        let mut cmp_defs: Vec<Option<CmpDef>> = vec![None; 3];
        cmp_defs[1] = Some(CmpDef {
            op: IrCmpOp::Ne,
            lhs: Operand::Value(Value(10)),
            rhs: Operand::Value(Value(11)),
            ty: IrType::I64,
        });
        let mut booleans = vec![false; 3];
        booleans[1] = true;

        let inst = Instruction::Cmp {
            dest: Value(2),
            op: IrCmpOp::Ne,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I64(1)),
            ty: IrType::I64,
        };
        let result = try_simplify(&inst, &[], &[], &cmp_defs, &[], &[], &booleans).unwrap();
        match result {
            Instruction::Cmp {
                op: IrCmpOp::Eq,
                lhs: Operand::Value(a),
                rhs: Operand::Value(b),
                ..
            } => {
                assert_eq!(a.0, 10);
                assert_eq!(b.0, 11);
            }
            _ => panic!("Expected Cmp(Eq, V10, V11), got {:?}", result),
        }
    }

    #[test]
    fn test_cmp_ne_boolean_and_zero() {
        // Cmp(Ne, And(cmp1, cmp2), 0) => Copy(And_result)
        let mut booleans = vec![false; 5];
        booleans[3] = true; // Value(3) = And(cmp1, cmp2) is boolean

        let inst = Instruction::Cmp {
            dest: Value(4),
            op: IrCmpOp::Ne,
            lhs: Operand::Value(Value(3)),
            rhs: Operand::Const(IrConst::I64(0)),
            ty: IrType::I64,
        };
        let result = try_simplify(&inst, &[], &[], &[], &[], &[], &booleans).unwrap();
        assert_copy_value(&result, 3);
    }

    #[test]
    fn test_cmp_ult_zero_always_false() {
        // Cmp(Ult, x, 0) is always false (nothing is < 0 unsigned)
        let inst = Instruction::Cmp {
            dest: Value(2),
            op: IrCmpOp::Ult,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I64(0)),
            ty: IrType::U64,
        };
        let result = simplify_default(&inst).unwrap();
        assert_copy_bool(&result, 0);
    }

    #[test]
    fn test_cmp_uge_zero_always_true() {
        // Cmp(Uge, x, 0) is always true (everything is >= 0 unsigned)
        let inst = Instruction::Cmp {
            dest: Value(2),
            op: IrCmpOp::Uge,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I64(0)),
            ty: IrType::U64,
        };
        let result = simplify_default(&inst).unwrap();
        assert_copy_bool(&result, 1);
    }

    #[test]
    fn test_cmp_no_simplify_non_boolean() {
        // Cmp(Ne, non_cmp_value, 0) should NOT simplify (value could be any integer)
        let inst = Instruction::Cmp {
            dest: Value(2),
            op: IrCmpOp::Ne,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I64(0)),
            ty: IrType::I64,
        };
        assert!(simplify_default(&inst).is_none());
    }

    // === All-ones pattern tests ===

    #[test]
    fn test_and_all_ones_i32() {
        // x & 0xFFFFFFFF => x (I32)
        let inst = binop_val_const(IrBinOp::And, IrConst::I32(-1), IrType::I32);
        let result = simplify_default(&inst).unwrap();
        assert_copy_value(&result, 1);
    }

    #[test]
    fn test_and_all_ones_i64() {
        // x & -1 => x (I64)
        let inst = binop_val_const(IrBinOp::And, IrConst::I64(-1), IrType::I64);
        let result = simplify_default(&inst).unwrap();
        assert_copy_value(&result, 1);
    }

    #[test]
    fn test_or_all_ones_i32() {
        // x | 0xFFFFFFFF => 0xFFFFFFFF (I32)
        let inst = binop_val_const(IrBinOp::Or, IrConst::I32(-1), IrType::I32);
        let result = simplify_default(&inst).unwrap();
        match result {
            Instruction::Copy {
                src: Operand::Const(c),
                ..
            } => {
                assert_eq!(c.to_i64(), Some(-1i32 as i64));
            }
            _ => panic!("Expected Copy with all-ones, got {:?}", result),
        }
    }

    #[test]
    fn test_or_all_ones_i64() {
        // x | -1 => -1 (I64)
        let inst = binop_val_const(IrBinOp::Or, IrConst::I64(-1), IrType::I64);
        let result = simplify_default(&inst).unwrap();
        match result {
            Instruction::Copy {
                src: Operand::Const(IrConst::I64(-1)),
                ..
            } => {}
            _ => panic!("Expected Copy with I64(-1), got {:?}", result),
        }
    }

    // === is_all_ones helper tests ===

    #[test]
    fn test_is_all_ones_i8() {
        assert!(is_all_ones(&Operand::Const(IrConst::I8(-1)), IrType::I8));
        assert!(is_all_ones(&Operand::Const(IrConst::I8(-1)), IrType::U8));
        assert!(!is_all_ones(&Operand::Const(IrConst::I8(0)), IrType::I8));
        assert!(!is_all_ones(&Operand::Const(IrConst::I8(1)), IrType::I8));
    }

    #[test]
    fn test_is_all_ones_i32() {
        assert!(is_all_ones(&Operand::Const(IrConst::I32(-1)), IrType::I32));
        assert!(!is_all_ones(&Operand::Const(IrConst::I32(0)), IrType::I32));
    }

    #[test]
    fn test_is_all_ones_not_float() {
        assert!(!is_all_ones(
            &Operand::Const(IrConst::F64(-1.0)),
            IrType::F64
        ));
    }

    // === Select simplification tests ===

    #[test]
    fn test_select_const_true() {
        // select 1, a, b => a
        let inst = Instruction::Select {
            dest: Value(3),
            cond: Operand::Const(IrConst::I32(1)),
            true_val: Operand::Value(Value(1)),
            false_val: Operand::Value(Value(2)),
            ty: IrType::I32,
        };
        let result = simplify_default(&inst).unwrap();
        assert_copy_value(&result, 1);
    }

    #[test]
    fn test_select_const_false() {
        // select 0, a, b => b
        let inst = Instruction::Select {
            dest: Value(3),
            cond: Operand::Const(IrConst::I32(0)),
            true_val: Operand::Value(Value(1)),
            false_val: Operand::Value(Value(2)),
            ty: IrType::I32,
        };
        let result = simplify_default(&inst).unwrap();
        assert_copy_value(&result, 2);
    }

    #[test]
    fn test_select_const_nonzero() {
        // select 42, a, b => a (any nonzero is true)
        let inst = Instruction::Select {
            dest: Value(3),
            cond: Operand::Const(IrConst::I32(42)),
            true_val: Operand::Value(Value(1)),
            false_val: Operand::Value(Value(2)),
            ty: IrType::I32,
        };
        let result = simplify_default(&inst).unwrap();
        assert_copy_value(&result, 1);
    }

    #[test]
    fn select_constant_cast_walk_is_bounded_and_folds_valid_chains() {
        let mut const_defs = vec![None; 10];
        const_defs[0] = Some(IrConst::I32(7));
        let mut cast_defs = vec![None; 10];
        let mut def_counts = vec![1; 10];
        for i in 1..=9 {
            cast_defs[i] = Some(CastDef {
                src: Operand::Value(Value((i - 1) as u32)),
                from_ty: IrType::I32,
                to_ty: IrType::I32,
            });
        }
        // Eight casts are within the analysis budget; a longer chain fails
        // closed rather than recursing without a bound.
        assert_eq!(
            resolve_select_const(
                Operand::Value(Value(8)),
                &const_defs,
                &cast_defs,
                &def_counts,
            ),
            Some(IrConst::I32(7))
        );
        assert_eq!(
            resolve_select_const(
                Operand::Value(Value(9)),
                &const_defs,
                &cast_defs,
                &def_counts,
            ),
            None
        );
        def_counts[1] = 2;
        assert_eq!(
            resolve_select_const(
                Operand::Value(Value(1)),
                &const_defs,
                &cast_defs,
                &def_counts,
            ),
            None
        );
    }

    #[test]
    fn trichotomy_masks_round_trip_every_integer_compare() {
        // The mask algebra is only sound if the mapping is total and
        // invertible on exactly the six non-trivial trichotomies; the
        // all-false and all-true masks must refuse (they are constants,
        // not compares).
        for (op, unsigned) in [
            (IrCmpOp::Eq, false),
            (IrCmpOp::Ne, false),
            (IrCmpOp::Slt, false),
            (IrCmpOp::Sle, false),
            (IrCmpOp::Sgt, false),
            (IrCmpOp::Sge, false),
            (IrCmpOp::Ult, true),
            (IrCmpOp::Ule, true),
            (IrCmpOp::Ugt, true),
            (IrCmpOp::Uge, true),
        ] {
            let mask = int_cmp_trichotomy_mask(op);
            assert_eq!(
                int_cmp_from_trichotomy_mask(mask, unsigned),
                Some(op),
                "{op:?} via mask {mask:#05b}"
            );
        }
        for unsigned in [false, true] {
            assert_eq!(int_cmp_from_trichotomy_mask(0b000, unsigned), None);
            assert_eq!(int_cmp_from_trichotomy_mask(0b111, unsigned), None);
        }
    }

    #[test]
    fn peel_bool_cond_is_bounded_and_narrowing_needs_a_proven_boolean() {
        // Seventeen widening casts over Value(0): a 16-step peel budget
        // reaches Value(0) from Value(16) and stops at Value(1) from
        // Value(17) instead of walking unbounded chains.
        let mut cast_defs: Vec<Option<CastDef>> = vec![None; 18];
        for i in 1..=17usize {
            cast_defs[i] = Some(CastDef {
                src: Operand::Value(Value((i - 1) as u32)),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            });
        }
        let is_boolean = vec![false; 18];
        let cmp_defs: Vec<Option<CmpDef>> = vec![None; 18];
        let def_counts = vec![1u8; 18];
        assert!(matches!(
            peel_bool_cond(
                Operand::Value(Value(16)),
                &cast_defs,
                &is_boolean,
                &cmp_defs,
                &def_counts
            ),
            Operand::Value(Value(0))
        ));
        assert!(matches!(
            peel_bool_cond(
                Operand::Value(Value(17)),
                &cast_defs,
                &is_boolean,
                &cmp_defs,
                &def_counts
            ),
            Operand::Value(Value(1))
        ));
        // One narrowing cast over a non-boolean base: 0x100 truncates to
        // zero, so the peel must refuse and return the operand unchanged.
        cast_defs[1] = Some(CastDef {
            src: Operand::Value(Value(0)),
            from_ty: IrType::I64,
            to_ty: IrType::I8,
        });
        assert!(matches!(
            peel_bool_cond(
                Operand::Value(Value(1)),
                &cast_defs,
                &is_boolean,
                &cmp_defs,
                &def_counts
            ),
            Operand::Value(Value(1))
        ));
        // The same narrowing cast over a proven boolean (0/1 survives any
        // truncation) peels to the base.
        let mut is_boolean = vec![false; 18];
        is_boolean[0] = true;
        assert!(matches!(
            peel_bool_cond(
                Operand::Value(Value(1)),
                &cast_defs,
                &is_boolean,
                &cmp_defs,
                &def_counts
            ),
            Operand::Value(Value(0))
        ));
    }

    // === Constant reassociation tests ===

    #[test]
    fn test_reassociate_add() {
        // (x + 10) + 20 => x + 30
        let mut binop_defs: Vec<Option<BinOpDef>> = vec![None; 4];
        binop_defs[1] = Some(BinOpDef {
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Const(IrConst::I32(10)),
            ty: IrType::I32,
        });
        let inst = Instruction::BinOp {
            dest: Value(2),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I32(20)),
            ty: IrType::I32,
        };
        let result = try_simplify(&inst, &[], &[], &[], &binop_defs, &[], &[]).unwrap();
        match result {
            Instruction::BinOp {
                op: IrBinOp::Add,
                lhs: Operand::Value(v),
                rhs: Operand::Const(IrConst::I32(30)),
                ..
            } => {
                assert_eq!(v.0, 0);
            }
            _ => panic!("Expected Add(V0, 30), got {:?}", result),
        }
    }

    #[test]
    fn test_reassociate_add_cancels() {
        // (x + 5) + (-5) => x
        let mut binop_defs: Vec<Option<BinOpDef>> = vec![None; 4];
        binop_defs[1] = Some(BinOpDef {
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Const(IrConst::I32(5)),
            ty: IrType::I32,
        });
        let inst = Instruction::BinOp {
            dest: Value(2),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I32(-5)),
            ty: IrType::I32,
        };
        let result = try_simplify(&inst, &[], &[], &[], &binop_defs, &[], &[]).unwrap();
        assert_copy_value(&result, 0);
    }

    #[test]
    fn test_reassociate_sub() {
        // (x - 10) - 20 => x - 30
        let mut binop_defs: Vec<Option<BinOpDef>> = vec![None; 4];
        binop_defs[1] = Some(BinOpDef {
            op: IrBinOp::Sub,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Const(IrConst::I32(10)),
            ty: IrType::I32,
        });
        let inst = Instruction::BinOp {
            dest: Value(2),
            op: IrBinOp::Sub,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I32(20)),
            ty: IrType::I32,
        };
        let result = try_simplify(&inst, &[], &[], &[], &binop_defs, &[], &[]).unwrap();
        match result {
            Instruction::BinOp {
                op: IrBinOp::Sub,
                lhs: Operand::Value(v),
                rhs: Operand::Const(IrConst::I32(30)),
                ..
            } => {
                assert_eq!(v.0, 0);
            }
            _ => panic!("Expected Sub(V0, 30), got {:?}", result),
        }
    }

    #[test]
    fn test_reassociate_and() {
        // (x & 0xFF) & 0x0F => x & 0x0F
        let mut binop_defs: Vec<Option<BinOpDef>> = vec![None; 4];
        binop_defs[1] = Some(BinOpDef {
            op: IrBinOp::And,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Const(IrConst::I32(0xFF)),
            ty: IrType::I32,
        });
        let inst = Instruction::BinOp {
            dest: Value(2),
            op: IrBinOp::And,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I32(0x0F)),
            ty: IrType::I32,
        };
        let result = try_simplify(&inst, &[], &[], &[], &binop_defs, &[], &[]).unwrap();
        match result {
            Instruction::BinOp {
                op: IrBinOp::And,
                lhs: Operand::Value(v),
                rhs: Operand::Const(IrConst::I32(0x0F)),
                ..
            } => {
                assert_eq!(v.0, 0);
            }
            _ => panic!("Expected And(V0, 0x0F), got {:?}", result),
        }
    }

    #[test]
    fn test_reassociate_xor_cancels() {
        // (x ^ 0xFF) ^ 0xFF => x (cancels out)
        let mut binop_defs: Vec<Option<BinOpDef>> = vec![None; 4];
        binop_defs[1] = Some(BinOpDef {
            op: IrBinOp::Xor,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Const(IrConst::I32(0xFF)),
            ty: IrType::I32,
        });
        let inst = Instruction::BinOp {
            dest: Value(2),
            op: IrBinOp::Xor,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I32(0xFF)),
            ty: IrType::I32,
        };
        let result = try_simplify(&inst, &[], &[], &[], &binop_defs, &[], &[]).unwrap();
        assert_copy_value(&result, 0);
    }

    #[test]
    fn arithmetic_shift_of_all_ones_stays_all_ones() {
        let inst = Instruction::BinOp {
            dest: Value(2),
            op: IrBinOp::AShr,
            lhs: Operand::Const(IrConst::I32(-1)),
            rhs: Operand::Value(Value(1)),
            ty: IrType::I32,
        };
        let result = simplify_default(&inst).unwrap();
        assert_copy_const_i32(&result, -1);
    }

    #[test]
    fn test_reassociate_shift() {
        // (x << 2) << 3 => x << 5
        let mut binop_defs: Vec<Option<BinOpDef>> = vec![None; 4];
        binop_defs[1] = Some(BinOpDef {
            op: IrBinOp::Shl,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Const(IrConst::I32(2)),
            ty: IrType::I32,
        });
        let inst = Instruction::BinOp {
            dest: Value(2),
            op: IrBinOp::Shl,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I32(3)),
            ty: IrType::I32,
        };
        let result = try_simplify(&inst, &[], &[], &[], &binop_defs, &[], &[]).unwrap();
        match result {
            Instruction::BinOp {
                op: IrBinOp::Shl,
                lhs: Operand::Value(v),
                rhs: Operand::Const(IrConst::I32(5)),
                ..
            } => {
                assert_eq!(v.0, 0);
            }
            _ => panic!("Expected Shl(V0, 5), got {:?}", result),
        }
    }

    #[test]
    fn test_reassociate_shift_overflow_to_zero() {
        // (x << 20) << 20 => 0 (combined shift >= 32 bits for I32)
        let mut binop_defs: Vec<Option<BinOpDef>> = vec![None; 4];
        binop_defs[1] = Some(BinOpDef {
            op: IrBinOp::Shl,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Const(IrConst::I32(20)),
            ty: IrType::I32,
        });
        let inst = Instruction::BinOp {
            dest: Value(2),
            op: IrBinOp::Shl,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I32(20)),
            ty: IrType::I32,
        };
        let result = try_simplify(&inst, &[], &[], &[], &binop_defs, &[], &[]).unwrap();
        assert_copy_const_i32(&result, 0);
    }

    // === Multiply by -1 tests ===

    #[test]
    fn test_mul_neg_one_rhs() {
        // x * (-1) => neg(x)
        let inst = binop_val_const(IrBinOp::Mul, IrConst::I32(-1), IrType::I32);
        let result = simplify_default(&inst).unwrap();
        match result {
            Instruction::UnaryOp {
                op: IrUnaryOp::Neg,
                src: Operand::Value(v),
                ty: IrType::I32,
                ..
            } => {
                assert_eq!(v.0, 1);
            }
            _ => panic!("Expected UnaryOp::Neg, got {:?}", result),
        }
    }

    #[test]
    fn test_mul_neg_one_lhs() {
        // (-1) * x => neg(x)
        let inst = binop(
            IrBinOp::Mul,
            Operand::Const(IrConst::I64(-1)),
            Operand::Value(Value(1)),
            IrType::I64,
        );
        let result = simplify_default(&inst).unwrap();
        match result {
            Instruction::UnaryOp {
                op: IrUnaryOp::Neg,
                src: Operand::Value(v),
                ty: IrType::I64,
                ..
            } => {
                assert_eq!(v.0, 1);
            }
            _ => panic!("Expected UnaryOp::Neg, got {:?}", result),
        }
    }

    #[test]
    fn test_mul_neg_one_float_not_simplified() {
        // Float x * (-1) should NOT be converted to neg (keep mul for IEEE semantics)
        let inst = binop_val_const(IrBinOp::Mul, IrConst::I64(-1), IrType::F64);
        // Note: is_neg_one checks ty.is_integer(), so this won't match
        assert!(simplify_default(&inst).is_none());
    }

    // === Multiply reassociation tests ===

    #[test]
    fn test_reassociate_mul() {
        // (x * 3) * 5 => x * 15
        let mut binop_defs: Vec<Option<BinOpDef>> = vec![None; 4];
        binop_defs[1] = Some(BinOpDef {
            op: IrBinOp::Mul,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Const(IrConst::I32(3)),
            ty: IrType::I32,
        });
        let inst = Instruction::BinOp {
            dest: Value(2),
            op: IrBinOp::Mul,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I32(5)),
            ty: IrType::I32,
        };
        let result = try_simplify(&inst, &[], &[], &[], &binop_defs, &[], &[]).unwrap();
        match result {
            Instruction::BinOp {
                op: IrBinOp::Mul,
                lhs: Operand::Value(v),
                rhs: Operand::Const(IrConst::I32(15)),
                ..
            } => {
                assert_eq!(v.0, 0, "Should use original non-const operand");
            }
            _ => panic!("Expected Mul(V0, 15), got {:?}", result),
        }
    }

    #[test]
    fn test_reassociate_mul_to_zero() {
        // (x * 3) * 0 => 0 (already handled by mul-by-zero, but also via reassociation)
        // This test verifies the case is handled correctly
        let inst = binop_val_const(IrBinOp::Mul, IrConst::I32(0), IrType::I32);
        let result = simplify_default(&inst).unwrap();
        assert_copy_const_i32(&result, 0);
    }

    #[test]
    fn test_reassociate_mul_commuted() {
        // 5 * (x * 3) => x * 15
        let mut binop_defs: Vec<Option<BinOpDef>> = vec![None; 4];
        binop_defs[1] = Some(BinOpDef {
            op: IrBinOp::Mul,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Const(IrConst::I64(3)),
            ty: IrType::I64,
        });
        let inst = Instruction::BinOp {
            dest: Value(2),
            op: IrBinOp::Mul,
            lhs: Operand::Const(IrConst::I64(5)),
            rhs: Operand::Value(Value(1)),
            ty: IrType::I64,
        };
        let result = try_simplify(&inst, &[], &[], &[], &binop_defs, &[], &[]).unwrap();
        match result {
            Instruction::BinOp {
                op: IrBinOp::Mul,
                lhs: Operand::Value(v),
                rhs: Operand::Const(IrConst::I64(15)),
                ..
            } => {
                assert_eq!(v.0, 0);
            }
            _ => panic!("Expected Mul(V0, 15), got {:?}", result),
        }
    }

    // === Neg-of-neg tests ===

    #[test]
    fn test_neg_of_neg_int_folds_to_copy() {
        // neg(neg(x)) => x (two's-complement self-inverse, INT_MIN included)
        let mut neg_defs: Vec<Option<NegDef>> = vec![None; 4];
        neg_defs[1] = Some(NegDef {
            src: Operand::Value(Value(0)),
        });
        let inst = Instruction::UnaryOp {
            dest: Value(2),
            op: IrUnaryOp::Neg,
            src: Operand::Value(Value(1)),
            ty: IrType::I32,
        };
        let result = try_simplify(&inst, &[], &[], &[], &[], &neg_defs, &[]).unwrap();
        assert_copy_value(&result, 0);
    }

    #[test]
    fn test_neg_of_neg_float_folds_to_copy() {
        // neg(neg(x)) => x for floats TOO: Neg is the sign-bit XOR, so the
        // double flip is a bitwise identity — NaN payloads, NaN signs and
        // ±0 survive exactly. This is the discriminator vs the integer-only
        // reassociation folds above.
        let mut neg_defs: Vec<Option<NegDef>> = vec![None; 4];
        neg_defs[1] = Some(NegDef {
            src: Operand::Value(Value(0)),
        });
        let inst = Instruction::UnaryOp {
            dest: Value(2),
            op: IrUnaryOp::Neg,
            src: Operand::Value(Value(1)),
            ty: IrType::F64,
        };
        let result = try_simplify(&inst, &[], &[], &[], &[], &neg_defs, &[]).unwrap();
        assert_copy_value(&result, 0);
    }

    #[test]
    fn test_neg_of_neg_f32_folds_to_copy() {
        // Same identity at f32 width (the fold is width-agnostic because
        // SSA typing forces the outer Neg to read at the inner's width).
        let mut neg_defs: Vec<Option<NegDef>> = vec![None; 4];
        neg_defs[1] = Some(NegDef {
            src: Operand::Value(Value(0)),
        });
        let inst = Instruction::UnaryOp {
            dest: Value(2),
            op: IrUnaryOp::Neg,
            src: Operand::Value(Value(1)),
            ty: IrType::F32,
        };
        let result = try_simplify(&inst, &[], &[], &[], &[], &neg_defs, &[]).unwrap();
        assert_copy_value(&result, 0);
    }

    #[test]
    fn test_neg_of_non_neg_source_stays() {
        // neg(x) where x is NOT a Neg def: no fold (the discriminating
        // negative control — only double negation collapses).
        let inst = Instruction::UnaryOp {
            dest: Value(2),
            op: IrUnaryOp::Neg,
            src: Operand::Value(Value(1)),
            ty: IrType::F64,
        };
        assert!(try_simplify(&inst, &[], &[], &[], &[], &[], &[]).is_none());
    }

    #[test]
    fn test_neg_of_const_stays() {
        // neg(Const) has no def to look through; constant folding is
        // handled elsewhere. The fold must not fire (let alone guess).
        let inst = Instruction::UnaryOp {
            dest: Value(2),
            op: IrUnaryOp::Neg,
            src: Operand::Const(IrConst::F64(1.5)),
            ty: IrType::F64,
        };
        assert!(try_simplify(&inst, &[], &[], &[], &[], &[], &[]).is_none());
    }

    #[test]
    fn test_neg_of_neg_chain_link_folds_via_def_map() {
        // The neg^4 chain's outermost link: dest=4 reads Value(3) whose
        // def-map entry points at Value(2). One sweep folds every even
        // link through the ORIGINAL def map — this pins the mechanics.
        let mut neg_defs: Vec<Option<NegDef>> = vec![None; 6];
        neg_defs[1] = Some(NegDef {
            src: Operand::Value(Value(0)),
        });
        neg_defs[2] = Some(NegDef {
            src: Operand::Value(Value(1)),
        });
        neg_defs[3] = Some(NegDef {
            src: Operand::Value(Value(2)),
        });
        let inst = Instruction::UnaryOp {
            dest: Value(4),
            op: IrUnaryOp::Neg,
            src: Operand::Value(Value(3)),
            ty: IrType::F64,
        };
        let result = try_simplify(&inst, &[], &[], &[], &[], &neg_defs, &[]).unwrap();
        assert_copy_value(&result, 2);
    }

    #[test]
    fn test_other_unary_op_never_folds_through_neg_def() {
        // A non-Neg unary op reading a Neg def must not be touched — the
        // arm is op-discriminated, not operand-discriminated.
        let mut neg_defs: Vec<Option<NegDef>> = vec![None; 4];
        neg_defs[1] = Some(NegDef {
            src: Operand::Value(Value(0)),
        });
        let inst = Instruction::UnaryOp {
            dest: Value(2),
            op: IrUnaryOp::Clz,
            src: Operand::Value(Value(1)),
            ty: IrType::I32,
        };
        assert!(try_simplify(&inst, &[], &[], &[], &[], &neg_defs, &[]).is_none());
    }

    // === Float add/sub-of-neg tests (the four-oracle consensus folds) ===

    #[test]
    fn float_add_of_neg_folds_to_sub() {
        // x + (neg y) => x - y (GCC 16.2, Clang 23.1, ICX and local GCC
        // 14.2 all emit the mirrored op — the four-oracle consensus).
        let mut neg_defs: Vec<Option<NegDef>> = vec![None; 4];
        neg_defs[2] = Some(NegDef {
            src: Operand::Value(Value(1)),
        });
        let inst = Instruction::BinOp {
            dest: Value(3),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Value(Value(2)),
            ty: IrType::F64,
        };
        let result = try_simplify_neg(&inst, &neg_defs).unwrap();
        match result {
            Instruction::BinOp {
                op: IrBinOp::Sub,
                lhs,
                rhs,
                ..
            } => {
                assert!(matches!(lhs, Operand::Value(v) if v.0 == 0));
                assert!(matches!(rhs, Operand::Value(v) if v.0 == 1));
            }
            _ => panic!("Expected Sub(V0, V1), got {:?}", result),
        }
    }

    #[test]
    fn float_add_of_neg_commuted_folds_to_sub() {
        // (neg y) + x => x - y (Sub is non-commutative: the plain operand
        // stays the minuend).
        let mut neg_defs: Vec<Option<NegDef>> = vec![None; 4];
        neg_defs[2] = Some(NegDef {
            src: Operand::Value(Value(1)),
        });
        let inst = Instruction::BinOp {
            dest: Value(3),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(2)),
            rhs: Operand::Value(Value(0)),
            ty: IrType::F64,
        };
        let result = try_simplify_neg(&inst, &neg_defs).unwrap();
        match result {
            Instruction::BinOp {
                op: IrBinOp::Sub,
                lhs,
                rhs,
                ..
            } => {
                assert!(
                    matches!(lhs, Operand::Value(v) if v.0 == 0),
                    "minuend must be the plain operand"
                );
                assert!(matches!(rhs, Operand::Value(v) if v.0 == 1));
            }
            _ => panic!("Expected Sub(V0, V1), got {:?}", result),
        }
    }

    #[test]
    fn float_sub_of_neg_folds_to_add() {
        // x - (neg y) => x + y for floats (the extension of the historical
        // integer fold to the float domain, matching every oracle).
        let mut neg_defs: Vec<Option<NegDef>> = vec![None; 4];
        neg_defs[2] = Some(NegDef {
            src: Operand::Value(Value(1)),
        });
        let inst = Instruction::BinOp {
            dest: Value(3),
            op: IrBinOp::Sub,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Value(Value(2)),
            ty: IrType::F64,
        };
        let result = try_simplify_neg(&inst, &neg_defs).unwrap();
        match result {
            Instruction::BinOp {
                op: IrBinOp::Add,
                lhs,
                rhs,
                ..
            } => {
                assert!(matches!(lhs, Operand::Value(v) if v.0 == 0));
                assert!(matches!(rhs, Operand::Value(v) if v.0 == 1));
            }
            _ => panic!("Expected Add(V0, V1), got {:?}", result),
        }
    }

    #[test]
    fn float_add_of_neg_f32_folds_to_sub() {
        // The fold is width-agnostic (f32 mirror).
        let mut neg_defs: Vec<Option<NegDef>> = vec![None; 4];
        neg_defs[2] = Some(NegDef {
            src: Operand::Value(Value(1)),
        });
        let inst = Instruction::BinOp {
            dest: Value(3),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Value(Value(2)),
            ty: IrType::F32,
        };
        let result = try_simplify_neg(&inst, &neg_defs).unwrap();
        assert!(matches!(
            result,
            Instruction::BinOp {
                op: IrBinOp::Sub,
                ..
            }
        ));
    }

    #[test]
    fn float_add_of_multi_use_neg_folds_too() {
        // The MULTI-USE adjudication pin: every oracle folds the add even
        // when the negation has other readers (gcc: `(x + t) + (x * t)`
        // with t = -y emits vsubsd + vfnmadd132sd — the add folds, the
        // product's negation rides the family). NOT folding here is what
        // would diverge from the competition on NaN-sign rows.
        let mut neg_defs: Vec<Option<NegDef>> = vec![None; 4];
        neg_defs[2] = Some(NegDef {
            src: Operand::Value(Value(1)),
        });
        let inst = Instruction::BinOp {
            dest: Value(3),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Value(Value(2)),
            ty: IrType::F64,
        };
        let result = try_simplify_neg(&inst, &neg_defs).unwrap();
        assert!(matches!(
            result,
            Instruction::BinOp {
                op: IrBinOp::Sub,
                ..
            }
        ));
    }

    #[test]
    fn int_sub_of_neg_still_folds() {
        // Historical behaviour pin: the INTEGER fold is exact (ring
        // identity) and keeps firing exactly as before.
        let mut neg_defs: Vec<Option<NegDef>> = vec![None; 4];
        neg_defs[2] = Some(NegDef {
            src: Operand::Value(Value(1)),
        });
        let inst = Instruction::BinOp {
            dest: Value(3),
            op: IrBinOp::Sub,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Value(Value(2)),
            ty: IrType::I32,
        };
        let result = try_simplify_neg(&inst, &neg_defs).unwrap();
        assert!(matches!(
            result,
            Instruction::BinOp {
                op: IrBinOp::Add,
                ..
            }
        ));
    }

    #[test]
    fn float_add_of_non_neg_stays() {
        // Discriminating control: no Neg def, no fold.
        let inst = Instruction::BinOp {
            dest: Value(3),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Value(Value(2)),
            ty: IrType::F64,
        };
        assert!(try_simplify_neg(&inst, &[]).is_none());
    }

    #[test]
    fn float_add_of_const_stays() {
        // A constant operand has no def to look through.
        let inst = Instruction::BinOp {
            dest: Value(3),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Const(IrConst::F64(1.5)),
            ty: IrType::F64,
        };
        assert!(try_simplify_neg(&inst, &[]).is_none());
    }

    // === Subtract-of-negation tests ===

    #[test]
    fn test_sub_neg_to_add() {
        // x - (neg y) => x + y
        let mut neg_defs: Vec<Option<NegDef>> = vec![None; 4];
        neg_defs[2] = Some(NegDef {
            src: Operand::Value(Value(1)),
        });
        let inst = Instruction::BinOp {
            dest: Value(3),
            op: IrBinOp::Sub,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Value(Value(2)),
            ty: IrType::I32,
        };
        let result = try_simplify(&inst, &[], &[], &[], &[], &neg_defs, &[]).unwrap();
        match result {
            Instruction::BinOp {
                op: IrBinOp::Add,
                lhs: Operand::Value(a),
                rhs: Operand::Value(b),
                ..
            } => {
                assert_eq!(a.0, 0, "lhs should be original lhs");
                assert_eq!(b.0, 1, "rhs should be negation source");
            }
            _ => panic!("Expected Add(V0, V1), got {:?}", result),
        }
    }

    #[test]
    fn test_sub_neg_float_not_simplified() {
        // REWRITTEN (the four-oracle adjudication): float x - (neg y) now
        // DOES fold to x + y. The historical "IEEE 754 concerns" pin was
        // more conservative than every compiler in the competition set —
        // GCC 16.2, Clang 23.1, ICX and local GCC 14.2 all emit the
        // mirrored vaddsd for `x - (-y)` (oracle-verified). The fold is
        // exact for every numeric input and both zeros (IEEE 754 defines
        // sub as the addition of the negation); the qNaN-sign latitude is
        // C11 6.5p8-sanctioned and, with every oracle taking it, NOT
        // folding is what diverges. The dedicated coverage lives in
        // float_sub_of_neg_folds_to_add; this pin keeps the historical
        // name as the record of the flip.
        let mut neg_defs: Vec<Option<NegDef>> = vec![None; 4];
        neg_defs[2] = Some(NegDef {
            src: Operand::Value(Value(1)),
        });
        let inst = Instruction::BinOp {
            dest: Value(3),
            op: IrBinOp::Sub,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Value(Value(2)),
            ty: IrType::F64,
        };
        let result = try_simplify_neg(&inst, &neg_defs).unwrap();
        assert!(matches!(
            result,
            Instruction::BinOp {
                op: IrBinOp::Add,
                ..
            }
        ));
    }

    // === Constant-zero negation tests ===

    #[test]
    fn test_zero_minus_x_folds_to_neg() {
        // `0 - x` is the modular negate; canonicalising it to `Neg` is a ring
        // identity, and it is the difference between `xor;sub` (which has to
        // materialise the zero) and one `negl` on x86 -- and between the
        // `x & -x` lowest-set-bit idiom being recognisable or not.
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
            let inst = Instruction::BinOp {
                dest: Value(3),
                op: IrBinOp::Sub,
                lhs: Operand::Const(IrConst::from_i64(0, ty)),
                rhs: Operand::Value(Value(1)),
                ty,
            };
            match simplify_default(&inst) {
                Some(Instruction::UnaryOp {
                    dest,
                    op: IrUnaryOp::Neg,
                    src: Operand::Value(v),
                    ty: t,
                }) => {
                    assert_eq!(dest.0, 3);
                    assert_eq!(v.0, 1);
                    assert_eq!(t, ty);
                }
                other => panic!("expected Neg for {ty:?}, got {other:?}"),
            }
        }
    }

    #[test]
    fn test_zero_minus_x_float_is_not_negated() {
        // IEEE 754: `0.0 - x` is not `-x` for `x = +0.0` (it is `+0.0`, while
        // `-x` is `-0.0`), and it is not `-x` for NaN payloads either.
        for ty in [IrType::F64, IrType::F32] {
            let (zero, one) = if ty == IrType::F64 {
                (IrConst::F64(0.0), IrConst::F64(1.0))
            } else {
                (IrConst::F32(0.0), IrConst::F32(1.0))
            };
            let inst = Instruction::BinOp {
                dest: Value(3),
                op: IrBinOp::Sub,
                lhs: Operand::Const(zero),
                rhs: Operand::Const(one),
                ty,
            };
            assert!(
                !matches!(
                    simplify_default(&inst),
                    Some(Instruction::UnaryOp {
                        op: IrUnaryOp::Neg,
                        ..
                    })
                ),
                "{ty:?}: float zero minus x must not become a negate"
            );
        }
    }

    #[test]
    fn test_zero_minus_x_keeps_rhs_zero_case() {
        // Order matters: `0 - 0` is the zero test, not a negation.
        let inst = Instruction::BinOp {
            dest: Value(3),
            op: IrBinOp::Sub,
            lhs: Operand::Const(IrConst::I32(0)),
            rhs: Operand::Const(IrConst::I32(0)),
            ty: IrType::I32,
        };
        let result = simplify_default(&inst).expect("0 - 0 must fold");
        assert!(
            matches!(
                result,
                Instruction::Copy {
                    src: Operand::Const(_),
                    ..
                }
            ),
            "expected a constant, got {result:?}"
        );
    }

    // === Truncated negate of an extension ===

    /// `v2 = sext v1 (i32 -> i64); v3 = neg v2; v4 = trunc v3 (i64 -> i32)`,
    /// with `v4` the instruction under test -- the shape the C integer
    /// promotions put around `-(x != 0)` and every other 32-bit negation whose
    /// value is carried at 64 bits in between.
    fn truncated_negate_i64(
        cast_defs: &mut [Option<CastDef>],
        neg_defs: &mut [Option<NegDef>],
    ) -> Instruction {
        cast_defs[2] = Some(CastDef {
            src: Operand::Value(Value(1)),
            from_ty: IrType::I32,
            to_ty: IrType::I64,
        });
        neg_defs[3] = Some(NegDef {
            src: Operand::Value(Value(2)),
        });
        Instruction::Cast {
            dest: Value(4),
            src: Operand::Value(Value(3)),
            from_ty: IrType::I64,
            to_ty: IrType::I32,
        }
    }

    #[test]
    fn test_truncated_negate_of_an_extension_folds_to_the_narrow_negate() {
        // `(int)(-(long)(int)x)` is `-(int)x`: the extension's high bits die in
        // the truncation and negation is a ring operation.  A 64-bit negate is
        // never cheaper than a 32-bit one, so the round trip is pure overhead.
        let mut cast_defs: Vec<Option<CastDef>> = vec![None; 5];
        let mut neg_defs: Vec<Option<NegDef>> = vec![None; 5];
        let inst = truncated_negate_i64(&mut cast_defs, &mut neg_defs);
        let result = try_simplify_with_types(
            &inst,
            &cast_defs,
            &[],
            &[],
            &[],
            &neg_defs,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
        )
        .expect("the truncated negate of an extension must fold");
        assert!(
            matches!(
                result,
                Instruction::UnaryOp {
                    dest: Value(4),
                    op: IrUnaryOp::Neg,
                    src: Operand::Value(Value(1)),
                    ty: IrType::I32,
                }
            ),
            "expected `neg v1 @ i32`, got {result:?}"
        );
    }

    #[test]
    fn test_truncated_negate_control_keeps_the_wide_negate() {
        // Same negation, no truncation: a widening cast must NOT relocate the
        // negate into the narrow width (the high bits are observed there).
        let mut cast_defs: Vec<Option<CastDef>> = vec![None; 5];
        let mut neg_defs: Vec<Option<NegDef>> = vec![None; 5];
        truncated_negate_i64(&mut cast_defs, &mut neg_defs);
        let widen = Instruction::Cast {
            dest: Value(4),
            src: Operand::Value(Value(3)),
            from_ty: IrType::I32,
            to_ty: IrType::I64,
        };
        assert!(
            try_simplify_with_types(
                &widen,
                &cast_defs,
                &[],
                &[],
                &[],
                &neg_defs,
                &[],
                &[],
                &[],
                &[],
                &[],
                &[],
                &[],
            )
            .is_none(),
            "widening the negate must not fold to a narrow negate"
        );

        // A 32-bit source and a 16-bit destination is the shape the fold
        // deliberately leaves alone (measured: it flips a vectorization
        // decision and costs a spill -- see the comment on the fold).
        let mut narrow: Vec<Option<CastDef>> = vec![None; 5];
        narrow[2] = Some(CastDef {
            src: Operand::Value(Value(1)),
            from_ty: IrType::U16,
            to_ty: IrType::I32,
        });
        let inst = Instruction::Cast {
            dest: Value(4),
            src: Operand::Value(Value(3)),
            from_ty: IrType::I32,
            to_ty: IrType::U16,
        };
        assert!(
            try_simplify_with_types(
                &inst,
                &narrow,
                &[],
                &[],
                &[],
                &neg_defs,
                &[],
                &[],
                &[],
                &[],
                &[],
                &[],
                &[],
            )
            .is_none(),
            "the 32 -> 16 shape must stay put"
        );

        // A negate whose operand is neither `to_ty`-wide nor an extension of a
        // `to_ty`-wide value has no `to_ty`-wide form at all.
        let mut unrelated: Vec<Option<CastDef>> = vec![None; 5];
        unrelated[2] = Some(CastDef {
            src: Operand::Value(Value(1)),
            from_ty: IrType::I8,
            to_ty: IrType::I64,
        });
        let inst = Instruction::Cast {
            dest: Value(4),
            src: Operand::Value(Value(3)),
            from_ty: IrType::I64,
            to_ty: IrType::I32,
        };
        assert!(
            try_simplify_with_types(
                &inst,
                &unrelated,
                &[],
                &[],
                &[],
                &neg_defs,
                &[],
                &[],
                &[],
                &[],
                &[],
                &[],
                &[],
            )
            .is_none(),
            "an extension from a different width must stay put"
        );
    }

    // === Truncated complement of an extension ===

    /// `v2 = ext v1 (i32 -> i64); v3 = not v2; v4 = trunc v3 (i64 -> i32)`,
    /// the shape the C promotions put around `~(1 << n)`.
    fn truncated_not_i64(
        cast_defs: &mut [Option<CastDef>],
        not_defs: &mut [Option<Operand>],
    ) -> Instruction {
        cast_defs[2] = Some(CastDef {
            src: Operand::Value(Value(1)),
            from_ty: IrType::I32,
            to_ty: IrType::I64,
        });
        not_defs[3] = Some(Operand::Value(Value(2)));
        Instruction::Cast {
            dest: Value(4),
            src: Operand::Value(Value(3)),
            from_ty: IrType::I64,
            to_ty: IrType::I32,
        }
    }

    #[test]
    fn test_truncated_complement_of_an_extension_folds_to_the_narrow_complement() {
        // `(int)(~(long)(int)x)` is `~(int)x`: `Not` is bitwise, so the low
        // bits of the complement are `~x` whatever the extension did above
        // them.
        let mut cast_defs: Vec<Option<CastDef>> = vec![None; 5];
        let mut not_defs: Vec<Option<Operand>> = vec![None; 5];
        let inst = truncated_not_i64(&mut cast_defs, &mut not_defs);
        let result = try_simplify_with_types(
            &inst,
            &cast_defs,
            &[],
            &[],
            &[],
            &[],
            &[],
            &not_defs,
            &[],
            &[],
            &[],
            &[],
            &[],
        )
        .expect("the truncated complement of an extension must fold");
        assert!(
            matches!(
                result,
                Instruction::UnaryOp {
                    dest: Value(4),
                    op: IrUnaryOp::Not,
                    src: Operand::Value(Value(1)),
                    ty: IrType::I32,
                }
            ),
            "expected `not v1 @ i32`, got {result:?}"
        );
    }

    #[test]
    fn test_truncated_complement_controls() {
        let mut cast_defs: Vec<Option<CastDef>> = vec![None; 5];
        let mut not_defs: Vec<Option<Operand>> = vec![None; 5];
        let _ = truncated_not_i64(&mut cast_defs, &mut not_defs);

        // A WIDENING cast: `ext(~x)` is not `~ext(x)` -- the high bits differ,
        // so the complement cannot be relocated into the narrow width.
        let widen = Instruction::Cast {
            dest: Value(4),
            src: Operand::Value(Value(3)),
            from_ty: IrType::I32,
            to_ty: IrType::I64,
        };
        assert!(
            try_simplify_with_types(
                &widen,
                &cast_defs,
                &[],
                &[],
                &[],
                &[],
                &[],
                &not_defs,
                &[],
                &[],
                &[],
                &[],
                &[],
            )
            .is_none(),
            "a widening cast must keep the wide complement"
        );

        // An extension from a DIFFERENT width: `trunc32(not64(ext64(i8)))` is
        // `trunc32(not64(v))`, not `~(i32)v`, whose high 24 bits would be v's
        // rather than the complement's.
        let mut unrelated: Vec<Option<CastDef>> = vec![None; 5];
        unrelated[2] = Some(CastDef {
            src: Operand::Value(Value(1)),
            from_ty: IrType::I8,
            to_ty: IrType::I64,
        });
        let inst = Instruction::Cast {
            dest: Value(4),
            src: Operand::Value(Value(3)),
            from_ty: IrType::I64,
            to_ty: IrType::I32,
        };
        assert!(
            try_simplify_with_types(
                &inst,
                &unrelated,
                &[],
                &[],
                &[],
                &[],
                &[],
                &not_defs,
                &[],
                &[],
                &[],
                &[],
                &[],
            )
            .is_none(),
            "an extension from a different width must stay put"
        );

        // A `Not` whose operand is already `to_ty`-wide needs no extension to
        // peel: the complement is simply taken at the observed width.
        let mut wide: Vec<Option<IrType>> = vec![None; 5];
        wide[2] = Some(IrType::I32);
        let direct = Instruction::Cast {
            dest: Value(4),
            src: Operand::Value(Value(3)),
            from_ty: IrType::I64,
            to_ty: IrType::I32,
        };
        let mut direct_not: Vec<Option<Operand>> = vec![None; 5];
        direct_not[3] = Some(Operand::Value(Value(2)));
        let result = try_simplify_with_types(
            &direct,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &direct_not,
            &[],
            &wide,
            &[],
            &[],
            &[],
        )
        .expect("a `to_ty`-wide complement folds");
        assert!(
            matches!(
                result,
                Instruction::UnaryOp {
                    dest: Value(4),
                    op: IrUnaryOp::Not,
                    src: Operand::Value(Value(2)),
                    ty: IrType::I32,
                }
            ),
            "expected `not v2 @ i32`, got {result:?}"
        );
    }

    // === Unsigned comparison simplification tests ===

    #[test]
    fn test_cmp_ule_zero() {
        // Cmp(Ule, x, 0) => Cmp(Eq, x, 0)
        let inst = Instruction::Cmp {
            dest: Value(2),
            op: IrCmpOp::Ule,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I32(0)),
            ty: IrType::U32,
        };
        let result = simplify_default(&inst).unwrap();
        match result {
            Instruction::Cmp {
                op: IrCmpOp::Eq,
                lhs: Operand::Value(v),
                rhs: Operand::Const(IrConst::I32(0)),
                ..
            } => {
                assert_eq!(v.0, 1);
            }
            _ => panic!("Expected Cmp(Eq, V1, 0), got {:?}", result),
        }
    }

    #[test]
    fn test_cmp_ugt_zero() {
        // Cmp(Ugt, x, 0) => Cmp(Ne, x, 0)
        let inst = Instruction::Cmp {
            dest: Value(2),
            op: IrCmpOp::Ugt,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I64(0)),
            ty: IrType::U64,
        };
        let result = simplify_default(&inst).unwrap();
        match result {
            Instruction::Cmp {
                op: IrCmpOp::Ne,
                lhs: Operand::Value(v),
                rhs: Operand::Const(IrConst::I64(0)),
                ..
            } => {
                assert_eq!(v.0, 1);
            }
            _ => panic!("Expected Cmp(Ne, V1, 0), got {:?}", result),
        }
    }

    // === Operand canonicalization tests ===

    #[test]
    fn test_cmp_canonicalize_const_lhs_slt() {
        // Cmp(Slt, Const(5), Value(1)) => Cmp(Sgt, Value(1), Const(5))
        let inst = Instruction::Cmp {
            dest: Value(2),
            op: IrCmpOp::Slt,
            lhs: Operand::Const(IrConst::I32(5)),
            rhs: Operand::Value(Value(1)),
            ty: IrType::I32,
        };
        let result = simplify_default(&inst).unwrap();
        match result {
            Instruction::Cmp {
                op: IrCmpOp::Sgt,
                lhs: Operand::Value(v),
                rhs: Operand::Const(IrConst::I32(5)),
                ..
            } => {
                assert_eq!(v.0, 1);
            }
            _ => panic!("Expected Cmp(Sgt, V1, I32(5)), got {:?}", result),
        }
    }

    #[test]
    fn test_cmp_canonicalize_const_lhs_eq() {
        // Cmp(Eq, Const(42), Value(1)) => Cmp(Eq, Value(1), Const(42))
        // Eq is symmetric so swap_cmp_op returns Eq
        let inst = Instruction::Cmp {
            dest: Value(2),
            op: IrCmpOp::Eq,
            lhs: Operand::Const(IrConst::I64(42)),
            rhs: Operand::Value(Value(1)),
            ty: IrType::I64,
        };
        let result = simplify_default(&inst).unwrap();
        match result {
            Instruction::Cmp {
                op: IrCmpOp::Eq,
                lhs: Operand::Value(v),
                rhs: Operand::Const(IrConst::I64(42)),
                ..
            } => {
                assert_eq!(v.0, 1);
            }
            _ => panic!("Expected Cmp(Eq, V1, I64(42)), got {:?}", result),
        }
    }

    #[test]
    fn test_cmp_no_canonicalize_already_canonical() {
        // Cmp(Slt, Value(1), Const(5)) - already canonical, no change
        let inst = Instruction::Cmp {
            dest: Value(2),
            op: IrCmpOp::Slt,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I32(5)),
            ty: IrType::I32,
        };
        assert!(simplify_default(&inst).is_none());
    }

    #[test]
    fn test_binop_canonicalize_commutative_add() {
        // BinOp(Add, Const(10), Value(1)) => BinOp(Add, Value(1), Const(10))
        let inst = Instruction::BinOp {
            dest: Value(2),
            op: IrBinOp::Add,
            lhs: Operand::Const(IrConst::I32(10)),
            rhs: Operand::Value(Value(1)),
            ty: IrType::I32,
        };
        let result = simplify_default(&inst).unwrap();
        match result {
            Instruction::BinOp {
                op: IrBinOp::Add,
                lhs: Operand::Value(v),
                rhs: Operand::Const(IrConst::I32(10)),
                ..
            } => {
                assert_eq!(v.0, 1);
            }
            _ => panic!("Expected BinOp(Add, V1, I32(10)), got {:?}", result),
        }
    }

    #[test]
    fn bit_test_canonicalizes_variable_shift_and_one() {
        let mut binops = vec![None; 8];
        binops[2] = Some(BinOpDef {
            op: IrBinOp::LShr,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Value(Value(3)),
            ty: IrType::I32,
        });
        let got = simplify_binop(
            Value(4),
            IrBinOp::And,
            &Operand::Value(Value(2)),
            &Operand::Const(IrConst::I64(1)),
            IrType::I32,
            &[],
            &binops,
            &[],
            &[],
        )
        .unwrap();
        match got {
            Instruction::BinOp {
                op: IrBinOp::BitTest,
                lhs,
                rhs,
                ty,
                ..
            } => {
                assert!(matches!(lhs, Operand::Value(Value(1))));
                assert!(matches!(rhs, Operand::Value(Value(3))));
                assert_eq!(ty, IrType::I32);
            }
            other => panic!("expected BitTest, got {other:?}"),
        }
    }

    #[test]
    fn bit_test_peels_same_width_integer_cast() {
        let mut binops = vec![None; 8];
        let mut casts = vec![None; 8];
        binops[2] = Some(BinOpDef {
            op: IrBinOp::AShr,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Value(Value(3)),
            ty: IrType::I32,
        });
        casts[2] = Some(CastDef {
            src: Operand::Value(Value(2)),
            from_ty: IrType::I32,
            to_ty: IrType::U32,
        });
        let got = simplify_binop(
            Value(4),
            IrBinOp::And,
            &Operand::Value(Value(2)),
            &Operand::Const(IrConst::I64(1)),
            IrType::U32,
            &casts,
            &binops,
            &[],
            &[],
        )
        .unwrap();
        match got {
            Instruction::BinOp {
                op: IrBinOp::BitTest,
                lhs,
                rhs,
                ty,
                ..
            } => {
                assert!(matches!(lhs, Operand::Value(Value(1))));
                assert!(matches!(rhs, Operand::Value(Value(3))));
                assert_eq!(ty, IrType::I32);
            }
            other => panic!("expected BitTest through cast, got {other:?}"),
        }
    }

    /// `x /s -1` is `-x` and `x %s -1` is 0 at every integer width (the
    /// INT_MIN quotient overflow is undefined); unsigned division by
    /// all-ones is NOT a negation and must stay untouched here, and so must
    /// float division.
    #[test]
    fn signed_div_rem_by_minus_one() {
        for ty in [
            IrType::I8,
            IrType::I16,
            IrType::I32,
            IrType::I64,
            IrType::I128,
        ] {
            let minus_one = if ty.is_128bit() {
                IrConst::I128(-1)
            } else {
                IrConst::from_i64(-1, ty)
            };
            let run = |op| {
                simplify_binop(
                    Value(4),
                    op,
                    &Operand::Value(Value(1)),
                    &Operand::Const(minus_one),
                    ty,
                    &[],
                    &[],
                    &[],
                    &[],
                )
            };
            assert!(
                matches!(
                    run(IrBinOp::SDiv),
                    Some(Instruction::UnaryOp { op: IrUnaryOp::Neg, src: Operand::Value(Value(1)), ty: t, .. }) if t == ty
                ),
                "{ty:?}"
            );
            assert!(
                matches!(run(IrBinOp::SRem), Some(Instruction::Copy { src: Operand::Const(ref z), .. }) if z.is_zero()),
                "{ty:?}"
            );
            assert!(
                !matches!(run(IrBinOp::UDiv), Some(Instruction::UnaryOp { .. })),
                "{ty:?}"
            );
        }
        let fdiv = simplify_binop(
            Value(4),
            IrBinOp::SDiv,
            &Operand::Value(Value(1)),
            &Operand::Const(IrConst::F64(-1.0)),
            IrType::F64,
            &[],
            &[],
            &[],
            &[],
        );
        assert!(
            !matches!(fdiv, Some(Instruction::UnaryOp { .. })),
            "{fdiv:?}"
        );
    }

    /// a shift or a constant; evaluating the rewrite must give the high half
    /// of the 128-bit product for every source, signed and unsigned.  Signed
    /// 2^63 is negative as an I64 operand, so it must NOT become a shift.
    #[test]
    fn mul_high_power_of_two_rewrites_are_exact() {
        let sources = [
            0i64,
            1,
            -1,
            2,
            7,
            i64::MAX,
            i64::MIN,
            0x0123_4567_89AB_CDEF,
            -12345,
        ];
        for (op, ty) in [
            (IrBinOp::UMulHigh, IrType::U64),
            (IrBinOp::SMulHigh, IrType::I64),
        ] {
            let signed = op == IrBinOp::SMulHigh;
            for k in 0..64u32 {
                let c = (1u64 << k) as i64;
                for const_lhs in [false, true] {
                    let (lhs, rhs) = if const_lhs {
                        (Operand::Const(IrConst::I64(c)), Operand::Value(Value(1)))
                    } else {
                        (Operand::Value(Value(1)), Operand::Const(IrConst::I64(c)))
                    };
                    let got = simplify_binop(Value(4), op, &lhs, &rhs, ty, &[], &[], &[], &[]);
                    if signed && k == 63 {
                        assert!(got.is_none(), "SMulHigh by i64::MIN rewritten: {got:?}");
                        continue;
                    }
                    let got = got.unwrap_or_else(|| panic!("{op:?} by 2^{k} not rewritten"));
                    for x in sources {
                        let value = match &got {
                            Instruction::Copy {
                                src: Operand::Const(z),
                                ..
                            } => z.to_i64().unwrap(),
                            Instruction::BinOp {
                                op: shift_op @ (IrBinOp::LShr | IrBinOp::AShr),
                                lhs: Operand::Value(Value(1)),
                                rhs: Operand::Const(n),
                                ty: shift_ty,
                                ..
                            } if *shift_ty == ty => {
                                assert_eq!(*shift_op == IrBinOp::AShr, signed);
                                shift_op.eval_i64(x, n.to_i64().unwrap()).unwrap()
                            }
                            other => panic!("unexpected {op:?} rewrite {other:?}"),
                        };
                        assert_eq!(
                            value,
                            op.eval_i64(x, c).unwrap(),
                            "{op:?}({x:#x}, 2^{k}) via {got:?}"
                        );
                    }
                }
            }
            // Non-powers stay multiplies; a zero operand is zero.
            assert!(
                simplify_binop(
                    Value(4),
                    op,
                    &Operand::Value(Value(1)),
                    &Operand::Const(IrConst::I64(5)),
                    ty,
                    &[],
                    &[],
                    &[],
                    &[],
                )
                .is_none()
            );
            let zero = simplify_binop(
                Value(4),
                op,
                &Operand::Value(Value(1)),
                &Operand::Const(IrConst::zero(ty)),
                ty,
                &[],
                &[],
                &[],
                &[],
            );
            assert!(
                matches!(zero, Some(Instruction::Copy { src: Operand::Const(ref z), .. }) if z.is_zero()),
                "{op:?} by zero: {zero:?}"
            );
        }
    }

    /// A constant `Bzhi` index becomes a copy, a zero or an `and`; evaluating
    /// the rewrite must give the instruction's result (Intel SDM BZHI: index
    /// bits 7:0, the source unchanged from the width on) for every source.
    #[test]
    fn bzhi_constant_index_rewrites_are_exact() {
        fn oracle(x: u64, n: u64, width: u32) -> u64 {
            let k = n & 0xFF;
            if k >= u64::from(width) {
                x
            } else {
                x & ((1u64 << k) - 1)
            }
        }
        let sources = [
            0u64,
            1,
            0x8000_0000,
            0xFFFF_FFFF,
            1 << 63,
            u64::MAX,
            0x0123_4567_89AB_CDEF,
        ];
        let indices = [
            0u64,
            1,
            31,
            32,
            33,
            63,
            64,
            255,
            256,
            257,
            0x1_0000_0020,
            u64::MAX,
            0xFFFF_FFFF_FFFF_FF1F,
        ];
        for ty in [IrType::I32, IrType::U32, IrType::I64, IrType::U64] {
            let width = (ty.size() * 8) as u32;
            for n in indices {
                let index = IrConst::from_i64(n as i64, ty);
                let got = simplify_binop(
                    Value(4),
                    IrBinOp::Bzhi,
                    &Operand::Value(Value(1)),
                    &Operand::Const(index),
                    ty,
                    &[],
                    &[],
                    &[],
                    &[],
                )
                .unwrap_or_else(|| panic!("constant-index Bzhi({n:#x}) at {ty:?} not rewritten"));
                for x in sources {
                    let x = ty.truncate_i64(x as i64);
                    let value = match &got {
                        Instruction::Copy {
                            src: Operand::Value(Value(1)),
                            ..
                        } => x,
                        Instruction::Copy {
                            src: Operand::Const(c),
                            ..
                        } => c.to_i64().unwrap(),
                        Instruction::BinOp {
                            op: IrBinOp::And,
                            lhs: Operand::Value(Value(1)),
                            rhs: Operand::Const(mask),
                            ty: and_ty,
                            ..
                        } if *and_ty == ty => x & mask.to_i64().unwrap(),
                        other => panic!("unexpected Bzhi rewrite {other:?}"),
                    };
                    let want = ty.truncate_i64(oracle(x as u64, n, width) as i64);
                    assert_eq!(
                        ty.truncate_i64(value),
                        want,
                        "Bzhi({x:#x}, {n:#x}) at {ty:?} via {got:?}"
                    );
                }
            }
            let zero = simplify_binop(
                Value(4),
                IrBinOp::Bzhi,
                &Operand::Const(IrConst::zero(ty)),
                &Operand::Value(Value(2)),
                ty,
                &[],
                &[],
                &[],
                &[],
            );
            assert!(
                matches!(zero, Some(Instruction::Copy { src: Operand::Const(c), .. }) if c.to_i64() == Some(0)),
                "Bzhi(0, n) at {ty:?} must fold to zero, got {zero:?}"
            );
            let opaque = simplify_binop(
                Value(4),
                IrBinOp::Bzhi,
                &Operand::Value(Value(1)),
                &Operand::Value(Value(2)),
                ty,
                &[],
                &[],
                &[],
                &[],
            );
            assert!(opaque.is_none(), "opaque Bzhi must stay, got {opaque:?}");
        }
    }

    /// `BitTest(0, index)` is `(0 >> index) & 1` == 0 for every index.
    #[test]
    fn bit_test_zero_base_is_zero() {
        for index in [0i64, 1, 5, 63] {
            let got = simplify_binop(
                Value(4),
                IrBinOp::BitTest,
                &Operand::Const(IrConst::I32(0)),
                &Operand::Const(IrConst::I32(index as i32)),
                IrType::I32,
                &[],
                &[],
                &[],
                &[],
            )
            .unwrap();
            assert!(
                matches!(got, Instruction::Copy { src: Operand::Const(c), .. } if c.to_i64() == Some(0)),
                "BitTest(0, {index}) must fold to zero, got {got:?}"
            );
        }
    }

    /// `BitTest(base, 0)` is `(base >> 0) & 1` == `base & 1` — bit zero of the
    /// base. It is NOT zero: folding it that way miscompiled
    /// `if (((x >> 0) & 1) ^ 1)` into a constant, taking the wrong arm of a
    /// diamond (scripts/gen_slot_stress.py seed 1, -O2).
    #[test]
    fn bit_test_zero_index_is_bit_zero_of_base() {
        let got = simplify_binop(
            Value(4),
            IrBinOp::BitTest,
            &Operand::Value(Value(1)),
            &Operand::Const(IrConst::I32(0)),
            IrType::I32,
            &[],
            &[],
            &[],
            &[],
        );
        assert!(
            got.is_none(),
            "BitTest(base, 0) must not be folded away: it is base & 1, got {got:?}"
        );
    }

    /// BitTest(0, 0) == 0: the zero base wins over the zero index.
    #[test]
    fn bit_test_zero_base_and_index_is_zero() {
        let got = simplify_binop(
            Value(4),
            IrBinOp::BitTest,
            &Operand::Const(IrConst::I32(0)),
            &Operand::Const(IrConst::I32(0)),
            IrType::I32,
            &[],
            &[],
            &[],
            &[],
        )
        .unwrap();
        assert!(
            matches!(got, Instruction::Copy { src: Operand::Const(c), .. } if c.to_i64() == Some(0))
        );
    }
    #[test]
    fn test_binop_canonicalize_commutative_and() {
        // BinOp(And, Const(0xFF), Value(1)) => BinOp(And, Value(1), Const(0xFF))
        let inst = Instruction::BinOp {
            dest: Value(2),
            op: IrBinOp::And,
            lhs: Operand::Const(IrConst::I64(0xFF)),
            rhs: Operand::Value(Value(1)),
            ty: IrType::I64,
        };
        let result = simplify_default(&inst).unwrap();
        match result {
            Instruction::BinOp {
                op: IrBinOp::And,
                lhs: Operand::Value(v),
                rhs: Operand::Const(IrConst::I64(0xFF)),
                ..
            } => {
                assert_eq!(v.0, 1);
            }
            _ => panic!("Expected BinOp(And, V1, I64(0xFF)), got {:?}", result),
        }
    }

    #[test]
    fn test_binop_no_canonicalize_non_commutative() {
        // BinOp(Sub, Const(10), Value(1)) - Sub is NOT commutative, should NOT swap
        let inst = Instruction::BinOp {
            dest: Value(2),
            op: IrBinOp::Sub,
            lhs: Operand::Const(IrConst::I32(10)),
            rhs: Operand::Value(Value(1)),
            ty: IrType::I32,
        };
        // Sub has its own simplifications (x-0, x-x, reassoc) but NOT canonicalization
        assert!(simplify_default(&inst).is_none());
    }

    #[test]
    fn test_binop_no_canonicalize_already_canonical() {
        // BinOp(Add, Value(1), Const(10)) - already canonical, no change
        let inst = Instruction::BinOp {
            dest: Value(2),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I32(10)),
            ty: IrType::I32,
        };
        assert!(simplify_default(&inst).is_none());
    }

    // ==================================================================
    // const_power_of_two: the predicate behind every `x / 2^k` and
    // `x % 2^k` strength reduction.
    //
    // This predicate is a trap detector.  It used to read the operand
    // through `to_i64()`, which TRUNCATES a 128-bit constant: the value
    // 0xffffffffffffffff_0000000000000001 arrived as `1`, was judged a
    // power of two, and `x * that` compiled to a copy of `x`.  It was
    // rewritten to read the bit pattern at the operation's width, which
    // then made shifts up to 127 reachable -- and that widening exposed
    // an i64 mask construction downstream (see `IrConst::low_mask`).
    //
    // Both halves of that history are pinned below: the truncation trap,
    // the full widened range, and the width masking.
    // ==================================================================

    #[test]
    fn const_power_of_two_rejects_the_to_i64_truncation_trap() {
        // The historical miscompile: low 64 bits are a power of two, the
        // high half is non-zero.  `to_i64()` saw only the low half.
        let trap = IrConst::I128(0xffffffffffffffff_0000000000000001_u128 as i128);
        assert_eq!(
            const_power_of_two(&Operand::Const(trap), IrType::I128),
            None,
            "a constant whose low half is a power of two but whose high half is \
             non-zero is NOT a power of two"
        );
        // The same value as an unsigned type.
        assert_eq!(
            const_power_of_two(&Operand::Const(trap), IrType::U128),
            None
        );
        // Mirrors of the trap: a power of two PLUS a low bit (here 2^64 + 1),
        // and one with the bit one position up (2^64 + 2).  Both have a single
        // set bit in the i64 view and must still be rejected.
        for v in [
            0x0000000000000001_0000000000000001_u128, // 2^64 + 1
            0x0000000000000001_0000000000000002_u128, // 2^64 + 2
            0x8000000000000000_8000000000000000_u128, // 2^127 + 2^63
        ] {
            assert_eq!(
                const_power_of_two(&Operand::Const(IrConst::I128(v as i128)), IrType::U128),
                None,
                "{v:#x} is not a power of two"
            );
        }
        // Sanity: the value that IS 2^64 must be recognised, so the rejections
        // above are about the extra bits and not about the width.
        let p64 = IrConst::I128(0x0000000000000001_0000000000000000_u128 as i128);
        assert_eq!(
            const_power_of_two(&Operand::Const(p64), IrType::U128),
            Some(64)
        );
    }

    #[test]
    fn const_power_of_two_covers_the_full_128_bit_range() {
        for k in 0..=127u32 {
            let v = 1u128 << k;
            assert_eq!(
                const_power_of_two(&Operand::Const(IrConst::I128(v as i128)), IrType::I128),
                Some(k),
                "I128 2^{k}"
            );
            assert_eq!(
                const_power_of_two(&Operand::Const(IrConst::I128(v as i128)), IrType::U128),
                Some(k),
                "U128 2^{k}"
            );
        }
    }

    #[test]
    fn const_power_of_two_at_the_64_bit_boundary() {
        // 2^63 is i64::MIN as a bit pattern, and was unreachable before the
        // predicate stopped demanding a positive i64.  It is the case whose
        // downstream i64 mask overflowed.
        assert_eq!(
            const_power_of_two(&Operand::Const(IrConst::I64(i64::MIN)), IrType::U64),
            Some(63)
        );
        assert_eq!(
            const_power_of_two(&Operand::Const(IrConst::I64(i64::MIN)), IrType::I64),
            Some(63)
        );
        // 2^64 and up only exist in the 128-bit types.
        assert_eq!(
            const_power_of_two(&Operand::Const(IrConst::I64(i64::MIN)), IrType::U128),
            Some(63),
            "a 128-bit operation on an I64 constant sees only its low 64 bits"
        );
        // An all-ones constant is 2^width - 1: never a power of two, at any
        // width.  As a 128-bit value `I64(-1)` is 2^64 - 1, one SHORT of a
        // power of two -- a distinction the old `to_i64()` predicate could not
        // make, since it could not see past bit 63 at all.
        for ty in [IrType::I64, IrType::U64, IrType::I128, IrType::U128] {
            assert_eq!(
                const_power_of_two(&Operand::Const(IrConst::I64(-1)), ty),
                None,
                "all-ones ({ty:?}) is 2^n - 1, not a power of two"
            );
        }
        // ...whereas the neighbouring value, 2^64, is one, at 128 bits.
        assert_eq!(
            const_power_of_two(
                &Operand::Const(IrConst::I128(
                    0x0000000000000001_0000000000000000_u128 as i128
                )),
                IrType::U128
            ),
            Some(64)
        );
    }

    #[test]
    fn const_power_of_two_on_narrow_signed_types() {
        // Signed narrow constants carrying a single set bit.
        assert_eq!(
            const_power_of_two(&Operand::Const(IrConst::I8(0x80u8 as i8)), IrType::I8),
            Some(7)
        );
        assert_eq!(
            const_power_of_two(&Operand::Const(IrConst::I16(0x8000u16 as i16)), IrType::I16),
            Some(15)
        );
        assert_eq!(
            const_power_of_two(
                &Operand::Const(IrConst::I32(0x0001_0000u32 as i32)),
                IrType::I32
            ),
            Some(16)
        );
        assert_eq!(
            const_power_of_two(
                &Operand::Const(IrConst::I32(0x4000_0000u32 as i32)),
                IrType::I32
            ),
            Some(30)
        );
    }

    #[test]
    fn const_power_of_two_masks_bits_above_the_operation_width() {
        // Bits above the width are not part of the value: a 2^40 bit in a
        // 32-bit operation is not a power of two, it is not anything.
        assert_eq!(
            const_power_of_two(&Operand::Const(IrConst::I64(1i64 << 40)), IrType::I32),
            None
        );
        assert_eq!(
            const_power_of_two(&Operand::Const(IrConst::I64(1i64 << 40)), IrType::U32),
            None
        );
        // 2^32 is off-width for a 32-bit type but in range for 64.
        assert_eq!(
            const_power_of_two(&Operand::Const(IrConst::I64(1i64 << 32)), IrType::U32),
            None
        );
        assert_eq!(
            const_power_of_two(&Operand::Const(IrConst::I64(1i64 << 32)), IrType::U64),
            Some(32)
        );
        // A 2^100 bit in a 64-bit operation likewise vanishes.
        assert_eq!(
            const_power_of_two(&Operand::Const(IrConst::I128(1i128 << 100)), IrType::U64),
            None
        );
    }

    #[test]
    fn const_power_of_two_rejects_non_constants_and_zero() {
        assert_eq!(
            const_power_of_two(&Operand::Const(IrConst::I64(0)), IrType::I64),
            None,
            "zero has no power-of-two exponent"
        );
        assert_eq!(
            const_power_of_two(&Operand::Const(IrConst::F64(4.0)), IrType::F64),
            None,
            "a float constant is not an integer power of two for this fold"
        );
        assert_eq!(
            const_power_of_two(&Operand::Value(Value(0)), IrType::I64),
            None,
            "a non-constant must never be classified"
        );
    }

    /// End-to-end through the simplification of an actual `urem`: the mask
    /// that comes out must be the exact `2^k - 1` at the operation's width.
    /// This is the unit-level twin of tests/regression/i128_divrem_pow2.c.
    #[test]
    fn urem_by_pow2_folds_to_the_exact_mask_at_128_bits() {
        for k in [1u32, 31, 32, 63, 64, 65, 100, 126, 127] {
            let d = IrConst::I128((1u128 << k) as i128);
            let inst = binop(
                IrBinOp::URem,
                Operand::Value(Value(0)),
                Operand::Const(d),
                IrType::U128,
            );
            let out = simplify_default(&inst).expect("urem by 2^k must fold");
            match out {
                Instruction::BinOp {
                    op: IrBinOp::And,
                    rhs: Operand::Const(c),
                    ..
                } => {
                    let got = c.to_i128().expect("mask is an integer") as u128;
                    let want = (1u128 << k) - 1;
                    assert_eq!(got, want, "mask for 2^{k} is not exact");
                }
                other => panic!("expected an AND fold for 2^{k}, got {other:?}"),
            }
        }
    }

    // === -fmath-errno sqrt guard ===

    use crate::ir::reexports::{BasicBlock, BlockId};

    fn errno_fn(pre: Vec<Instruction>, callee: &str, arg: Operand) -> IrModule {
        let ty = if callee == "sqrtf" {
            IrType::F32
        } else {
            IrType::F64
        };
        let mut call = make_call(callee, vec![arg], ty);
        if let Instruction::Call { info, .. } = &mut call {
            info.arg_types = vec![ty];
            info.num_fixed_args = 1;
        }
        let mut instructions = pre;
        instructions.push(call);
        let spans = vec![crate::common::source::Span::dummy(); instructions.len()];
        let mut func = IrFunction::new("f".to_string(), ty, vec![], false);
        func.next_value_id = 64;
        func.next_label = 1;
        func.blocks = vec![BasicBlock {
            label: BlockId(0),
            instructions,
            terminator: Terminator::Return(Some(Operand::Value(Value(10)))),
            source_spans: spans,
        }];
        let mut module = IrModule::new();
        module.functions.push(func);
        module
    }

    fn fbin(dest: u32, op: IrBinOp, lhs: Operand, rhs: Operand) -> Instruction {
        Instruction::BinOp {
            dest: Value(dest),
            op,
            lhs,
            rhs,
            ty: IrType::F64,
        }
    }

    fn v(id: u32) -> Operand {
        Operand::Value(Value(id))
    }

    fn library_sqrt_calls(module: &IrModule) -> Vec<CallInfo> {
        module.functions[0]
            .blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .filter_map(|inst| match inst {
                Instruction::Call { func, info } if func.starts_with("sqrt") => Some(info.clone()),
                _ => None,
            })
            .collect()
    }

    /// An operand that may be negative keeps the library call on a cold
    /// slow path: fast instruction + `r != r` (NaN) test + call + phi.
    #[test]
    fn errno_guard_splits_unprovable_sqrt() {
        for callee in ["sqrt", "sqrtf"] {
            let mut module = errno_fn(vec![], callee, v(1));
            guard_errno_sqrt_calls(&mut module);
            let func = &module.functions[0];
            assert_eq!(func.blocks.len(), 3, "{callee}: B, J and the slow block S");
            let calls = library_sqrt_calls(&module);
            assert_eq!(calls.len(), 1, "{callee}");
            assert!(!calls[0].assume_library, "slow call must never be refolded");
            assert!(calls[0].cold, "slow call carries the cold-path hint");
            let all: Vec<&Instruction> = func.blocks.iter().flat_map(|b| &b.instructions).collect();
            assert!(all.iter().any(|i| matches!(
                i,
                Instruction::Intrinsic {
                    op: IntrinsicOp::SqrtF64 | IntrinsicOp::SqrtF32,
                    ..
                }
            )));
            assert!(all.iter().any(|i| matches!(
                i,
                Instruction::Cmp { op: IrCmpOp::Ne, lhs, rhs, .. } if lhs == rhs
            )));
            assert!(all.iter().any(|i| matches!(i, Instruction::Phi { .. })));
            for b in &func.blocks {
                assert_eq!(
                    b.instructions.len(),
                    b.source_spans.len(),
                    "spans stay parallel"
                );
            }
        }
    }

    /// `-fno-math-errno`: no guard; the plain call is left for the O2 fold.
    #[test]
    fn errno_guard_is_inert_without_math_errno() {
        set_math_errno(false);
        let mut module = errno_fn(vec![], "sqrt", v(1));
        guard_errno_sqrt_calls(&mut module);
        assert_eq!(module.functions[0].blocks.len(), 1);
        let calls = library_sqrt_calls(&module);
        assert!(calls.len() == 1 && calls[0].assume_library && !calls[0].cold);
    }

    /// `x*x + y*y` (and `fabs`, unsigned conversions, widening) can never be
    /// `< 0`: the call becomes the bare instruction with no CFG change.
    #[test]
    fn errno_guard_elides_provably_nonnegative_operands() {
        let sum_of_squares = vec![
            fbin(2, IrBinOp::Mul, v(1), v(1)),
            fbin(3, IrBinOp::Mul, v(4), v(4)),
            fbin(5, IrBinOp::Add, v(2), v(3)),
            fbin(6, IrBinOp::Mul, v(5), Operand::Const(IrConst::F64(0.5))),
        ];
        let unsigned = vec![Instruction::Cast {
            dest: Value(6),
            src: v(1),
            from_ty: IrType::U32,
            to_ty: IrType::F64,
        }];
        for (pre, arg) in [(sum_of_squares, v(6)), (unsigned, v(6))] {
            let mut module = errno_fn(pre, "sqrt", arg);
            guard_errno_sqrt_calls(&mut module);
            assert_eq!(module.functions[0].blocks.len(), 1);
            assert!(library_sqrt_calls(&module).is_empty());
            assert!(matches!(
                module.functions[0].blocks[0].instructions.last(),
                Some(Instruction::Intrinsic {
                    op: IntrinsicOp::SqrtF64,
                    ..
                })
            ));
        }
    }

    /// Division (`1 / -0 = -inf`), subtraction, a negative constant factor
    /// and a value-cycle phi are not proofs: the guard stays.
    #[test]
    fn errno_guard_keeps_unproven_operands() {
        let cases = vec![
            vec![
                fbin(2, IrBinOp::Mul, v(1), v(1)),
                fbin(6, IrBinOp::SDiv, v(2), v(2)),
            ],
            vec![
                fbin(2, IrBinOp::Mul, v(1), v(1)),
                fbin(6, IrBinOp::Sub, v(2), v(2)),
            ],
            vec![fbin(
                6,
                IrBinOp::Mul,
                v(1),
                Operand::Const(IrConst::F64(-1.0)),
            )],
            vec![Instruction::Phi {
                dest: Value(6),
                ty: IrType::F64,
                incoming: vec![
                    (v(6), BlockId(0)),
                    (Operand::Const(IrConst::F64(1.0)), BlockId(0)),
                ],
            }],
        ];
        for pre in cases {
            let mut module = errno_fn(pre, "sqrt", v(6));
            guard_errno_sqrt_calls(&mut module);
            assert_eq!(module.functions[0].blocks.len(), 3);
            assert_eq!(library_sqrt_calls(&module).len(), 1);
        }
    }
}

#[cfg(test)]
mod safe_clz_review_tests {
    use super::*;
    #[test]
    fn safe_clz_guards_require_matching_width_and_zero_equivalent_input() {
        let v = |n| Operand::Value(Value(n));
        for ty in [IrType::U32, IrType::I32, IrType::U64, IrType::I64] {
            let width = (ty.size() * 8) as i64;
            let mut casts = vec![None; 8];
            let mut unary = vec![None; 8];
            let mut bins = vec![None; 8];
            bins[2] = Some(BinOpDef {
                op: IrBinOp::Or,
                lhs: v(1),
                rhs: Operand::Const(IrConst::I64(1)),
                ty,
            });
            unary[3] = Some(UnaryDef {
                op: IrUnaryOp::ClzNonZero,
                src: v(2),
                ty,
            });
            let call = |cond, zero, out, casts: &[_], unary: &[_], bins: &[_]| {
                safe_clz_guard_replacement(
                    Value(7),
                    cond,
                    v(3),
                    Operand::Const(IrConst::I64(zero)),
                    out,
                    casts,
                    unary,
                    bins,
                )
            };
            assert!(
                matches!(call(v(1),width,ty,&casts,&unary,&bins),Some(Instruction::UnaryOp {op:IrUnaryOp::Clz,src,..}) if src==v(1))
            );
            assert!(
                matches!(call(v(1),width-1,ty,&casts,&unary,&bins),Some(Instruction::Copy {src,..}) if src==v(3))
            );
            assert!(call(v(1), width - 2, ty, &casts, &unary, &bins).is_none());
            assert!(call(v(6), width, ty, &casts, &unary, &bins).is_none());
            casts[4] = Some(CastDef {
                src: v(1),
                from_ty: ty,
                to_ty: IrType::U8,
            });
            assert!(call(v(4), width, ty, &casts, &unary, &bins).is_none());
            casts[5] = Some(CastDef {
                src: v(4),
                from_ty: IrType::U8,
                to_ty: ty,
            });
            assert!(call(v(5), width, ty, &casts, &unary, &bins).is_none());
            let other = if ty.size() == 4 {
                IrType::U64
            } else {
                IrType::U32
            };
            assert!(call(v(1), width, other, &casts, &unary, &bins).is_none());
            bins[2].as_mut().unwrap().ty = other;
            assert!(call(v(1), width, ty, &casts, &unary, &bins).is_none());
            unary[3].as_mut().unwrap().src = v(1); // bare NonZero has no safe-zero proof
            assert!(call(v(1), width, ty, &casts, &unary, &bins).is_none());
        }
    }
    #[test]
    fn safe_clz_result_cast_matrix_preserves_copy_and_count_width() {
        let v = |n| Operand::Value(Value(n));
        let mut cases = 0;
        for carrier in [IrType::I32, IrType::U32, IrType::I64, IrType::U64] {
            let width = (carrier.size() * 8) as i64;
            let same_size = if carrier == IrType::I32 {
                IrType::U32
            } else if carrier == IrType::U32 {
                IrType::I32
            } else if carrier == IrType::I64 {
                IrType::U64
            } else {
                IrType::I64
            };
            let wide = if width == 32 {
                IrType::I64
            } else {
                IrType::I128
            };
            for result_type in [
                None,
                Some(same_size),
                Some(IrType::U16),
                Some(wide),
                Some(IrType::I8),
                Some(IrType::U8),
            ] {
                for fallback in [width - 1, width, width - 2] {
                    let mut casts = vec![None; 8];
                    let mut unaries = vec![None; 8];
                    let mut binaries = vec![None; 8];
                    binaries[2] = Some(BinOpDef {
                        op: IrBinOp::Or,
                        lhs: v(1),
                        rhs: Operand::Const(IrConst::I32(1)),
                        ty: carrier,
                    });
                    unaries[3] = Some(UnaryDef {
                        op: IrUnaryOp::ClzNonZero,
                        src: v(2),
                        ty: carrier,
                    });
                    let result = if let Some(to_ty) = result_type {
                        casts[4] = Some(CastDef {
                            src: v(3),
                            from_ty: carrier,
                            to_ty,
                        });
                        v(4)
                    } else {
                        v(3)
                    };
                    let select_ty = result_type.unwrap_or(carrier);
                    let got = safe_clz_guard_replacement(
                        Value(7),
                        v(1),
                        result,
                        Operand::Const(IrConst::I64(fallback)),
                        select_ty,
                        &casts,
                        &unaries,
                        &binaries,
                    );
                    if fallback == width - 1 {
                        assert!(
                            matches!(got, Some(Instruction::Copy { dest: Value(7), src }) if src == result),
                            "Copy must retain the result cast: {carrier:?}, {result_type:?}, {got:?}"
                        );
                    } else if fallback == width && select_ty.size() == carrier.size() {
                        assert!(
                            matches!(got, Some(Instruction::UnaryOp { op: IrUnaryOp::Clz, src, ty, .. })
                            if src == v(1) && ty == select_ty)
                        );
                    } else {
                        assert!(got.is_none(), "must not change count width: {got:?}");
                    }
                    cases += 1;
                }
            }
        }
        // Requested 48-case matrix plus 24 signed/unsigned byte-result cases.
        assert_eq!(cases, 72);
    }
}

#[cfg(test)]
mod exact_product_half_tests {
    use super::*;

    #[test]
    fn even_product_bias_proof_checks_every_operation_and_width() {
        let v = |n| Operand::Value(Value(n));
        let c = |n| Operand::Const(IrConst::I64(n));
        for ty in [IrType::I32, IrType::U32, IrType::I64, IrType::U64] {
            for pair_op in [IrBinOp::Add, IrBinOp::Sub, IrBinOp::Xor] {
                for odd in [-3, -1, 1, 3] {
                    let w = (ty.size() * 8) as i64;
                    let mut bins = vec![None; 7];
                    for (id, op, lhs, rhs) in [
                        (1, pair_op, v(0), c(odd)),
                        (2, IrBinOp::Mul, v(0), v(1)),
                        (3, IrBinOp::AShr, v(2), c(w - 1)),
                        (4, IrBinOp::LShr, v(3), c(w - 1)),
                        (5, IrBinOp::Add, v(2), v(4)),
                    ] {
                        bins[id] = Some(BinOpDef { op, lhs, rhs, ty });
                    }
                    assert_eq!(unbiased_even_product_half(v(5), ty, &[], &bins), Some(v(2)));
                    for id in 1..=5 {
                        let mut bad = bins.clone();
                        bad[id].as_mut().unwrap().ty = IrType::U16;
                        assert!(
                            unbiased_even_product_half(v(5), ty, &[], &bad).is_none(),
                            "node {id}"
                        );
                    }
                    bins[1].as_mut().unwrap().rhs = c(2);
                    assert!(unbiased_even_product_half(v(5), ty, &[], &bins).is_none());
                    bins[1].as_mut().unwrap().rhs = c(odd);
                    bins[3].as_mut().unwrap().lhs = v(0);
                    assert!(unbiased_even_product_half(v(5), ty, &[], &bins).is_none());
                }
            }
        }
    }

    #[test]
    fn exact_half_identity_includes_negative_wrapped_products() {
        let mut state = 0xa5a5_0123u64;
        for i in 0..10000 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let x = if i == 0 { i64::MIN } else { state as i64 };
            for delta in [-3i64, -1, 1, 3] {
                let p = x.wrapping_mul(x.wrapping_add(delta));
                assert_eq!(p / 2, p >> 1);
                let p32 = (x as i32).wrapping_mul((x as i32).wrapping_add(delta as i32));
                assert_eq!(p32 / 2, p32 >> 1);
            }
        }
        assert_ne!(-1i64 / 2, -1i64 >> 1); // odd negative near-miss
    }

    #[test]
    fn three_way_comparator_select_tree_folds_to_single_cmp() {
        use crate::ir::reexports::{BasicBlock, BlockId, IrFunction, Terminator};

        for (cmp_ty, lt_op, gt_op, outer_op, expected_op) in [
            (
                IrType::U64,
                IrCmpOp::Ult,
                IrCmpOp::Ugt,
                IrCmpOp::Sle,
                IrCmpOp::Ule,
            ),
            (
                IrType::U64,
                IrCmpOp::Ult,
                IrCmpOp::Ugt,
                IrCmpOp::Slt,
                IrCmpOp::Ult,
            ),
            (
                IrType::U64,
                IrCmpOp::Ult,
                IrCmpOp::Ugt,
                IrCmpOp::Sgt,
                IrCmpOp::Ugt,
            ),
            (
                IrType::U64,
                IrCmpOp::Ult,
                IrCmpOp::Ugt,
                IrCmpOp::Eq,
                IrCmpOp::Eq,
            ),
            (
                IrType::I64,
                IrCmpOp::Slt,
                IrCmpOp::Sgt,
                IrCmpOp::Sle,
                IrCmpOp::Sle,
            ),
            (
                IrType::I64,
                IrCmpOp::Slt,
                IrCmpOp::Sgt,
                IrCmpOp::Sge,
                IrCmpOp::Sge,
            ),
        ] {
            let mut func = IrFunction::new("cmp3".to_string(), IrType::I32, vec![], false);
            func.blocks.push(BasicBlock {
                label: BlockId(0),
                instructions: vec![
                    Instruction::ParamRef {
                        dest: Value(0),
                        param_idx: 0,
                        ty: cmp_ty,
                    },
                    Instruction::ParamRef {
                        dest: Value(1),
                        param_idx: 1,
                        ty: cmp_ty,
                    },
                    Instruction::Cmp {
                        dest: Value(2),
                        op: lt_op,
                        lhs: Operand::Value(Value(0)),
                        rhs: Operand::Value(Value(1)),
                        ty: cmp_ty,
                    },
                    Instruction::Cmp {
                        dest: Value(3),
                        op: gt_op,
                        lhs: Operand::Value(Value(0)),
                        rhs: Operand::Value(Value(1)),
                        ty: cmp_ty,
                    },
                    Instruction::Select {
                        dest: Value(4),
                        cond: Operand::Value(Value(3)),
                        true_val: Operand::Const(IrConst::I32(1)),
                        false_val: Operand::Const(IrConst::I32(0)),
                        ty: IrType::I32,
                    },
                    Instruction::Select {
                        dest: Value(5),
                        cond: Operand::Value(Value(2)),
                        true_val: Operand::Const(IrConst::I32(-1)),
                        false_val: Operand::Value(Value(4)),
                        ty: IrType::I32,
                    },
                    Instruction::Cmp {
                        dest: Value(6),
                        op: outer_op,
                        lhs: Operand::Value(Value(5)),
                        rhs: Operand::Const(IrConst::I32(0)),
                        ty: IrType::I32,
                    },
                ],
                terminator: Terminator::Return(Some(Operand::Value(Value(6)))),
                source_spans: Vec::new(),
            });
            func.next_value_id = 7;

            assert!(simplify_function(&mut func) > 0);
            let last = func.blocks[0].instructions.last().unwrap();
            match last {
                Instruction::Cmp {
                    dest: Value(6),
                    op,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Value(Value(1)),
                    ty,
                } => {
                    assert_eq!(*op, expected_op);
                    assert_eq!(*ty, cmp_ty);
                }
                Instruction::Copy {
                    dest: Value(6),
                    src: Operand::Value(src),
                } => {
                    let def = func.blocks[0]
                        .instructions
                        .iter()
                        .find(|i| i.dest() == Some(*src))
                        .unwrap();
                    assert!(matches!(
                        def,
                        Instruction::Cmp {
                            op,
                            lhs: Operand::Value(Value(0)),
                            rhs: Operand::Value(Value(1)),
                            ty,
                            ..
                        } if *op == expected_op && *ty == cmp_ty
                    ));
                }
                other => panic!("unexpected folded instruction: {other:?}"),
            }
        }
    }

    /// `fn(a: i32) -> (a < 0 ? -a : a) <op> k`, the frontend's `abs(a)`
    /// shape with a constant comparison against it.
    fn iabs_cmp_func(op: IrCmpOp, k: i32, wrap: bool) -> IrFunction {
        use crate::ir::reexports::{BasicBlock, BlockId, IrFunction, Terminator};
        let mut func = IrFunction::new("iabs_cmp".to_string(), IrType::I32, vec![], false);
        func.pass_policy.wrap_signed_overflow = wrap;
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::ParamRef {
                    dest: Value(0),
                    param_idx: 0,
                    ty: IrType::I32,
                },
                Instruction::Cmp {
                    dest: Value(1),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I32(0)),
                    ty: IrType::I32,
                },
                Instruction::UnaryOp {
                    dest: Value(2),
                    op: IrUnaryOp::Neg,
                    src: Operand::Value(Value(0)),
                    ty: IrType::I32,
                },
                Instruction::Select {
                    dest: Value(3),
                    cond: Operand::Value(Value(1)),
                    true_val: Operand::Value(Value(2)),
                    false_val: Operand::Value(Value(0)),
                    ty: IrType::I32,
                },
                Instruction::Cmp {
                    dest: Value(4),
                    op,
                    lhs: Operand::Value(Value(3)),
                    rhs: Operand::Const(IrConst::I32(k)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(4)))),
            source_spans: Vec::new(),
        });
        func.next_value_id = 5;
        func
    }

    #[test]
    fn iabs_cmp_nonpositive_bound_folds() {
        // (outer op, k, expected): expected is either a `a == 0` / `a != 0`
        // comparison or a constant boolean.
        enum Expect {
            CmpZero(IrCmpOp),
            Const(bool),
        }
        for (op, k, expect) in [
            (IrCmpOp::Sle, 0, Expect::CmpZero(IrCmpOp::Eq)),
            (IrCmpOp::Eq, 0, Expect::CmpZero(IrCmpOp::Eq)),
            (IrCmpOp::Sgt, 0, Expect::CmpZero(IrCmpOp::Ne)),
            (IrCmpOp::Ne, 0, Expect::CmpZero(IrCmpOp::Ne)),
            (IrCmpOp::Slt, 0, Expect::Const(false)),
            (IrCmpOp::Sge, 0, Expect::Const(true)),
            (IrCmpOp::Sle, -3, Expect::Const(false)),
            (IrCmpOp::Sgt, -3, Expect::Const(true)),
            (IrCmpOp::Eq, -1, Expect::Const(false)),
            (IrCmpOp::Ne, -1, Expect::Const(true)),
        ] {
            let mut func = iabs_cmp_func(op, k, false);
            assert!(simplify_function(&mut func) > 0, "{op:?} {k}");
            match (func.blocks[0].instructions.last().unwrap(), &expect) {
                (
                    Instruction::Cmp {
                        dest: Value(4),
                        op: got,
                        lhs: Operand::Value(Value(0)),
                        rhs: Operand::Const(c),
                        ty,
                    },
                    Expect::CmpZero(want),
                ) => {
                    assert_eq!(*got, *want, "{op:?} {k}");
                    assert_eq!(*ty, IrType::I32);
                    assert!(c.is_zero());
                }
                (
                    Instruction::Copy {
                        dest: Value(4),
                        src: Operand::Const(c),
                    },
                    Expect::Const(want),
                ) => {
                    assert_eq!(c.to_i64(), Some(i64::from(*want)), "{op:?} {k}");
                }
                (other, _) => panic!("unexpected fold for {op:?} {k}: {other:?}"),
            }
        }
    }

    #[test]
    fn iabs_cmp_positive_bound_rewrites_to_unsigned_range_test() {
        // (outer op, k, addend, bound, unsigned op)
        for (op, k, addend, bound, uop) in [
            (IrCmpOp::Slt, 5, 4i64, 8i64, IrCmpOp::Ule),
            (IrCmpOp::Sle, 3, 3, 6, IrCmpOp::Ule),
            (IrCmpOp::Sgt, 3, 3, 6, IrCmpOp::Ugt),
            (IrCmpOp::Sge, 5, 4, 8, IrCmpOp::Ugt),
            (IrCmpOp::Slt, 1, 0, 0, IrCmpOp::Ule),
        ] {
            let mut func = iabs_cmp_func(op, k, false);
            assert!(simplify_function(&mut func) > 0, "{op:?} {k}");
            let insts = &func.blocks[0].instructions;
            assert_eq!(insts.len(), 6, "{op:?} {k}");
            match &insts[4] {
                Instruction::BinOp {
                    dest: Value(5),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(c),
                    ty,
                } => {
                    assert_eq!(c.to_i64(), Some(addend), "{op:?} {k}");
                    assert_eq!(*ty, IrType::I32);
                }
                other => panic!("unexpected bias add for {op:?} {k}: {other:?}"),
            }
            match &insts[5] {
                Instruction::Cmp {
                    dest: Value(4),
                    op: got,
                    lhs: Operand::Value(Value(5)),
                    rhs: Operand::Const(c),
                    ty,
                } => {
                    assert_eq!(*got, uop, "{op:?} {k}");
                    assert_eq!(c.to_i64(), Some(bound), "{op:?} {k}");
                    assert_eq!(*ty, IrType::I32);
                }
                other => panic!("unexpected range cmp for {op:?} {k}: {other:?}"),
            }
        }
    }

    #[test]
    fn iabs_cmp_folds_suppressed_under_wrapv() {
        // With wrapping signed overflow, abs(INT_MIN) == INT_MIN is
        // negative, so both fold families must leave the select in place.
        for (op, k) in [(IrCmpOp::Sle, 0), (IrCmpOp::Slt, 5), (IrCmpOp::Sge, 0)] {
            let mut func = iabs_cmp_func(op, k, true);
            simplify_function(&mut func);
            let insts = &func.blocks[0].instructions;
            assert_eq!(insts.len(), 5, "{op:?} {k}");
            assert!(
                matches!(
                    insts.last().unwrap(),
                    Instruction::Cmp {
                        dest: Value(4),
                        op: got,
                        lhs: Operand::Value(Value(3)),
                        ..
                    } if *got == op
                ),
                "{op:?} {k}: {:?}",
                insts.last().unwrap()
            );
        }
    }

    #[test]
    fn iabs_recognition_requires_the_abs_shape() {
        use crate::ir::reexports::{BasicBlock, BlockId, IrFunction, Terminator};
        // `a < 0 ? -b : a` with `b` a different value: not an abs, no fold.
        let mut func = IrFunction::new("not_abs".to_string(), IrType::I32, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::ParamRef {
                    dest: Value(0),
                    param_idx: 0,
                    ty: IrType::I32,
                },
                Instruction::ParamRef {
                    dest: Value(1),
                    param_idx: 1,
                    ty: IrType::I32,
                },
                Instruction::Cmp {
                    dest: Value(2),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I32(0)),
                    ty: IrType::I32,
                },
                Instruction::UnaryOp {
                    dest: Value(3),
                    op: IrUnaryOp::Neg,
                    src: Operand::Value(Value(1)),
                    ty: IrType::I32,
                },
                Instruction::Select {
                    dest: Value(4),
                    cond: Operand::Value(Value(2)),
                    true_val: Operand::Value(Value(3)),
                    false_val: Operand::Value(Value(0)),
                    ty: IrType::I32,
                },
                Instruction::Cmp {
                    dest: Value(5),
                    op: IrCmpOp::Sle,
                    lhs: Operand::Value(Value(4)),
                    rhs: Operand::Const(IrConst::I32(0)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(5)))),
            source_spans: Vec::new(),
        });
        func.next_value_id = 6;
        simplify_function(&mut func);
        assert!(matches!(
            func.blocks[0].instructions.last().unwrap(),
            Instruction::Cmp {
                op: IrCmpOp::Sle,
                lhs: Operand::Value(Value(4)),
                ..
            }
        ));
    }
}

#[cfg(test)]
mod highbit_tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, Terminator};

    /// `ret K - (int)count(x)` with `count = op(x)` at `cty`.
    fn highbit_fn(op: IrUnaryOp, cty: IrType, k: i64) -> IrFunction {
        let mut f = IrFunction::new("f".into(), IrType::I32, vec![], false);
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::UnaryOp {
                    dest: Value(1),
                    op,
                    src: Operand::Value(Value(0)),
                    ty: cty,
                },
                Instruction::Cast {
                    dest: Value(2),
                    src: Operand::Value(Value(1)),
                    from_ty: cty,
                    to_ty: IrType::I32,
                },
                Instruction::BinOp {
                    dest: Value(3),
                    op: IrBinOp::Sub,
                    lhs: Operand::Const(IrConst::I32(k as i32)),
                    rhs: Operand::Value(Value(2)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(3)))),
            source_spans: Vec::new(),
        });
        f.next_value_id = 4;
        f
    }

    fn last_op(f: &IrFunction) -> Option<(IrBinOp, Operand, Operand)> {
        match f.blocks[0].instructions.last() {
            Some(Instruction::BinOp { op, lhs, rhs, .. }) => Some((*op, *lhs, *rhs)),
            _ => None,
        }
    }

    #[test]
    fn highest_set_bit_subtraction_becomes_xor() {
        for (cty, k) in [(IrType::U32, 31), (IrType::U64, 63), (IrType::I32, 31)] {
            let mut f = highbit_fn(IrUnaryOp::ClzNonZero, cty, k);
            assert_eq!(remove_redundant_bitop_guards(&mut f), 1, "{cty:?}");
            assert_eq!(
                last_op(&f),
                Some((
                    IrBinOp::Xor,
                    Operand::Value(Value(2)),
                    Operand::Const(IrConst::I32(k as i32))
                ))
            );
            // Exhaustive over the count's range: K - c == K ^ c.
            for c in 0..=k {
                assert_eq!(k - c, k ^ c);
            }
        }
    }

    #[test]
    fn highbit_rewrite_requires_nonzero_clz_and_its_exact_width() {
        // Defined-zero Clz reaches W: W-1 - W = -1 but (W-1) ^ W = 2W-1.
        let mut f = highbit_fn(IrUnaryOp::Clz, IrType::U32, 31);
        assert_eq!(remove_redundant_bitop_guards(&mut f), 0);
        // 63 - clz32 equals 63 ^ c as well (any all-ones K covering the
        // count's bits does), but only K == W-1 cancels against BSR's own
        // XOR, so the rule stays on the idiom; 32 is not all ones.
        let mut f = highbit_fn(IrUnaryOp::ClzNonZero, IrType::U32, 63);
        assert_eq!(remove_redundant_bitop_guards(&mut f), 0);
        let mut f = highbit_fn(IrUnaryOp::ClzNonZero, IrType::U32, 32);
        assert_eq!(remove_redundant_bitop_guards(&mut f), 0);
        let mut f = highbit_fn(IrUnaryOp::CtzNonZero, IrType::U32, 31);
        assert_eq!(remove_redundant_bitop_guards(&mut f), 0);
        assert_eq!(last_op(&f).map(|t| t.0), Some(IrBinOp::Sub));
    }

    /// `ret x ? count_nz(x) : zero_case`, optionally with a second reader.
    fn guard_fn(op: IrUnaryOp, zero_case: i64, extra_reader: bool) -> IrFunction {
        let mut f = IrFunction::new("g".into(), IrType::U32, vec![], false);
        let mut instructions = vec![
            Instruction::UnaryOp {
                dest: Value(1),
                op,
                src: Operand::Value(Value(0)),
                ty: IrType::U32,
            },
            Instruction::Select {
                dest: Value(2),
                cond: Operand::Value(Value(0)),
                true_val: Operand::Value(Value(1)),
                false_val: Operand::Const(IrConst::I32(zero_case as i32)),
                ty: IrType::U32,
            },
        ];
        if extra_reader {
            instructions.push(Instruction::BinOp {
                dest: Value(3),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Value(Value(2)),
                ty: IrType::U32,
            });
        }
        let ret = if extra_reader { 3 } else { 2 };
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions,
            terminator: Terminator::Return(Some(Operand::Value(Value(ret)))),
            source_spans: Vec::new(),
        });
        f.next_value_id = 4;
        f
    }

    fn count_op(f: &IrFunction) -> IrUnaryOp {
        match &f.blocks[0].instructions[0] {
            Instruction::UnaryOp { op, .. } => *op,
            other => panic!("{other:?}"),
        }
    }

    /// `x ? __builtin_ctz(x) : 32` (and `_tzcnt_u32`): the NonZero count is
    /// refined to the defined one and the guard collapses onto it.
    #[test]
    fn sole_reader_guard_refines_the_nonzero_count() {
        for (nz, defined) in [
            (IrUnaryOp::CtzNonZero, IrUnaryOp::Ctz),
            (IrUnaryOp::ClzNonZero, IrUnaryOp::Clz),
        ] {
            let mut f = guard_fn(nz, 32, false);
            assert_eq!(remove_redundant_bitop_guards(&mut f), 1);
            assert_eq!(count_op(&f), defined);
            assert!(matches!(
                f.blocks[0].instructions[1],
                Instruction::Copy {
                    src: Operand::Value(Value(1)),
                    ..
                }
            ));
        }
    }

    #[test]
    fn guard_is_kept_with_another_reader_or_a_wrong_zero_case() {
        let mut f = guard_fn(IrUnaryOp::CtzNonZero, 32, true);
        assert_eq!(remove_redundant_bitop_guards(&mut f), 0);
        assert_eq!(count_op(&f), IrUnaryOp::CtzNonZero);
        assert!(matches!(
            f.blocks[0].instructions[1],
            Instruction::Select { .. }
        ));
        let mut f = guard_fn(IrUnaryOp::CtzNonZero, 31, false);
        assert_eq!(remove_redundant_bitop_guards(&mut f), 0);
        assert_eq!(count_op(&f), IrUnaryOp::CtzNonZero);
    }
}

#[cfg(test)]
mod pack_extract_tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, Terminator};

    /// The two-register carrier the call-site unpack meets after inlining:
    /// `v5 = or(shl(ext_hi(v1), 64), ext_lo(v0))` (operands swapped when
    /// `swap`), then `v6 = trunc_T(v5)` and `v8 = trunc_T(shr(v5, 64))`.
    fn carrier(lo_ty: IrType, hi_ty: IrType, t: IrType, shr: IrBinOp, swap: bool) -> IrFunction {
        carrier_in(IrType::I128, 64, lo_ty, hi_ty, t, shr, swap)
    }

    /// `carrier` in a `wide`-typed pack with shift `sh`.
    fn carrier_in(
        wide: IrType,
        sh: i64,
        lo_ty: IrType,
        hi_ty: IrType,
        t: IrType,
        shr: IrBinOp,
        swap: bool,
    ) -> IrFunction {
        let mut f = IrFunction::new("f".into(), t, vec![], false);
        let (or_l, or_r) = if swap {
            (Operand::Value(Value(3)), Operand::Value(Value(4)))
        } else {
            (Operand::Value(Value(4)), Operand::Value(Value(3)))
        };
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Cast {
                    dest: Value(2),
                    src: Operand::Value(Value(1)),
                    from_ty: hi_ty,
                    to_ty: wide,
                },
                Instruction::Cast {
                    dest: Value(3),
                    src: Operand::Value(Value(0)),
                    from_ty: lo_ty,
                    to_ty: wide,
                },
                Instruction::BinOp {
                    dest: Value(4),
                    op: IrBinOp::Shl,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Const(IrConst::I64(sh)),
                    ty: wide,
                },
                Instruction::BinOp {
                    dest: Value(5),
                    op: IrBinOp::Or,
                    lhs: or_l,
                    rhs: or_r,
                    ty: wide,
                },
                Instruction::Cast {
                    dest: Value(6),
                    src: Operand::Value(Value(5)),
                    from_ty: wide,
                    to_ty: t,
                },
                Instruction::BinOp {
                    dest: Value(7),
                    op: shr,
                    lhs: Operand::Value(Value(5)),
                    rhs: Operand::Const(IrConst::I64(sh)),
                    ty: wide,
                },
                Instruction::Cast {
                    dest: Value(8),
                    src: Operand::Value(Value(7)),
                    from_ty: wide,
                    to_ty: t,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(8)))),
            source_spans: Vec::new(),
        });
        f.next_value_id = 9;
        f
    }

    /// The narrow (source, from_ty) the extract at `idx` now reads, if it was
    /// folded off the carrier.
    fn reads(f: &IrFunction, idx: usize) -> Option<(Operand, Option<IrType>)> {
        match &f.blocks[0].instructions[idx] {
            Instruction::Copy { src, .. } => Some((*src, None)),
            Instruction::Cast { src, from_ty, .. }
                if !matches!(from_ty, IrType::I128 | IrType::U128) =>
            {
                Some((*src, Some(*from_ty)))
            }
            _ => None,
        }
    }

    #[test]
    fn both_halves_fold_to_the_packed_scalars() {
        for swap in [false, true] {
            for shr in [IrBinOp::LShr, IrBinOp::AShr] {
                let mut f = carrier(IrType::U64, IrType::U64, IrType::I64, shr, swap);
                simplify_function(&mut f);
                assert_eq!(
                    reads(&f, 4),
                    Some((Operand::Value(Value(0)), Some(IrType::U64))),
                    "low half, swap={swap}"
                );
                assert_eq!(
                    reads(&f, 6),
                    Some((Operand::Value(Value(1)), Some(IrType::U64))),
                    "high half, {shr:?} swap={swap}"
                );
            }
        }
    }

    #[test]
    fn zstd_repcodes_shape_folds_with_the_exact_width_high_store() {
        // 12-byte {u32 x3}: the high eightbyte is a U32 member, the unpack
        // extracts it at U32 (exact-width store).
        let mut f = carrier(IrType::U64, IrType::U32, IrType::U32, IrBinOp::LShr, false);
        simplify_function(&mut f);
        assert_eq!(
            reads(&f, 6),
            Some((Operand::Value(Value(1)), None)),
            "U32 -> I128 -> U32 is the member itself"
        );
    }

    #[test]
    fn low_half_folds_for_a_sign_extended_low_but_high_half_does_not() {
        // A sign-extended low part can set bits 64..128, so the shifted
        // value is no longer the high scalar; the low extract is still exact.
        let mut f = carrier(IrType::I64, IrType::U64, IrType::I64, IrBinOp::LShr, false);
        simplify_function(&mut f);
        assert_eq!(
            reads(&f, 4),
            Some((Operand::Value(Value(0)), None)),
            "I64 -> I128 -> I64 low half"
        );
        assert_eq!(reads(&f, 6), None, "high half must keep the shift");
    }

    #[test]
    fn wrong_shift_amount_or_wide_result_is_refused() {
        let mut f = carrier(IrType::U64, IrType::U64, IrType::I64, IrBinOp::LShr, false);
        if let Instruction::BinOp { rhs, .. } = &mut f.blocks[0].instructions[2] {
            *rhs = Operand::Const(IrConst::I64(63));
        }
        simplify_function(&mut f);
        // shl 63 is a pack whose low part is only 63 bits wide: a 64-bit
        // low extract would see the high scalar's bit 0, and `shr 64` does
        // not undo `shl 63`.
        assert_eq!(reads(&f, 4), None, "shl 63 overlaps the low word");
        assert_eq!(reads(&f, 6), None, "shr 64 of a shl-63 pack");

        let mut f = carrier(IrType::U64, IrType::U64, IrType::U128, IrBinOp::LShr, false);
        simplify_function(&mut f);
        assert!(
            matches!(
                f.blocks[0].instructions[6],
                Instruction::Cast {
                    from_ty: IrType::I128,
                    ..
                }
            ),
            "a 128-bit result is not a half"
        );
    }

    #[test]
    fn i128_packs_are_left_alone_under_the_i686_low_pair_model() {
        let saved = crate::common::types::target_ptr_size();
        crate::common::types::set_target_ptr_size(4);
        let mut f = carrier(IrType::U64, IrType::U64, IrType::I64, IrBinOp::LShr, false);
        simplify_function(&mut f);
        crate::common::types::set_target_ptr_size(saved);
        assert_eq!(reads(&f, 4), None);
        assert_eq!(reads(&f, 6), None);
        // 64-bit packs are real on i686 (register pairs) and still fold.
        crate::common::types::set_target_ptr_size(4);
        let mut f = carrier_in(
            IrType::U64,
            32,
            IrType::U32,
            IrType::U32,
            IrType::U32,
            IrBinOp::LShr,
            false,
        );
        simplify_function(&mut f);
        crate::common::types::set_target_ptr_size(saved);
        assert_eq!(reads(&f, 4), Some((Operand::Value(Value(0)), None)));
        assert_eq!(reads(&f, 6), Some((Operand::Value(Value(1)), None)));
    }

    /// The pack aggregate SROA's covering load builds for a `U32` view over
    /// `{i16 c; i8 d; i8 pad}`: signed parts are masked after their sign
    /// extension, parts are shifted to their byte offsets and or-ed.
    #[test]
    fn signed_masked_three_part_pack_yields_each_member() {
        let w = IrType::U32;
        let v = |n| Operand::Value(Value(n));
        let k = |n| Operand::Const(IrConst::from_i64(n, w));
        let bin = |dest, op, lhs, rhs| Instruction::BinOp {
            dest: Value(dest),
            op,
            lhs,
            rhs,
            ty: w,
        };
        let cast = |dest, src, from_ty, to_ty| Instruction::Cast {
            dest: Value(dest),
            src: v(src),
            from_ty,
            to_ty,
        };
        let mut f = IrFunction::new("f".into(), IrType::I8, vec![], false);
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                cast(3, 0, IrType::I16, w),            // sext c
                bin(4, IrBinOp::And, v(3), k(0xFFFF)), // masked
                cast(5, 1, IrType::I8, w),             // sext d
                bin(6, IrBinOp::And, v(5), k(0xFF)),   //
                bin(7, IrBinOp::Shl, v(6), k(16)),     // at byte 2
                bin(8, IrBinOp::Or, v(4), v(7)),       //
                cast(9, 2, IrType::I8, w),             // carrier byte
                bin(10, IrBinOp::And, v(9), k(0xFF)),  //
                bin(11, IrBinOp::Shl, v(10), k(24)),   // at byte 3
                bin(12, IrBinOp::Or, v(8), v(11)),     // the pack
                cast(13, 12, w, IrType::I16),          // c
                bin(14, IrBinOp::LShr, v(12), k(16)),  //
                cast(15, 14, w, IrType::I8),           // d
                bin(16, IrBinOp::AShr, v(12), k(8)),   //
                cast(17, 16, w, IrType::I16),          // bits 8..24
            ],
            terminator: Terminator::Return(Some(v(15))),
            source_spans: Vec::new(),
        });
        f.next_value_id = 18;
        simplify_function(&mut f);
        assert_eq!(reads(&f, 10), Some((v(0), None)), "c = low 16 bits");
        assert_eq!(reads(&f, 12), Some((v(1), None)), "d = bits 16..24");
        // Bits 8..24 straddle c's high byte and d: no single source.
        assert!(
            matches!(
                f.blocks[0].instructions[14],
                Instruction::Cast {
                    src: Operand::Value(Value(16)),
                    ..
                }
            ),
            "straddling extract must keep the shift"
        );
    }

    #[test]
    fn double_narrow_does_not_add_an_i128_reader() {
        // trunc32(trunc64(v: I128)) keeps the I64 step: composing would read
        // the I128 a second time. From an I64 source the chain still folds.
        for (wide, folds) in [(IrType::I128, false), (IrType::I64, true)] {
            let mid = if wide == IrType::I128 {
                IrType::I64
            } else {
                IrType::I32
            };
            let mut f = IrFunction::new("f".into(), IrType::I16, vec![], false);
            f.blocks.push(BasicBlock {
                label: BlockId(0),
                instructions: vec![
                    Instruction::Cast {
                        dest: Value(1),
                        src: Operand::Value(Value(0)),
                        from_ty: wide,
                        to_ty: mid,
                    },
                    Instruction::Cast {
                        dest: Value(2),
                        src: Operand::Value(Value(1)),
                        from_ty: mid,
                        to_ty: IrType::I16,
                    },
                ],
                terminator: Terminator::Return(Some(Operand::Value(Value(2)))),
                source_spans: Vec::new(),
            });
            f.next_value_id = 3;
            simplify_function(&mut f);
            let Instruction::Cast { src, from_ty, .. } = f.blocks[0].instructions[1] else {
                panic!("cast expected");
            };
            assert_eq!(src == Operand::Value(Value(0)), folds, "{wide:?}");
            assert_eq!(from_ty == wide, folds, "{wide:?}");
        }
    }

    #[test]
    fn sign_variant_chain_links_still_fold() {
        // Cast(Cast(x: U32, U32->I64), U64->I16): I64 and U64 are the same
        // bits, so the widen-then-narrow chain folds to Cast(x, U32->I16).
        let mut f = IrFunction::new("f".into(), IrType::I16, vec![], false);
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Cast {
                    dest: Value(1),
                    src: Operand::Value(Value(0)),
                    from_ty: IrType::U32,
                    to_ty: IrType::I64,
                },
                Instruction::Cast {
                    dest: Value(2),
                    src: Operand::Value(Value(1)),
                    from_ty: IrType::U64,
                    to_ty: IrType::I16,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(2)))),
            source_spans: Vec::new(),
        });
        f.next_value_id = 3;
        simplify_function(&mut f);
        assert!(matches!(
            f.blocks[0].instructions[1],
            Instruction::Cast {
                src: Operand::Value(Value(0)),
                from_ty: IrType::U32,
                to_ty: IrType::I16,
                ..
            }
        ));
        // A widening outer step keeps its own signedness: I8 -> I16 (sext,
        // 0xFF -> 0xFFFF) read as U16 and zero-extended is 65535, not
        // sext(I8 -> I32) = -1, so this chain must stay.
        let mut f = IrFunction::new("f".into(), IrType::I32, vec![], false);
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Cast {
                    dest: Value(1),
                    src: Operand::Value(Value(0)),
                    from_ty: IrType::I8,
                    to_ty: IrType::I16,
                },
                Instruction::Cast {
                    dest: Value(2),
                    src: Operand::Value(Value(1)),
                    from_ty: IrType::U16,
                    to_ty: IrType::I32,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(2)))),
            source_spans: Vec::new(),
        });
        f.next_value_id = 3;
        simplify_function(&mut f);
        assert!(matches!(
            f.blocks[0].instructions[1],
            Instruction::Cast {
                src: Operand::Value(Value(1)),
                from_ty: IrType::U16,
                ..
            }
        ));
    }

    #[test]
    fn narrow_constants_are_known_only_where_every_extension_agrees() {
        // I64(-1) in an I128 op: low 64 bits are ones; the high half is
        // zero on x86-64 (zero-extended pair) but ones if sign-extended.
        let m = IrConst::I64(-1);
        assert!(const_range_is(&m, 0, 64, true));
        assert!(
            !const_range_is(&m, 64, 64, true),
            "high half is not a known all-ones mask"
        );
        assert!(!const_range_is(&m, 64, 64, false), "nor known zero");
        // Non-negative: zeros above the width under either extension.
        let k = IrConst::I64(0xFFFF);
        assert!(const_range_is(&k, 0, 16, true));
        assert!(const_range_is(&k, 16, 112, false));
        assert!(const_range_is(&IrConst::I128(-1), 64, 64, true));
        assert!(!const_range_is(&IrConst::F64(1.0), 0, 8, false));
    }

    #[test]
    fn plain_cast_chains_are_left_to_the_cast_rules() {
        // trunc(sext(x)) on a signed I32 is `cast_same`'s call (refused as a
        // Copy there); the pack walk must not take it over.
        let mut f = IrFunction::new("f".into(), IrType::I32, vec![], false);
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Cast {
                    dest: Value(1),
                    src: Operand::Value(Value(0)),
                    from_ty: IrType::I32,
                    to_ty: IrType::I64,
                },
                Instruction::Cast {
                    dest: Value(2),
                    src: Operand::Value(Value(1)),
                    from_ty: IrType::I64,
                    to_ty: IrType::I32,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(2)))),
            source_spans: Vec::new(),
        });
        f.next_value_id = 3;
        simplify_function(&mut f);
        assert!(matches!(
            f.blocks[0].instructions[1],
            Instruction::Cast {
                from_ty: IrType::I64,
                ..
            }
        ));
    }

    #[test]
    fn u32_pair_in_a_u64_folds_both_halves() {
        // The covering-access shape aggregate SROA builds: I64 over {U32, U32}.
        let mut f = carrier_in(
            IrType::I64,
            32,
            IrType::U32,
            IrType::U32,
            IrType::U32,
            IrBinOp::LShr,
            true,
        );
        simplify_function(&mut f);
        assert_eq!(reads(&f, 4), Some((Operand::Value(Value(0)), None)));
        assert_eq!(reads(&f, 6), Some((Operand::Value(Value(1)), None)));
    }

    #[test]
    fn extracts_wider_than_their_half_are_refused() {
        // Low: an I64 extract of a 32-bit-shifted pack includes hi's bits.
        let mut f = carrier_in(
            IrType::I128,
            32,
            IrType::U32,
            IrType::U64,
            IrType::I64,
            IrBinOp::LShr,
            false,
        );
        simplify_function(&mut f);
        assert_eq!(reads(&f, 4), None, "|T| = 64 > S = 32");
        // High: a U64 low part shifted by only 32 reaches the high field.
        let mut f = carrier_in(
            IrType::I128,
            32,
            IrType::U64,
            IrType::U32,
            IrType::U32,
            IrBinOp::LShr,
            false,
        );
        simplify_function(&mut f);
        assert_eq!(reads(&f, 6), None, "|A| = 64 > S = 32");
    }

    #[test]
    fn high_bit_lshr_elides_zero_extended_xor_operand_only_when_shift_covers_width() {
        let make = |from_ty: IrType, shift: i64| {
            let mut f = IrFunction::new("crc_lshr".into(), IrType::U64, vec![], false);
            f.blocks.push(crate::ir::reexports::BasicBlock {
                label: crate::ir::reexports::BlockId(0),
                instructions: vec![
                    Instruction::ParamRef {
                        dest: Value(0),
                        param_idx: 0,
                        ty: IrType::U64,
                    },
                    Instruction::ParamRef {
                        dest: Value(1),
                        param_idx: 1,
                        ty: from_ty,
                    },
                    Instruction::Cast {
                        dest: Value(2),
                        src: Operand::Value(Value(1)),
                        from_ty,
                        to_ty: IrType::U64,
                    },
                    Instruction::BinOp {
                        dest: Value(3),
                        op: IrBinOp::Xor,
                        lhs: Operand::Value(Value(0)),
                        rhs: Operand::Value(Value(2)),
                        ty: IrType::U64,
                    },
                    Instruction::BinOp {
                        dest: Value(4),
                        op: IrBinOp::LShr,
                        lhs: Operand::Value(Value(3)),
                        rhs: Operand::Const(IrConst::I64(shift)),
                        ty: IrType::U64,
                    },
                ],
                terminator: Terminator::Return(Some(Operand::Value(Value(4)))),
                source_spans: vec![],
            });
            f.next_value_id = 5;
            f
        };

        // `zext(u32)` has bits 32..63 == 0 -> `>> 32` and `>> 48` bypass the Xor.
        for s in [32, 48] {
            let mut f = make(IrType::U32, s);
            assert!(simplify_function(&mut f) > 0);
            assert!(f.blocks[0].instructions.iter().any(|i| matches!(
                i,
                Instruction::BinOp {
                    dest: Value(4),
                    op: IrBinOp::LShr,
                    lhs: Operand::Value(Value(0)),
                    ..
                }
            )));
        }

        // `>> 24` overlaps `u32` bit 24..31 -> must NOT bypass Xor; signed `I32`
        // sign-extends into bits 32..63 -> must NOT bypass Xor.
        for (ty, s) in [(IrType::U32, 24), (IrType::I32, 48)] {
            let mut f = make(ty, s);
            simplify_function(&mut f);
            assert!(f.blocks[0].instructions.iter().any(|i| matches!(
                i,
                Instruction::BinOp {
                    dest: Value(4),
                    op: IrBinOp::LShr,
                    lhs: Operand::Value(Value(3)),
                    ..
                }
            )));
        }
    }
}

#[cfg(test)]
mod cast_narrowing_census_tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, Terminator};

    /// `v1 = cast(v0: from -> to)`, then `body`, then `term`; blocks 1 and 2
    /// are plain returns so a `CondBranch` terminator has real targets.
    fn func(from: IrType, to: IrType, body: Vec<Instruction>, term: Terminator) -> IrFunction {
        let mut f = IrFunction::new("f".into(), IrType::I32, vec![], false);
        let mut instructions = vec![Instruction::Cast {
            dest: Value(1),
            src: Operand::Value(Value(0)),
            from_ty: from,
            to_ty: to,
        }];
        instructions.extend(body);
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions,
            terminator: term,
            source_spans: Vec::new(),
        });
        for label in [1, 2] {
            f.blocks.push(BasicBlock {
                label: BlockId(label),
                instructions: Vec::new(),
                terminator: Terminator::Return(Some(Operand::Const(IrConst::I32(label as i32)))),
                source_spans: Vec::new(),
            });
        }
        f.next_value_id = 64;
        f
    }

    fn cmp(dest: u32, op: IrCmpOp, c: i64, ty: IrType) -> Instruction {
        Instruction::Cmp {
            dest: Value(dest),
            op,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I64(c)),
            ty,
        }
    }

    /// `v40 = v10 | v11 | ...`: keeps every compare result read.
    fn or_all(dests: &[u32]) -> Vec<Instruction> {
        let mut acc = dests[0];
        let mut out = Vec::new();
        for (k, &d) in dests[1..].iter().enumerate() {
            let dest = 40 + k as u32;
            out.push(Instruction::BinOp {
                dest: Value(dest),
                op: IrBinOp::Or,
                lhs: Operand::Value(Value(acc)),
                rhs: Operand::Value(Value(d)),
                ty: IrType::I8,
            });
            acc = dest;
        }
        out
    }

    fn ret(v: u32) -> Terminator {
        Terminator::Return(Some(Operand::Value(Value(v))))
    }

    /// `(lhs, ty)` of every compare, in order.
    fn compares(f: &IrFunction) -> Vec<(Operand, IrType)> {
        f.blocks[0]
            .instructions
            .iter()
            .filter_map(|i| match i {
                Instruction::Cmp { lhs, ty, .. } => Some((*lhs, *ty)),
                _ => None,
            })
            .collect()
    }

    fn all_read(f: &IrFunction, v: u32, ty: IrType) -> bool {
        let got = compares(f);
        !got.is_empty()
            && got
                .iter()
                .all(|(l, t)| *l == Operand::Value(Value(v)) && *t == ty)
    }

    #[test]
    fn a_byte_classified_by_several_compares_is_compared_as_a_byte() {
        // Expat's name scan: c < 0x80, c >= 0xc2, c <= 0xdf on one promoted byte.
        let mut body = vec![
            cmp(10, IrCmpOp::Ult, 0x80, IrType::U32),
            cmp(11, IrCmpOp::Uge, 0xc2, IrType::U32),
            cmp(12, IrCmpOp::Ule, 0xdf, IrType::U32),
        ];
        body.extend(or_all(&[10, 11, 12]));
        let mut f = func(IrType::U8, IrType::U32, body, ret(41));
        simplify_function(&mut f);
        assert!(all_read(&f, 0, IrType::U8), "{:?}", compares(&f));
    }

    #[test]
    fn sign_extended_compares_narrow_together() {
        let mut body = vec![
            cmp(10, IrCmpOp::Slt, 0, IrType::I32),
            cmp(11, IrCmpOp::Sgt, -5, IrType::I32),
        ];
        body.extend(or_all(&[10, 11]));
        let mut f = func(IrType::I8, IrType::I32, body, ret(40));
        simplify_function(&mut f);
        assert!(all_read(&f, 0, IrType::I8), "{:?}", compares(&f));
    }

    #[test]
    fn one_non_compare_reader_keeps_every_compare_wide() {
        // sqlite: `serial_type >= 128` beside a call reading serial_type.
        let mut body = vec![
            cmp(10, IrCmpOp::Uge, 128, IrType::U32),
            cmp(11, IrCmpOp::Ult, 12, IrType::U32),
            Instruction::BinOp {
                dest: Value(20),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::U32,
            },
        ];
        body.extend(or_all(&[10, 11, 20]));
        let mut f = func(IrType::U8, IrType::U32, body, ret(41));
        simplify_function(&mut f);
        assert!(all_read(&f, 1, IrType::U32), "{:?}", compares(&f));
    }

    #[test]
    fn one_compare_that_cannot_narrow_keeps_the_others_wide() {
        for blocker in [
            cmp(11, IrCmpOp::Ult, 300, IrType::U32), // 300 does not fit in u8
            cmp(11, IrCmpOp::Slt, 5, IrType::U32),   // signed order on a zero-extension
        ] {
            let mut body = vec![cmp(10, IrCmpOp::Ult, 0x80, IrType::U32), blocker];
            body.extend(or_all(&[10, 11]));
            let mut f = func(IrType::U8, IrType::U32, body, ret(40));
            simplify_function(&mut f);
            let got = compares(&f);
            assert_eq!(got[0], (Operand::Value(Value(1)), IrType::U32), "{got:?}");
        }
    }

    #[test]
    fn a_branch_on_the_cast_narrows_with_its_compares() {
        let body = vec![cmp(10, IrCmpOp::Ult, 0x80, IrType::U32)];
        let term = Terminator::CondBranch {
            cond: Operand::Value(Value(1)),
            true_label: BlockId(1),
            false_label: BlockId(2),
        };
        let mut f = func(IrType::U8, IrType::U32, body, term);
        simplify_function(&mut f);
        assert!(all_read(&f, 0, IrType::U8), "{:?}", compares(&f));
        assert!(matches!(
            f.blocks[0].terminator,
            Terminator::CondBranch {
                cond: Operand::Value(Value(0)),
                ..
            }
        ));
    }

    fn census(f: &IrFunction, widened_cond_live: bool) -> bool {
        let mut cast_defs = vec![None; 64];
        cast_defs[1] = Some(CastDef {
            src: Operand::Value(Value(0)),
            from_ty: IrType::U8,
            to_ty: IrType::U32,
        });
        casts_dying_under_narrowing(f, &cast_defs, widened_cond_live)[1]
    }

    /// `cmp(v1 < 0x80)`, `v20 = copy v1`, `v21 = copy v20`; the function
    /// returns `v21` when `copy_live`, else the compare.
    fn with_copy_chain(copy_live: bool) -> IrFunction {
        let body = vec![
            cmp(10, IrCmpOp::Ult, 0x80, IrType::U32),
            Instruction::Copy {
                dest: Value(20),
                src: Operand::Value(Value(1)),
            },
            Instruction::Copy {
                dest: Value(21),
                src: Operand::Value(Value(20)),
            },
        ];
        func(
            IrType::U8,
            IrType::U32,
            body,
            ret(if copy_live { 21 } else { 10 }),
        )
    }

    #[test]
    fn an_unread_copy_chain_does_not_keep_the_cast_but_a_read_copy_does() {
        // Inlining leaves unread copies of the promoted byte; DCE deletes the
        // whole chain, so it must not veto the narrowing.
        assert!(census(&with_copy_chain(false), true));
        let mut f = with_copy_chain(false);
        simplify_function(&mut f);
        assert!(all_read(&f, 0, IrType::U8), "{:?}", compares(&f));
        // Returned, the chain is a live non-compare reader.
        assert!(!census(&with_copy_chain(true), true));
        let mut f = with_copy_chain(true);
        simplify_function(&mut f);
        assert!(all_read(&f, 1, IrType::U32), "{:?}", compares(&f));
    }

    #[test]
    fn dead_values_follow_dce() {
        let f = with_copy_chain(false);
        let dead = dead_pure_values(&f);
        assert!(dead[20] && dead[21], "the unread copy chain dies whole");
        assert!(
            !dead[1] && !dead[10],
            "the cast and the returned compare live"
        );
        let mut f = with_copy_chain(false);
        f.blocks[0].instructions.push(Instruction::Store {
            val: Operand::Value(Value(21)),
            ptr: Value(0),
            ty: IrType::U32,
            seg_override: Default::default(),
            volatile: false,
        });
        let dead = dead_pure_values(&f);
        assert!(!dead[20] && !dead[21], "a store keeps its operand chain");
    }

    #[test]
    fn the_census_counts_a_branch_only_when_its_rewrite_runs() {
        let term = Terminator::CondBranch {
            cond: Operand::Value(Value(1)),
            true_label: BlockId(1),
            false_label: BlockId(2),
        };
        let f = func(
            IrType::U8,
            IrType::U32,
            vec![cmp(10, IrCmpOp::Ult, 0x80, IrType::U32)],
            term,
        );
        assert!(census(&f, true));
        assert!(!census(&f, false));
    }

    #[test]
    fn the_census_refuses_a_compare_reading_the_cast_twice_and_an_unread_cast() {
        let both_sides = Instruction::Cmp {
            dest: Value(11),
            op: IrCmpOp::Eq,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Value(Value(1)),
            ty: IrType::U32,
        };
        let mut body = vec![cmp(10, IrCmpOp::Ult, 0x80, IrType::U32), both_sides];
        body.extend(or_all(&[10, 11]));
        assert!(!census(&func(IrType::U8, IrType::U32, body, ret(40)), true));
        // No reader at all: nothing to narrow, so nothing claims the cast dies.
        let unread = func(IrType::U8, IrType::U32, Vec::new(), ret(0));
        assert!(!census(&unread, true));
    }
}
