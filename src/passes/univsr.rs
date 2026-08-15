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
use crate::ir::reexports::{
    BlockId,
    Instruction,
    IrBinOp,
    IrConst,
    IrFunction,
    Operand,
    Value,
};

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

    let ivsr_pointers = detect_ivsr_pointer_ivs(func);
    if ivsr_pointers.is_empty() {
        return 0;
    }

    let mut num_reverted = 0;
    for ptr_iv in &ivsr_pointers {
        if !is_valid_sib_scale(ptr_iv.stride) {
            continue;
        }

        if revert_pointer_iv(func, ptr_iv) {
            num_reverted += 1;
        }
    }

    num_reverted
}

/// Detect IVSR-created pointer IVs in the function.
fn detect_ivsr_pointer_ivs(func: &IrFunction) -> Vec<IvsrPointerIV> {
    let mut result = Vec::new();

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
            if let Instruction::GetElementPtr { dest, base, offset, .. } = inst {
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
                    if let Instruction::GetElementPtr { dest, base, offset, .. } = inst {
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

            // Verify that this integer Phi is incremented by a constant in the loop
            if is_basic_iv(func, dest, incoming) {
                return Some(*dest);
            }
        }
    }

    None
}

/// Check if a Phi node represents a basic induction variable (incremented by a constant on backedge).
fn is_basic_iv(func: &IrFunction, dest: &Value, incoming: &[(Operand, BlockId)]) -> bool {
    for (op, _block) in incoming {
        if let Operand::Value(v) = op {
            if is_value_from_iv_increment(func, *v, *dest) {
                return true;
            }
        }
    }
    false
}

/// Check if `val` is defined as `Add(%phi, const)` or through Copy/Cast chains from such an addition.
fn is_value_from_iv_increment(func: &IrFunction, val: Value, phi_dest: Value) -> bool {
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::BinOp { dest: bd, op: IrBinOp::Add, lhs, rhs, .. } if bd.0 == val.0 => {
                    match (lhs, rhs) {
                        (Operand::Value(v), Operand::Const(c)) | (Operand::Const(c), Operand::Value(v)) => {
                            if v.0 == phi_dest.0 && c.to_i64().is_some() {
                                return true;
                            }
                        }
                        _ => {}
                    }
                }
                Instruction::Copy { dest: cd, src: Operand::Value(v) } if cd.0 == val.0 => {
                    if is_value_from_iv_increment(func, *v, phi_dest) {
                        return true;
                    }
                }
                Instruction::Cast { dest: cd, src: Operand::Value(v), .. } if cd.0 == val.0 => {
                    if is_value_from_iv_increment(func, *v, phi_dest) {
                        return true;
                    }
                }
                _ => {}
            }
        }
    }
    false
}

/// Validate that the pointer IV does not escape (e.g. stored to memory, passed to calls, or returned).
fn validate_transformation_safety(func: &IrFunction, ptr_iv: &IvsrPointerIV) -> bool {
    let phi_id = ptr_iv.ptr_phi_dest.0;

    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                // Pointer stored as data value to memory
                Instruction::Store { val: Operand::Value(v), .. } if v.0 == phi_id => {
                    return false;
                }
                // Pointer passed as argument to function call
                Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
                    for arg in &info.args {
                        if let Operand::Value(v) = arg {
                            if v.0 == phi_id {
                                return false;
                            }
                        }
                    }
                }
                // Pointer used in comparison
                Instruction::Cmp { lhs, rhs, .. } => {
                    let is_phi = |op: &Operand| matches!(op, Operand::Value(v) if v.0 == phi_id);
                    if is_phi(lhs) || is_phi(rhs) {
                        return false;
                    }
                }
                // Pointer passed to an unrelated Phi node
                Instruction::Phi { dest, incoming, .. } if dest.0 != phi_id => {
                    for (op, _) in incoming {
                        if let Operand::Value(v) = op {
                            if v.0 == phi_id {
                                return false;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    true
}

/// Find all transitive Load/Store uses of the pointer phi, tracking accumulated constant offsets.
fn find_transitive_ptr_uses(func: &IrFunction, ptr_iv: &IvsrPointerIV) -> Vec<PtrUse> {
    let mut results = Vec::new();
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
                    Instruction::GetElementPtr { dest, base, offset, .. } if base.0 == val_to_check.0 => {
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
                    Instruction::Copy { dest, src: Operand::Value(v) } if v.0 == val_to_check.0 => {
                        if visited_values.insert(dest.0) {
                            worklist.push((*dest, current_offset));
                        }
                    }
                    Instruction::Cast { dest, src: Operand::Value(v), .. } if v.0 == val_to_check.0 => {
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
            | Instruction::Intrinsic { dest: Some(dest), .. }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::reexports::{IrBlock, Span};

    fn make_test_function() -> IrFunction {
        IrFunction {
            name: "test_kernel".to_string(),
            params: Vec::new(),
            return_ty: IrType::Void,
            blocks: vec![
                IrBlock {
                    label: BlockId(0),
                    instructions: Vec::new(),
                    source_spans: Vec::new(),
                },
                IrBlock {
                    label: BlockId(1),
                    instructions: Vec::new(),
                    source_spans: Vec::new(),
                },
            ],
            next_value_id: 1,
            is_declaration: false,
            attributes: Default::default(),
        }
    }

    #[test]
    fn test_univsr_basic_reversion() {
        let mut func = make_test_function();

        // Block 0: Preheader
        func.blocks[0].instructions = vec![
            Instruction::GlobalAddr { dest: Value(1), name: "arr".to_string() },
            Instruction::Copy { dest: Value(2), src: Operand::Const(IrConst::I64(0)) },
            Instruction::GetElementPtr {
                dest: Value(3),
                base: Value(1),
                offset: Operand::Const(IrConst::I64(0)),
                ty: IrType::Ptr,
            },
        ];
        func.blocks[0].source_spans = vec![Span::new(0, 0, 0); 3];

        // Block 1: Header + Loop Body
        func.blocks[1].instructions = vec![
            Instruction::Phi {
                dest: Value(4),
                ty: IrType::I64,
                incoming: vec![
                    (Operand::Value(Value(2)), BlockId(0)),
                    (Operand::Value(Value(7)), BlockId(1)),
                ],
            },
            Instruction::Phi {
                dest: Value(5),
                ty: IrType::Ptr,
                incoming: vec![
                    (Operand::Value(Value(3)), BlockId(0)),
                    (Operand::Value(Value(8)), BlockId(1)),
                ],
            },
            Instruction::Load {
                dest: Value(6),
                ptr: Value(5),
                ty: IrType::I32,
            },
            Instruction::BinOp {
                dest: Value(7),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(4)),
                rhs: Operand::Const(IrConst::I64(1)),
                ty: IrType::I64,
            },
            Instruction::GetElementPtr {
                dest: Value(8),
                base: Value(5),
                offset: Operand::Const(IrConst::I64(4)),
                ty: IrType::Ptr,
            },
        ];
        func.blocks[1].source_spans = vec![Span::new(0, 0, 0); 5];
        func.next_value_id = 9;

        let count = run_univsr(&mut func);
        assert_eq!(count, 1);

        // Verify that Load now uses the newly generated GEP result
        let has_indexed_load = func.blocks[1].instructions.iter().any(|inst| {
            if let Instruction::Load { ptr, .. } = inst {
                *ptr != Value(5) // Pointer was replaced!
            } else {
                false
            }
        });
        assert!(has_indexed_load);

        // Verify 1:1 sync of instructions and source_spans
        assert_eq!(func.blocks[1].instructions.len(), func.blocks[1].source_spans.len());
    }

    #[test]
    fn test_univsr_transitive_gep_offset_preserved() {
        let mut func = make_test_function();

        // Block 1: Header with 2 accesses: a[i] and a[i+1] (offset +4)
        func.blocks[1].instructions = vec![
            Instruction::Phi {
                dest: Value(1),
                ty: IrType::I64,
                incoming: vec![
                    (Operand::Const(IrConst::I64(0)), BlockId(0)),
                    (Operand::Value(Value(6)), BlockId(1)),
                ],
            },
            Instruction::Phi {
                dest: Value(2),
                ty: IrType::Ptr,
                incoming: vec![
                    (Operand::Value(Value(10)), BlockId(0)),
                    (Operand::Value(Value(7)), BlockId(1)),
                ],
            },
            Instruction::Load { dest: Value(3), ptr: Value(2), ty: IrType::I32 },
            Instruction::GetElementPtr {
                dest: Value(4),
                base: Value(2),
                offset: Operand::Const(IrConst::I64(4)),
                ty: IrType::Ptr,
            },
            Instruction::Load { dest: Value(5), ptr: Value(4), ty: IrType::I32 },
            Instruction::BinOp {
                dest: Value(6),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I64(1)),
                ty: IrType::I64,
            },
            Instruction::GetElementPtr {
                dest: Value(7),
                base: Value(2),
                offset: Operand::Const(IrConst::I64(8)), // Stride 8
                ty: IrType::Ptr,
            },
        ];
        func.blocks[1].source_spans = vec![Span::new(0, 0, 0); 7];
        func.next_value_id = 11;

        let count = run_univsr(&mut func);
        assert_eq!(count, 1);

        // Verify that the second load has an addition with constant 4
        let has_add_offset_4 = func.blocks[1].instructions.iter().any(|inst| {
            matches!(inst, Instruction::BinOp { op: IrBinOp::Add, rhs: Operand::Const(c), .. } if c.to_i64() == Some(4))
        });
        assert!(has_add_offset_4, "Transitive GEP offset +4 must be preserved!");
    }

    #[test]
    fn test_univsr_stride_1_no_shl() {
        let mut func = make_test_function();

        func.blocks[1].instructions = vec![
            Instruction::Phi {
                dest: Value(1),
                ty: IrType::I64,
                incoming: vec![
                    (Operand::Const(IrConst::I64(0)), BlockId(0)),
                    (Operand::Value(Value(4)), BlockId(1)),
                ],
            },
            Instruction::Phi {
                dest: Value(2),
                ty: IrType::Ptr,
                incoming: vec![
                    (Operand::Value(Value(10)), BlockId(0)),
                    (Operand::Value(Value(5)), BlockId(1)),
                ],
            },
            Instruction::Load { dest: Value(3), ptr: Value(2), ty: IrType::I8 },
            Instruction::BinOp {
                dest: Value(4),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I64(1)),
                ty: IrType::I64,
            },
            Instruction::GetElementPtr {
                dest: Value(5),
                base: Value(2),
                offset: Operand::Const(IrConst::I64(1)), // Stride 1
                ty: IrType::Ptr,
            },
        ];
        func.blocks[1].source_spans = vec![Span::new(0, 0, 0); 5];
        func.next_value_id = 11;

        let count = run_univsr(&mut func);
        assert_eq!(count, 1);

        // Stride 1: No Shl instruction should be emitted
        let has_shl = func.blocks[1].instructions.iter().any(|inst| {
            matches!(inst, Instruction::BinOp { op: IrBinOp::Shl, .. })
        });
        assert!(!has_shl, "Stride 1 should not emit Shl instruction");
    }

    #[test]
    fn test_univsr_multiple_accesses_deduplication() {
        let mut func = make_test_function();

        // 3 accesses to the exact same pointer in the same block
        func.blocks[1].instructions = vec![
            Instruction::Phi {
                dest: Value(1),
                ty: IrType::I64,
                incoming: vec![
                    (Operand::Const(IrConst::I64(0)), BlockId(0)),
                    (Operand::Value(Value(6)), BlockId(1)),
                ],
            },
            Instruction::Phi {
                dest: Value(2),
                ty: IrType::Ptr,
                incoming: vec![
                    (Operand::Value(Value(10)), BlockId(0)),
                    (Operand::Value(Value(7)), BlockId(1)),
                ],
            },
            Instruction::Load { dest: Value(3), ptr: Value(2), ty: IrType::I32 },
            Instruction::Store { val: Operand::Value(Value(3)), ptr: Value(2), ty: IrType::I32 },
            Instruction::Load { dest: Value(5), ptr: Value(2), ty: IrType::I32 },
            Instruction::BinOp {
                dest: Value(6),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I64(1)),
                ty: IrType::I64,
            },
            Instruction::GetElementPtr {
                dest: Value(7),
                base: Value(2),
                offset: Operand::Const(IrConst::I64(4)), // Stride 4
                ty: IrType::Ptr,
            },
        ];
        func.blocks[1].source_spans = vec![Span::new(0, 0, 0); 7];
        func.next_value_id = 11;

        let count = run_univsr(&mut func);
        assert_eq!(count, 1);

        // Count how many Shl instructions were inserted: must be exactly 1 due to CSE deduplication
        let shl_count = func.blocks[1].instructions.iter().filter(|inst| {
            matches!(inst, Instruction::BinOp { op: IrBinOp::Shl, .. })
        }).count();
        assert_eq!(shl_count, 1, "Shl must be deduplicated across multiple accesses in the same block");
    }

    #[test]
    fn test_univsr_skip_invalid_scale() {
        let mut func = make_test_function();

        // Stride 3 (not a valid SIB scale 1, 2, 4, 8)
        func.blocks[1].instructions = vec![
            Instruction::Phi {
                dest: Value(1),
                ty: IrType::I64,
                incoming: vec![
                    (Operand::Const(IrConst::I64(0)), BlockId(0)),
                    (Operand::Value(Value(4)), BlockId(1)),
                ],
            },
            Instruction::Phi {
                dest: Value(2),
                ty: IrType::Ptr,
                incoming: vec![
                    (Operand::Value(Value(10)), BlockId(0)),
                    (Operand::Value(Value(5)), BlockId(1)),
                ],
            },
            Instruction::Load { dest: Value(3), ptr: Value(2), ty: IrType::I8 },
            Instruction::BinOp {
                dest: Value(4),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I64(1)),
                ty: IrType::I64,
            },
            Instruction::GetElementPtr {
                dest: Value(5),
                base: Value(2),
                offset: Operand::Const(IrConst::I64(3)), // Invalid SIB stride
                ty: IrType::Ptr,
            },
        ];
        func.blocks[1].source_spans = vec![Span::new(0, 0, 0); 5];
        func.next_value_id = 11;

        let count = run_univsr(&mut func);
        assert_eq!(count, 0, "Invalid SIB scale 3 must be skipped");
    }
}
