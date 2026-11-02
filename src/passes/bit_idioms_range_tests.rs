use super::*;

fn v(n: u32) -> Operand {
    Operand::Value(Value(n))
}
fn c(n: i32) -> Operand {
    Operand::Const(IrConst::I32(n))
}
fn param(dest: u32) -> Instruction {
    Instruction::ParamRef {
        dest: Value(dest),
        param_idx: 0,
        ty: IrType::U32,
    }
}
fn mask(dest: u32, src: u32) -> Instruction {
    Instruction::BinOp {
        dest: Value(dest),
        op: IrBinOp::And,
        lhs: v(src),
        rhs: c(255),
        ty: IrType::U32,
    }
}
fn maps(insts: &[Instruction]) -> (Vec<Option<Instruction>>, Vec<u32>) {
    let max = insts
        .iter()
        .filter_map(|i| i.dest())
        .map(|d| d.0)
        .max()
        .unwrap_or(0) as usize
        + 1;
    let mut defs = vec![None; max];
    let mut counts = vec![0; max];
    for i in insts {
        let d = i.dest().unwrap().0 as usize;
        defs[d] = Some(i.clone());
        counts[d] += 1;
    }
    (defs, counts)
}

#[test]
fn range_proofs_reject_ambiguous_direct_and_transitive_definitions() {
    for duplicate in [1, 2, 3] {
        for bounded_last in [false, true] {
            let mut insts = vec![
                param(0),
                mask(1, 0),
                Instruction::Copy {
                    dest: Value(2),
                    src: v(1),
                },
                Instruction::Cast {
                    dest: Value(3),
                    src: v(2),
                    from_ty: IrType::U32,
                    to_ty: IrType::U8,
                },
                Instruction::Cast {
                    dest: Value(4),
                    src: v(3),
                    from_ty: IrType::U8,
                    to_ty: IrType::U32,
                },
            ];
            // Exercise both map orders. An unbounded last definition already
            // returns unknown without uniqueness checks; a bounded last definition
            // is the adversarial case that must not establish a reaching fact.
            if bounded_last {
                insts.insert(1, param(duplicate));
            } else {
                insts.push(param(duplicate));
            }
            let (defs, counts) = maps(&insts);
            // Query the copy chain for duplicate 1/2; truncation itself establishes
            // a range without needing the pre-truncation source for duplicate 3.
            let root = if duplicate == 3 { 4 } else { 2 };
            assert!(
                !fits_low_bits(v(root), 8, 32, &defs, &counts),
                "duplicate={duplicate} bounded_last={bounded_last}"
            );
        }
    }
}

#[test]
fn range_walk_is_bounded_and_handles_cycles_without_recursion() {
    for cycle in [
        vec![Instruction::Copy {
            dest: Value(0),
            src: v(0),
        }],
        vec![
            Instruction::Copy {
                dest: Value(0),
                src: v(1),
            },
            Instruction::Copy {
                dest: Value(1),
                src: v(0),
            },
        ],
        vec![
            Instruction::Copy {
                dest: Value(0),
                src: v(1),
            },
            Instruction::Cast {
                dest: Value(1),
                src: v(0),
                from_ty: IrType::U8,
                to_ty: IrType::U32,
            },
        ],
    ] {
        let (defs, counts) = maps(&cycle);
        assert!(!fits_low_bits(v(0), 8, 32, &defs, &counts));
    }
    let mut chain = vec![param(0), mask(1, 0)];
    for n in 2..10000 {
        chain.push(Instruction::Copy {
            dest: Value(n),
            src: v(n - 1),
        });
    }
    let (defs, counts) = maps(&chain);
    assert!(fits_low_bits(v(10), 8, 32, &defs, &counts));
    assert!(!fits_low_bits(v(9999), 8, 32, &defs, &counts));
}

fn crc(duplicate: Option<u32>, cast_form: bool, bounded_last: bool) -> IrFunction {
    let mut insts = vec![
        param(0),
        param(1),
        mask(2, 1),
        Instruction::Copy {
            dest: Value(3),
            src: v(2),
        },
        Instruction::BinOp {
            dest: Value(4),
            op: IrBinOp::Xor,
            lhs: v(0),
            rhs: v(3),
            ty: IrType::U32,
        },
    ];
    if cast_form {
        insts.push(Instruction::Cast {
            dest: Value(5),
            src: v(4),
            from_ty: IrType::U32,
            to_ty: IrType::U8,
        });
        insts.push(Instruction::Cast {
            dest: Value(6),
            src: v(5),
            from_ty: IrType::U8,
            to_ty: IrType::U32,
        });
    } else {
        insts.push(mask(6, 4));
    }
    if let Some(d) = duplicate {
        if bounded_last {
            let index = insts
                .iter()
                .position(|i| i.dest() == Some(Value(d)))
                .unwrap();
            let bounded = std::mem::replace(&mut insts[index], param(d));
            // This proof-producing definition comes AFTER the rewritten use.
            // Last-definition lookup must not make the earlier value bounded.
            insts.push(bounded);
        } else {
            insts.push(param(d));
        }
    }
    let mut f = IrFunction::new("crc_proof".into(), IrType::U32, vec![], false);
    f.blocks.push(crate::ir::reexports::BasicBlock {
        label: crate::ir::reexports::BlockId(0),
        instructions: insts,
        terminator: crate::ir::reexports::Terminator::Return(Some(v(6))),
        source_spans: vec![],
    });
    f.next_value_id = 7;
    f.next_label = 1;
    f
}

#[test]
fn both_crc_callers_fail_closed_for_ambiguous_dependencies() {
    for cast_form in [false, true] {
        for duplicate in [2, 3, 4, 6] {
            for bounded_last in [false, true] {
                let mut f = crc(Some(duplicate), cast_form, bounded_last);
                let before = format!("{:?}", f.blocks);
                assert_eq!(
                    recognize_function(&mut f, false, 64, 8, false, true),
                    0,
                    "cast={cast_form} duplicate={duplicate} bounded_last={bounded_last}"
                );
                assert_eq!(format!("{:?}", f.blocks), before);
            }
        }
        let mut positive = crc(None, cast_form, false);
        assert!(recognize_function(&mut positive, false, 64, 8, false, true) > 0);
        assert!(matches!(
            positive.blocks[0].instructions[4],
            Instruction::BinOp {
                op: IrBinOp::And,
                ..
            }
        ));
    }
}

#[test]
fn signed_narrow_load_cast_is_retyped_only_when_nonvolatile_and_single_use() {
    use crate::common::types::AddressSpace;
    use crate::ir::reexports::{BasicBlock, BlockId, Terminator};
    for (signed, unsigned) in [(IrType::I8, IrType::U8), (IrType::I16, IrType::U16)] {
        for volatile in [false, true] {
            for extra_use in [false, true] {
                let mut f = IrFunction::new("load_cast".into(), unsigned, vec![], false);
                let mut instructions = vec![
                    Instruction::Load {
                        dest: Value(1),
                        ptr: Value(0),
                        ty: signed,
                        seg_override: AddressSpace::Default,
                        volatile,
                    },
                    Instruction::Cast {
                        dest: Value(2),
                        src: v(1),
                        from_ty: signed,
                        to_ty: unsigned,
                    },
                ];
                if extra_use {
                    instructions.push(Instruction::Copy {
                        dest: Value(3),
                        src: v(1),
                    });
                }
                f.blocks.push(BasicBlock {
                    label: BlockId(0),
                    instructions,
                    terminator: Terminator::Return(Some(v(2))),
                    source_spans: vec![],
                });
                let changes = canonicalize_narrow_crc_casts(&mut f);
                let should_fold = !volatile && !extra_use;
                assert_eq!(changes, if should_fold { 2 } else { 0 });
                assert!(
                    matches!(f.blocks[0].instructions[0], Instruction::Load { ty, .. } if ty == if should_fold { unsigned } else { signed })
                );
            }
        }
    }
}
