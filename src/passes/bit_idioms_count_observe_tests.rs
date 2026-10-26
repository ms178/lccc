use super::*;
fn v(n: u32) -> Operand {
    Operand::Value(Value(n))
}
fn context(insts: &[Instruction], f: impl FnOnce(&Context<'_>)) {
    let mut defs = vec![None; 32];
    for i in insts {
        defs[i.dest().unwrap().0 as usize] = Some(i.clone());
    }
    // Deliberately shared: these are consumer observations, not producer edits.
    f(&Context {
        defs: &defs,
        uses: &[3; 32],
        visits: std::cell::Cell::new(512),
        disable_logic: false,
        wrap_signed_overflow: true,
    });
}
fn unary(op: IrUnaryOp, ty: IrType) -> Instruction {
    Instruction::UnaryOp {
        dest: Value(1),
        op,
        src: v(0),
        ty,
    }
}
fn actual(op: IrUnaryOp, x: u64, width: usize) -> u64 {
    match op {
        IrUnaryOp::Popcount => u64::from(x.count_ones()),
        IrUnaryOp::Clz => u64::from(x.leading_zeros()) - (64 - width) as u64,
        IrUnaryOp::Ctz => u64::from(x.trailing_zeros()).min(width as u64),
        _ => unreachable!(),
    }
}
#[test]
fn byte_count_bounds_exhaust_all_words_and_partial_bit_facts() {
    for op in [IrUnaryOp::Popcount, IrUnaryOp::Clz, IrUnaryOp::Ctz] {
        context(&[unary(op, IrType::U8)], |cx| {
            for x in 0..256u64 {
                for known in 0..256u64 {
                    cx.visits.set(512);
                    let facts = Facts(vec![(v(0), 8, (!x & known), x & known)]);
                    let (lo, hi) = bounds(cx, v(1), IrType::U8, &facts).unwrap();
                    assert!(lo <= actual(op, x, 8) && actual(op, x, 8) <= hi);
                }
            }
        });
    }
}
#[test]
fn comparison_bounds_cover_all_scalar_carriers_and_predicates() {
    let predicates = [
        IrCmpOp::Eq,
        IrCmpOp::Ne,
        IrCmpOp::Ult,
        IrCmpOp::Ule,
        IrCmpOp::Ugt,
        IrCmpOp::Uge,
        IrCmpOp::Slt,
        IrCmpOp::Sle,
        IrCmpOp::Sgt,
        IrCmpOp::Sge,
    ];
    for ty in [
        IrType::U8,
        IrType::I8,
        IrType::U16,
        IrType::I16,
        IrType::U32,
        IrType::I32,
        IrType::U64,
        IrType::I64,
    ] {
        let width = ty.size() * 8;
        let mask = u64::MAX >> (64 - width);
        for kind in [IrUnaryOp::Popcount, IrUnaryOp::Clz, IrUnaryOp::Ctz] {
            context(&[unary(kind, ty)], |cx| {
                let facts = Facts(vec![(v(0), width, mask & !255, 0)]);
                for op in predicates {
                    for k in [0, 1, 7, 8, 9, 63, 64, 65, 127, 128, mask] {
                        for swapped in [false, true] {
                            cx.visits.set(512);
                            let c = constant(k, ty);
                            let (a, b) = if swapped { (c, v(1)) } else { (v(1), c) };
                            if let Some(result) = comparison(cx, op, a, b, ty, &facts) {
                                for x in 0..=255u64 {
                                    let n = actual(kind, x, width);
                                    assert_eq!(
                                        result,
                                        if swapped {
                                            compare(op, k, n, width)
                                        } else {
                                            compare(op, n, k, width)
                                        }
                                    );
                                }
                            }
                        }
                    }
                }
            });
        }
    }
}
#[test]
fn count_output_casts_preserve_full_width_source_facts() {
    for to_ty in [IrType::I8, IrType::U16, IrType::I32, IrType::U64] {
        let insts = [
            unary(IrUnaryOp::Popcount, IrType::U64),
            Instruction::Cast {
                dest: Value(2),
                src: v(1),
                from_ty: IrType::U64,
                to_ty,
            },
        ];
        context(&insts, |cx| {
            let facts = Facts(vec![(v(0), 64, u64::MAX ^ 255, 0)]);
            assert_eq!(
                comparison(cx, IrCmpOp::Ule, v(2), constant(8, to_ty), to_ty, &facts),
                Some(true)
            );
        });
    }
}
#[test]
fn population_count_zero_preimage_requires_matching_source_and_word() {
    let insts = [
        unary(IrUnaryOp::Popcount, IrType::U64),
        Instruction::Cast {
            dest: Value(2),
            src: v(1),
            from_ty: IrType::U64,
            to_ty: IrType::I32,
        },
    ];
    context(&insts, |cx| {
        for n in [0, 1, 2, 64] {
            let facts = Facts(vec![(v(2), 32, (!n) & 0xffffffff, n)]);
            assert_eq!(
                comparison(
                    cx,
                    IrCmpOp::Eq,
                    v(0),
                    constant(0, IrType::U64),
                    IrType::U64,
                    &facts
                ),
                Some(n == 0)
            );
            cx.visits.set(512);
            assert_eq!(
                comparison(
                    cx,
                    IrCmpOp::Ne,
                    constant(0, IrType::U64),
                    v(0),
                    IrType::U64,
                    &facts
                ),
                Some(n != 0)
            );
            cx.visits.set(512);
            assert_eq!(source_zero(cx, v(3), IrType::U64, &facts), None);
            assert_eq!(source_zero(cx, v(0), IrType::U32, &facts), None);
        }
    });
}
#[test]
fn nonzero_scans_contradictions_and_fp_casts_refuse() {
    for kind in [IrUnaryOp::ClzNonZero, IrUnaryOp::CtzNonZero] {
        context(&[unary(kind, IrType::U32)], |cx| {
            assert_eq!(bounds(cx, v(1), IrType::U32, &Facts(vec![])), None);
            assert!(bounds(cx, v(1), IrType::U32, &Facts(vec![(v(0), 32, 0, 1)])).is_some());
            assert_eq!(
                bounds(cx, v(1), IrType::U32, &Facts(vec![(v(0), 32, 1, 1)])),
                None
            );
            assert_eq!(
                source_zero(cx, v(0), IrType::U32, &Facts(vec![(v(1), 32, 0, 1)])),
                None
            );
        });
    }
    context(
        &[
            unary(IrUnaryOp::Popcount, IrType::U32),
            Instruction::Cast {
                dest: Value(2),
                src: v(1),
                from_ty: IrType::U32,
                to_ty: IrType::F32,
            },
        ],
        |cx| {
            assert_eq!(bounds(cx, v(2), IrType::F32, &Facts(vec![])), None);
        },
    );
}
#[test]
fn conditional_unsigned_bounds_validate_both_edges_and_operand_orders() {
    for op in [
        IrCmpOp::Ult,
        IrCmpOp::Ule,
        IrCmpOp::Ugt,
        IrCmpOp::Uge,
        IrCmpOp::Slt,
        IrCmpOp::Sge,
    ] {
        for k in [0, 1, 127, 128, 255] {
            for swapped in [false, true] {
                let c = constant(k, IrType::U8);
                let (lhs, rhs) = if swapped { (c, v(0)) } else { (v(0), c) };
                context(
                    &[Instruction::Cmp {
                        dest: Value(1),
                        op,
                        lhs,
                        rhs,
                        ty: IrType::U8,
                    }],
                    |cx| {
                        for truth in [false, true] {
                            cx.visits.set(512);
                            let mut facts = Facts(vec![]);
                            cx.facts(v(1), truth, &mut facts, 8);
                            for x in 0..256u64 {
                                let matches = if swapped {
                                    compare(op, k, x, 8)
                                } else {
                                    compare(op, x, k, 8)
                                };
                                if matches == truth {
                                    for &(input, _, zero, one) in &facts.0 {
                                        if input == v(0) {
                                            assert_eq!(x & zero, 0);
                                            assert_eq!(x & one, one);
                                        }
                                    }
                                }
                            }
                        }
                    },
                );
            }
        }
    }
}
#[test]
fn integrated_select_replaces_only_the_observation_and_keeps_shared_scan() {
    let insts = [
        unary(IrUnaryOp::Popcount, IrType::U32),
        Instruction::Cmp {
            dest: Value(2),
            op: IrCmpOp::Ult,
            lhs: v(0),
            rhs: constant(256, IrType::U32),
            ty: IrType::U32,
        },
        Instruction::Cmp {
            dest: Value(3),
            op: IrCmpOp::Ule,
            lhs: v(1),
            rhs: constant(8, IrType::U32),
            ty: IrType::U32,
        },
        Instruction::Select {
            dest: Value(4),
            cond: v(2),
            true_val: v(3),
            false_val: constant(1, IrType::I32),
            ty: IrType::I32,
        },
    ];
    context(&insts, |cx| {
        let before = format!("{:?}", cx.defs);
        let result =
            super::super::select_facts::fold(&insts[3], cx, &Facts(vec![]), Value(5)).unwrap();
        assert!(
            matches!(result.as_slice(), [Instruction::Copy {dest: Value(4), src}]
            if *src == constant(1, IrType::I32))
        );
        assert_eq!(before, format!("{:?}", cx.defs));
    });
}

#[test]
fn frontend_count_result_carrier_is_not_the_input_word_width() {
    context(&[unary(IrUnaryOp::Popcount, IrType::U64)], |cx| {
        let facts = Facts(vec![(v(0), 64, u64::MAX ^ 255, 0)]);
        assert_eq!(
            comparison(
                cx,
                IrCmpOp::Sle,
                v(1),
                constant(8, IrType::I32),
                IrType::I32,
                &facts
            ),
            Some(true)
        );
        let facts = Facts(vec![(v(1), 32, 0xfffffffe, 1)]);
        assert_eq!(source_zero(cx, v(0), IrType::U64, &facts), Some(false));
    });
}

#[test]
fn private_exact_observations_bridge_word_and_result_types() {
    for word in [IrType::U8, IrType::U16, IrType::U32, IrType::U64] {
        let width = word.size() * 8;
        for kind in [IrUnaryOp::Popcount, IrUnaryOp::Clz, IrUnaryOp::Ctz] {
            context(&[unary(kind, word)], |shared| {
                let private = Context {
                    uses: &[1; 32],
                    defs: shared.defs,
                    visits: std::cell::Cell::new(512),
                    disable_logic: false,
                    wrap_signed_overflow: true,
                };
                for op in [IrCmpOp::Eq, IrCmpOp::Ne] {
                    for k in [0, width as u64] {
                        private.visits.set(512);
                        let inst = Instruction::Cmp {
                            dest: Value(2),
                            op,
                            lhs: v(1),
                            rhs: constant(k, IrType::I32),
                            ty: IrType::I32,
                        };
                        assert!(fold(&inst, shared).is_none());
                        let result = fold(&inst, &private);
                        if kind != IrUnaryOp::Popcount && k == 0 {
                            assert!(result.is_none());
                            continue;
                        }
                        let result = result.unwrap();
                        let [
                            Instruction::Cmp {
                                lhs,
                                rhs,
                                ty,
                                op: got,
                                ..
                            },
                        ] = result.as_slice()
                        else {
                            panic!("unexpected replacement")
                        };
                        assert_eq!((*lhs, *ty, *got), (v(0), word, op));
                        let wanted = const_in_ring(*rhs, width).unwrap();
                        for x in (0..256).chain([u64::MAX >> (64 - width)]) {
                            assert_eq!(
                                compare(op, actual(kind, x, width), k, 32),
                                compare(op, x, wanted, width)
                            );
                        }
                    }
                }
            });
        }
    }
}
#[test]
fn exhausted_budget_and_copy_cycles_fail_closed() {
    let insts = [
        Instruction::Copy {
            dest: Value(1),
            src: v(2),
        },
        Instruction::Copy {
            dest: Value(2),
            src: v(1),
        },
    ];
    context(&insts, |cx| {
        assert_eq!(bounds(cx, v(1), IrType::U64, &Facts(vec![])), None);
    });
    context(&[unary(IrUnaryOp::Popcount, IrType::U64)], |cx| {
        cx.visits.set(0);
        assert_eq!(bounds(cx, v(1), IrType::I32, &Facts(vec![])), None);
    });
}

#[test]
fn terminal_source_observations_require_real_source_type_and_identity() {
    for word in [IrType::U32, IrType::U64] {
        let insts = [
            Instruction::ParamRef {
                dest: Value(0),
                param_idx: 0,
                ty: word,
            },
            unary(IrUnaryOp::Popcount, word),
        ];
        context(&insts, |cx| {
            for n in [0, 1, 2, 64] {
                cx.visits.set(512);
                assert_eq!(
                    branch(v(0), cx, &Facts(vec![(v(1), 32, (!n) & 0xffffffff, n)]), 8),
                    Some(n != 0)
                );
            }
            assert_eq!(
                branch(v(3), cx, &Facts(vec![(v(1), 32, 0xfffffffe, 1)]), 8),
                None
            );
        });
    }
    context(
        &[
            Instruction::ParamRef {
                dest: Value(0),
                param_idx: 0,
                ty: IrType::U64,
            },
            unary(IrUnaryOp::Popcount, IrType::U32),
        ],
        |cx| {
            assert_eq!(
                branch(v(0), cx, &Facts(vec![(v(1), 32, 0xffffffff, 0)]), 8),
                None
            );
        },
    );
}

#[test]
fn terminal_rewrite_keeps_edges_and_refuses_joins_or_identical_successors() {
    use crate::ir::reexports::{BasicBlock, BlockId, Terminator};
    let block = |n, instructions, terminator| BasicBlock {
        label: BlockId(n),
        instructions,
        source_spans: vec![],
        terminator,
    };
    for mode in 0..3 {
        let mut f = IrFunction::new("count_edge".into(), IrType::I32, vec![], false);
        f.blocks = vec![
            block(
                0,
                vec![
                    Instruction::ParamRef {
                        dest: Value(0),
                        param_idx: 0,
                        ty: IrType::U64,
                    },
                    unary(IrUnaryOp::Popcount, IrType::U64),
                    Instruction::Cmp {
                        dest: Value(2),
                        op: IrCmpOp::Eq,
                        lhs: v(1),
                        rhs: constant(1, IrType::I32),
                        ty: IrType::I32,
                    },
                ],
                Terminator::CondBranch {
                    cond: v(2),
                    true_label: BlockId(1),
                    false_label: BlockId(if mode == 1 {
                        4
                    } else if mode == 2 {
                        1
                    } else {
                        2
                    }),
                },
            ),
            block(
                1,
                vec![],
                Terminator::CondBranch {
                    cond: v(0),
                    true_label: BlockId(2),
                    false_label: BlockId(3),
                },
            ),
            block(
                2,
                vec![],
                Terminator::Return(Some(constant(0, IrType::I32))),
            ),
            block(
                3,
                vec![],
                Terminator::Return(Some(constant(u32::MAX as u64, IrType::I32))),
            ),
        ];
        if mode == 1 {
            f.blocks
                .push(block(4, vec![], Terminator::Branch(BlockId(1))));
        }
        let changes = super::super::run(&mut f);
        let Terminator::CondBranch {
            cond,
            true_label,
            false_label,
        } = &f.blocks[1].terminator
        else {
            panic!("edges changed before CFG cleanup")
        };
        assert_eq!((*true_label, *false_label), (BlockId(2), BlockId(3)));
        if mode == 0 {
            assert_eq!(*cond, constant(1, IrType::I32));
            assert!(changes > 0);
        } else {
            assert_eq!(*cond, v(0));
        }
    }
}
