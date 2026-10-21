//! Forward an eight-byte SIMD store to the identical next-iteration pair
//! load. Only counted, definitely-entered, two-block loops with a single
//! store and lane-local zero-upper-half computations are accepted.
use super::loop_analysis;
use crate::common::fx_hash::FxHashSet;
use crate::common::types::IrType;
use crate::ir::{analysis, reexports::*};

pub(crate) fn run(f: &mut IrFunction) -> usize {
    if !f.blocks.iter().flat_map(|b| &b.instructions).any(|i| {
        matches!(
            i,
            Instruction::Intrinsic {
                op: IntrinsicOp::VecStoreI32x4Pair,
                ..
            }
        )
    }) {
        return 0;
    }
    let mut definitions = FxHashSet::default();
    if f.blocks
        .iter()
        .flat_map(|b| &b.instructions)
        .filter_map(Instruction::dest)
        .any(|v| !definitions.insert(v))
    {
        return 0;
    }
    let cfg = analysis::CfgAnalysis::build(f);
    let loops =
        loop_analysis::find_merged_natural_loops(f.blocks.len(), &cfg.preds, &cfg.succs, &cfg.idom);
    let mut n = 0;
    for lp in loops {
        if lp.body.len() != 2 {
            continue;
        }
        let h = lp.header;
        let Some(b) = lp.body.iter().copied().find(|&b| b != h) else {
            continue;
        };
        let Some(pre) = loop_analysis::find_preheader(h, &lp.body, &cfg.preds) else {
            continue;
        };
        if cfg.preds.row(b) != [h as u32]
            || !matches!(f.blocks[b].terminator,Terminator::Branch(l) if l==f.blocks[h].label)
            || !matches!(f.blocks[pre].terminator,Terminator::Branch(l) if l==f.blocks[h].label)
        {
            continue;
        }
        if apply(f, h, b, pre) {
            n += 1;
        }
    }
    n
}
fn apply(f: &mut IrFunction, h: usize, b: usize, pre: usize) -> bool {
    use IntrinsicOp as O;
    let prelabel = f.blocks[pre].label;
    let backlabel = f.blocks[b].label;
    let instructions = &f.blocks[h].instructions;
    // Prove that the initial header visit enters the body. Moving a load
    // out of a possibly-zero-trip loop would otherwise introduce a fault.
    let [
        Instruction::Phi {
            dest: iv,
            ty: IrType::I32,
            incoming: ii,
        },
        Instruction::Phi {
            dest: ptr,
            ty: IrType::Ptr,
            incoming: pi,
        },
        Instruction::Cmp {
            dest: cmp,
            op: IrCmpOp::Slt,
            lhs: Operand::Value(ci),
            rhs: Operand::Const(IrConst::I32(limit)),
            ty: IrType::I32,
        },
    ] = instructions.as_slice()
    else {
        return false;
    };
    if ci != iv || ii.len() != 2 || pi.len() != 2 {
        return false;
    }
    let Some((Operand::Const(IrConst::I32(start)), _)) = ii.iter().find(|(_, l)| *l == prelabel)
    else {
        return false;
    };
    if start >= limit
        || !matches!(f.blocks[h].terminator,Terminator::CondBranch {cond:Operand::Value(c),true_label,..} if c==*cmp && true_label==backlabel)
    {
        return false;
    }
    let Some((Operand::Value(initial_ptr), _)) = pi.iter().find(|(_, l)| *l == prelabel) else {
        return false;
    };
    let Some((Operand::Value(next_ptr), _)) = pi.iter().find(|(_, l)| *l == backlabel) else {
        return false;
    };
    let ptr = *ptr;
    let initial_ptr = *initial_ptr;
    let next_ptr = *next_ptr;
    let mut zero_vectors = FxHashSet::default();
    let mut candidate = None;
    let mut store = None;
    let mut stepped = false;
    for (index, i) in f.blocks[b].instructions.iter().enumerate() {
        match i {
            Instruction::Intrinsic {
                dest: Some(dest),
                op: O::VecLoadI32x4Pair,
                dest_ptr: None,
                args,
            } => {
                // Exact byte-address form after IVSR, not an element index.
                if let [
                    Operand::Value(base),
                    Operand::Const(IrConst::I64(0)),
                    Operand::Const(IrConst::I64(disp)),
                ] = args.as_slice()
                {
                    if *base != ptr {
                        return false;
                    }
                    if *disp == -8 {
                        if candidate.replace((index, *dest)).is_some() {
                            return false;
                        }
                    }
                } else {
                    return false;
                }
                zero_vectors.insert(*dest);
            }
            Instruction::Intrinsic {
                dest: Some(dest),
                op,
                dest_ptr: None,
                args,
            } if matches!(
                op,
                O::VecAddI32x4 | O::VecXorI32x4 | O::VecOrI32x4 | O::VecShlI32x4 | O::VecLShrI32x4
            ) =>
            {
                // All four lanes are independent. Pair loads zero lanes 2/3;
                // these operations keep them zero (no cross-lane consumers).
                let valid = match (op, args.as_slice()) {
                    (O::VecShlI32x4 | O::VecLShrI32x4, [Operand::Value(a), Operand::Const(c)]) => {
                        zero_vectors.contains(a) && c.to_i64().is_some_and(|n| (0..32).contains(&n))
                    }
                    (_, [Operand::Value(a), Operand::Value(b)]) => {
                        zero_vectors.contains(a) && zero_vectors.contains(b)
                    }
                    _ => false,
                };
                if !valid {
                    return false;
                }
                zero_vectors.insert(*dest);
            }
            Instruction::Intrinsic {
                dest: None,
                op: O::VecStoreI32x4Pair,
                dest_ptr: Some(base),
                args,
            } => {
                let [
                    Operand::Value(data),
                    Operand::Value(base2),
                    Operand::Const(IrConst::I64(0)),
                    Operand::Const(IrConst::I64(0)),
                ] = args.as_slice()
                else {
                    return false;
                };
                if *base != ptr
                    || *base2 != ptr
                    || !zero_vectors.contains(data)
                    || store.replace((index, *data)).is_some()
                {
                    return false;
                }
            }
            Instruction::GetElementPtr {
                dest,
                base,
                offset: Operand::Const(IrConst::I64(8)),
                ..
            } if *dest == next_ptr && *base == ptr => stepped = true,
            Instruction::BinOp {
                dest,
                op: IrBinOp::Add,
                lhs: Operand::Value(v),
                rhs: Operand::Const(step),
                ty: IrType::I32,
            } if v == iv
                && step
                    .to_i64()
                    .is_some_and(|s| (1..=i32::MAX as i64).contains(&s))
                && ii
                    .iter()
                    .any(|(o, l)| *l == backlabel && *o == Operand::Value(*dest)) => {}
            // In particular: no other stores/calls/atomics/volatile loads or
            // inline asm may change the value between store and next load.
            _ => return false,
        }
    }
    let (Some((load_index, load)), Some((store_index, data))) = (candidate, store) else {
        return false;
    };
    if !stepped || load_index >= store_index {
        return false;
    }
    // Non-vector scalar instructions cannot inspect upper lanes or export a
    // vector, and no body-defined vector may escape this loop body.
    for (bi, block) in f.blocks.iter().enumerate() {
        for i in &block.instructions {
            let mut invalid = false;
            crate::backend::liveness::for_each_operand_in_instruction(i, |o| {
                if let Operand::Value(v) = o
                    && zero_vectors.contains(v)
                    && (bi != b || !matches!(i, Instruction::Intrinsic { .. }))
                {
                    invalid = true;
                }
            });
            crate::backend::liveness::for_each_value_use_in_instruction(i, |v| {
                if zero_vectors.contains(v) {
                    invalid = true;
                }
            });
            if invalid {
                return false;
            }
        }
        let mut invalid = false;
        crate::backend::liveness::for_each_operand_in_terminator(&block.terminator, |o| {
            if let Operand::Value(v) = o
                && zero_vectors.contains(v)
            {
                invalid = true;
            }
        });
        if invalid {
            return false;
        }
    }
    // Transactional: all proof obligations precede the first mutation.
    let init = Value(f.next_value_id);
    f.next_value_id += 1;
    let mut init_load = f.blocks[b].instructions[load_index].clone();
    if let Instruction::Intrinsic { dest, args, .. } = &mut init_load {
        *dest = Some(init);
        args[0] = Operand::Value(initial_ptr);
    }
    f.blocks[pre].instructions.push(init_load);
    if !f.blocks[pre].source_spans.is_empty() {
        f.blocks[pre]
            .source_spans
            .push(crate::common::source::Span::dummy());
    }
    f.blocks[h].instructions.insert(
        0,
        Instruction::Phi {
            dest: load,
            ty: IrType::I128,
            incoming: vec![
                (Operand::Value(init), prelabel),
                (Operand::Value(data), backlabel),
            ],
        },
    );
    if !f.blocks[h].source_spans.is_empty() {
        f.blocks[h]
            .source_spans
            .insert(0, crate::common::source::Span::dummy());
    }
    f.blocks[b].instructions.remove(load_index);
    if !f.blocks[b].source_spans.is_empty() {
        f.blocks[b].source_spans.remove(load_index);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(start: i32, limit: i32) -> IrFunction {
        let v = |n| Operand::Value(Value(n));
        let c = |n| Operand::Const(IrConst::I64(n));
        let mut f = IrFunction::new("pair_carry".into(), IrType::Void, vec![], false);
        let pre = vec![
            Instruction::Alloca {
                dest: Value(0),
                ty: IrType::I8,
                size: 64,
                align: 4,
                volatile: false,
                semantic_volatile: false,
            },
            Instruction::GetElementPtr {
                dest: Value(1),
                base: Value(0),
                offset: c(16),
                ty: IrType::Ptr,
            },
        ];
        let header = vec![
            Instruction::Phi {
                dest: Value(2),
                ty: IrType::I32,
                incoming: vec![
                    (Operand::Const(IrConst::I32(start)), BlockId(0)),
                    (v(5), BlockId(2)),
                ],
            },
            Instruction::Phi {
                dest: Value(3),
                ty: IrType::Ptr,
                incoming: vec![(v(1), BlockId(0)), (v(9), BlockId(2))],
            },
            Instruction::Cmp {
                dest: Value(4),
                op: IrCmpOp::Slt,
                lhs: v(2),
                rhs: Operand::Const(IrConst::I32(limit)),
                ty: IrType::I32,
            },
        ];
        let body = vec![
            Instruction::BinOp {
                dest: Value(5),
                op: IrBinOp::Add,
                lhs: v(2),
                rhs: Operand::Const(IrConst::I32(2)),
                ty: IrType::I32,
            },
            Instruction::Intrinsic {
                dest: Some(Value(6)),
                op: IntrinsicOp::VecLoadI32x4Pair,
                dest_ptr: None,
                args: vec![v(3), c(0), c(-8)],
            },
            Instruction::Intrinsic {
                dest: Some(Value(7)),
                op: IntrinsicOp::VecLoadI32x4Pair,
                dest_ptr: None,
                args: vec![v(3), c(0), c(-16)],
            },
            Instruction::Intrinsic {
                dest: Some(Value(8)),
                op: IntrinsicOp::VecAddI32x4,
                dest_ptr: None,
                args: vec![v(6), v(7)],
            },
            Instruction::Intrinsic {
                dest: None,
                op: IntrinsicOp::VecStoreI32x4Pair,
                dest_ptr: Some(Value(3)),
                args: vec![v(8), v(3), c(0), c(0)],
            },
            Instruction::GetElementPtr {
                dest: Value(9),
                base: Value(3),
                offset: c(8),
                ty: IrType::Ptr,
            },
        ];
        for (label, instructions, terminator) in [
            (0, pre, Terminator::Branch(BlockId(1))),
            (
                1,
                header,
                Terminator::CondBranch {
                    cond: v(4),
                    true_label: BlockId(2),
                    false_label: BlockId(3),
                },
            ),
            (2, body, Terminator::Branch(BlockId(1))),
            (3, vec![], Terminator::Return(None)),
        ] {
            let source_spans = vec![crate::common::source::Span::dummy(); instructions.len()];
            f.blocks.push(BasicBlock {
                label: BlockId(label),
                instructions,
                terminator,
                source_spans,
            });
        }
        f.next_value_id = 10;
        f.next_label = 4;
        f
    }
    #[test]
    fn carries_pair_with_original_value_id_and_matching_spans() {
        let mut f = fixture(2, 8);
        assert_eq!(run(&mut f), 1);
        assert!(matches!(
            f.blocks[1].instructions[0],
            Instruction::Phi {
                dest: Value(6),
                ty: IrType::I128,
                ..
            }
        ));
        assert!(matches!(
            f.blocks[0].instructions.last(),
            Some(Instruction::Intrinsic {
                op: IntrinsicOp::VecLoadI32x4Pair,
                dest: Some(Value(10)),
                ..
            })
        ));
        assert!(
            !f.blocks[2]
                .instructions
                .iter()
                .any(|i| i.dest() == Some(Value(6)))
        );
        for b in &f.blocks {
            assert_eq!(b.instructions.len(), b.source_spans.len());
        }
        assert_eq!(run(&mut f), 0);
    }
    fn declines(mut f: IrFunction) {
        let before = format!("{f:?}");
        assert_eq!(run(&mut f), 0);
        assert_eq!(format!("{f:?}"), before);
    }
    #[test]
    fn rejects_zero_trip_and_unknown_entry() {
        declines(fixture(8, 8));
        declines(fixture(9, 8));
        let mut f = fixture(2, 8);
        if let Instruction::Phi { incoming, .. } = &mut f.blocks[1].instructions[0] {
            incoming[0].0 = Operand::Value(Value(0));
        }
        declines(f);
    }
    #[test]
    fn rejects_stride_mismatch_aliasing_and_cross_lane_consumers() {
        let mut f = fixture(2, 8);
        if let Instruction::GetElementPtr { offset, .. } = &mut f.blocks[2].instructions[5] {
            *offset = Operand::Const(IrConst::I64(4));
        }
        declines(f);
        let mut f = fixture(2, 8);
        let extra = f.blocks[2].instructions[4].clone();
        f.blocks[2].instructions.push(extra);
        declines(f);
        let mut f = fixture(2, 8);
        if let Instruction::Intrinsic { op, .. } = &mut f.blocks[2].instructions[3] {
            *op = IntrinsicOp::VecShufdI32x4;
        }
        declines(f);
        let mut f = fixture(2, 8);
        f.blocks[3].terminator = Terminator::Return(Some(Operand::Value(Value(8))));
        declines(f);
    }
    #[test]
    fn rejects_volatile_memory_calls_escapes_and_duplicate_definitions() {
        let mut f = fixture(2, 8);
        f.blocks[2].instructions.insert(
            0,
            Instruction::Load {
                dest: Value(10),
                ptr: Value(0),
                ty: IrType::U64,
                volatile: true,
                seg_override: crate::common::types::AddressSpace::Default,
            },
        );
        f.next_value_id = 11;
        declines(f);
        let mut f = fixture(2, 8);
        f.blocks[2].instructions.push(Instruction::Call {
            func: "barrier".into(),
            info: CallInfo::default(),
        });
        declines(f);
        let mut f = fixture(2, 8);
        f.blocks[2].instructions.push(Instruction::Copy {
            dest: Value(10),
            src: Operand::Value(Value(8)),
        });
        f.next_value_id = 11;
        declines(f);
        let mut f = fixture(2, 8);
        let extra = f.blocks[2].instructions[1].clone();
        f.blocks[2].instructions.push(extra);
        declines(f);
    }
}
