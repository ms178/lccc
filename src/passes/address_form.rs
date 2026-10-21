//! Normalize explicit full-width pointer additions before IVSR. This exposes
//! ordinary base+index addressing to the existing GEP folding/RA machinery,
//! instead of requiring a second loop-carried pointer for a free x86 SIB.
use crate::common::types::IrType;
use crate::ir::reexports::{Instruction, IrBinOp, IrConst, IrFunction, Operand, Value};

pub(crate) fn run(f: &mut IrFunction) -> usize {
    let mut defs = vec![None; f.max_value_id() as usize + 1];
    let mut addressed = vec![false; defs.len()];
    for i in f.blocks.iter().flat_map(|b| &b.instructions) {
        if let Some(d) = i.dest() {
            if defs[d.0 as usize].replace(i).is_some() {
                return 0;
            }
        }
        match i {
            Instruction::Load { ptr, .. } | Instruction::Store { ptr, .. } => {
                addressed[ptr.0 as usize] = true
            }
            Instruction::GetElementPtr { base, .. } => addressed[base.0 as usize] = true,
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
    count
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
}
