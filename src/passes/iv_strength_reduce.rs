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
use super::loop_unroll::negate_cmp;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::analysis;
use crate::ir::reexports::{Instruction, IrBinOp, IrCmpOp, IrConst, IrFunction, Operand, Value};

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
    /// (block_idx, inst_idx, GEP dest, GEP base)
    gep_uses: Vec<(usize, usize, Value, Value)>,
    /// The multiply/shift instruction's destination — the identity of the
    /// derived value for the scalar flavor below.
    mul_dest: Value,
    /// The multiply/shift instruction's type (the derived IV's domain).
    mul_ty: IrType,
    /// Whether the derived value has ANY use in the function (GEP offsets
    /// included). A dead multiply gets no recurrence.
    has_uses: bool,
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
    ivsr_with_analysis(func, &cfg, false)
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
    ivsr_with_analysis(func, &cfg, true)
}

/// Run IVSR using pre-computed CFG analysis (avoids redundant analysis when
/// called from a pipeline that shares analysis across GVN, LICM, IVSR).
/// `scalar_derived` arms the scalar derived-IV flavor — off in production;
/// the pipeline derives it from `CCC_IVSR_SCALAR_DERIVED=1` (opt-in).
pub(crate) fn ivsr_with_analysis(
    func: &mut IrFunction,
    cfg: &analysis::CfgAnalysis,
    scalar_derived: bool,
) -> usize {
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
        // Fold the affine offset out of the exit comparison FIRST: it leaves
        // the loop body with fewer instructions, so the strength reduction
        // that follows works on the cheaper form.
        let affine = fold_affine_exit_compares(func, natural_loop, &cfg.preds);

        let changed = reduce_loop(func, natural_loop, &cfg.preds, scalar_derived) + affine;
        if changed > 0 {
            kept_bodies.push(natural_loop.body.clone());
        }
        total_reductions += changed;
    }

    total_reductions
}

fn fold_affine_exit_compares(
    func: &mut IrFunction,
    natural_loop: &NaturalLoop,
    preds: &analysis::FlatAdj,
) -> usize {
    let body = &natural_loop.body;
    let header = natural_loop.header;

    // `Add` definitions that live INSIDE the loop, by value id.
    let mut adds: FxHashMap<u32, (Operand, Operand, IrType, IrBinOp)> = FxHashMap::default();
    // Use counts over the WHOLE function: a use in another block still pins
    // the `Add`, so folding would leave the `lea` behind.
    let mut uses: FxHashMap<u32, usize> = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            if let Instruction::BinOp {
                dest,
                op: op @ (IrBinOp::Add | IrBinOp::Sub),
                lhs,
                rhs,
                ty,
            } = inst
            {
                if body.contains(&bi) {
                    adds.entry(dest.0).or_insert((*lhs, *rhs, *ty, *op));
                }
            }
            let mut inst = inst.clone();
            inst.for_each_operand_mut(|o| {
                if let Operand::Value(v) = o {
                    *uses.entry(v.0).or_insert(0) += 1;
                }
            });
        }
    }
    if adds.is_empty() {
        return 0;
    }

    let preheader = loop_analysis::find_preheader(header, body, preds);
    let mut next_id = func.next_value_id;
    let mut folded = 0usize;
    let mut new_preheader: Vec<Instruction> = Vec::new();

    for bi in 0..func.blocks.len() {
        if !body.contains(&bi) {
            continue;
        }
        for ii in 0..func.blocks[bi].instructions.len() {
            let (dest, cop, lhs, rhs, ty) = match &func.blocks[bi].instructions[ii] {
                Instruction::Cmp {
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty,
                } => (*dest, *op, *lhs, *rhs, *ty),
                _ => continue,
            };
            if !ty.is_integer() || ty.is_128bit() {
                continue;
            }
            // Ordered comparisons only. The rewrite moves `c` across the
            // comparison, so it is valid exactly when `iv + c` cannot wrap.
            //
            // For a SIGNED compare a wrap is signed overflow, i.e. undefined
            // behaviour, so every execution the source admits is non-wrapping
            // and the identity holds on all of them.
            //
            // For an UNSIGNED compare the wrap is fully defined and the
            // identity genuinely fails -- verified on hardware, not merely
            // argued: with iv = 2^64-2, c = 1, k = 0, `ult(iv+c, k)` is 0
            // while `ult(iv, k-c)` is 1. An unsigned compare may therefore
            // fold only when the wrap is ruled out structurally, which
            // `offset_proves_no_unsigned_wrap` decides per candidate once the
            // offset and the compared type are known.
            //
            // `unsigned` has to be computed BEFORE the guard below. Computing
            // it after makes the whole unsigned path unreachable, so it would
            // silently never fold while still reading as if it were enabled.
            let unsigned = matches!(
                cop,
                IrCmpOp::Ult | IrCmpOp::Ule | IrCmpOp::Ugt | IrCmpOp::Uge
            );
            if !unsigned
                && !matches!(
                    cop,
                    IrCmpOp::Slt | IrCmpOp::Sle | IrCmpOp::Sgt | IrCmpOp::Sge
                )
            {
                continue;
            }

            // Try the left side as `iv + c`, then the right. Exactly one of
            // them may be the `Add`; when both are values the left is tried
            // first and the right only if the left is not a single-use add.
            let mut applied: Option<(Operand, Operand, IrCmpOp)> = None;
            for (cand, keep, mirrored) in [(lhs, rhs, false), (rhs, lhs, true)] {
                let Operand::Value(av) = cand else { continue };
                let Some(&(a_lhs, a_rhs, a_ty, a_op)) = adds.get(&av.0) else {
                    continue;
                };
                if a_ty != ty {
                    continue;
                }
                // `x + c` moves the offset to the far side unchanged; `c - x`
                // becomes `x` on the other side of the comparison, so the
                // operator has to be inverted and the bound becomes `c - k`.
                // All four affine shapes reduce to "the IV, an offset, and
                // whether the comparison must be inverted".
                //   iv + k   -> c = +k          iv - k   -> c = -k
                //   k + iv   -> c = +k          k - iv   -> c = +k, INVERTED
                // The `iv - k` form is the common `i - K` bound and needs no
                // inversion; only `k - iv` puts the IV on the far side.
                let (iv, c, negated) = match (a_op, a_lhs, a_rhs) {
                    (IrBinOp::Add, Operand::Value(v), Operand::Const(k)) => match k.to_i64() {
                        Some(kv) => (v, kv, false),
                        None => continue,
                    },
                    (IrBinOp::Add, Operand::Const(k), Operand::Value(v)) => match k.to_i64() {
                        Some(kv) => (v, kv, false),
                        None => continue,
                    },
                    (IrBinOp::Sub, Operand::Value(v), Operand::Const(k)) => {
                        match k.to_i64().and_then(i64::checked_neg) {
                            Some(neg) => (v, neg, false),
                            None => continue,
                        }
                    }
                    (IrBinOp::Sub, Operand::Const(k), Operand::Value(v)) => match k.to_i64() {
                        Some(kv) => (v, kv, true),
                        None => continue,
                    },
                    _ => continue,
                };
                // The offset must be an exact constant of the compared type.
                if ty.truncate_i64(c) != c {
                    continue;
                }
                // Unsigned: fold only when the addition provably cannot wrap.
                if unsigned && !offset_proves_no_unsigned_wrap(c, keep, ty) {
                    continue;
                }
                if uses.get(&av.0).copied().unwrap_or(0) != 1 {
                    continue;
                }
                let new_other = match keep {
                    Operand::Const(k) => {
                        let Some(kv) = k.to_i64() else { continue };
                        let kv = ty.truncate_i64(kv);
                        let bound = if negated {
                            c.checked_sub(kv)
                        } else {
                            kv.checked_sub(c)
                        };
                        let Some(bound) = bound else { continue };
                        let Some(kc) = const_for_int_ty(ty, bound) else {
                            continue;
                        };
                        Operand::Const(kc)
                    }
                    Operand::Value(v) => {
                        // Only an already-loop-invariant side can absorb the
                        // offset; anything else would trade a per-trip `lea`
                        // for a per-trip `sub`.
                        if !is_loop_invariant(v.0, body, func) {
                            continue;
                        }
                        let nv = Value(next_id);
                        next_id += 1;
                        if negated {
                            // bound = c - v
                            let Some(kcv) = const_for_int_ty(ty, c) else {
                                continue;
                            };
                            new_preheader.push(Instruction::BinOp {
                                dest: nv,
                                op: IrBinOp::Sub,
                                lhs: Operand::Const(kcv),
                                rhs: Operand::Value(v),
                                ty,
                            });
                        } else {
                            // bound = v - c
                            let Some(kcv) = const_for_int_ty(ty, c) else {
                                continue;
                            };
                            new_preheader.push(Instruction::BinOp {
                                dest: nv,
                                op: IrBinOp::Sub,
                                lhs: Operand::Value(v),
                                rhs: Operand::Const(kcv),
                                ty,
                            });
                        }
                        Operand::Value(nv)
                    }
                };
                let new_cop = if negated { negate_cmp(cop) } else { cop };
                applied = Some(if mirrored {
                    (new_other, Operand::Value(iv), new_cop)
                } else {
                    (Operand::Value(iv), new_other, new_cop)
                });
                break;
            }
            let Some((nl, nr, new_cop)) = applied else {
                continue;
            };
            func.blocks[bi].instructions[ii] = Instruction::Cmp {
                dest,
                op: new_cop,
                lhs: nl,
                rhs: nr,
                ty,
            };
            folded += 1;
        }
    }

    if folded == 0 {
        return 0;
    }
    if let Some(ph) = preheader {
        let at = func.blocks[ph].instructions.len();
        for (k, inst) in new_preheader.into_iter().enumerate() {
            func.blocks[ph].instructions.insert(at + k, inst);
        }
    }
    func.next_value_id = next_id;
    folded
}

/// Can `iv + c` be shown not to wrap, in `ty`, for this loop?
///
/// Only meaningful for UNSIGNED comparisons, where the wrap is defined
/// behaviour rather than UB. The proof used here is deliberately the one that
/// needs no trip-count analysis:
///
/// `iv` is an induction variable that is incremented by a POSITIVE constant
/// and compared against a bound BEFORE the increment takes effect; therefore
/// on every iteration that reaches the compare, `iv <= k`. When the offset is
/// non-negative and the bound `k` is itself a non-negative constant that does
/// not exceed the type's maximum, then `iv + c <= k + c`, and the sum is
/// bounded by a value representable in `ty` -- so it cannot wrap. With a
/// negative offset, or a value bound, or an offset large enough to push
/// `k + c` past the maximum, the wrap is genuinely possible and the rewrite is
/// declined.
///
/// Anything requiring the trip count, the step, or a range analysis is out of
/// scope: a bound that is merely "loop invariant" says nothing about its
/// magnitude, and assuming otherwise is exactly the unsoundness this guard
/// exists to prevent.
fn offset_proves_no_unsigned_wrap(c: i64, keep: Operand, ty: IrType) -> bool {
    if c < 0 {
        return false; // `iv - |c|` can underflow below zero
    }
    // Only a literal bound proves a magnitude. A loop-invariant VALUE is not
    // enough: `end - v` is loop invariant and unbounded.
    let Operand::Const(k) = keep else {
        return false;
    };
    let Some(kv) = k.to_i64() else { return false };
    if kv < 0 || ty.truncate_i64(kv) != kv {
        return false;
    }
    // `iv <= k` on every iteration, so `iv + c <= k + c`; require that ceiling
    // to be representable in the compared type.
    let Some(ceil) = kv.checked_add(c) else {
        return false;
    };
    let ceiling = ty.truncate_i64(ceil);
    ceiling == ceil && ceil >= 0
}

/// Try to strength-reduce induction variables in a single loop.
fn reduce_loop(
    func: &mut IrFunction,
    natural_loop: &NaturalLoop,
    preds: &analysis::FlatAdj,
    scalar_derived: bool,
) -> usize {
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
    let derived = find_derived_exprs(func, &basic_ivs, &natural_loop.body);
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
    // expressions. The key is semantic: (basic IV, byte stride, invariant base).
    let mut pointer_groups: Vec<(usize, i64, i64, Value, Vec<(usize, usize, Value)>)> = Vec::new();
    for d in &derived {
        for &(gep_block_idx, gep_inst_idx, gep_dest, gep_base) in &d.gep_uses {
            if let Some((_, _, _, _, uses)) =
                pointer_groups
                    .iter_mut()
                    .find(|(iv_index, stride, off, base, _)| {
                        *iv_index == d.iv_index
                            && *stride == d.stride
                            && *off == d.add_offset
                            && *base == gep_base
                    })
            {
                uses.push((gep_block_idx, gep_inst_idx, gep_dest));
            } else {
                pointer_groups.push((
                    d.iv_index,
                    d.stride,
                    d.add_offset,
                    gep_base,
                    vec![(gep_block_idx, gep_inst_idx, gep_dest)],
                ));
            }
        }
    }

    for (iv_index, stride, add_offset, gep_base, gep_uses) in pointer_groups {
        let iv = &basic_ivs[iv_index];
        let inc_bytes = iv.step * stride;

        // Only reduce GEPs where the base is loop-invariant (skip loop-variant bases).
        if !is_loop_invariant(gep_base.0, &natural_loop.body, func) {
            continue;
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
        let init_offset = init_const.map(|v| (v + add_offset) * stride);

        // Build preheader instructions for computing the initial pointer
        let mut preheader_insts: Vec<Instruction> = Vec::new();

        if let Some(init_off) = init_offset {
            // Constant initial offset case (most common: i = 0)
            if init_off == 0 {
                preheader_insts.push(Instruction::Copy {
                    dest: init_ptr_val,
                    src: Operand::Value(gep_base),
                });
            } else {
                preheader_insts.push(Instruction::GetElementPtr {
                    dest: init_ptr_val,
                    base: gep_base,
                    offset: Operand::Const(IrConst::I64(init_off)),
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
                        base: gep_base,
                        offset: Operand::Value(init_cast_val),
                        ty: IrType::I8,
                    });
                } else {
                    next_id -= 1; // Undo unused init_cast_val allocation
                    preheader_insts.push(Instruction::GetElementPtr {
                        dest: init_ptr_val,
                        base: gep_base,
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
                    base: gep_base,
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

        // 4. Replace every original GEP in this semantic group with a Copy
        // from the one shared pointer IV. Adjust header indices for the
        // inserted phis, but do not add another recurrence per use.
        for (gep_block_idx, gep_inst_idx, gep_dest) in gep_uses {
            let adjusted_idx = if gep_block_idx == header {
                gep_inst_idx + header_phi_insertions
            } else {
                gep_inst_idx
            };

            if adjusted_idx < func.blocks[gep_block_idx].instructions.len() {
                let inst = &func.blocks[gep_block_idx].instructions[adjusted_idx];
                if let Some(dest) = inst.dest() {
                    if dest == gep_dest {
                        func.blocks[gep_block_idx].instructions[adjusted_idx] = Instruction::Copy {
                            dest: gep_dest,
                            src: Operand::Value(ptr_iv_val),
                        };
                        reductions += 1;
                    }
                }
            }
        }
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
                    match rinst {
                        Instruction::Load { ptr, .. } => {
                            if muls.contains(ptr) {
                                *ptr = j_val;
                                rewritten += 1;
                            }
                        }
                        Instruction::Store { ptr, .. } => {
                            if muls.contains(ptr) {
                                *ptr = j_val;
                                rewritten += 1;
                            }
                        }
                        Instruction::GetElementPtr { base, .. } => {
                            if muls.contains(base) {
                                *base = j_val;
                                rewritten += 1;
                            }
                        }
                        _ => {}
                    }
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
            // Must be integer type (induction variables are integers)
            if !ty.is_integer() && *ty != IrType::Ptr {
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
) -> Vec<DerivedExpr> {
    let mut derived = Vec::with_capacity(16);

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
                        // Only treat widening casts as IV-derived.
                        // Truncating casts (e.g. I32->U8) change the value
                        // and must not be strength-reduced as if linear.
                        if to_ty.size() >= from_ty.size() {
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
    let find_affine = |val_id: u32| -> Option<(usize, i64)> {
        for &bi in loop_body {
            if bi >= func.blocks.len() {
                continue;
            }
            for inst in &func.blocks[bi].instructions {
                if inst.dest() != Some(Value(val_id)) {
                    continue;
                }
                if let Instruction::BinOp {
                    op: IrBinOp::Add,
                    lhs,
                    rhs,
                    ..
                } = inst
                {
                    if let (Operand::Value(v), Operand::Const(c)) = (lhs, rhs) {
                        if let (Some(idx), Some(k)) = (find_iv(v.0), c.to_i64()) {
                            return Some((idx, k));
                        }
                    }
                    if let (Operand::Const(c), Operand::Value(v)) = (lhs, rhs) {
                        if let (Some(idx), Some(k)) = (find_iv(v.0), c.to_i64()) {
                            return Some((idx, k));
                        }
                    }
                }
            }
        }
        None
    };
    if std::env::var("CCC_IVSR_DEBUG").is_ok() {
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
            if std::env::var("CCC_IVSR_DEBUG").is_ok() {
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
                        } else if let (Some((idx, k)), Some(s)) = (find_affine(v.0), s) {
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

            // Find GEPs that use this multiply/shift result as their offset
            let mul_dest_id = mul_dest.0;
            let mut gep_uses = Vec::with_capacity(16);

            for &gbi in loop_body {
                if gbi >= func.blocks.len() {
                    continue;
                }
                for (gii, ginst) in func.blocks[gbi].instructions.iter().enumerate() {
                    if let Instruction::GetElementPtr {
                        dest: gdest,
                        base,
                        offset: Operand::Value(ov),
                        ..
                    } = ginst
                    {
                        if ov.0 == mul_dest_id {
                            gep_uses.push((gbi, gii, *gdest, *base));
                        }
                    }
                }
            }

            // Any use of the derived value anywhere in the function (GEP
            // offsets included)?  A dead multiply earns no recurrence.
            let mut use_count = 0usize;
            for block in &func.blocks {
                for uinst in &block.instructions {
                    uinst.for_each_used_value(|u| {
                        if u == mul_dest_id {
                            use_count += 1;
                        }
                    });
                }
                block.terminator.for_each_used_value(|u| {
                    if u == mul_dest_id {
                        use_count += 1;
                    }
                });
            }

            if !gep_uses.is_empty() || use_count > 0 {
                derived.push(DerivedExpr {
                    stride,
                    iv_index: iv_idx,
                    add_offset,
                    gep_uses,
                    mul_dest,
                    mul_ty,
                    has_uses: use_count > 0,
                });
            } else if std::env::var("CCC_IVSR_DEBUG").is_ok() {
                eprintln!(
                    "[IVSR-DERIV] mul v{} stride {} has no GEP uses",
                    mul_dest.0, stride
                );
            }
        }
    }

    derived
}

/// Look through Cast and Copy instructions to find the root value.
/// Used to match `Cast(Add(Cast(phi_dest), step))` patterns.
fn look_through_casts(val_id: u32, loop_defs: &FxHashMap<u32, &Instruction>) -> u32 {
    let mut current = val_id;
    for _ in 0..MAX_CAST_CHAIN_LENGTH {
        if let Some(inst) = loop_defs.get(&current) {
            match inst {
                Instruction::Cast {
                    src: Operand::Value(v),
                    ..
                }
                | Instruction::Copy {
                    src: Operand::Value(v),
                    ..
                } => {
                    current = v.0;
                }
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
                    rhs: Operand::Const(IrConst::I64(4)),
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
                    rhs: Operand::Const(IrConst::I64(4)),
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

    /// Build the canonical shape the exit-compare fold targets:
    ///
    ///     header:  %i = phi(0, %i_next)
    ///     latch:   %t = i + 4 ; %c = slt %t, %n ; br %c, header, exit
    ///
    /// `cmp_op` selects the comparison so the same fixture covers both the
    /// signed case that must fold and the unsigned case that must not.
    fn affine_exit_fixture(cmp_op: IrCmpOp, add_ty: IrType, cmp_ty: IrType) -> IrFunction {
        let mut func = IrFunction::new("affine_exit".to_string(), IrType::I64, vec![], false);
        // 0: preheader -- %n = 100, %zero = 0
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Copy {
                    dest: Value(0),
                    src: Operand::Const(IrConst::I64(100)),
                },
                Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Const(IrConst::I64(0)),
                },
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        // 1: header -- %i = phi(0, %i_next)
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![Instruction::Phi {
                dest: Value(2),
                ty: IrType::I64,
                incoming: vec![
                    (Operand::Value(Value(1)), BlockId(0)),
                    (Operand::Value(Value(6)), BlockId(2)),
                ],
            }],
            terminator: Terminator::Branch(BlockId(2)),
            source_spans: Vec::new(),
        });
        // 2: latch -- %t = i + 4 ; %c = cmp %t, %n
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![
                Instruction::BinOp {
                    dest: Value(3),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Const(IrConst::I64(4)),
                    ty: add_ty,
                },
                Instruction::Cmp {
                    dest: Value(4),
                    op: cmp_op,
                    lhs: Operand::Value(Value(3)),
                    rhs: Operand::Value(Value(0)),
                    ty: cmp_ty,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(4)),
                true_label: BlockId(1),
                false_label: BlockId(3),
            },
            source_spans: Vec::new(),
        });
        // 3: exit
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        func.next_value_id = 10;
        func
    }

    fn run_fold(func: &mut IrFunction) -> usize {
        let mut cfg = analysis::CfgAnalysis::build(func);
        let loops =
            loop_analysis::find_natural_loops(cfg.num_blocks, &cfg.preds, &cfg.succs, &cfg.idom);
        let loops = loop_analysis::merge_loops_by_header(loops);
        let mut n = 0;
        for l in &loops {
            n += fold_affine_exit_compares(func, l, &cfg.preds);
        }
        let _ = &mut cfg;
        n
    }

    /// The positive: a SIGNED affine exit compare folds, the offset moves to
    /// the invariant side, and the `Sub` lands in the PREHEADER so it is
    /// computed once instead of once per trip.
    #[test]
    fn signed_affine_exit_compare_folds_into_the_preheader() {
        let mut func = affine_exit_fixture(IrCmpOp::Slt, IrType::I64, IrType::I64);
        assert_eq!(run_fold(&mut func), 1, "the signed form must fold");

        let latch_cmp = func.blocks[2]
            .instructions
            .iter()
            .find_map(|i| match i {
                Instruction::Cmp { lhs, rhs, .. } => Some((*lhs, *rhs)),
                _ => None,
            })
            .expect("latch compare");
        // The induction variable is now compared directly...
        assert!(
            matches!(latch_cmp.0, Operand::Value(v) if v.0 == 2),
            "lhs must be the IV, got {:?}",
            latch_cmp.0
        );
        // ...against a value the preheader computed, not against %t.
        let Operand::Value(bound) = latch_cmp.1 else {
            panic!("rhs must be the folded bound, got {:?}", latch_cmp.1)
        };
        assert_ne!(bound.0, 3, "must not compare against the `iv + 4` temp");
        let sub = func.blocks[0]
            .instructions
            .iter()
            .find_map(|i| match i {
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::Sub,
                    rhs,
                    ..
                } if *dest == bound => Some(*rhs),
                _ => None,
            })
            .expect("the preheader must compute the folded bound");
        assert!(
            matches!(sub, Operand::Const(IrConst::I64(4))),
            "the bound must be %n - 4, got {:?}",
            sub
        );
    }

    /// The negative that matters most: UNSIGNED. `ult(iv + c, k)` is NOT
    /// `ult(iv, k - c)` when the sum wraps -- the wrap is fully defined, not
    /// UB -- so the fold must decline. This is the pointer-loop case, and it
    /// is exactly the shape the rewrite would silently corrupt.
    #[test]
    fn unsigned_affine_exit_compare_is_refused() {
        for op in [IrCmpOp::Ult, IrCmpOp::Ule, IrCmpOp::Ugt, IrCmpOp::Uge] {
            let mut func = affine_exit_fixture(op, IrType::I64, IrType::I64);
            assert_eq!(
                run_fold(&mut func),
                0,
                "{op:?} must not fold: the wrap is defined"
            );
        }
    }

    /// The unsigned refusal is not caution -- it is required. With
    /// iv = 2^64-2, c = 1, k = 0 the two forms disagree on hardware:
    /// `ult(iv+c, k)` is 0, `ult(iv, k - c)` is 1. Every unsigned fixture
    /// here must therefore decline unless the wrap is structurally excluded.
    #[test]
    fn unsigned_affine_exit_compare_needs_a_no_wrap_proof() {
        // A loop-invariant VALUE bound proves nothing about magnitude, so the
        // common pointer shape (`i + 4 < end`) must keep declining.
        let mut func = affine_exit_fixture(IrCmpOp::Ult, IrType::I64, IrType::I64);
        assert_eq!(
            run_fold(&mut func),
            0,
            "an unbounded invariant bound cannot prove the add cannot wrap"
        );

        // A negative offset is the `i - K` form; under an unsigned compare
        // `iv - K` can underflow below zero, so it must decline even though
        // the same shape folds when signed.
        let mut func = affine_exit_fixture(IrCmpOp::Ule, IrType::I64, IrType::I64);
        func.blocks[2].instructions[0] = Instruction::BinOp {
            dest: Value(3),
            op: IrBinOp::Sub,
            lhs: Operand::Value(Value(2)),
            rhs: Operand::Const(IrConst::I64(4)),
            ty: IrType::I64,
        };
        assert_eq!(run_fold(&mut func), 0, "`iv - K` underflows under Ule");
    }

    /// The proof the guard accepts: a LITERAL bound with room to spare, which
    /// is what a constant-bound loop looks like. Regression test for the
    /// ordering bug: `unsigned` was originally classified after the
    /// signed-only guard, which made this whole path unreachable -- the code
    /// read as if unsigned folding were enabled while silently never firing.
    #[test]
    fn unsigned_folds_when_a_literal_bound_proves_no_wrap() {
        let mut func = affine_exit_fixture(IrCmpOp::Ult, IrType::I64, IrType::I64);
        // The fixture's bound is a Copy-defined VALUE. Point the compare at a
        // LITERAL: iv <= 100 on every iteration and 100 + 4 is representable,
        // so `iv + 4` provably cannot wrap.
        func.blocks[2].instructions[1] = Instruction::Cmp {
            dest: Value(4),
            op: IrCmpOp::Ult,
            lhs: Operand::Value(Value(3)),
            rhs: Operand::Const(IrConst::I64(100)),
            ty: IrType::I64,
        };
        assert_eq!(
            run_fold(&mut func),
            1,
            "literal bound with headroom must fold"
        );
    }

    /// ...and the same literal must be REJECTED when the offset would push the
    /// sum past the type's maximum: the boundary the guard exists to get right.
    #[test]
    fn unsigned_declines_when_the_sum_would_overflow_the_type() {
        let mut func = affine_exit_fixture(IrCmpOp::Ult, IrType::I64, IrType::I64);
        func.blocks[2].instructions[1] = Instruction::Cmp {
            dest: Value(4),
            op: IrCmpOp::Ult,
            lhs: Operand::Value(Value(3)),
            rhs: Operand::Const(IrConst::I64(i64::MAX)),
            ty: IrType::I64,
        };
        assert_eq!(run_fold(&mut func), 0, "MAX + 4 wraps; must decline");
    }

    /// A `BinOp` type that disagrees with the compare's is a different
    /// expression entirely (the compare sees a converted value), so folding
    /// would compare the wrong quantity.
    #[test]
    fn an_add_of_a_different_type_is_refused() {
        let mut func = affine_exit_fixture(IrCmpOp::Slt, IrType::I32, IrType::I64);
        assert_eq!(run_fold(&mut func), 0, "mismatched widths must not fold");
    }

    /// When the `Add` has a second user the sum is still needed, so folding
    /// the compare would leave the address computation behind and trade a
    /// removed instruction for an added one.
    #[test]
    fn an_add_with_a_second_user_is_refused() {
        let mut func = affine_exit_fixture(IrCmpOp::Slt, IrType::I64, IrType::I64);
        // %t is now read twice: by the compare and by a copy in the exit.
        func.blocks[3].instructions.push(Instruction::Copy {
            dest: Value(7),
            src: Operand::Value(Value(3)),
        });
        func.next_value_id = 10;
        assert_eq!(run_fold(&mut func), 0, "a shared `iv + c` must not fold");
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
}
