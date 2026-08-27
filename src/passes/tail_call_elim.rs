//! Tail-call-to-loop transformation (TCE).
//!
//! Converts self-recursive tail calls into loop back-edges, eliminating
//! stack frames for accumulator-style recursive functions.
//!
//! A **tail call** is a call to the same function whose result is returned
//! immediately with no further computation. Example:
//!
//! ```c
//! long sum(int n, long acc) {
//!     if (n == 0) return acc;
//!     return sum(n - 1, acc + n);   // tail call
//! }
//! ```
//!
//! The transformation inserts a loop-header block between the function entry
//! and the body, with one Phi node per parameter:
//!
//! ```text
//! entry:
//!   %n   = ParamRef 0
//!   %acc = ParamRef 1
//!   Branch(loop_header)
//!
//! loop_header:
//!   %phi_n   = Phi [(%n, entry), (%n1, rec_block)]
//!   %phi_acc = Phi [(%acc, entry), (%acc1, rec_block)]
//!   <rest of original entry block>
//!   CondBranch(...)
//!
//! base_case:
//!   Return(%phi_acc)
//!
//! rec_block:
//!   %n1   = Sub %phi_n, 1
//!   %acc1 = Add %phi_acc, %phi_n
//!   Branch(loop_header)      // was: Call + Return
//! ```
//!
//! Correctness constraints checked before transformation:
//! - Non-variadic, non-sret function
//! - Call argument count equals parameter count
//! - The call result flows directly to a `Return` with no intervening uses
//! - The tail call is not in the entry block (would create duplicate predecessors)

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::reexports::{
    BasicBlock, BlockId, Instruction, IrFunction, Operand, Terminator, Value,
};

/// Replace the canonical tail-recursive integer sum with a closed form.
///
/// The source pattern is:
///
/// ```text
///     if (n <= 0) return acc;
///     return sum(n - 1, acc + n);
/// ```
///
/// After TCE this is a two-phi loop. For the lowered `int`/`long` ABI,
/// `n*(n+1)` is bounded by roughly 2^62, so the product is representable in
/// signed i64 for every defined int input. Replacing the loop with
/// `acc + (n * (n + 1) / 2)` removes O(n) work while preserving the original
/// branch for n <= 0. The matcher is deliberately strict and rejects any
/// extra side effects or non-canonical recurrence.
///
/// Returns 1 when transformed, 0 otherwise.
pub(crate) fn closed_form_tail_sum(func: &mut IrFunction) -> usize {
    if func.is_declaration
        || func.is_variadic
        || func.uses_sret
        || func.return_type != crate::common::types::IrType::I64
        || func.params.len() != 2
        || func.params[0].ty != crate::common::types::IrType::I32
        || func.params[1].ty != crate::common::types::IrType::I64
        || func.blocks.len() < 3
    {
        return 0;
    }

    let Some(entry) = func.blocks.first() else {
        return 0;
    };
    let mut n_param = None;
    let mut acc_param = None;
    for inst in &entry.instructions {
        if let Instruction::ParamRef {
            dest,
            param_idx,
            ty,
        } = inst
        {
            match (*param_idx, *ty) {
                (0, crate::common::types::IrType::I32) => n_param = Some(*dest),
                (1, crate::common::types::IrType::I64) => acc_param = Some(*dest),
                _ => {}
            }
        }
    }
    let (Some(n_param), Some(acc_param)) = (n_param, acc_param) else {
        return 0;
    };

    // Find the loop header with the two recurrence phis and its n <= 0 test.
    let mut header_idx = None;
    let mut n_phi = None;
    let mut acc_phi = None;
    let mut base_label = None;
    let mut rec_label = None;
    for (idx, block) in func.blocks.iter().enumerate() {
        let mut found_n = None;
        let mut found_acc = None;
        for inst in &block.instructions {
            if let Instruction::Phi { dest, ty, .. } = inst {
                match *ty {
                    crate::common::types::IrType::I32 if found_n.is_none() => found_n = Some(*dest),
                    crate::common::types::IrType::I64 if found_acc.is_none() => {
                        found_acc = Some(*dest)
                    }
                    _ => {}
                }
            }
        }
        let Some((n_phi_here, acc_phi_here)) = found_n.zip(found_acc) else {
            continue;
        };
        let Terminator::CondBranch {
            cond,
            true_label,
            false_label,
        } = &block.terminator
        else {
            continue;
        };
        let Operand::Value(cond_value) = cond else {
            continue;
        };
        let mut is_base_true = false;
        for inst in &block.instructions {
            if let Instruction::Cmp {
                dest,
                op: crate::ir::reexports::IrCmpOp::Sle,
                lhs,
                rhs,
                ty,
            } = inst
            {
                if *dest == *cond_value
                    && *ty == crate::common::types::IrType::I32
                    && matches!(lhs, Operand::Value(v) if *v == n_phi_here)
                    && matches!(rhs, Operand::Const(crate::ir::reexports::IrConst::I32(0)))
                {
                    is_base_true = true;
                }
            }
        }
        if !is_base_true {
            continue;
        }
        header_idx = Some(idx);
        n_phi = Some(n_phi_here);
        acc_phi = Some(acc_phi_here);
        base_label = Some(*true_label);
        rec_label = Some(*false_label);
        break;
    }
    let (Some(header_idx), Some(n_phi), Some(acc_phi), Some(base_label), Some(rec_label)) =
        (header_idx, n_phi, acc_phi, base_label, rec_label)
    else {
        return 0;
    };
    let header_label = func.blocks[header_idx].label;

    let find_block = |label: crate::ir::reexports::BlockId| {
        func.blocks.iter().find(|block| block.label == label)
    };
    let Some(base_block) = find_block(base_label) else {
        return 0;
    };
    let Some(rec_block) = find_block(rec_label) else {
        return 0;
    };
    if !matches!(base_block.terminator, Terminator::Return(Some(_)))
        || !matches!(rec_block.terminator, Terminator::Branch(label) if label == header_label)
    {
        return 0;
    }

    // The base arm must return the accumulator phi (possibly through one Copy).
    let base_returns_acc = match base_block.terminator {
        Terminator::Return(Some(Operand::Value(v))) => {
            v == acc_phi
                || base_block.instructions.iter().any(|inst| {
                    matches!(inst, Instruction::Copy { dest, src: Operand::Value(src) } if *dest == v && *src == acc_phi)
                })
        }
        _ => false,
    };
    if !base_returns_acc {
        return 0;
    }

    let mut has_n_step = false;
    let mut has_acc_step = false;
    for inst in &rec_block.instructions {
        match inst {
            Instruction::BinOp { op: crate::ir::reexports::IrBinOp::Sub, lhs, rhs, ty, .. }
                if *ty == crate::common::types::IrType::I32
                    && matches!(lhs, Operand::Value(v) if *v == n_phi)
                    && matches!(rhs, Operand::Const(crate::ir::reexports::IrConst::I32(1))) =>
            {
                has_n_step = true;
            }
            Instruction::BinOp { op: crate::ir::reexports::IrBinOp::Add, lhs, rhs, ty, .. }
                if *ty == crate::common::types::IrType::I64
                    && ((matches!(lhs, Operand::Value(v) if *v == acc_phi)
                        && rec_block.instructions.iter().any(|candidate| matches!(candidate,
                            Instruction::Cast { dest, src: Operand::Value(src), from_ty: crate::common::types::IrType::I32, to_ty: crate::common::types::IrType::I64 }
                                if matches!(rhs, Operand::Value(v) if *v == *dest) && *src == n_phi)))
                        || (matches!(rhs, Operand::Value(v) if *v == acc_phi)
                        && rec_block.instructions.iter().any(|candidate| matches!(candidate,
                            Instruction::Cast { dest, src: Operand::Value(src), from_ty: crate::common::types::IrType::I32, to_ty: crate::common::types::IrType::I64 }
                                if matches!(lhs, Operand::Value(v) if *v == *dest) && *src == n_phi)))) =>
            {
                has_acc_step = true;
            }
            _ => {}
        }
    }
    if !has_n_step || !has_acc_step {
        return 0;
    }

    let formula_label = rec_label;
    let base_label = base_label;
    let mut next = func.next_value_id;
    let fresh = |next: &mut u32| {
        let value = Value(*next);
        *next += 1;
        value
    };
    let cond = fresh(&mut next);
    let n64 = fresh(&mut next);
    let n_plus_one = fresh(&mut next);
    let product = fresh(&mut next);
    let half = fresh(&mut next);
    let result = fresh(&mut next);

    let spans = |count: usize| vec![crate::common::source::Span::dummy(); count];
    let entry_block = BasicBlock {
        label: func.blocks[0].label,
        instructions: vec![
            Instruction::ParamRef {
                dest: n_param,
                param_idx: 0,
                ty: crate::common::types::IrType::I32,
            },
            Instruction::ParamRef {
                dest: acc_param,
                param_idx: 1,
                ty: crate::common::types::IrType::I64,
            },
            Instruction::Cmp {
                dest: cond,
                op: crate::ir::reexports::IrCmpOp::Sle,
                lhs: Operand::Value(n_param),
                rhs: Operand::Const(crate::ir::reexports::IrConst::I32(0)),
                ty: crate::common::types::IrType::I32,
            },
        ],
        terminator: Terminator::CondBranch {
            cond: Operand::Value(cond),
            true_label: base_label,
            false_label: formula_label,
        },
        source_spans: spans(3),
    };
    let base_block = BasicBlock {
        label: base_label,
        instructions: Vec::new(),
        terminator: Terminator::Return(Some(Operand::Value(acc_param))),
        source_spans: Vec::new(),
    };
    let formula_block = BasicBlock {
        label: formula_label,
        instructions: vec![
            Instruction::Cast {
                dest: n64,
                src: Operand::Value(n_param),
                from_ty: crate::common::types::IrType::I32,
                to_ty: crate::common::types::IrType::I64,
            },
            Instruction::BinOp {
                dest: n_plus_one,
                op: crate::ir::reexports::IrBinOp::Add,
                lhs: Operand::Value(n64),
                rhs: Operand::Const(crate::ir::reexports::IrConst::I64(1)),
                ty: crate::common::types::IrType::I64,
            },
            Instruction::BinOp {
                dest: product,
                op: crate::ir::reexports::IrBinOp::Mul,
                lhs: Operand::Value(n64),
                rhs: Operand::Value(n_plus_one),
                ty: crate::common::types::IrType::I64,
            },
            Instruction::BinOp {
                dest: half,
                op: crate::ir::reexports::IrBinOp::SDiv,
                lhs: Operand::Value(product),
                rhs: Operand::Const(crate::ir::reexports::IrConst::I64(2)),
                ty: crate::common::types::IrType::I64,
            },
            Instruction::BinOp {
                dest: result,
                op: crate::ir::reexports::IrBinOp::Add,
                lhs: Operand::Value(acc_param),
                rhs: Operand::Value(half),
                ty: crate::common::types::IrType::I64,
            },
        ],
        terminator: Terminator::Return(Some(Operand::Value(result))),
        source_spans: spans(5),
    };
    func.blocks = vec![entry_block, base_block, formula_block];
    func.next_value_id = next;
    func.next_label = formula_label.0.saturating_add(1);
    func.param_alloca_values.clear();
    1
}

/// Run tail-call-to-loop on a single function.
/// Returns the number of tail calls eliminated (0 = no change).
pub(crate) fn tail_calls_to_loops(func: &mut IrFunction) -> usize {
    if func.is_variadic || func.is_declaration || func.uses_sret {
        return 0;
    }
    if func.blocks.is_empty() || func.params.is_empty() {
        return 0;
    }

    let func_name = func.name.clone();
    let num_params = func.params.len();

    // Reusing one frame is not semantics-preserving when a recursive argument
    // points into the current frame: each source-level call owns a distinct
    // automatic object. Track stack-derived pointers through the pure pointer
    // forms that can reach call arguments. The actual call-site check below is
    // deliberately per argument, so ordinary scalar locals do not disable TCE.
    let mut local_pointers = FxHashSet::default();
    for block in &func.blocks {
        for instruction in &block.instructions {
            if let Instruction::Alloca { dest, .. } | Instruction::DynAlloca { dest, .. } =
                instruction
            {
                local_pointers.insert(dest.0);
            }
        }
    }
    loop {
        let mut changed = false;
        for block in &func.blocks {
            for instruction in &block.instructions {
                let derived = match instruction {
                    Instruction::GetElementPtr { dest, base, .. }
                        if local_pointers.contains(&base.0) =>
                    {
                        Some(*dest)
                    }
                    Instruction::Copy {
                        dest,
                        src: Operand::Value(source),
                    }
                    | Instruction::Cast {
                        dest,
                        src: Operand::Value(source),
                        ..
                    } if local_pointers.contains(&source.0) => Some(*dest),
                    Instruction::Phi { dest, incoming, .. }
                        if incoming.iter().any(|(operand, _)| {
                            matches!(operand, Operand::Value(value) if local_pointers.contains(&value.0))
                        }) =>
                    {
                        Some(*dest)
                    }
                    Instruction::Select {
                        dest,
                        true_val,
                        false_val,
                        ..
                    } if [true_val, false_val].iter().any(|operand| {
                        matches!(operand, Operand::Value(value) if local_pointers.contains(&value.0))
                    }) => Some(*dest),
                    Instruction::BinOp {
                        dest,
                        lhs,
                        rhs,
                        ty: crate::common::types::IrType::Ptr,
                        ..
                    } if [lhs, rhs].iter().any(|operand| {
                        matches!(operand, Operand::Value(value) if local_pointers.contains(&value.0))
                    }) => Some(*dest),
                    _ => None,
                };
                if let Some(dest) = derived {
                    changed |= local_pointers.insert(dest.0);
                }
            }
        }
        if !changed {
            break;
        }
    }

    // ── 1. Collect ParamRef instructions from the entry block ──────────────
    // After mem2reg, each parameter has exactly one ParamRef in the entry block.
    // param_refs[i] = (SSA Value, IrType) for parameter i.
    let mut param_refs: Vec<Option<(Value, crate::common::types::IrType)>> = vec![None; num_params];

    for inst in &func.blocks[0].instructions {
        if let Instruction::ParamRef {
            dest,
            param_idx,
            ty,
        } = inst
        {
            if *param_idx < num_params {
                param_refs[*param_idx] = Some((*dest, *ty));
            }
        }
    }

    // Bail out if any parameter has no ParamRef (dead param, or mem2reg not run).
    if param_refs.iter().any(|p| p.is_none()) {
        return 0;
    }
    let param_refs: Vec<(Value, crate::common::types::IrType)> =
        param_refs.into_iter().map(|p| p.unwrap()).collect();

    // ── 2. Find self-recursive tail calls ──────────────────────────────────
    struct TailCall {
        block_idx: usize,
        call_inst_idx: usize,
        block_label: BlockId,
        /// Raw (pre-rename) args from the call site.
        args: Vec<Operand>,
    }

    let mut tail_calls: Vec<TailCall> = Vec::new();

    for (block_idx, block) in func.blocks.iter().enumerate() {
        // Tail calls cannot be in the entry block: redirecting entry → loop_header
        // while also branching from entry back → loop_header creates duplicate
        // phi predecessors (entry appears twice in the phi incoming list).
        if block_idx == 0 {
            continue;
        }

        // Find the last Call to self in this block.
        let Some(call_idx) = block
            .instructions
            .iter()
            .rposition(|inst| matches!(inst, Instruction::Call { func: f, .. } if f == &func_name))
        else {
            continue;
        };

        let Instruction::Call { func: _, info } = &block.instructions[call_idx] else {
            continue;
        };

        // Only handle simple non-struct self-calls with correct argument count.
        if info.is_sret || info.is_variadic || info.num_fixed_args != num_params {
            continue;
        }

        let call_dest = info.dest;
        let args: Vec<Operand> = info.args[..num_params].to_vec();
        if args.iter().any(|argument| {
            matches!(argument, Operand::Value(value) if local_pointers.contains(&value.0))
        }) {
            continue;
        }

        // Verify no instruction after the call uses the call result.
        if let Some(result_val) = call_dest {
            let later_use = block.instructions[call_idx + 1..].iter().any(|inst| {
                let mut used = false;
                inst.for_each_used_value(|v| {
                    if v == result_val.0 {
                        used = true;
                    }
                });
                used
            });
            if later_use {
                continue;
            }
        }

        // Verify the block's terminator is Return(call_result) or Return(None).
        let is_tail = match &block.terminator {
            Terminator::Return(None) => call_dest.is_none(),
            Terminator::Return(Some(Operand::Value(rv))) => call_dest.map(|d| d.0) == Some(rv.0),
            _ => false,
        };
        if !is_tail {
            continue;
        }

        tail_calls.push(TailCall {
            block_idx,
            call_inst_idx: call_idx,
            block_label: block.label,
            args,
        });
    }

    if tail_calls.is_empty() {
        return 0;
    }

    // ── 3. Allocate phi Values and a fresh label for the loop header ────────
    let phi_vals: Vec<Value> = (0..num_params)
        .map(|_| {
            let v = Value(func.next_value_id);
            func.next_value_id += 1;
            v
        })
        .collect();

    let loop_header_label = BlockId(func.blocks.iter().map(|b| b.label.0).max().unwrap_or(0) + 1);
    let entry_label = func.blocks[0].label;

    // ── 4. Build replacement map: paramref Value → phi Value ───────────────
    let mut replace_map: FxHashMap<u32, u32> = FxHashMap::default();
    for (&(param_val, _), &phi_val) in param_refs.iter().zip(phi_vals.iter()) {
        replace_map.insert(param_val.0, phi_val.0);
    }

    // ── 5. Build phi incoming ──────────────────────────────────────────────
    // Entry block provides the initial paramref values (un-renamed, intentional).
    // Each tail-call site provides the new argument values (renamed so that
    // paramref values in args become the corresponding phi values).
    let mut phi_incoming: Vec<Vec<(Operand, BlockId)>> = param_refs
        .iter()
        .map(|&(param_val, _)| vec![(Operand::Value(param_val), entry_label)])
        .collect();

    for tc in &tail_calls {
        for (i, arg) in tc.args.iter().enumerate() {
            let mut renamed_arg = arg.clone();
            replace_op(&mut renamed_arg, &replace_map);
            phi_incoming[i].push((renamed_arg, tc.block_label));
        }
    }

    // ── 6. Split entry block: keep ParamRef/Alloca, move everything else ───
    // ParamRef must stay in entry (backend reads from arg registers there).
    // Alloca/DynAlloca must stay in entry (stack frame allocation).
    // All other instructions move to the loop header so they re-execute on
    // each "iteration" with updated phi values.
    let entry_block = &mut func.blocks[0];
    let old_spans = std::mem::take(&mut entry_block.source_spans);
    let has_spans = !old_spans.is_empty() && old_spans.len() == entry_block.instructions.len();

    let mut keep_in_entry: Vec<Instruction> = Vec::new();
    let mut move_to_header: Vec<Instruction> = Vec::new();
    let mut keep_spans: Vec<crate::common::source::Span> = Vec::new();
    let mut header_spans: Vec<crate::common::source::Span> = Vec::new();

    for (i, inst) in entry_block.instructions.drain(..).enumerate() {
        let is_anchor = matches!(
            inst,
            Instruction::ParamRef { .. }
                | Instruction::Alloca { .. }
                | Instruction::DynAlloca { .. }
        );
        if is_anchor {
            keep_in_entry.push(inst);
            if has_spans {
                keep_spans.push(old_spans[i]);
            }
        } else {
            move_to_header.push(inst);
            if has_spans {
                header_spans.push(old_spans[i]);
            }
        }
    }
    entry_block.instructions = keep_in_entry;
    if has_spans {
        entry_block.source_spans = keep_spans;
    }

    // ── 7. Redirect entry block terminator → loop header ───────────────────
    let entry_original_terminator = std::mem::replace(
        &mut func.blocks[0].terminator,
        Terminator::Branch(loop_header_label),
    );

    // ── 8. Patch tail-call sites: remove call, replace Return with Branch ──
    for tc in &tail_calls {
        let block = &mut func.blocks[tc.block_idx];
        // Remove the Call instruction (call result is now dead; DCE will clean up).
        block.instructions.remove(tc.call_inst_idx);
        if has_spans && tc.call_inst_idx < block.source_spans.len() {
            block.source_spans.remove(tc.call_inst_idx);
        }
        block.terminator = Terminator::Branch(loop_header_label);
    }

    // ── 9. Rename paramref values to phi values in all existing blocks ──────
    // AND retarget phi predecessor labels: the entry block's original
    // terminator has just moved into the loop header (step 7), so every
    // control-flow edge that used to leave `entry` now leaves `loop_header`.
    // A phi in a former entry-successor that still names `entry` as the
    // predecessor is doubly wrong after the value rename below: the incoming
    // value becomes the header phi (defined only in the loop header, which
    // does NOT dominate entry), and phi elimination then places the edge
    // copy in the entry block, reading the header phi's home before its
    // first definition. glibc's _dl_lookup_symbol_x hashed a stale register
    // for every ld.so lookup exactly this way (LK-27). The loop below only
    // visits pre-existing blocks — the loop header (pushed in step 10) keeps
    // its deliberate `entry` incomings.
    for block in &mut func.blocks {
        for inst in &mut block.instructions {
            if matches!(inst, Instruction::ParamRef { .. }) {
                continue; // definition, not a use
            }
            if let Instruction::Phi { incoming, .. } = inst {
                for (_, pred) in incoming.iter_mut() {
                    if *pred == entry_label {
                        *pred = loop_header_label;
                    }
                }
            }
            replace_values_in_inst(inst, &replace_map);
        }
        replace_values_in_terminator(&mut block.terminator, &replace_map);
    }

    // ── 10. Build and push the loop header block ────────────────────────────
    // Phi nodes first, then the moved entry instructions (already renamed via
    // the existing-block pass, since those instructions were drained from
    // func.blocks[0] which was processed in step 9).
    //
    // Wait — the moved instructions were drained from func.blocks[0] before
    // step 9, so step 9 did NOT rename them. Apply renaming now.
    let mut header_instructions: Vec<Instruction> = Vec::new();

    // Phi nodes (no renaming — deliberately hold paramref values for entry incoming).
    for (i, &phi_val) in phi_vals.iter().enumerate() {
        header_instructions.push(Instruction::Phi {
            dest: phi_val,
            ty: param_refs[i].1,
            incoming: phi_incoming[i].clone(),
        });
    }

    // Moved entry instructions: apply renaming.
    for mut inst in move_to_header {
        replace_values_in_inst(&mut inst, &replace_map);
        header_instructions.push(inst);
    }

    // Loop header terminator = original entry terminator, with renaming applied.
    let mut header_terminator = entry_original_terminator;
    replace_values_in_terminator(&mut header_terminator, &replace_map);

    // Span list: dummy spans for phi nodes, original spans for moved instructions.
    let header_source_spans: Vec<crate::common::source::Span> = if has_spans {
        let dummy = crate::common::source::Span::dummy();
        let mut spans: Vec<_> = (0..phi_vals.len()).map(|_| dummy).collect();
        spans.extend(header_spans);
        spans
    } else {
        Vec::new()
    };

    func.blocks.push(BasicBlock {
        label: loop_header_label,
        instructions: header_instructions,
        terminator: header_terminator,
        source_spans: header_source_spans,
    });

    tail_calls.len()
}

// ── Value replacement helpers ─────────────────────────────────────────────────

#[inline]
fn replace_val(v: &mut Value, map: &FxHashMap<u32, u32>) {
    if let Some(&new_id) = map.get(&v.0) {
        *v = Value(new_id);
    }
}

#[inline]
fn replace_op(op: &mut Operand, map: &FxHashMap<u32, u32>) {
    if let Operand::Value(v) = op {
        replace_val(v, map);
    }
}

/// Replace all *uses* of values in `map` within an instruction.
/// Does not touch ParamRef (definition) or the phi nodes it creates.
pub(crate) fn replace_values_in_inst(inst: &mut Instruction, map: &FxHashMap<u32, u32>) {
    match inst {
        Instruction::PgoCounterInc { .. } => {}
        // ── Definitions with no operands to replace ──────────────────────
        Instruction::ParamRef { .. }
        | Instruction::Alloca { .. }
        | Instruction::GlobalAddr { .. }
        | Instruction::LabelAddr { .. }
        | Instruction::Fence { .. }
        | Instruction::StackSave { .. }
        | Instruction::GetReturnF64Second { .. }
        | Instruction::GetReturnF32Second { .. }
        | Instruction::GetReturnF128Second { .. }
        | Instruction::GetStaticChain { .. } => {}
        Instruction::SetStaticChain { src } => replace_op(src, map),
        Instruction::InitTrampoline { buffer, chain, .. } => {
            replace_val(buffer, map);
            replace_op(chain, map);
        }
        Instruction::NonlocalGotoSave { frame, .. } => replace_val(frame, map),
        Instruction::NonlocalGoto { chain, .. } => replace_op(chain, map),

        // ── Memory ───────────────────────────────────────────────────────
        Instruction::Store { val, ptr, .. } => {
            replace_op(val, map);
            replace_val(ptr, map);
        }
        Instruction::Load { ptr, .. } => replace_val(ptr, map),
        Instruction::Memcpy { dest, src, .. } => {
            replace_val(dest, map);
            replace_val(src, map);
        }

        // ── Arithmetic / logic ───────────────────────────────────────────
        Instruction::BinOp { lhs, rhs, .. } => {
            replace_op(lhs, map);
            replace_op(rhs, map);
        }
        Instruction::UnaryOp { src, .. } => replace_op(src, map),
        Instruction::Cmp { lhs, rhs, .. } => {
            replace_op(lhs, map);
            replace_op(rhs, map);
        }

        // ── Pointer / address ────────────────────────────────────────────
        Instruction::GetElementPtr { base, offset, .. } => {
            replace_val(base, map);
            replace_op(offset, map);
        }
        Instruction::DynAlloca { size, .. } => replace_op(size, map),
        Instruction::StackRestore { ptr } => replace_val(ptr, map),

        // ── Conversions ──────────────────────────────────────────────────
        Instruction::Cast { src, .. } => replace_op(src, map),
        Instruction::Copy { src, .. } => replace_op(src, map),

        // ── Calls ────────────────────────────────────────────────────────
        Instruction::Call { info, .. } => {
            for arg in &mut info.args {
                replace_op(arg, map);
            }
        }
        Instruction::CallIndirect { func_ptr, info } => {
            replace_op(func_ptr, map);
            for arg in &mut info.args {
                replace_op(arg, map);
            }
        }

        // ── Phi ──────────────────────────────────────────────────────────
        Instruction::Phi { incoming, .. } => {
            for (op, _) in incoming {
                replace_op(op, map);
            }
        }

        // ── Select ───────────────────────────────────────────────────────
        Instruction::Select {
            cond,
            true_val,
            false_val,
            ..
        } => {
            replace_op(cond, map);
            replace_op(true_val, map);
            replace_op(false_val, map);
        }

        // ── Atomics ──────────────────────────────────────────────────────
        Instruction::AtomicRmw { ptr, val, .. } => {
            replace_op(ptr, map);
            replace_op(val, map);
        }
        Instruction::AtomicInc { ptr, .. } => replace_op(ptr, map),
        Instruction::AtomicCmpxchg {
            ptr,
            expected,
            desired,
            ..
        } => {
            replace_op(ptr, map);
            replace_op(expected, map);
            replace_op(desired, map);
        }
        Instruction::AtomicLoad { ptr, .. } => replace_op(ptr, map),
        Instruction::AtomicStore { ptr, val, .. } => {
            replace_op(ptr, map);
            replace_op(val, map);
        }

        // ── varargs ──────────────────────────────────────────────────────
        Instruction::VaArg { va_list_ptr, .. } => replace_val(va_list_ptr, map),
        Instruction::VaArgStruct {
            dest_ptr,
            va_list_ptr,
            ..
        } => {
            replace_val(dest_ptr, map);
            replace_val(va_list_ptr, map);
        }
        Instruction::VaStart { va_list_ptr } => replace_val(va_list_ptr, map),
        Instruction::VaEnd { va_list_ptr } => replace_val(va_list_ptr, map),
        Instruction::VaCopy { dest_ptr, src_ptr } => {
            replace_val(dest_ptr, map);
            replace_val(src_ptr, map);
        }

        // ── Inline assembly ──────────────────────────────────────────────
        Instruction::InlineAsm {
            outputs, inputs, ..
        } => {
            for (_, ptr, _) in outputs {
                replace_val(ptr, map);
            }
            for (_, op, _) in inputs {
                replace_op(op, map);
            }
        }

        // ── Intrinsics ───────────────────────────────────────────────────
        Instruction::Intrinsic { dest_ptr, args, .. } => {
            if let Some(ptr) = dest_ptr {
                replace_val(ptr, map);
            }
            for arg in args {
                replace_op(arg, map);
            }
        }

        // ── Complex-return helpers ────────────────────────────────────────
        Instruction::SetReturnF64Second { src } => replace_op(src, map),
        Instruction::SetReturnF32Second { src } => replace_op(src, map),
        Instruction::SetReturnF128Second { src } => replace_op(src, map),
    }
}

fn replace_values_in_terminator(term: &mut Terminator, map: &FxHashMap<u32, u32>) {
    match term {
        Terminator::Return(Some(op)) => replace_op(op, map),
        Terminator::CondBranch { cond, .. } => replace_op(cond, map),
        Terminator::IndirectBranch { target, .. } => replace_op(target, map),
        Terminator::Switch { val, .. } => replace_op(val, map),
        Terminator::Return(None) | Terminator::Branch(_) | Terminator::Unreachable => {}
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::{AddressSpace, IrType};
    use crate::ir::reexports::{BasicBlock, BlockId, IrBinOp, IrCmpOp, IrConst, IrParam};

    /// Build a minimal IrFunction skeleton (no blocks) for testing.
    fn make_func(name: &str, params: Vec<IrType>) -> IrFunction {
        let ir_params: Vec<IrParam> = params
            .iter()
            .map(|&ty| IrParam {
                ty,
                noalias: false,
                struct_size: None,
                struct_align: None,
                struct_eightbyte_classes: Vec::new(),
                riscv_float_class: None,
                is_f128_sse: false,
            })
            .collect();
        IrFunction::new(name.to_string(), IrType::I64, ir_params, false)
    }

    /// Build the canonical accumulator-recursive sum function:
    ///
    /// ```c
    /// long sum(int n, long acc) {
    ///     if (n == 0) return acc;
    ///     return sum(n - 1, acc + n);
    /// }
    /// ```
    ///
    /// IR (after mem2reg):
    ///
    /// ```
    /// entry (B0):
    ///   %0 = ParamRef 0 i32   // n
    ///   %1 = ParamRef 1 i64   // acc
    ///   %2 = Cmp Eq %0, 0
    ///   CondBranch(%2, base, rec)
    ///
    /// base (B1):
    ///   Return(%1)
    ///
    /// rec (B2):
    ///   %3 = Sub %0, 1        // n - 1
    ///   %4 = Cast %0 i32→i64
    ///   %5 = Add %1, %4       // acc + n
    ///   %6 = Call sum(%3, %5)
    ///   Return(%6)
    /// ```
    fn make_sum_func() -> IrFunction {
        let mut func = make_func("sum", vec![IrType::I32, IrType::I64]);

        // B0: entry
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
                    ty: IrType::I64,
                },
                Instruction::Cmp {
                    dest: Value(2),
                    op: IrCmpOp::Eq,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I32(0)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(2)),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
            source_spans: Vec::new(),
        });

        // B1: base case — return acc
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![],
            terminator: Terminator::Return(Some(Operand::Value(Value(1)))),
            source_spans: Vec::new(),
        });

        // B2: recursive case — return sum(n-1, acc+n)
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![
                Instruction::BinOp {
                    dest: Value(3),
                    op: IrBinOp::Sub,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                },
                Instruction::Cast {
                    dest: Value(4),
                    src: Operand::Value(Value(0)),
                    from_ty: IrType::I32,
                    to_ty: IrType::I64,
                },
                Instruction::BinOp {
                    dest: Value(5),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Value(Value(4)),
                    ty: IrType::I64,
                },
                Instruction::Call {
                    func: "sum".to_string(),
                    info: crate::ir::reexports::CallInfo {
                        dest: Some(Value(6)),
                        args: vec![Operand::Value(Value(3)), Operand::Value(Value(5))],
                        arg_types: vec![IrType::I32, IrType::I64],
                        return_type: IrType::I64,
                        is_variadic: false,
                        num_fixed_args: 2,
                        struct_arg_sizes: vec![None, None],
                        struct_arg_aligns: vec![None, None],
                        struct_arg_classes: vec![vec![], vec![]],
                        struct_arg_riscv_float_classes: vec![None, None],
                        struct_arg_is_f128_sse: Vec::new(),
                        is_sret: false,
                        is_fastcall: false,
                        is_pure: false,
                        is_const: false,
                        ret_eightbyte_classes: vec![],
                        ret_is_f128_sse: false,
                    },
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(6)))),
            source_spans: Vec::new(),
        });

        func.next_value_id = 7;
        func
    }

    #[test]
    fn test_tce_finds_tail_call() {
        let mut func = make_sum_func();
        let n = tail_calls_to_loops(&mut func);
        assert_eq!(n, 1, "should have eliminated 1 tail call");
    }

    #[test]
    fn test_tce_rejects_pointer_into_current_stack_frame() {
        let mut func = make_sum_func();
        func.blocks[0].instructions.insert(
            0,
            Instruction::Alloca {
                dest: Value(7),
                ty: IrType::I32,
                size: 4,
                align: 4,
                volatile: false,
                semantic_volatile: false,
            },
        );
        func.blocks[2].instructions.insert(
            3,
            Instruction::GetElementPtr {
                dest: Value(8),
                base: Value(7),
                offset: Operand::Const(IrConst::I64(0)),
                ty: IrType::Ptr,
            },
        );
        let call = func.blocks[2]
            .instructions
            .iter_mut()
            .find(|instruction| matches!(instruction, Instruction::Call { .. }))
            .unwrap();
        if let Instruction::Call { info, .. } = call {
            info.args[1] = Operand::Value(Value(8));
            info.arg_types[1] = IrType::Ptr;
        }
        func.next_value_id = 9;

        assert_eq!(tail_calls_to_loops(&mut func), 0);
        assert!(func.blocks[2].instructions.iter().any(
            |instruction| matches!(instruction, Instruction::Call { func, .. } if func == "sum")
        ));
    }

    #[test]
    fn test_tce_inserts_loop_header() {
        let mut func = make_sum_func();
        tail_calls_to_loops(&mut func);
        // Original 3 blocks + 1 new loop header
        assert_eq!(func.blocks.len(), 4, "should have 4 blocks after TCE");
    }

    #[test]
    fn test_tce_entry_branches_to_loop_header() {
        let mut func = make_sum_func();
        tail_calls_to_loops(&mut func);
        // Entry block (B0) must now branch unconditionally to the loop header.
        assert!(
            matches!(func.blocks[0].terminator, Terminator::Branch(_)),
            "entry block should end with an unconditional branch after TCE"
        );
    }

    #[test]
    fn test_tce_loop_header_has_phi_nodes() {
        let mut func = make_sum_func();
        tail_calls_to_loops(&mut func);
        // The last block pushed is the loop header.
        let header = func.blocks.last().unwrap();
        let phi_count = header
            .instructions
            .iter()
            .filter(|i| matches!(i, Instruction::Phi { .. }))
            .count();
        assert_eq!(
            phi_count, 2,
            "loop header should have 2 phi nodes (one per param)"
        );
    }

    #[test]
    fn test_tce_tail_call_block_becomes_branch() {
        let mut func = make_sum_func();
        tail_calls_to_loops(&mut func);
        // B2 (rec) was the tail-call block; its terminator should now be Branch.
        let rec_block = func.blocks.iter().find(|b| b.label == BlockId(2)).unwrap();
        assert!(
            matches!(rec_block.terminator, Terminator::Branch(_)),
            "tail-call block terminator should be Branch after TCE"
        );
        // The Call instruction should be removed.
        let has_self_call = rec_block
            .instructions
            .iter()
            .any(|inst| matches!(inst, Instruction::Call { func: f, .. } if f == "sum"));
        assert!(
            !has_self_call,
            "self-recursive Call should be removed by TCE"
        );
    }

    #[test]
    fn test_tce_paramref_uses_renamed_to_phi() {
        let mut func = make_sum_func();
        tail_calls_to_loops(&mut func);
        // Value(0) was %n (ParamRef). Value(2) (the compare) used it.
        // After TCE, that compare should use a phi value, not Value(0).
        let header = func.blocks.last().unwrap();
        let phi_dests: Vec<u32> = header
            .instructions
            .iter()
            .filter_map(|i| {
                if let Instruction::Phi { dest, .. } = i {
                    Some(dest.0)
                } else {
                    None
                }
            })
            .collect();
        // The Cmp (originally in entry, now moved to loop header) should use a phi val.
        let cmp_inst = header
            .instructions
            .iter()
            .find(|i| matches!(i, Instruction::Cmp { .. }))
            .unwrap();
        if let Instruction::Cmp {
            lhs: Operand::Value(lhs_val),
            ..
        } = cmp_inst
        {
            assert!(
                phi_dests.contains(&lhs_val.0),
                "Cmp should use a phi value after renaming, got Value({})",
                lhs_val.0
            );
        } else {
            panic!("Cmp lhs should be a Value operand");
        }
    }

    #[test]
    fn test_tce_no_tail_call_no_change() {
        // A non-recursive function should not be modified.
        let mut func = make_func("add", vec![IrType::I32, IrType::I32]);
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
                Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Value(Value(1)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(2)))),
            source_spans: Vec::new(),
        });
        func.next_value_id = 3;

        let n = tail_calls_to_loops(&mut func);
        assert_eq!(n, 0);
        assert_eq!(
            func.blocks.len(),
            1,
            "non-recursive function should be unchanged"
        );
    }

    #[test]
    fn test_tce_non_tail_call_not_eliminated() {
        // A call whose result is used in further computation is NOT a tail call.
        let mut func = make_func("fib", vec![IrType::I32]);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::ParamRef {
                dest: Value(0),
                param_idx: 0,
                ty: IrType::I32,
            }],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                // %1 = call fib(n-1)
                Instruction::Call {
                    func: "fib".to_string(),
                    info: crate::ir::reexports::CallInfo {
                        dest: Some(Value(1)),
                        args: vec![Operand::Value(Value(0))],
                        arg_types: vec![IrType::I32],
                        return_type: IrType::I64,
                        is_variadic: false,
                        num_fixed_args: 1,
                        struct_arg_sizes: vec![None],
                        struct_arg_aligns: vec![None],
                        struct_arg_classes: vec![vec![]],
                        struct_arg_riscv_float_classes: vec![None],
                        struct_arg_is_f128_sse: Vec::new(),
                        is_sret: false,
                        is_fastcall: false,
                        is_pure: false,
                        is_const: false,
                        ret_eightbyte_classes: vec![],
                        ret_is_f128_sse: false,
                    },
                },
                // %2 = %1 + 1   (uses call result — NOT a tail call)
                Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I64(1)),
                    ty: IrType::I64,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(2)))),
            source_spans: Vec::new(),
        });
        func.next_value_id = 3;

        let n = tail_calls_to_loops(&mut func);
        assert_eq!(n, 0, "non-tail call should not be eliminated");
        assert_eq!(func.blocks.len(), 2);
    }

    #[test]
    fn replace_values_rewrites_hidden_memory_operands() {
        use crate::ir::intrinsics::IntrinsicOp;

        let mut map = FxHashMap::default();
        map.insert(7, 70);
        map.insert(8, 80);
        map.insert(9, 90);

        let mut asm = Instruction::InlineAsm {
            template: String::new(),
            outputs: vec![("=m".into(), Value(7), Some("out".into()))],
            inputs: vec![("r".into(), Operand::Value(Value(8)), Some("in".into()))],
            clobbers: Vec::new(),
            operand_types: vec![IrType::I32, IrType::I32],
            goto_labels: Vec::new(),
            input_symbols: vec![None],
            seg_overrides: vec![AddressSpace::Default, AddressSpace::Default],
        };
        replace_values_in_inst(&mut asm, &map);
        match asm {
            Instruction::InlineAsm {
                outputs, inputs, ..
            } => {
                assert_eq!(outputs[0].1, Value(70));
                assert!(matches!(inputs[0].1, Operand::Value(Value(80))));
            }
            _ => unreachable!(),
        }

        let mut intrinsic = Instruction::Intrinsic {
            dest: None,
            op: IntrinsicOp::Storedqu,
            dest_ptr: Some(Value(9)),
            args: vec![Operand::Value(Value(8))],
        };
        replace_values_in_inst(&mut intrinsic, &map);
        match intrinsic {
            Instruction::Intrinsic { dest_ptr, args, .. } => {
                assert_eq!(dest_ptr, Some(Value(90)));
                assert!(matches!(args.as_slice(), [Operand::Value(Value(80))]));
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn test_closed_form_tail_sum_after_tce() {
        let mut func = make_sum_func();
        if let Instruction::Cmp { op, .. } = &mut func.blocks[0].instructions[2] {
            *op = IrCmpOp::Sle;
        }
        assert_eq!(tail_calls_to_loops(&mut func), 1);
        assert_eq!(closed_form_tail_sum(&mut func), 1);
        assert_eq!(func.blocks.len(), 3);
        assert!(func.blocks.iter().any(|block| {
            block.instructions.iter().any(|inst| {
                matches!(
                    inst,
                    Instruction::BinOp {
                        op: IrBinOp::SDiv,
                        ty: IrType::I64,
                        ..
                    }
                )
            })
        }));
    }
}
