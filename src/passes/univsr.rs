//! Un-IVSR pass: Reverse pointer IV strength reduction when indexed addressing is beneficial.
//!
//! This pass runs after IVSR and detects pointer induction variables created by IVSR.
//! When the target architecture supports indexed addressing (like x86-64 SIB: `base + index * scale + disp`),
//! it is often more efficient to use indexed addressing than pointer arithmetic.
//!
//! The pass transforms:
//! ```text
//!   %ptr = Phi(%initial_ptr, %ptr_next)
//!   %val = Load(%ptr)
//!   %ptr_next = GEP(%ptr, stride)
//! ```
//! Back to:
//! ```text
//!   %index = Phi(%init_index, %index_next)
//!   %offset = Shl(%index, scale)
//!   %addr = GEP(%base, %offset + disp)
//!   %val = Load(%addr)
//!   %index_next = Add(%index, 1)
//! ```
//! This allows the backend's indexed addressing detection to emit single-instruction SIB-form memory accesses.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::reexports::{BlockId, Instruction, IrBinOp, IrConst, IrFunction, Operand, Value};

/// Information about an IVSR-created pointer IV that should be reverted to indexed form.
#[derive(Debug, Clone)]
struct IvsrPointerIV {
    /// The pointer phi's destination value.
    ptr_phi_dest: Value,
    /// The original array base pointer.
    base_ptr: Value,
    /// Element stride in bytes per iteration (must be 1, 2, 4, or 8 for SIB).
    stride: i64,
    /// Initial base offset (usually 0).
    init_offset: i64,
    /// The associated index IV.
    index_iv: Value,
    /// The backedge increment GEP's destination value (`%ptr_next = GEP(%ptr, stride)`).
    increment_gep_dest: Value,
    /// Block where the phi resides.
    #[allow(dead_code)]
    header_block: BlockId,
}

/// Recorded memory use site of a pointer IV (direct or through transitive GEPs/Copies).
#[derive(Debug, Clone)]
struct PtrUse {
    block_idx: usize,
    inst_idx: usize,
    /// Accumulated byte displacement for this specific use (init_offset + intermediate GEP offsets).
    offset: i64,
    /// Value ID used as pointer operand in the Load/Store.
    #[allow(dead_code)]
    use_val_id: u32,
}

/// Run the un-IVSR pass on a function.
/// Returns the number of pointer IVs that were reverted.
pub(crate) fn run_univsr(func: &mut IrFunction) -> usize {
    // Fast-path: if function has no pointer Phis, exit immediately with 0 allocations.
    let has_ptr_phi = func.blocks.iter().any(|b| {
        b.instructions.iter().any(|inst| {
            matches!(inst, Instruction::Phi { ty: IrType::Ptr, incoming, .. } if incoming.len() == 2)
        })
    });

    if !has_ptr_phi {
        return 0;
    }

    let debug = std::env::var("LCCC_DEBUG_UNIVSR").is_ok();
    let ivsr_pointers = detect_ivsr_pointer_ivs(func);
    if debug {
        eprintln!(
            "[univsr] {}: {} candidate pointer IVs",
            func.name,
            ivsr_pointers.len()
        );
    }
    if ivsr_pointers.is_empty() {
        return 0;
    }

    let mut num_reverted = 0;
    for ptr_iv in &ivsr_pointers {
        if !is_valid_sib_scale(ptr_iv.stride) {
            if debug {
                eprintln!(
                    "[univsr]   skip: stride {} not SIB-encodable",
                    ptr_iv.stride
                );
            }
            continue;
        }

        let ok = revert_pointer_iv(func, ptr_iv);
        if debug {
            eprintln!(
                "[univsr]   phi v{} stride {} -> {}",
                ptr_iv.ptr_phi_dest.0,
                ptr_iv.stride,
                if ok {
                    "REVERTED"
                } else {
                    "rejected (uses/safety)"
                }
            );
        }
        if ok {
            num_reverted += 1;
        }
    }

    num_reverted
}

/// Detect IVSR-created pointer IVs in the function.
fn detect_ivsr_pointer_ivs(func: &IrFunction) -> Vec<IvsrPointerIV> {
    let mut result = Vec::with_capacity(16);

    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Phi { dest, ty, incoming } = inst {
                if *ty != IrType::Ptr || incoming.len() != 2 {
                    continue;
                }

                if let Some(ptr_iv) = analyze_pointer_phi(func, dest, incoming, block.label) {
                    result.push(ptr_iv);
                }
            }
        }
    }

    result
}

/// Analyze a pointer phi to verify whether it matches the IVSR pattern.
/// Handles arbitrary incoming edge orders (init vs backedge).
fn analyze_pointer_phi(
    func: &IrFunction,
    dest: &Value,
    incoming: &[(Operand, BlockId)],
    header_block: BlockId,
) -> Option<IvsrPointerIV> {
    // Identify which edge is the backedge GEP (%ptr_next = GEP(%ptr, stride))
    // and which is the initialization value.
    let mut backedge_info = None;
    let mut init_op = None;

    for (op, _block) in incoming {
        if let Operand::Value(v) = op {
            if let Some((base, stride)) = find_gep_with_const_offset(func, v.0) {
                if base.0 == dest.0 {
                    backedge_info = Some((*v, stride));
                    continue;
                }
            }
        }
        init_op = Some(op);
    }

    let (increment_gep_dest, stride) = backedge_info?;
    let init_operand = init_op?;

    if !is_valid_sib_scale(stride) {
        return None;
    }

    // Extract base pointer and initial displacement from the init operand
    let (base_ptr, init_offset) = extract_base_from_init(func, init_operand)?;

    // Find the matching integer induction variable in the header block
    let index_iv = find_index_iv_in_header(func, header_block)?;

    Some(IvsrPointerIV {
        ptr_phi_dest: *dest,
        base_ptr,
        stride,
        init_offset,
        index_iv,
        increment_gep_dest,
        header_block,
    })
}

/// Find a GEP instruction defining `val_id` with a constant offset.
fn find_gep_with_const_offset(func: &IrFunction, val_id: u32) -> Option<(Value, i64)> {
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::GetElementPtr {
                dest, base, offset, ..
            } = inst
            {
                if dest.0 == val_id {
                    let stride = match offset {
                        Operand::Const(c) => c.to_i64()?,
                        _ => return None,
                    };
                    return Some((*base, stride));
                }
            }
        }
    }
    None
}

/// Extract base pointer and initial constant offset from the initialization operand.
fn extract_base_from_init(func: &IrFunction, init_op: &Operand) -> Option<(Value, i64)> {
    match init_op {
        Operand::Value(v) => {
            // Check if init value is a GEP (e.g. %init = GEP(%arr, 0) or GEP(%arr, offset))
            for block in &func.blocks {
                for inst in &block.instructions {
                    if let Instruction::GetElementPtr {
                        dest, base, offset, ..
                    } = inst
                    {
                        if dest.0 == v.0 {
                            let init_offset = match offset {
                                Operand::Const(c) => c.to_i64().unwrap_or(0),
                                _ => 0,
                            };
                            return Some((*base, init_offset));
                        }
                    }
                }
            }
            // Direct pointer value without GEP wrapper
            Some((*v, 0))
        }
        _ => None,
    }
}

/// Find the matching integer index IV in the loop header block.
///
/// SOUNDNESS: the rewrite computes `base + (index << log2(stride)) + disp`,
/// which equals the pointer IV's value at iteration k ONLY IF the index IV
/// takes the exact value k, i.e. it starts at 0 and is incremented by
/// exactly +1 on the backedge. Any other (init, step) combination silently
/// addresses the wrong element, so both are verified here. Candidates with
/// other steps/inits are skipped rather than mis-used.
fn find_index_iv_in_header(func: &IrFunction, header: BlockId) -> Option<Value> {
    let header_block = func.blocks.iter().find(|b| b.label.0 == header.0)?;

    for inst in &header_block.instructions {
        if let Instruction::Phi { dest, ty, incoming } = inst {
            if !matches!(ty, IrType::I32 | IrType::I64 | IrType::U32 | IrType::U64) {
                continue;
            }

            if incoming.len() != 2 {
                continue;
            }

            if is_canonical_counter_iv(func, dest, incoming) {
                return Some(*dest);
            }
        }
    }

    None
}

/// Check that a Phi is a canonical counter: init == 0 and backedge = phi + 1.
fn is_canonical_counter_iv(
    func: &IrFunction,
    dest: &Value,
    incoming: &[(Operand, BlockId)],
) -> bool {
    let mut has_unit_increment = false;
    let mut has_zero_init = false;

    for (op, _block) in incoming {
        match op {
            Operand::Value(v) => {
                if is_value_unit_increment_of(func, *v, *dest, 0) {
                    has_unit_increment = true;
                } else if is_value_const_zero(func, *v, 0) {
                    has_zero_init = true;
                }
            }
            Operand::Const(c) => {
                if c.to_i64() == Some(0) {
                    has_zero_init = true;
                }
            }
        }
    }

    has_unit_increment && has_zero_init
}

/// Check if `val` is defined as `Add(%phi, 1)`, possibly through Copy/Cast chains.
fn is_value_unit_increment_of(
    func: &IrFunction,
    val: Value,
    phi_dest: Value,
    depth: usize,
) -> bool {
    if depth > 8 {
        return false;
    }
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::BinOp {
                    dest: bd,
                    op: IrBinOp::Add,
                    lhs,
                    rhs,
                    ..
                } if bd.0 == val.0 => match (lhs, rhs) {
                    (Operand::Value(v), Operand::Const(c))
                    | (Operand::Const(c), Operand::Value(v)) => {
                        if v.0 == phi_dest.0 && c.to_i64() == Some(1) {
                            return true;
                        }
                    }
                    _ => {}
                },
                Instruction::Copy {
                    dest: cd,
                    src: Operand::Value(v),
                } if cd.0 == val.0 => {
                    if is_value_unit_increment_of(func, *v, phi_dest, depth + 1) {
                        return true;
                    }
                }
                Instruction::Cast {
                    dest: cd,
                    src: Operand::Value(v),
                    ..
                } if cd.0 == val.0 => {
                    if is_value_unit_increment_of(func, *v, phi_dest, depth + 1) {
                        return true;
                    }
                }
                _ => {}
            }
        }
    }
    false
}

/// Check if a Value resolves to the constant 0 (through Copy/Cast chains).
fn is_value_const_zero(func: &IrFunction, val: Value, depth: usize) -> bool {
    if depth > 8 {
        return false;
    }
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Copy { dest, src } if dest.0 == val.0 => {
                    return match src {
                        Operand::Const(c) => c.to_i64() == Some(0),
                        Operand::Value(v) => is_value_const_zero(func, *v, depth + 1),
                    };
                }
                Instruction::Cast { dest, src, .. } if dest.0 == val.0 => {
                    return match src {
                        Operand::Const(c) => c.to_i64() == Some(0),
                        Operand::Value(v) => is_value_const_zero(func, *v, depth + 1),
                    };
                }
                _ => {}
            }
        }
    }
    false
}

/// Validate that rewriting is sound: after the transformation the pointer
/// phi and the increment GEP are both replaced by `Copy(base)`, so EVERY use
/// of either value (and of every derived intermediate) must be one we
/// rewrite or remove. This is a strict WHITELIST over all instructions and
/// terminators - anything unrecognized rejects the transformation:
///
///  * uses of the phi: Load/Store pointer operands, derived GEP/Copy/Cast
///    (tracked transitively), and the increment GEP itself
///  * uses of the increment GEP: ONLY the phi backedge (exit conditions
///    like `p != end` compare it - those loops must NOT be transformed)
///  * uses of derived intermediates: Load/Store pointer operands and
///    further derived values only
fn validate_transformation_safety(func: &IrFunction, ptr_iv: &IvsrPointerIV) -> bool {
    let phi_id = ptr_iv.ptr_phi_dest.0;
    let gep_id = ptr_iv.increment_gep_dest.0;

    // Recompute the derived-value set exactly as find_transitive_ptr_uses does.
    let mut derived: FxHashSet<u32> = FxHashSet::default();
    derived.insert(phi_id);
    let mut worklist = vec![phi_id];
    while let Some(cur) = worklist.pop() {
        for block in &func.blocks {
            for inst in &block.instructions {
                match inst {
                    Instruction::GetElementPtr {
                        dest,
                        base,
                        offset: Operand::Const(_),
                        ..
                    } if base.0 == cur && dest.0 != gep_id => {
                        if derived.insert(dest.0) {
                            worklist.push(dest.0);
                        }
                    }
                    Instruction::Copy {
                        dest,
                        src: Operand::Value(v),
                    } if v.0 == cur => {
                        if derived.insert(dest.0) {
                            worklist.push(dest.0);
                        }
                    }
                    Instruction::Cast {
                        dest,
                        src: Operand::Value(v),
                        ..
                    } if v.0 == cur => {
                        if derived.insert(dest.0) {
                            worklist.push(dest.0);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Whitelist check over every operand use in the function.
    let is_tracked = |id: u32| derived.contains(&id) || id == gep_id;
    let op_is_tracked = |op: &Operand| matches!(op, Operand::Value(v) if is_tracked(v.0));

    for block in &func.blocks {
        for inst in &block.instructions {
            let escapes = match inst {
                // Load THROUGH a tracked pointer is the use we rewrite.
                Instruction::Load { .. } => false,
                // A tracked value STORED AS DATA escapes - reject.
                Instruction::Store { val, .. } => op_is_tracked(val),
                // The derived chain itself.
                Instruction::GetElementPtr {
                    dest, base, offset, ..
                } => {
                    // Any GEP consuming a tracked value as its base must be
                    // itself tracked (or be the increment GEP); a tracked
                    // value used as a GEP OFFSET escapes.
                    (is_tracked(base.0) && !is_tracked(dest.0) && dest.0 != gep_id)
                        || op_is_tracked(offset)
                }
                Instruction::Copy { dest, src } => op_is_tracked(src) && !derived.contains(&dest.0),
                Instruction::Cast { dest, src, .. } => {
                    op_is_tracked(src) && !derived.contains(&dest.0)
                }
                // The phi consumes the increment GEP (backedge) - allowed.
                Instruction::Phi { dest, incoming, .. } => {
                    incoming.iter().any(|(op, _)| {
                        if let Operand::Value(v) = op {
                            if v.0 == gep_id && dest.0 == phi_id {
                                return false; // the IV cycle itself
                            }
                            is_tracked(v.0)
                        } else {
                            false
                        }
                    })
                }
                // Every other instruction: any tracked operand escapes.
                other => {
                    let mut used = false;
                    other.for_each_used_value(|v| {
                        if is_tracked(v) {
                            used = true;
                        }
                    });
                    used
                }
            };
            if escapes {
                return false;
            }
        }

        // Terminators: Return(ptr), CondBranch(cond=...), Switch, IndirectBranch.
        use crate::ir::reexports::Terminator;
        match &block.terminator {
            Terminator::Return(Some(Operand::Value(v))) if is_tracked(v.0) => return false,
            Terminator::CondBranch {
                cond: Operand::Value(v),
                ..
            } if is_tracked(v.0) => return false,
            Terminator::Switch {
                val: Operand::Value(v),
                ..
            } if is_tracked(v.0) => return false,
            Terminator::IndirectBranch {
                target: Operand::Value(v),
                ..
            } if is_tracked(v.0) => return false,
            _ => {}
        }
    }

    // The increment GEP's ONLY permitted consumer is the phi backedge.
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Phi { dest, .. } if dest.0 == phi_id => continue,
                _ => {}
            }
            let mut uses_gep = false;
            inst.for_each_used_value(|v| {
                if v == gep_id {
                    uses_gep = true;
                }
            });
            if uses_gep {
                if let Instruction::Phi { dest, .. } = inst {
                    if dest.0 == phi_id {
                        continue;
                    }
                }
                return false;
            }
        }
    }

    true
}

/// Find all transitive Load/Store uses of the pointer phi, tracking accumulated constant offsets.
fn find_transitive_ptr_uses(func: &IrFunction, ptr_iv: &IvsrPointerIV) -> Vec<PtrUse> {
    let mut results = Vec::with_capacity(16);
    let mut visited_values = FxHashSet::default();

    // Queue of (Value, accumulated_offset)
    let mut worklist: Vec<(Value, i64)> = vec![(ptr_iv.ptr_phi_dest, ptr_iv.init_offset)];
    visited_values.insert(ptr_iv.ptr_phi_dest.0);

    while let Some((val_to_check, current_offset)) = worklist.pop() {
        for (block_idx, block) in func.blocks.iter().enumerate() {
            for (inst_idx, inst) in block.instructions.iter().enumerate() {
                match inst {
                    Instruction::Load { ptr, .. } if ptr.0 == val_to_check.0 => {
                        results.push(PtrUse {
                            block_idx,
                            inst_idx,
                            offset: current_offset,
                            use_val_id: val_to_check.0,
                        });
                    }
                    Instruction::Store { ptr, .. } if ptr.0 == val_to_check.0 => {
                        results.push(PtrUse {
                            block_idx,
                            inst_idx,
                            offset: current_offset,
                            use_val_id: val_to_check.0,
                        });
                    }
                    Instruction::GetElementPtr {
                        dest, base, offset, ..
                    } if base.0 == val_to_check.0 => {
                        // Skip the loop backedge increment GEP
                        if dest.0 != ptr_iv.increment_gep_dest.0 {
                            if let Operand::Const(c) = offset {
                                if let Some(off) = c.to_i64() {
                                    if visited_values.insert(dest.0) {
                                        worklist.push((*dest, current_offset + off));
                                    }
                                }
                            }
                        }
                    }
                    Instruction::Copy {
                        dest,
                        src: Operand::Value(v),
                    } if v.0 == val_to_check.0 => {
                        if visited_values.insert(dest.0) {
                            worklist.push((*dest, current_offset));
                        }
                    }
                    Instruction::Cast {
                        dest,
                        src: Operand::Value(v),
                        ..
                    } if v.0 == val_to_check.0 => {
                        if visited_values.insert(dest.0) {
                            worklist.push((*dest, current_offset));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    results
}

/// Revert a pointer IV back to indexed addressing form using a single-pass block rewrite.
fn revert_pointer_iv(func: &mut IrFunction, ptr_iv: &IvsrPointerIV) -> bool {
    let ptr_uses = find_transitive_ptr_uses(func, ptr_iv);
    if ptr_uses.is_empty() {
        return false;
    }

    if !validate_transformation_safety(func, ptr_iv) {
        return false;
    }

    let mut next_val_id = func.next_value_id.max(compute_max_value_id(func) + 1);

    // Group uses by (block_idx, inst_idx) for O(1) matching during linear block rewrite
    let mut use_map: FxHashMap<(usize, usize), i64> = FxHashMap::default();
    let mut affected_blocks = FxHashSet::default();
    for u in &ptr_uses {
        use_map.insert((u.block_idx, u.inst_idx), u.offset);
        affected_blocks.insert(u.block_idx);
    }

    for block_idx in affected_blocks {
        if block_idx >= func.blocks.len() {
            continue;
        }

        let block = &mut func.blocks[block_idx];
        let has_spans = !block.source_spans.is_empty();

        let old_insts = std::mem::take(&mut block.instructions);
        let old_spans = std::mem::take(&mut block.source_spans);

        let mut new_insts = Vec::with_capacity(old_insts.len() + 8);
        let mut new_spans = Vec::with_capacity(old_insts.len() + 8);

        // Per-block address cache:
        // - cached_scaled_index: Some(Value) if Shl was emitted in this block
        // - cached_gep_for_offset: map from byte offset -> GEP Value
        let mut cached_scaled_index: Option<Value> = None;
        let mut cached_gep_for_offset: FxHashMap<i64, Value> = FxHashMap::default();

        for (inst_idx, mut inst) in old_insts.into_iter().enumerate() {
            let span = if has_spans && inst_idx < old_spans.len() {
                old_spans[inst_idx]
            } else {
                crate::common::source::Span::new(0, 0, 0)
            };

            if let Some(&offset) = use_map.get(&(block_idx, inst_idx)) {
                // Step 1: Ensure base offset (index * stride) is available in this block
                let (index_operand, is_scaled) = if ptr_iv.stride == 1 {
                    (Operand::Value(ptr_iv.index_iv), false)
                } else {
                    let scaled_val = match cached_scaled_index {
                        Some(val) => val,
                        None => {
                            let shift_amount = ptr_iv.stride.trailing_zeros();
                            let val = Value(next_val_id);
                            next_val_id += 1;

                            new_insts.push(Instruction::BinOp {
                                dest: val,
                                op: IrBinOp::Shl,
                                lhs: Operand::Value(ptr_iv.index_iv),
                                rhs: Operand::Const(IrConst::I32(shift_amount as i32)),
                                ty: IrType::I64,
                            });
                            if has_spans {
                                new_spans.push(span);
                            }

                            cached_scaled_index = Some(val);
                            val
                        }
                    };
                    (Operand::Value(scaled_val), true)
                };

                // Step 2: Ensure GEP for this specific offset is available in this block
                let target_ptr_val = match cached_gep_for_offset.get(&offset) {
                    Some(&cached_ptr) => cached_ptr,
                    None => {
                        let final_offset_operand = if offset != 0 {
                            let adj_val = Value(next_val_id);
                            next_val_id += 1;

                            new_insts.push(Instruction::BinOp {
                                dest: adj_val,
                                op: IrBinOp::Add,
                                lhs: index_operand,
                                rhs: Operand::Const(IrConst::I64(offset)),
                                ty: IrType::I64,
                            });
                            if has_spans {
                                new_spans.push(span);
                            }

                            Operand::Value(adj_val)
                        } else if is_scaled {
                            index_operand
                        } else {
                            index_operand
                        };

                        let new_ptr = Value(next_val_id);
                        next_val_id += 1;

                        new_insts.push(Instruction::GetElementPtr {
                            dest: new_ptr,
                            base: ptr_iv.base_ptr,
                            offset: final_offset_operand,
                            ty: IrType::Ptr,
                        });
                        if has_spans {
                            new_spans.push(span);
                        }

                        cached_gep_for_offset.insert(offset, new_ptr);
                        new_ptr
                    }
                };

                // Step 3: Rewrite Load/Store pointer to target_ptr_val
                match &mut inst {
                    Instruction::Load { ptr, .. } => *ptr = target_ptr_val,
                    Instruction::Store { ptr, .. } => *ptr = target_ptr_val,
                    _ => {}
                }

                new_insts.push(inst);
                if has_spans {
                    new_spans.push(span);
                }
            } else {
                new_insts.push(inst);
                if has_spans {
                    new_spans.push(span);
                }
            }
        }

        block.instructions = new_insts;
        if has_spans {
            block.source_spans = new_spans;
        }
    }

    func.next_value_id = next_val_id;

    // Break the dead pointer IV cycle so DCE can safely eliminate the unused phi and increment GEP
    remove_dead_pointer_iv_cycle(func, ptr_iv);

    true
}

/// Break the dead pointer IV cycle after reverting to indexed addressing.
fn remove_dead_pointer_iv_cycle(func: &mut IrFunction, ptr_iv: &IvsrPointerIV) {
    let phi_id = ptr_iv.ptr_phi_dest.0;
    let gep_id = ptr_iv.increment_gep_dest.0;

    for block in &mut func.blocks {
        for inst in &mut block.instructions {
            if let Instruction::Phi { dest, .. } = inst {
                if dest.0 == phi_id {
                    *inst = Instruction::Copy {
                        dest: ptr_iv.ptr_phi_dest,
                        src: Operand::Value(ptr_iv.base_ptr),
                    };
                }
            }

            if let Instruction::GetElementPtr { dest, .. } = inst {
                if dest.0 == gep_id {
                    *inst = Instruction::Copy {
                        dest: ptr_iv.increment_gep_dest,
                        src: Operand::Value(ptr_iv.base_ptr),
                    };
                }
            }
        }
    }
}

/// Check if a stride in bytes is valid for x86-64 SIB encoding (1, 2, 4, or 8 bytes).
#[inline(always)]
fn is_valid_sib_scale(stride: i64) -> bool {
    matches!(stride, 1 | 2 | 4 | 8)
}

/// Compute the maximum Value ID used in the function.
fn compute_max_value_id(func: &IrFunction) -> u32 {
    let mut max_id = 0u32;
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some(dest_id) = inst.dest_value_id() {
                max_id = max_id.max(dest_id);
            }
            visit_instruction_values(inst, &mut |v| {
                max_id = max_id.max(v.0);
            });
        }
    }
    max_id
}

/// Visit all value uses in an instruction.
fn visit_instruction_values<F>(inst: &Instruction, f: &mut F)
where
    F: FnMut(Value),
{
    match inst {
        Instruction::Load { ptr, .. } => f(*ptr),
        Instruction::Store { val, ptr, .. } => {
            if let Operand::Value(v) = val {
                f(*v);
            }
            f(*ptr);
        }
        Instruction::BinOp { lhs, rhs, .. } => {
            if let Operand::Value(v) = lhs {
                f(*v);
            }
            if let Operand::Value(v) = rhs {
                f(*v);
            }
        }
        Instruction::GetElementPtr { base, offset, .. } => {
            f(*base);
            if let Operand::Value(v) = offset {
                f(*v);
            }
        }
        Instruction::Phi { incoming, .. } => {
            for (op, _) in incoming {
                if let Operand::Value(v) = op {
                    f(*v);
                }
            }
        }
        _ => {}
    }
}

/// Helper trait to get the destination value ID from an instruction.
trait DestValue {
    fn dest_value_id(&self) -> Option<u32>;
}

impl DestValue for Instruction {
    fn dest_value_id(&self) -> Option<u32> {
        match self {
            Instruction::BinOp { dest, .. }
            | Instruction::UnaryOp { dest, .. }
            | Instruction::Cast { dest, .. }
            | Instruction::GetElementPtr { dest, .. }
            | Instruction::Load { dest, .. }
            | Instruction::Cmp { dest, .. }
            | Instruction::Phi { dest, .. }
            | Instruction::Alloca { dest, .. }
            | Instruction::DynAlloca { dest, .. }
            | Instruction::Copy { dest, .. }
            | Instruction::GlobalAddr { dest, .. }
            | Instruction::VaArg { dest, .. }
            | Instruction::AtomicRmw { dest, .. }
            | Instruction::AtomicCmpxchg { dest, .. }
            | Instruction::AtomicLoad { dest, .. }
            | Instruction::Intrinsic {
                dest: Some(dest), ..
            }
            | Instruction::Select { dest, .. }
            | Instruction::LabelAddr { dest, .. }
            | Instruction::GetReturnF64Second { dest }
            | Instruction::GetReturnF32Second { dest }
            | Instruction::GetReturnF128Second { dest } => Some(dest.0),
            Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
                info.dest.map(|v| v.0)
            }
            _ => None,
        }
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

// ─── Tests ─────────────────────────────────────────────────────────────────
//
// Rewritten against the real IR API (PR #59's version used a non-existent
// IrFunction/IrBlock shape). Each test builds the canonical IVSR output
// shape - a pointer phi advanced by a constant-stride GEP plus a matching
// i=0,+1 counter phi - and checks both the transformation and the
// soundness rejections.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::AddressSpace;
    use crate::ir::reexports::{BasicBlock, IrCmpOp, Terminator};

    /// Two blocks: preheader (0) and loop header/body (1).
    fn make_test_function() -> IrFunction {
        let mut f = IrFunction::new("test_kernel".to_string(), IrType::Void, Vec::new(), false);
        for i in 0..2u32 {
            f.blocks.push(BasicBlock {
                label: BlockId(i),
                instructions: Vec::new(),
                terminator: if i == 1 {
                    Terminator::Branch(BlockId(1))
                } else {
                    Terminator::Branch(BlockId(1))
                },
                source_spans: Vec::new(),
            });
        }
        f
    }

    fn load(dest: u32, ptr: u32, ty: IrType) -> Instruction {
        Instruction::Load {
            volatile: false,
            dest: Value(dest),
            ptr: Value(ptr),
            ty,
            seg_override: AddressSpace::Default,
        }
    }

    fn store_val(val: u32, ptr: u32, ty: IrType) -> Instruction {
        Instruction::Store {
            volatile: false,
            val: Operand::Value(Value(val)),
            ptr: Value(ptr),
            ty,
            seg_override: AddressSpace::Default,
        }
    }

    /// Canonical counter phi: i = phi [0 (from pre), i+1 (from latch)].
    fn counter_phi(dest: u32, next: u32) -> Instruction {
        Instruction::Phi {
            dest: Value(dest),
            ty: IrType::I64,
            incoming: vec![
                (Operand::Const(IrConst::I64(0)), BlockId(0)),
                (Operand::Value(Value(next)), BlockId(1)),
            ],
        }
    }

    fn counter_inc(dest: u32, phi: u32) -> Instruction {
        Instruction::BinOp {
            dest: Value(dest),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(phi)),
            rhs: Operand::Const(IrConst::I64(1)),
            ty: IrType::I64,
        }
    }

    fn ptr_phi(dest: u32, init: u32, next: u32) -> Instruction {
        Instruction::Phi {
            dest: Value(dest),
            ty: IrType::Ptr,
            incoming: vec![
                (Operand::Value(Value(init)), BlockId(0)),
                (Operand::Value(Value(next)), BlockId(1)),
            ],
        }
    }

    fn gep_const(dest: u32, base: u32, off: i64) -> Instruction {
        Instruction::GetElementPtr {
            dest: Value(dest),
            base: Value(base),
            offset: Operand::Const(IrConst::I64(off)),
            ty: IrType::Ptr,
        }
    }

    /// The canonical IVSR shape this pass exists to revert:
    ///   pre:  %10 = GlobalAddr arr ; (init ptr = %10)
    ///   hdr:  %1 = phi i64 [0, pre], [%6, hdr]     (counter)
    ///         %2 = phi ptr [%10, pre], [%7, hdr]   (pointer IV)
    ///         %3 = load %2
    ///         %6 = add %1, 1
    ///         %7 = gep %2, stride
    fn build_canonical(stride: i64, load_ty: IrType) -> IrFunction {
        let mut f = make_test_function();
        f.blocks[0].instructions = vec![Instruction::GlobalAddr {
            dest: Value(10),
            name: "arr".to_string(),
        }];
        f.blocks[1].instructions = vec![
            counter_phi(1, 6),
            ptr_phi(2, 10, 7),
            load(3, 2, load_ty),
            counter_inc(6, 1),
            gep_const(7, 2, stride),
        ];
        f.next_value_id = 11;
        f
    }

    #[test]
    fn basic_reversion_stride4() {
        let mut func = build_canonical(4, IrType::I32);
        let count = run_univsr(&mut func);
        assert_eq!(count, 1);

        // The load must no longer address the (now dead) pointer phi.
        let load_ptr = func.blocks[1]
            .instructions
            .iter()
            .find_map(|inst| {
                if let Instruction::Load { ptr, .. } = inst {
                    Some(ptr.0)
                } else {
                    None
                }
            })
            .expect("load survived");
        assert_ne!(
            load_ptr, 2,
            "load must be rewritten away from the pointer phi"
        );

        // Stride 4 -> a Shl by 2 must exist.
        let has_shl = func.blocks[1].instructions.iter().any(|inst| {
            matches!(
                inst,
                Instruction::BinOp {
                    op: IrBinOp::Shl,
                    ..
                }
            )
        });
        assert!(has_shl, "stride 4 must scale the index with Shl");

        // The pointer IV cycle is broken: phi and increment GEP became Copies.
        let copies = func.blocks[1]
            .instructions
            .iter()
            .filter(|inst| matches!(inst, Instruction::Copy { .. }))
            .count();
        assert_eq!(copies, 2, "phi + increment GEP must both become Copy(base)");
    }

    #[test]
    fn stride1_no_shl() {
        let mut func = build_canonical(1, IrType::I8);
        let count = run_univsr(&mut func);
        assert_eq!(count, 1);

        let has_shl = func.blocks[1].instructions.iter().any(|inst| {
            matches!(
                inst,
                Instruction::BinOp {
                    op: IrBinOp::Shl,
                    ..
                }
            )
        });
        assert!(!has_shl, "stride 1 must not emit a Shl");
    }

    #[test]
    fn invalid_sib_scale_rejected() {
        // Stride 3 is not encodable as an x86 SIB scale.
        let mut func = build_canonical(3, IrType::I8);
        assert_eq!(run_univsr(&mut func), 0);
    }

    #[test]
    fn transitive_gep_offset_preserved() {
        // Loop body accesses p[0] and p[1] (via +4 GEP): the derived access
        // must keep its +4 displacement after reversion.
        let mut f = make_test_function();
        f.blocks[0].instructions = vec![Instruction::GlobalAddr {
            dest: Value(10),
            name: "arr".to_string(),
        }];
        f.blocks[1].instructions = vec![
            counter_phi(1, 6),
            ptr_phi(2, 10, 7),
            load(3, 2, IrType::I32),
            gep_const(4, 2, 4), // q = p + 4 bytes
            load(5, 4, IrType::I32),
            counter_inc(6, 1),
            gep_const(7, 2, 8), // stride 8
        ];
        f.next_value_id = 11;

        assert_eq!(run_univsr(&mut f), 1);

        // Somewhere an Add of +4 must survive to preserve the displacement.
        let has_add4 = f.blocks[1].instructions.iter().any(|inst| {
            matches!(inst,
                Instruction::BinOp { op: IrBinOp::Add, rhs: Operand::Const(c), .. }
                    if c.to_i64() == Some(4))
        });
        assert!(
            has_add4,
            "the +4 displacement of the second access must be preserved"
        );
    }

    #[test]
    fn shl_deduplicated_across_uses() {
        // Three accesses through the same pointer in one block: exactly one
        // Shl (scale computation) may be emitted.
        let mut f = make_test_function();
        f.blocks[0].instructions = vec![Instruction::GlobalAddr {
            dest: Value(10),
            name: "arr".to_string(),
        }];
        f.blocks[1].instructions = vec![
            counter_phi(1, 6),
            ptr_phi(2, 10, 7),
            load(3, 2, IrType::I32),
            store_val(3, 2, IrType::I32),
            load(5, 2, IrType::I32),
            counter_inc(6, 1),
            gep_const(7, 2, 4),
        ];
        f.next_value_id = 11;

        assert_eq!(run_univsr(&mut f), 1);
        let shl_count = f.blocks[1]
            .instructions
            .iter()
            .filter(|inst| {
                matches!(
                    inst,
                    Instruction::BinOp {
                        op: IrBinOp::Shl,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(shl_count, 1, "scale computation must be CSE'd per block");
    }

    #[test]
    fn spans_stay_in_sync() {
        let mut func = build_canonical(4, IrType::I32);
        // give the rewritten block spans so the parallel-array invariant is tested
        func.blocks[1].source_spans =
            vec![crate::common::source::Span::new(0, 0, 0); func.blocks[1].instructions.len()];
        assert_eq!(run_univsr(&mut func), 1);
        assert_eq!(
            func.blocks[1].instructions.len(),
            func.blocks[1].source_spans.len(),
            "source_spans must stay parallel to instructions"
        );
    }

    // ── Soundness rejections ────────────────────────────────────────────

    #[test]
    fn reject_nonzero_init_counter() {
        // Counter starts at 5, not 0: base + i*stride would address the
        // wrong element. Must be rejected.
        let mut f = make_test_function();
        f.blocks[0].instructions = vec![Instruction::GlobalAddr {
            dest: Value(10),
            name: "arr".to_string(),
        }];
        f.blocks[1].instructions = vec![
            Instruction::Phi {
                dest: Value(1),
                ty: IrType::I64,
                incoming: vec![
                    (Operand::Const(IrConst::I64(5)), BlockId(0)), // init 5!
                    (Operand::Value(Value(6)), BlockId(1)),
                ],
            },
            ptr_phi(2, 10, 7),
            load(3, 2, IrType::I32),
            counter_inc(6, 1),
            gep_const(7, 2, 4),
        ];
        f.next_value_id = 11;
        assert_eq!(
            run_univsr(&mut f),
            0,
            "counter with init!=0 must be rejected"
        );
    }

    #[test]
    fn reject_non_unit_step_counter() {
        // Counter steps by 2: pointer moves stride bytes per iteration but
        // the index moves 2 - the products diverge. Must be rejected.
        let mut f = make_test_function();
        f.blocks[0].instructions = vec![Instruction::GlobalAddr {
            dest: Value(10),
            name: "arr".to_string(),
        }];
        f.blocks[1].instructions = vec![
            counter_phi(1, 6),
            ptr_phi(2, 10, 7),
            load(3, 2, IrType::I32),
            Instruction::BinOp {
                dest: Value(6),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I64(2)), // step 2!
                ty: IrType::I64,
            },
            gep_const(7, 2, 4),
        ];
        f.next_value_id = 11;
        assert_eq!(
            run_univsr(&mut f),
            0,
            "counter with step!=1 must be rejected"
        );
    }

    #[test]
    fn reject_pointer_compared_in_exit_condition() {
        // `for (p = a; p != end; p++)` shape: the increment GEP feeds the
        // exit compare. Rewriting the GEP to Copy(base) would make the exit
        // condition never advance -> infinite loop. Must be rejected.
        let mut f = make_test_function();
        f.blocks[0].instructions = vec![
            Instruction::GlobalAddr {
                dest: Value(10),
                name: "arr".to_string(),
            },
            Instruction::GlobalAddr {
                dest: Value(11),
                name: "end".to_string(),
            },
        ];
        f.blocks[1].instructions = vec![
            counter_phi(1, 6),
            ptr_phi(2, 10, 7),
            load(3, 2, IrType::I32),
            counter_inc(6, 1),
            gep_const(7, 2, 4),
            Instruction::Cmp {
                dest: Value(8),
                op: IrCmpOp::Ne,
                lhs: Operand::Value(Value(7)), // increment GEP in compare!
                rhs: Operand::Value(Value(11)),
                ty: IrType::Ptr,
            },
        ];
        f.next_value_id = 12;
        assert_eq!(
            run_univsr(&mut f),
            0,
            "pointer-exit-condition loops must not be reverted"
        );
    }

    #[test]
    fn reject_pointer_escaping_to_call() {
        let mut f = make_test_function();
        f.blocks[0].instructions = vec![Instruction::GlobalAddr {
            dest: Value(10),
            name: "arr".to_string(),
        }];
        f.blocks[1].instructions = vec![
            counter_phi(1, 6),
            ptr_phi(2, 10, 7),
            load(3, 2, IrType::I32),
            {
                // Build CallInfo via struct-update from a template so the test
                // stays robust against new ABI metadata fields.
                let mut info: crate::ir::reexports::CallInfo = Default::default();
                info.dest = None;
                info.args = vec![Operand::Value(Value(2))]; // phi escapes!
                info.arg_types = vec![IrType::Ptr];
                info.return_type = IrType::Void;
                Instruction::Call {
                    func: "consume".to_string(),
                    info,
                }
            },
            counter_inc(6, 1),
            gep_const(7, 2, 4),
        ];
        f.next_value_id = 11;
        assert_eq!(
            run_univsr(&mut f),
            0,
            "pointer escaping to a call must be rejected"
        );
    }

    #[test]
    fn reject_pointer_stored_as_data() {
        let mut f = make_test_function();
        f.blocks[0].instructions = vec![
            Instruction::GlobalAddr {
                dest: Value(10),
                name: "arr".to_string(),
            },
            Instruction::GlobalAddr {
                dest: Value(11),
                name: "slot".to_string(),
            },
        ];
        f.blocks[1].instructions = vec![
            counter_phi(1, 6),
            ptr_phi(2, 10, 7),
            Instruction::Store {
                volatile: false,
                val: Operand::Value(Value(2)), // phi stored as DATA!
                ptr: Value(11),
                ty: IrType::Ptr,
                seg_override: AddressSpace::Default,
            },
            counter_inc(6, 1),
            gep_const(7, 2, 4),
        ];
        f.next_value_id = 12;
        assert_eq!(
            run_univsr(&mut f),
            0,
            "pointer stored as data must be rejected"
        );
    }

    #[test]
    fn no_ptr_phi_fast_path() {
        let mut f = make_test_function();
        f.blocks[1].instructions = vec![counter_phi(1, 6), counter_inc(6, 1)];
        f.next_value_id = 7;
        assert_eq!(run_univsr(&mut f), 0);
    }
}
