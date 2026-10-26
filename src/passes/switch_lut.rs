//! Register Bit-Packed Switch Lookup Table (`switch_lut`).
//!
//! Collapses dense and semi-dense multi-arm `Switch` terminators whose arms all
//! produce integer constants (either returning directly or feeding a single
//! merge `Phi`) into a branchless register-resident bit-packed lookup table:
//!
//! 1. **1-bit Two-Value Mask (`span <= 64` on 64-bit, `<= 32` on 32-bit)**:
//!    When all in-range slots `[min_case ..= max_case]` choose between two
//!    constants `{v0, v1}`, packs membership into a single immediate `mask`:
//!      `idx = (word)val - (word)min_case`
//!      `in_range = idx <_u span`
//!      `safe_idx = Select(in_range, idx, 0)`
//!      `bit = (mask >>_u safe_idx) & 1`
//!    - For boolean `{0, 1}` with `default == 0` (character classifiers, token
//!      predicates, state-set checks), emits `Select(in_range, bit, 0)` — zero
//!      memory loads, zero jump tables, zero indirect branches.
//!    - For general `{v0, v1}` with arbitrary `default`, emits two `Select`s.
//!
//! 2. **2-bit / 4-bit / 8-bit Affine-Packed Immediate LUT (`span * k <= word_bits`,
//!    `k in {2, 4, 8}`)**:
//!    When all in-range constants lie within `[base, base + (2^k - 1)]`, packs
//!    all `span` `k`-bit unsigned deltas into a single 64-bit (or 32-bit on
//!    ILP32) immediate constant:
//!      `shift = safe_idx << log2(k)`
//!      `delta = (packed >>_u shift) & ((1 << k) - 1)`
//!      `in_val = base + delta`
//!      `res = Select(in_range, in_val, default_val)`
//!
//! Pass name for `CCC_DISABLE_PASSES`: `"switch_lut"`.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::source::Span;
use crate::common::types::IrType;
use crate::ir::reexports::{
    BasicBlock, BlockId, Instruction, IrBinOp, IrCmpOp, IrConst, IrFunction, Operand, Terminator,
    Value,
};

#[derive(Clone, Copy)]
enum SinkKind {
    DirectReturn { ret_ty: IrType },
    JoinPhi { join_block: BlockId, phi_ty: IrType },
}

enum LutEncoding {
    /// Two-value bitmask: slot `i` yields `v1` if `(mask >> i) & 1 == 1`, else `v0`.
    TwoVal {
        mask: u64,
        v0: i64,
        v1: i64,
        def_val: i64,
    },
    /// `k`-bit (`k` in `{2, 4, 8}`) packed unsigned deltas from `base`.
    Packed {
        packed: u64,
        shift_log2: u32,
        entry_mask: u64,
        base: i64,
        def_val: i64,
    },
}

struct SwitchPlan {
    block_idx: usize,
    val: Operand,
    sw_ty: IrType,
    min_case: i64,
    span: u64,
    sink: SinkKind,
    encoding: LutEncoding,
    absorbed_sources: FxHashSet<BlockId>,
}

/// Run the register bit-packed switch LUT pass across one function.
pub(crate) fn run_function(func: &mut IrFunction, is_32bit: bool) -> usize {
    if func.blocks.len() < 4 {
        return 0;
    }
    let word_bits: u64 = if is_32bit { 32 } else { 64 };
    let mut next_vid = func.max_value_id().saturating_add(1);
    let mut changes = 0usize;

    loop {
        let Some(plan) = find_next_plan(func, word_bits) else {
            break;
        };
        apply_plan(func, plan, is_32bit, &mut next_vid);
        changes += 1;
    }

    if changes > 0 {
        func.next_value_id = next_vid;
    }
    changes
}

fn find_next_plan(func: &IrFunction, word_bits: u64) -> Option<SwitchPlan> {
    let mut label_to_idx: FxHashMap<BlockId, usize> =
        FxHashMap::with_capacity_and_hasher(func.blocks.len(), Default::default());
    for (idx, b) in func.blocks.iter().enumerate() {
        label_to_idx.insert(b.label, idx);
    }

    let mut pred_edges: FxHashMap<BlockId, usize> =
        FxHashMap::with_capacity_and_hasher(func.blocks.len(), Default::default());
    for b in &func.blocks {
        match &b.terminator {
            Terminator::Branch(t) => *pred_edges.entry(*t).or_insert(0) += 1,
            Terminator::CondBranch {
                true_label,
                false_label,
                ..
            } => {
                *pred_edges.entry(*true_label).or_insert(0) += 1;
                *pred_edges.entry(*false_label).or_insert(0) += 1;
            }
            Terminator::Switch { cases, default, .. } => {
                *pred_edges.entry(*default).or_insert(0) += 1;
                for (_, t) in cases {
                    *pred_edges.entry(*t).or_insert(0) += 1;
                }
            }
            Terminator::IndirectBranch {
                possible_targets, ..
            } => {
                for t in possible_targets {
                    *pred_edges.entry(*t).or_insert(0) += 1;
                }
            }
            Terminator::Return(_) | Terminator::Unreachable => {}
        }
    }

    for (block_idx, block) in func.blocks.iter().enumerate() {
        let Terminator::Switch {
            val,
            cases,
            default,
            ty: sw_ty,
        } = &block.terminator
        else {
            continue;
        };
        if cases.len() < 3
            || matches!(val, Operand::Const(_))
            || !sw_ty.is_integer()
            || sw_ty.is_128bit()
        {
            continue;
        }
        if let Some(plan) = analyze_switch(
            func,
            block_idx,
            block.label,
            *val,
            cases,
            *default,
            *sw_ty,
            word_bits,
            &label_to_idx,
            &pred_edges,
        ) {
            return Some(plan);
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn analyze_switch(
    func: &IrFunction,
    block_idx: usize,
    sw_label: BlockId,
    val: Operand,
    cases: &[(i64, BlockId)],
    default: BlockId,
    sw_ty: IrType,
    word_bits: u64,
    label_to_idx: &FxHashMap<BlockId, usize>,
    pred_edges: &FxHashMap<BlockId, usize>,
) -> Option<SwitchPlan> {
    let mut norm_cases: Vec<(i64, BlockId)> = Vec::with_capacity(cases.len());
    for &(raw_c, target) in cases {
        norm_cases.push((const_to_i64_typed(&IrConst::I64(raw_c), sw_ty)?, target));
    }
    let cases = norm_cases.as_slice();

    let mut min_c = cases[0].0;
    let mut max_c = cases[0].0;
    let is_unsigned = sw_ty.is_unsigned();
    for &(raw_c, _) in &cases[1..] {
        if is_unsigned {
            if (raw_c as u64) < (min_c as u64) {
                min_c = raw_c;
            }
            if (raw_c as u64) > (max_c as u64) {
                max_c = raw_c;
            }
        } else {
            if raw_c < min_c {
                min_c = raw_c;
            }
            if raw_c > max_c {
                max_c = raw_c;
            }
        }
    }

    let span_u128 = if is_unsigned {
        (max_c as u64 as u128)
            .checked_sub(min_c as u64 as u128)?
            .checked_add(1)?
    } else {
        ((max_c as i128) - (min_c as i128)).checked_add(1)? as u128
    };
    if span_u128 == 0 || span_u128 > word_bits as u128 {
        return None;
    }
    let span = span_u128 as u64;

    // Try Subform A1 (DirectReturn) or Subform A2 (JoinPhi).
    let (sink, def_val, case_vals, absorbed_sources) = if let Some((ret_ty, dv, cvs)) =
        try_collect_direct_returns(func, cases, default, label_to_idx)
    {
        (
            SinkKind::DirectReturn { ret_ty },
            dv,
            cvs,
            FxHashSet::default(),
        )
    } else {
        let (join_block, phi_ty, dv, cvs, absorbed) =
            try_collect_join_phi(func, sw_label, cases, default, label_to_idx, pred_edges)?;
        (SinkKind::JoinPhi { join_block, phi_ty }, dv, cvs, absorbed)
    };

    let res_ty = match sink {
        SinkKind::DirectReturn { ret_ty } => ret_ty,
        SinkKind::JoinPhi { phi_ty, .. } => phi_ty,
    };
    if !res_ty.is_integer() || res_ty.is_128bit() {
        return None;
    }

    let mut table = vec![def_val; span as usize];
    for (&(raw_c, _), &cv) in cases.iter().zip(case_vals.iter()) {
        let offset = if is_unsigned {
            (raw_c as u64).wrapping_sub(min_c as u64)
        } else {
            ((raw_c as i128) - (min_c as i128)) as u64
        };
        if offset >= span {
            return None;
        }
        table[offset as usize] = cv;
    }

    let encoding = choose_encoding(&table, def_val, word_bits)?;
    Some(SwitchPlan {
        block_idx,
        val,
        sw_ty,
        min_case: min_c,
        span,
        sink,
        encoding,
        absorbed_sources,
    })
}

fn const_to_i64_typed(c: &IrConst, ty: IrType) -> Option<i64> {
    if !ty.is_integer() || ty.is_128bit() {
        return None;
    }
    let raw = c.to_i64()?;
    Some(match ty {
        IrType::I8 => (raw as i8) as i64,
        IrType::U8 => (raw as u8) as i64,
        IrType::I16 => (raw as i16) as i64,
        IrType::U16 => (raw as u16) as i64,
        IrType::I32 => (raw as i32) as i64,
        IrType::U32 => (raw as u32) as i64,
        IrType::I64 | IrType::U64 => raw,
        _ => return None,
    })
}

fn block_const_return(b: &BasicBlock, ret_ty: IrType) -> Option<i64> {
    if !b.instructions.is_empty() {
        return None;
    }
    match &b.terminator {
        Terminator::Return(Some(Operand::Const(c))) => const_to_i64_typed(c, ret_ty),
        _ => None,
    }
}

fn try_collect_direct_returns(
    func: &IrFunction,
    cases: &[(i64, BlockId)],
    default: BlockId,
    label_to_idx: &FxHashMap<BlockId, usize>,
) -> Option<(IrType, i64, Vec<i64>)> {
    let ret_ty = func.return_type;
    if !ret_ty.is_integer() || ret_ty.is_128bit() {
        return None;
    }
    let def_b = &func.blocks[*label_to_idx.get(&default)?];
    let def_val = block_const_return(def_b, ret_ty)?;
    let mut vals = Vec::with_capacity(cases.len());
    for &(_, target) in cases {
        let tb = &func.blocks[*label_to_idx.get(&target)?];
        vals.push(block_const_return(tb, ret_ty)?);
    }
    Some((ret_ty, def_val, vals))
}

fn resolve_join_target(
    func: &IrFunction,
    sw_label: BlockId,
    arm_target: BlockId,
    expected_join: Option<BlockId>,
    arm_edge_counts: &FxHashMap<BlockId, usize>,
    label_to_idx: &FxHashMap<BlockId, usize>,
    pred_edges: &FxHashMap<BlockId, usize>,
) -> Option<(BlockId, BlockId)> {
    let tb = &func.blocks[*label_to_idx.get(&arm_target)?];
    if let Some(j) = expected_join {
        if arm_target == j {
            return Some((j, sw_label));
        }
    }
    if tb.instructions.is_empty()
        && let Terminator::Branch(j) = tb.terminator
    {
        if expected_join.is_none_or(|ej| ej == j) {
            // Forwarding block must ONLY be reachable from this switch's arms.
            let total_preds = pred_edges.get(&arm_target).copied().unwrap_or(0);
            let from_switch = arm_edge_counts.get(&arm_target).copied().unwrap_or(0);
            if total_preds == from_switch {
                return Some((j, arm_target));
            }
        }
    }
    if expected_join.is_none() {
        // `arm_target` itself might be the join block (when default jumps directly to join).
        return Some((arm_target, sw_label));
    }
    None
}

fn try_collect_join_phi(
    func: &IrFunction,
    sw_label: BlockId,
    cases: &[(i64, BlockId)],
    default: BlockId,
    label_to_idx: &FxHashMap<BlockId, usize>,
    pred_edges: &FxHashMap<BlockId, usize>,
) -> Option<(BlockId, IrType, i64, Vec<i64>, FxHashSet<BlockId>)> {
    let mut arm_edge_counts: FxHashMap<BlockId, usize> = FxHashMap::default();
    *arm_edge_counts.entry(default).or_insert(0) += 1;
    for &(_, t) in cases {
        *arm_edge_counts.entry(t).or_insert(0) += 1;
    }

    // Discover candidate join block from default or first case.
    let mut candidate_joins = Vec::with_capacity(2);
    let def_b = &func.blocks[*label_to_idx.get(&default)?];
    candidate_joins.push(default);
    if def_b.instructions.is_empty()
        && let Terminator::Branch(j) = def_b.terminator
    {
        candidate_joins.push(j);
    }

    for join_block in candidate_joins {
        if join_block == sw_label {
            continue;
        }
        let jb = &func.blocks[*label_to_idx.get(&join_block)?];
        let mut phis = jb.instructions.iter().filter_map(|i| match i {
            Instruction::Phi { ty, incoming, .. } => Some((*ty, incoming)),
            _ => None,
        });
        let Some((phi_ty, incoming)) = phis.next() else {
            continue;
        };
        if phis.next().is_some() || !phi_ty.is_integer() || phi_ty.is_128bit() {
            continue;
        }

        let Some((_, def_src)) = resolve_join_target(
            func,
            sw_label,
            default,
            Some(join_block),
            &arm_edge_counts,
            label_to_idx,
            pred_edges,
        ) else {
            continue;
        };

        let lookup_phi_const = |src_bb: BlockId| -> Option<i64> {
            let (op, _) = incoming.iter().find(|(_, b)| *b == src_bb)?;
            match op {
                Operand::Const(c) => const_to_i64_typed(c, phi_ty),
                _ => None,
            }
        };

        let Some(def_val) = lookup_phi_const(def_src) else {
            continue;
        };
        let mut absorbed: FxHashSet<BlockId> = FxHashSet::default();
        absorbed.insert(def_src);
        let mut vals = Vec::with_capacity(cases.len());
        let mut all_ok = true;
        for &(_, t) in cases {
            let Some((_, src_bb)) = resolve_join_target(
                func,
                sw_label,
                t,
                Some(join_block),
                &arm_edge_counts,
                label_to_idx,
                pred_edges,
            ) else {
                all_ok = false;
                break;
            };
            let Some(cv) = lookup_phi_const(src_bb) else {
                all_ok = false;
                break;
            };
            absorbed.insert(src_bb);
            vals.push(cv);
        }
        if all_ok {
            return Some((join_block, phi_ty, def_val, vals, absorbed));
        }
    }
    None
}

fn choose_encoding(table: &[i64], def_val: i64, word_bits: u64) -> Option<LutEncoding> {
    let span = table.len() as u64;
    if span == 0 || span > word_bits {
        return None;
    }

    // 1. Check if table has <= 2 distinct values (prefer `v0 == def_val` when possible).
    let mut distinct: Vec<i64> = Vec::with_capacity(3);
    for &v in table {
        if !distinct.contains(&v) {
            distinct.push(v);
            if distinct.len() > 2 {
                break;
            }
        }
    }
    if distinct.len() <= 2 {
        let (v0, v1) = match distinct.as_slice() {
            [only] => (*only, *only),
            [a, b] if *b == def_val => (*b, *a),
            [a, b] => (*a, *b),
            _ => return None,
        };
        let mut mask = 0u64;
        for (i, &v) in table.iter().enumerate() {
            if v == v1 && v1 != v0 {
                mask |= 1u64 << i;
            }
        }
        return Some(LutEncoding::TwoVal {
            mask,
            v0,
            v1,
            def_val,
        });
    }

    // 2. Check 2-bit, 4-bit, or 8-bit affine-packed table (`val_i = base + delta_i`).
    let min_v = *table.iter().min()?;
    let max_v = *table.iter().max()?;
    let range = (max_v as i128).checked_sub(min_v as i128)?;
    if range < 0 {
        return None;
    }
    let (bits_per_entry, shift_log2): (u64, u32) = if range <= 3 {
        (2, 1)
    } else if range <= 15 {
        (4, 2)
    } else if range <= 255 {
        (8, 3)
    } else {
        return None;
    };
    if span.checked_mul(bits_per_entry)? > word_bits {
        return None;
    }
    let entry_mask = (1u64 << bits_per_entry) - 1;
    let mut packed = 0u64;
    for (i, &v) in table.iter().enumerate() {
        let delta = ((v as i128) - (min_v as i128)) as u64;
        packed |= (delta & entry_mask) << (i as u64 * bits_per_entry);
    }
    Some(LutEncoding::Packed {
        packed,
        shift_log2,
        entry_mask,
        base: min_v,
        def_val,
    })
}

fn make_typed_const(val: i64, ty: IrType) -> IrConst {
    match ty {
        IrType::I8 | IrType::U8 => IrConst::I8(val as i8),
        IrType::I16 | IrType::U16 => IrConst::I16(val as i16),
        IrType::I32 | IrType::U32 => IrConst::I32(val as i32),
        _ => IrConst::I64(val),
    }
}

fn push_inst(block: &mut BasicBlock, tracked_spans: bool, tail_span: Span, inst: Instruction) {
    block.instructions.push(inst);
    if tracked_spans {
        block.source_spans.push(tail_span);
    }
}

fn fresh(next_vid: &mut u32) -> Value {
    let v = Value(*next_vid);
    *next_vid += 1;
    v
}

fn apply_plan(func: &mut IrFunction, plan: SwitchPlan, is_32bit: bool, next_vid: &mut u32) {
    let word_ty = if is_32bit { IrType::U32 } else { IrType::U64 };
    let word_const = |v: u64| -> Operand {
        Operand::Const(if is_32bit {
            IrConst::I32(v as u32 as i32)
        } else {
            IrConst::I64(v as i64)
        })
    };
    let res_ty = match plan.sink {
        SinkKind::DirectReturn { ret_ty } => ret_ty,
        SinkKind::JoinPhi { phi_ty, .. } => phi_ty,
    };

    let sw_label = func.blocks[plan.block_idx].label;
    let sw_val = {
        let block = &mut func.blocks[plan.block_idx];
        let tracked = !block.source_spans.is_empty();
        let span = block
            .source_spans
            .last()
            .copied()
            .unwrap_or_else(Span::dummy);

        // Cast switch operand (preserving C signedness to word width first when
        // widening a signed narrow type like `char` -> `int` -> `u64`!).
        let val_word = if plan.sw_ty == word_ty {
            plan.val
        } else if plan.sw_ty.size() < word_ty.size() && plan.sw_ty.is_signed() {
            let signed_word = if is_32bit { IrType::I32 } else { IrType::I64 };
            let sext = fresh(next_vid);
            push_inst(
                block,
                tracked,
                span,
                Instruction::Cast {
                    dest: sext,
                    src: plan.val,
                    from_ty: plan.sw_ty,
                    to_ty: signed_word,
                },
            );
            let u = fresh(next_vid);
            push_inst(
                block,
                tracked,
                span,
                Instruction::Cast {
                    dest: u,
                    src: Operand::Value(sext),
                    from_ty: signed_word,
                    to_ty: word_ty,
                },
            );
            Operand::Value(u)
        } else {
            let u = fresh(next_vid);
            push_inst(
                block,
                tracked,
                span,
                Instruction::Cast {
                    dest: u,
                    src: plan.val,
                    from_ty: plan.sw_ty,
                    to_ty: word_ty,
                },
            );
            Operand::Value(u)
        };

        let min_u = if is_32bit {
            plan.min_case as u32 as u64
        } else {
            plan.min_case as u64
        };
        let idx = if min_u == 0 {
            val_word
        } else {
            let d = fresh(next_vid);
            push_inst(
                block,
                tracked,
                span,
                Instruction::BinOp {
                    dest: d,
                    op: IrBinOp::Sub,
                    lhs: val_word,
                    rhs: word_const(min_u),
                    ty: word_ty,
                },
            );
            Operand::Value(d)
        };

        // For 64-bit switch types on 32-bit targets, `plan.span <= 32`, and if
        // `plan.sw_ty` was 64-bit on 32-bit we must ensure high bits match!
        // Guard against `sw_ty.size() > word_ty.size()` by computing `idx` in `U64`
        // for range test, or simply perform range check in `U64` when `sw_ty` is 64-bit.
        let in_range = if is_32bit && plan.sw_ty.size() == 8 {
            let u64_val = fresh(next_vid);
            push_inst(
                block,
                tracked,
                span,
                Instruction::Cast {
                    dest: u64_val,
                    src: plan.val,
                    from_ty: plan.sw_ty,
                    to_ty: IrType::U64,
                },
            );
            let idx64 = fresh(next_vid);
            push_inst(
                block,
                tracked,
                span,
                Instruction::BinOp {
                    dest: idx64,
                    op: IrBinOp::Sub,
                    lhs: Operand::Value(u64_val),
                    rhs: Operand::Const(IrConst::I64(plan.min_case)),
                    ty: IrType::U64,
                },
            );
            let cmp = fresh(next_vid);
            push_inst(
                block,
                tracked,
                span,
                Instruction::Cmp {
                    dest: cmp,
                    op: IrCmpOp::Ult,
                    lhs: Operand::Value(idx64),
                    rhs: Operand::Const(IrConst::I64(plan.span as i64)),
                    ty: IrType::U64,
                },
            );
            cmp
        } else {
            let cmp = fresh(next_vid);
            push_inst(
                block,
                tracked,
                span,
                Instruction::Cmp {
                    dest: cmp,
                    op: IrCmpOp::Ult,
                    lhs: idx,
                    rhs: word_const(plan.span),
                    ty: word_ty,
                },
            );
            cmp
        };

        let safe_idx = fresh(next_vid);
        push_inst(
            block,
            tracked,
            span,
            Instruction::Select {
                dest: safe_idx,
                cond: Operand::Value(in_range),
                true_val: idx,
                false_val: word_const(0),
                ty: word_ty,
            },
        );

        let final_val = match plan.encoding {
            LutEncoding::TwoVal {
                mask,
                v0,
                v1,
                def_val,
            } => emit_two_val_lookup(
                block, tracked, span, is_32bit, word_ty, res_ty, in_range, safe_idx, mask, v0, v1,
                def_val, next_vid,
            ),
            LutEncoding::Packed {
                packed,
                shift_log2,
                entry_mask,
                base,
                def_val,
            } => emit_packed_lookup(
                block, tracked, span, is_32bit, word_ty, res_ty, in_range, safe_idx, packed,
                shift_log2, entry_mask, base, def_val, next_vid,
            ),
        };

        block.terminator = match plan.sink {
            SinkKind::DirectReturn { .. } => Terminator::Return(Some(Operand::Value(final_val))),
            SinkKind::JoinPhi { join_block, .. } => Terminator::Branch(join_block),
        };
        final_val
    };

    if let SinkKind::JoinPhi { join_block, .. } = plan.sink {
        if let Some(jb) = func.blocks.iter_mut().find(|b| b.label == join_block) {
            for inst in &mut jb.instructions {
                if let Instruction::Phi { incoming, .. } = inst {
                    incoming.retain(|(_, bb)| !plan.absorbed_sources.contains(bb));
                    incoming.push((Operand::Value(sw_val), sw_label));
                }
            }
        }
    }
}

fn make_word_const(v: u64, is_32bit: bool) -> Operand {
    Operand::Const(if is_32bit {
        IrConst::I32(v as u32 as i32)
    } else {
        IrConst::I64(v as i64)
    })
}

#[allow(clippy::too_many_arguments)]
fn emit_two_val_lookup(
    block: &mut BasicBlock,
    tracked: bool,
    span: Span,
    is_32bit: bool,
    word_ty: IrType,
    res_ty: IrType,
    in_range: Value,
    safe_idx: Value,
    mask: u64,
    v0: i64,
    v1: i64,
    def_val: i64,
    next_vid: &mut u32,
) -> Value {
    if v0 == v1 {
        let res = fresh(next_vid);
        push_inst(
            block,
            tracked,
            span,
            Instruction::Select {
                dest: res,
                cond: Operand::Value(in_range),
                true_val: Operand::Const(make_typed_const(v0, res_ty)),
                false_val: Operand::Const(make_typed_const(def_val, res_ty)),
                ty: res_ty,
            },
        );
        return res;
    }

    let shr = fresh(next_vid);
    push_inst(
        block,
        tracked,
        span,
        Instruction::BinOp {
            dest: shr,
            op: IrBinOp::LShr,
            lhs: make_word_const(mask, is_32bit),
            rhs: Operand::Value(safe_idx),
            ty: word_ty,
        },
    );
    let bit = fresh(next_vid);
    push_inst(
        block,
        tracked,
        span,
        Instruction::BinOp {
            dest: bit,
            op: IrBinOp::And,
            lhs: Operand::Value(shr),
            rhs: make_word_const(1, is_32bit),
            ty: word_ty,
        },
    );
    if v0 == 0 && v1 == 1 && def_val == 0 {
        let casted = if word_ty == res_ty {
            bit
        } else {
            let c = fresh(next_vid);
            push_inst(
                block,
                tracked,
                span,
                Instruction::Cast {
                    dest: c,
                    src: Operand::Value(bit),
                    from_ty: word_ty,
                    to_ty: res_ty,
                },
            );
            c
        };
        let res = fresh(next_vid);
        push_inst(
            block,
            tracked,
            span,
            Instruction::Select {
                dest: res,
                cond: Operand::Value(in_range),
                true_val: Operand::Value(casted),
                false_val: Operand::Const(make_typed_const(0, res_ty)),
                ty: res_ty,
            },
        );
        res
    } else {
        let in_val = fresh(next_vid);
        push_inst(
            block,
            tracked,
            span,
            Instruction::Select {
                dest: in_val,
                cond: Operand::Value(bit),
                true_val: Operand::Const(make_typed_const(v1, res_ty)),
                false_val: Operand::Const(make_typed_const(v0, res_ty)),
                ty: res_ty,
            },
        );
        let res = fresh(next_vid);
        push_inst(
            block,
            tracked,
            span,
            Instruction::Select {
                dest: res,
                cond: Operand::Value(in_range),
                true_val: Operand::Value(in_val),
                false_val: Operand::Const(make_typed_const(def_val, res_ty)),
                ty: res_ty,
            },
        );
        res
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_packed_lookup(
    block: &mut BasicBlock,
    tracked: bool,
    span: Span,
    is_32bit: bool,
    word_ty: IrType,
    res_ty: IrType,
    in_range: Value,
    safe_idx: Value,
    packed: u64,
    shift_log2: u32,
    entry_mask: u64,
    base: i64,
    def_val: i64,
    next_vid: &mut u32,
) -> Value {
    let shift_amt = fresh(next_vid);
    push_inst(
        block,
        tracked,
        span,
        Instruction::BinOp {
            dest: shift_amt,
            op: IrBinOp::Shl,
            lhs: Operand::Value(safe_idx),
            rhs: make_word_const(shift_log2 as u64, is_32bit),
            ty: word_ty,
        },
    );
    let shr = fresh(next_vid);
    push_inst(
        block,
        tracked,
        span,
        Instruction::BinOp {
            dest: shr,
            op: IrBinOp::LShr,
            lhs: make_word_const(packed, is_32bit),
            rhs: Operand::Value(shift_amt),
            ty: word_ty,
        },
    );
    let delta = fresh(next_vid);
    push_inst(
        block,
        tracked,
        span,
        Instruction::BinOp {
            dest: delta,
            op: IrBinOp::And,
            lhs: Operand::Value(shr),
            rhs: make_word_const(entry_mask, is_32bit),
            ty: word_ty,
        },
    );
    let delta_typed = if word_ty == res_ty {
        delta
    } else {
        let c = fresh(next_vid);
        push_inst(
            block,
            tracked,
            span,
            Instruction::Cast {
                dest: c,
                src: Operand::Value(delta),
                from_ty: word_ty,
                to_ty: res_ty,
            },
        );
        c
    };
    let in_val = if base == 0 {
        delta_typed
    } else {
        let s = fresh(next_vid);
        push_inst(
            block,
            tracked,
            span,
            Instruction::BinOp {
                dest: s,
                op: IrBinOp::Add,
                lhs: Operand::Value(delta_typed),
                rhs: Operand::Const(make_typed_const(base, res_ty)),
                ty: res_ty,
            },
        );
        s
    };
    let res = fresh(next_vid);
    push_inst(
        block,
        tracked,
        span,
        Instruction::Select {
            dest: res,
            cond: Operand::Value(in_range),
            true_val: Operand::Value(in_val),
            false_val: Operand::Const(make_typed_const(def_val, res_ty)),
            ty: res_ty,
        },
    );
    res
}
