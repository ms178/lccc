//! Normalize explicit full-width pointer additions before IVSR, and
//! reassociate `GEP(GEP(root, Const(c)), Value(off))` into
//! `GEP(root, Add(off, Const(c)))` when every consumer is a foldable Load/Store
//! so x86-64 SIB addressing emits `c(%root, %idx, scale)` off a single base
//! register instead of holding one hoisted `leaq c(%root), %rN` per row/field.
use crate::common::types::{AddressSpace, IrType};
use crate::ir::reexports::{Instruction, IrBinOp, IrConst, IrFunction, Operand, Terminator, Value};

pub(crate) fn run(f: &mut IrFunction) -> usize {
    let reassociated = reassociate_const_base_indexed_geps(f);
    let mut defs = vec![None; f.max_value_id() as usize + 1];
    let mut addressed = vec![false; defs.len()];
    for i in f.blocks.iter().flat_map(|b| &b.instructions) {
        if let Some(d) = i.dest() {
            if defs[d.0 as usize].replace(i).is_some() {
                return reassociated;
            }
        }
        match i {
            Instruction::Load { ptr, .. } | Instruction::Store { ptr, .. } => {
                addressed[ptr.0 as usize] = true
            }
            Instruction::GetElementPtr {
                base,
                offset: Operand::Const(_),
                ..
            } => addressed[base.0 as usize] = true,
            _ => {}
        }
    }
    fn ty(mut v: Value, defs: &[Option<&Instruction>]) -> Option<IrType> {
        for _ in 0..16 {
            match defs.get(v.0 as usize).copied().flatten()? {
                Instruction::Copy {
                    src: Operand::Value(src),
                    ..
                } => v = *src,
                i => return i.result_type(),
            }
        }
        None
    }
    let mut plans = Vec::new();
    for (bi, b) in f.blocks.iter().enumerate() {
        for (ii, i) in b.instructions.iter().enumerate() {
            let Instruction::BinOp {
                dest,
                op: IrBinOp::Add,
                lhs,
                rhs,
                ty: carrier @ (IrType::Ptr | IrType::I64 | IrType::U64),
            } = i
            else {
                continue;
            };
            if !addressed[dest.0 as usize] {
                continue;
            }
            for (base, offset) in [(*lhs, *rhs), (*rhs, *lhs)] {
                let Operand::Value(base) = base else { continue };
                if ty(base, &defs) != Some(IrType::Ptr) {
                    continue;
                }
                let wide = match offset {
                    Operand::Const(IrConst::I64(_)) => true,
                    Operand::Value(v) => matches!(ty(v, &defs), Some(IrType::I64 | IrType::U64)),
                    _ => false,
                };
                if wide {
                    plans.push((
                        bi,
                        ii,
                        Instruction::GetElementPtr {
                            dest: *dest,
                            base,
                            offset,
                            ty: *carrier,
                        },
                    ));
                    break;
                }
            }
        }
    }
    let count = plans.len();
    for (bi, ii, i) in plans {
        f.blocks[bi].instructions[ii] = i;
    }
    reassociated + count
}

fn reassociate_const_base_indexed_geps(f: &mut IrFunction) -> usize {
    let max_id = f.max_value_id() as usize;
    let mut defs: Vec<Option<Instruction>> = vec![None; max_id + 1];
    let mut multi_def = vec![false; max_id + 1];
    let mut alloca_ids = vec![false; max_id + 1];
    let mut total_uses = vec![0u32; max_id + 1];
    let mut foldable_mem_uses = vec![0u32; max_id + 1];

    let bump = |v: Value, counts: &mut [u32]| {
        if let Some(slot) = counts.get_mut(v.0 as usize) {
            *slot += 1;
        }
    };

    for b in &f.blocks {
        for inst in &b.instructions {
            if let Some(d) = inst.dest() {
                let idx = d.0 as usize;
                if defs[idx].is_some() {
                    multi_def[idx] = true;
                } else {
                    defs[idx] = Some(inst.clone());
                }
            }
            if let Instruction::Alloca { dest, .. } | Instruction::DynAlloca { dest, .. } = inst {
                alloca_ids[dest.0 as usize] = true;
            }
            crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    bump(*v, &mut total_uses);
                }
            });
            crate::backend::liveness::for_each_value_use_in_instruction(inst, |v| {
                bump(*v, &mut total_uses);
            });
            match inst {
                Instruction::Load {
                    ptr,
                    ty,
                    seg_override: AddressSpace::Default,
                    ..
                } if !matches!(ty, IrType::I128 | IrType::U128) => {
                    bump(*ptr, &mut foldable_mem_uses);
                }
                Instruction::Store {
                    val,
                    ptr,
                    ty,
                    seg_override: AddressSpace::Default,
                    ..
                } if !matches!(ty, IrType::I128 | IrType::U128) && *val != Operand::Value(*ptr) => {
                    bump(*ptr, &mut foldable_mem_uses);
                }
                _ => {}
            }
        }
        match &b.terminator {
            Terminator::Return(Some(Operand::Value(v)))
            | Terminator::CondBranch {
                cond: Operand::Value(v),
                ..
            }
            | Terminator::Switch {
                val: Operand::Value(v),
                ..
            }
            | Terminator::IndirectBranch {
                target: Operand::Value(v),
                ..
            } => bump(*v, &mut total_uses),
            _ => {}
        }
    }

    if multi_def.iter().any(|&m| m) {
        return 0;
    }

    let val_ty = |mut v: Value| -> Option<IrType> {
        for _ in 0..16 {
            match defs.get(v.0 as usize).and_then(Option::as_ref)? {
                Instruction::Copy {
                    src: Operand::Value(src),
                    ..
                } => v = *src,
                inst => return inst.result_type(),
            }
        }
        None
    };

    // Refuse offsets that already carry a peeled constant `Add`/`Sub` inside a
    // shift/cast chain (`resolve_index` in `generation.rs` peels at most one
    // constant `Add`/`Sub`).
    let offset_has_inner_const_add = |mut v: Value| -> bool {
        for _ in 0..16 {
            let Some(inst) = defs.get(v.0 as usize).and_then(Option::as_ref) else {
                return false;
            };
            match inst {
                Instruction::Copy {
                    src: Operand::Value(src),
                    ..
                }
                | Instruction::Cast {
                    src: Operand::Value(src),
                    ..
                } => v = *src,
                Instruction::BinOp {
                    op: IrBinOp::Shl | IrBinOp::Mul,
                    lhs: Operand::Value(src),
                    rhs: Operand::Const(_),
                    ..
                } => v = *src,
                Instruction::BinOp {
                    op: IrBinOp::Add | IrBinOp::Sub,
                    lhs: Operand::Value(_),
                    rhs: Operand::Const(_),
                    ..
                }
                | Instruction::BinOp {
                    op: IrBinOp::Add,
                    lhs: Operand::Const(_),
                    rhs: Operand::Value(_),
                    ..
                } => return true,
                _ => return false,
            }
        }
        true
    };

    // Map eligible g1 (`GEP(root, Const(disp))` with non-zero i32 disp) -> (root, disp).
    let mut const_geps: std::collections::HashMap<u32, (Value, i64)> =
        std::collections::HashMap::new();
    for (id, def) in defs.iter().enumerate() {
        let Some(Instruction::GetElementPtr {
            dest,
            base,
            offset: Operand::Const(c),
            ..
        }) = def
        else {
            continue;
        };
        if dest.0 as usize != id
            || total_uses[id] == 0
            || alloca_ids.get(base.0 as usize).copied().unwrap_or(false)
        {
            continue;
        }
        let Some(disp) = c.to_i64() else {
            continue;
        };
        if disp == 0 || !(i32::MIN as i64..=i32::MAX as i64).contains(&disp) {
            continue;
        }
        const_geps.insert(dest.0, (*base, disp));
    }
    if const_geps.is_empty() {
        return 0;
    }

    // Count how many uses of each g1 are eligible indexed g2s whose own uses
    // are 100% foldable Load/Store pointer operands. Only rewrite g1s where
    // EVERY use is an eligible g2, guaranteeing g1 becomes dead after DCE.
    let mut eligible_g1_uses: std::collections::HashMap<u32, u32> =
        std::collections::HashMap::new();
    let mut candidate_g2s: Vec<(usize, usize, Value, Value, i64, Value, IrType)> = Vec::new();

    for (bi, b) in f.blocks.iter().enumerate() {
        for (ii, inst) in b.instructions.iter().enumerate() {
            let Instruction::GetElementPtr {
                dest,
                base,
                offset: Operand::Value(off),
                ty,
            } = inst
            else {
                continue;
            };
            let Some(&(root, disp)) = const_geps.get(&base.0) else {
                continue;
            };
            let g2_idx = dest.0 as usize;
            if total_uses[g2_idx] == 0 || total_uses[g2_idx] != foldable_mem_uses[g2_idx] {
                continue;
            }
            let Some(off_ty) = val_ty(*off) else {
                continue;
            };
            if !matches!(off_ty, IrType::I64 | IrType::U64) || offset_has_inner_const_add(*off) {
                continue;
            }
            *eligible_g1_uses.entry(base.0).or_insert(0) += 1;
            candidate_g2s.push((bi, ii, *dest, root, disp, *off, *ty));
        }
    }

    candidate_g2s.retain(|(_, _, _, _, _, _, _)| true);
    let mut rewrites_by_block: std::collections::BTreeMap<
        usize,
        Vec<(usize, Value, Value, i64, Value, IrType)>,
    > = std::collections::BTreeMap::new();
    for (bi, ii, dest, root, disp, off, ty) in candidate_g2s {
        let g1_id = match &f.blocks[bi].instructions[ii] {
            Instruction::GetElementPtr { base, .. } => base.0,
            _ => continue,
        };
        if eligible_g1_uses.get(&g1_id).copied().unwrap_or(0) != total_uses[g1_id as usize] {
            continue;
        }
        rewrites_by_block
            .entry(bi)
            .or_default()
            .push((ii, dest, root, disp, off, ty));
    }
    if rewrites_by_block.is_empty() {
        return 0;
    }

    let mut next_id = f.next_value_id.max(max_id as u32 + 1);
    let mut total_rewritten = 0usize;
    for (bi, mut block_rewrites) in rewrites_by_block {
        block_rewrites.sort_unstable_by_key(|r| r.0);
        let block = &mut f.blocks[bi];
        let keep_spans =
            block.source_spans.len() == block.instructions.len() && !block.source_spans.is_empty();
        let old_insts = std::mem::take(&mut block.instructions);
        let old_spans = std::mem::take(&mut block.source_spans);
        let mut new_insts = Vec::with_capacity(old_insts.len() + block_rewrites.len());
        let mut new_spans = Vec::with_capacity(if keep_spans {
            old_spans.len() + block_rewrites.len()
        } else {
            0
        });
        let mut rw_iter = block_rewrites.into_iter().peekable();
        for (idx, inst) in old_insts.into_iter().enumerate() {
            if let Some(&(_, dest, root, disp, off, ty)) = rw_iter.peek().filter(|r| r.0 == idx) {
                rw_iter.next();
                let add_dest = Value(next_id);
                next_id += 1;
                let off_ty = val_ty(off).unwrap_or(IrType::I64);
                new_insts.push(Instruction::BinOp {
                    dest: add_dest,
                    op: IrBinOp::Add,
                    lhs: Operand::Value(off),
                    rhs: Operand::Const(IrConst::I64(disp)),
                    ty: off_ty,
                });
                new_insts.push(Instruction::GetElementPtr {
                    dest,
                    base: root,
                    offset: Operand::Value(add_dest),
                    ty,
                });
                if keep_spans {
                    let sp = old_spans[idx];
                    new_spans.push(sp);
                    new_spans.push(sp);
                }
                total_rewritten += 1;
            } else {
                new_insts.push(inst);
                if keep_spans {
                    new_spans.push(old_spans[idx]);
                }
            }
        }
        block.instructions = new_insts;
        if keep_spans {
            block.source_spans = new_spans;
        }
    }
    f.next_value_id = next_id;
    total_rewritten
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, Terminator};
    fn function(offset: IrType, carrier: IrType, reverse: bool) -> IrFunction {
        let mut f = IrFunction::new("address".into(), IrType::Void, vec![], false);
        let (a, b) = (Operand::Value(Value(0)), Operand::Value(Value(1)));
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(0),
                    name: "data".into(),
                },
                Instruction::BinOp {
                    dest: Value(1),
                    op: IrBinOp::Add,
                    lhs: Operand::Const(IrConst::I64(1)),
                    rhs: Operand::Const(IrConst::I64(2)),
                    ty: offset,
                },
                Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::Add,
                    lhs: if reverse { b } else { a },
                    rhs: if reverse { a } else { b },
                    ty: carrier,
                },
                Instruction::Load {
                    dest: Value(3),
                    ptr: Value(2),
                    ty: IrType::U32,
                    volatile: false,
                    seg_override: crate::common::types::AddressSpace::Default,
                },
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        f.next_value_id = 4;
        f
    }
    #[test]
    fn normalizes_full_width_in_both_orders_preserving_carrier() {
        for offset in [IrType::I64, IrType::U64] {
            for carrier in [IrType::Ptr, IrType::I64, IrType::U64] {
                for reverse in [false, true] {
                    let mut f = function(offset, carrier, reverse);
                    assert_eq!(run(&mut f), 1);
                    assert!(
                        matches!(f.blocks[0].instructions[2],Instruction::GetElementPtr {base:Value(0),offset:Operand::Value(Value(1)),ty,..} if ty==carrier)
                    );
                    assert_eq!(run(&mut f), 0);
                }
            }
        }
    }
    #[test]
    fn rejects_narrow_offsets_unused_addresses_and_non_ssa() {
        for offset in [IrType::U32, IrType::I32, IrType::U16] {
            let mut f = function(offset, IrType::Ptr, false);
            assert_eq!(run(&mut f), 0);
        }
        let mut f = function(IrType::U64, IrType::Ptr, false);
        f.blocks[0].instructions.pop();
        assert_eq!(run(&mut f), 0);
        let mut f = function(IrType::U64, IrType::Ptr, false);
        f.blocks[0].instructions.push(Instruction::Copy {
            dest: Value(0),
            src: Operand::Const(IrConst::I64(0)),
        });
        assert_eq!(run(&mut f), 0);
    }
    #[test]
    fn preserves_variable_offset_gep_base_for_reg_off_sym_fold() {
        let mut f = function(IrType::U64, IrType::Ptr, false);
        f.blocks[0].instructions[3] = Instruction::GetElementPtr {
            dest: Value(3),
            base: Value(2),
            offset: Operand::Value(Value(1)),
            ty: IrType::Ptr,
        };
        assert_eq!(run(&mut f), 0);
        f.blocks[0].instructions[3] = Instruction::GetElementPtr {
            dest: Value(3),
            base: Value(2),
            offset: Operand::Const(IrConst::I64(4)),
            ty: IrType::Ptr,
        };
        assert_eq!(run(&mut f), 1);
    }

    #[test]
    fn folds_const_base_gep_into_variable_gep_only_for_load_store_consumers() {
        let make = |load_consumer: bool, nested_add_const: bool| {
            let mut f = IrFunction::new("slice_gep".into(), IrType::Void, Vec::new(), false);
            let mut insns = vec![
                Instruction::GlobalAddr {
                    dest: Value(0),
                    name: "crc_braid_table".into(),
                },
                Instruction::GetElementPtr {
                    dest: Value(1),
                    base: Value(0),
                    offset: Operand::Const(IrConst::I64(1024)),
                    ty: IrType::Ptr,
                },
                Instruction::ParamRef {
                    dest: Value(2),
                    param_idx: 0,
                    ty: IrType::I64,
                },
            ];
            let idx_val = if nested_add_const {
                insns.push(Instruction::BinOp {
                    dest: Value(5),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Const(IrConst::I64(4)),
                    ty: IrType::I64,
                });
                Value(5)
            } else {
                Value(2)
            };
            insns.push(Instruction::GetElementPtr {
                dest: Value(3),
                base: Value(1),
                offset: Operand::Value(idx_val),
                ty: IrType::Ptr,
            });
            if load_consumer {
                insns.push(Instruction::Load {
                    dest: Value(4),
                    ptr: Value(3),
                    ty: IrType::U32,
                    volatile: false,
                    seg_override: crate::common::types::AddressSpace::Default,
                });
            }
            f.blocks.push(BasicBlock {
                label: BlockId(0),
                instructions: insns,
                terminator: if load_consumer {
                    Terminator::Return(None)
                } else {
                    Terminator::Return(Some(Operand::Value(Value(3))))
                },
                source_spans: Vec::new(),
            });
            f.next_value_id = 6;
            f
        };

        let mut foldable = make(true, false);
        assert_eq!(run(&mut foldable), 1);
        assert!(foldable.blocks[0].instructions.iter().any(|i| matches!(
            i,
            Instruction::GetElementPtr {
                dest: Value(3),
                base: Value(0),
                ..
            }
        )));

        let mut returned_ptr = make(false, false);
        assert_eq!(run(&mut returned_ptr), 0);

        let mut nested_const = make(true, true);
        assert_eq!(run(&mut nested_const), 0);
    }
}
