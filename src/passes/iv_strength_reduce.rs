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
    ivsr_with_analysis(func, &cfg, false, true)
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
    ivsr_with_analysis(func, &cfg, true, true)
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
pub(crate) fn ivsr_with_analysis(
    func: &mut IrFunction,
    cfg: &analysis::CfgAnalysis,
    scalar_derived: bool,
    ptr_add: bool,
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
        let changed = reduce_loop(func, natural_loop, &cfg.preds, scalar_derived, ptr_add);
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
    preds: &analysis::FlatAdj,
    scalar_derived: bool,
    ptr_add: bool,
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
    let derived = find_derived_exprs(func, &basic_ivs, &natural_loop.body, ptr_add);
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
    let mut pointer_groups: Vec<(usize, i64, i64, IrType, Value, Vec<(usize, usize, Value)>)> =
        Vec::new();
    for d in &derived {
        for &(gep_block_idx, gep_inst_idx, gep_dest, gep_base) in &d.gep_uses {
            if let Some((_, _, _, _, _, uses)) =
                pointer_groups
                    .iter_mut()
                    .find(|(iv_index, stride, off, mty, base, _)| {
                        *iv_index == d.iv_index
                            && *stride == d.stride
                            && *off == d.add_offset
                            && *mty == d.mul_ty
                            && *base == gep_base
                    })
            {
                uses.push((gep_block_idx, gep_inst_idx, gep_dest));
            } else {
                pointer_groups.push((
                    d.iv_index,
                    d.stride,
                    d.add_offset,
                    d.mul_ty,
                    gep_base,
                    vec![(gep_block_idx, gep_inst_idx, gep_dest)],
                ));
            }
        }
    }

    for (iv_index, stride, add_offset, mul_ty, gep_base, gep_uses) in pointer_groups {
        let iv = &basic_ivs[iv_index];
        // A narrow unsigned counter is modular: zero-extending it into an
        // address recurrence is linear only while the executed backedges cannot
        // wrap. That is a proof obligation, not an inference from "there is a
        // comparison somewhere". IVs at or above the pointer width, and every
        // sub-32-bit counter (whose C promotion makes the arithmetic 32-bit and
        // therefore already covered by IVSR-WRAP-1's backedge rule), are handled
        // by the other arms.
        let modular_iv = iv.ty.size() < crate::common::types::target_ptr_size();
        let unsigned_bound = if modular_iv && (iv.ty.size() as usize) >= 4 && iv.ty.is_unsigned() {
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
        let init_offset = if let Some(v) = init_const {
            // IrConst's signed storage is not the IV's mathematical domain.
            let v = match iv.ty {
                IrType::U8 => i64::from(v as u8),
                IrType::U16 => i64::from(v as u16),
                IrType::U32 => i64::from(v as u32),
                _ => v,
            };
            let Some(off) = v
                .checked_add(add_offset)
                .and_then(|v| v.checked_mul(stride))
            else {
                continue;
            };
            Some(off)
        } else {
            None
        };

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
    ptr_add: bool,
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
                        // Value preservation, not equal storage width. In
                        // particular U32 -> I32 -> I64 maps UINT_MAX to -1,
                        // whereas widening the original U32 maps it to 2^32-1.
                        if cast_preserves_offset_value(*from_ty, *to_ty) {
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
    let find_affine = |val_id: u32, result_ty: IrType| -> Option<(usize, i64)> {
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
                    ty,
                    ..
                } = inst
                {
                    if *ty != result_ty {
                        continue;
                    }
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
                        // The offset's own ring is checked once per group in
                        // `reduce_loop` (`offset_product_cannot_overflow`),
                        // not by vetoing every sub-pointer-width multiply:
                        // a 32-bit `i * 4` whose bound is a compile-time
                        // constant is exactly as linear as a 64-bit one.
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
        // `mul_dest` -> index into `derived`, so an Add whose offset is an
        // already-recognised scaled index JOINS that group instead of forming a
        // second recurrence for the same address.
        let mut by_mul_dest: FxHashMap<u32, usize> = FxHashMap::default();
        for (i, d) in derived.iter().enumerate() {
            by_mul_dest.entry(d.mul_dest.0).or_insert(i);
        }
        let ptr_ring = crate::common::types::target_ptr_size();
        let mut attaches: Vec<(usize, usize, usize, Value, Value)> = Vec::new();
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
                // induction of a single pointer either.
                for (off, base) in [(a, b), (b, a)] {
                    if !is_loop_invariant(base.0, loop_body, func) {
                        continue;
                    }
                    if let Some(&di) = by_mul_dest.get(&off.0) {
                        attaches.push((di, bi, ii, *dest, *base));
                        break;
                    }
                    let (idx, add_off) = if let Some(i) = find_iv(off.0) {
                        (i, 0)
                    } else {
                        match find_affine(off.0, *ty) {
                            Some(pair) => pair,
                            None => continue,
                        }
                    };
                    fresh.push(DerivedExpr {
                        stride: 1,
                        iv_index: idx,
                        add_offset: add_off,
                        gep_uses: vec![(bi, ii, *dest, *base)],
                        mul_dest: *dest,
                        mul_ty: *ty,
                        has_uses: true,
                    });
                    break;
                }
            }
        }

        for (di, bi, ii, dest, base) in attaches {
            derived[di].gep_uses.push((bi, ii, dest, base));
        }
        derived.extend(fresh);
    }

    derived
}

/// Whether `val_id` is dereferenced or used as an addressing base anywhere in
/// the function — i.e. whether it is an ADDRESS and not merely an integer that
/// happens to be pointer-width. Conservative by construction: anything not
/// positively identified as a memory operand returns false. A value used both
/// as an address and as an integer still qualifies (the rewrite preserves its
/// value either way); a pure integer accumulation does not.
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

/// Cast legality for a derived address expression. Both predicates live on
/// `IrType` next to the size/signedness facts they are stated in terms of, and
/// are shared with `src/backend/generation.rs`'s SIB-index peel so the two
/// cannot drift apart again (IVSR-DOMAIN-1 was exactly that drift). The
/// exhaustive per-domain tests live with the predicates, in
/// `src/common/types.rs::cast_predicate_tests`; the IVSR-specific consequences
/// are pinned by `derived_cast_proof_is_about_values_not_storage_size` and
/// `offset_cast_proof_adds_only_the_pointer_ring_reinterpretation` below.
#[inline]
fn cast_preserves_integer_value(from: IrType, to: IrType) -> bool {
    from.cast_preserves_integer_value(to)
}

#[inline]
fn cast_preserves_offset_value(from: IrType, to: IrType) -> bool {
    from.cast_preserves_offset_value(to)
}

/// Proven execution range of a narrow UNSIGNED induction variable.
///
/// `hi` bounds every value the loop body can observe. `exact` records whether
/// that bound came from a compile-time constant; only an exact bound can also
/// certify that a narrower product `iv * stride` cannot overflow its own ring
/// (an invariant bound proves the IV is monotone but says nothing numeric).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UnsignedIvBound {
    lo: i128,
    hi: i128,
    exact: bool,
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
        let exact = matches!(other, Operand::Const(_));
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
        let lo = if iv.step > 0 { 0 } else { 1 };
        // A constant limit yields the exact body maximum `limit - 1`; an
        // invariant limit proves monotonicity but no numeric maximum.
        let hi = match other {
            Operand::Const(c) => c.to_i64().map(|n| n as i128 - 1).unwrap_or(max),
            Operand::Value(_) => max,
        };
        return Some(UnsignedIvBound {
            lo: lo as i128,
            hi,
            exact,
        });
    }
    None
}

/// Whether replacing `offset(iv) = (iv + add_offset) * stride`, evaluated in
/// `mul_ty`, by the pointer recurrence `p += step * stride` can diverge.
///
/// Divergence needs the PRODUCT to wrap inside `mul_ty` while the pointer ring
/// keeps counting linearly. A signed `mul_ty` cannot wrap in a defined program
/// (C17 6.5/5 — the same UB theorem `iv_widen` uses), and a product at or above
/// the pointer width wraps in exactly the ring the recurrence lives in, so both
/// are equivalent by construction. Only an UNSIGNED product NARROWER than the
/// pointer ring is a genuine hazard, and an exact constant bound discharges it.
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
    let Some(b) = bound.filter(|b| b.exact) else {
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
                    cast_preserves_offset_value(from, to),
                    cast_preserves_integer_value(from, to) || ring,
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
            assert!(!cast_preserves_offset_value(from, to), "{from:?}->{to:?}");
        }
        // The pointer-ring reinterpretation is admitted only at pointer width,
        // which is what keeps the 64-bit `size_t`/`ptrdiff_t` spelling reducible.
        let wide = (IrType::U64.size() as usize) >= ptr;
        assert_eq!(cast_preserves_offset_value(IrType::U64, IrType::I64), wide);
        assert_eq!(cast_preserves_offset_value(IrType::I64, IrType::U64), wide);
        assert!(!cast_preserves_offset_value(IrType::I32, IrType::F32));
        assert!(!cast_preserves_offset_value(IrType::F64, IrType::I64));
        assert!(!cast_preserves_offset_value(IrType::I64, IrType::Ptr));
    }

    #[test]
    fn offset_product_proof_needs_an_exact_bound_only_below_the_pointer_ring() {
        let ptr = crate::common::types::target_ptr_size();
        let exact = Some(UnsignedIvBound {
            lo: 0,
            hi: 1023,
            exact: true,
        });
        let inexact = Some(UnsignedIvBound {
            lo: 0,
            hi: (1i128 << 32) - 1,
            exact: false,
        });
        // Signed and pointer-ring products are UB/ring-equivalent: always OK.
        for ty in [IrType::I32, IrType::I64, IrType::U64] {
            if ty.size() >= ptr || !ty.is_unsigned() {
                assert!(offset_product_cannot_overflow(ty, 4, 0, None), "{ty:?}");
            }
        }
        // Unsigned 32-bit product: needs the exact bound, and must really fit.
        assert!(offset_product_cannot_overflow(IrType::U32, 4, 0, exact));
        assert!(!offset_product_cannot_overflow(IrType::U32, 4, 0, inexact));
        assert!(!offset_product_cannot_overflow(IrType::U32, 4, 0, None));
        let too_big = Some(UnsignedIvBound {
            lo: 0,
            hi: (1i128 << 31) - 1,
            exact: true,
        });
        assert!(!offset_product_cannot_overflow(IrType::U32, 4, 0, too_big));
        // An affine offset participates in the product bound.
        assert!(offset_product_cannot_overflow(IrType::U32, 4, 8, exact));
        let edge = Some(UnsignedIvBound {
            lo: 0,
            hi: (1i128 << 30) - 9,
            exact: true,
        });
        assert!(offset_product_cannot_overflow(IrType::U32, 4, 8, edge));
        let over = Some(UnsignedIvBound {
            lo: 0,
            hi: (1i128 << 30) - 8,
            exact: true,
        });
        assert!(!offset_product_cannot_overflow(IrType::U32, 4, 8, over));
        // The predicate only ever sees the integer type of a Mul/Shl/Add;
        // a non-integer `mul_ty` is vacuously admitted and cannot arise.
        assert!(offset_product_cannot_overflow(IrType::F32, 4, 0, exact));
    }

    #[test]
    fn unsigned_bound_records_whether_the_limit_was_a_constant() {
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
        // Constant limit 8: body values are exactly 0..=7.
        let b = unsigned_iv_bound(&f, &lp, &iv, &[2]).expect("constant bound");
        assert!(b.exact && b.lo == 0 && b.hi == 7, "{b:?}");
        // Replace the limit with a loop-invariant value: monotone but inexact.
        f.blocks[0].instructions.push(Instruction::Copy {
            dest: Value(7),
            src: Operand::Const(IrConst::I32(8)),
        });
        if let Instruction::Cmp { rhs, .. } = &mut f.blocks[1].instructions[1] {
            *rhs = Operand::Value(Value(7));
        }
        let b = unsigned_iv_bound(&f, &lp, &iv, &[2]).expect("invariant bound");
        assert!(!b.exact && b.hi == (1i128 << 32) - 1, "{b:?}");
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
                    cast_preserves_integer_value(from, to),
                    lo >= tl && hi <= th,
                    "{from:?}->{to:?}"
                );
            }
        }
        assert!(!cast_preserves_integer_value(IrType::I32, IrType::F32));
        assert!(!cast_preserves_integer_value(IrType::F64, IrType::I64));
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
                    rhs: Operand::Const(IrConst::I64(4)),
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
    fn address_add_arm_is_gated_by_the_kill_switch_and_ilp32() {
        // LP64: the arm fires. This also pins that the gate reads the TARGET,
        // not a hard-coded width, since the test suite runs on the host default.
        let mut f = address_add_loop(true, false);
        let fired = ivsr_function(&mut f) > 0;
        assert_eq!(fired, !crate::common::types::target_is_32bit());
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
