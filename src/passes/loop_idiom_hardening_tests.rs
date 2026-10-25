// Direct IR contracts: these exercise the matcher independently of frontend
// simplification, and verify CFG/SSA and debug-span invariants after rewriting.
use super::*;
use crate::passes::module_symbol_facts::ModuleSymbolFacts;

fn rotated(narrow_condition: bool, compare: Option<IrCmpOp>) -> IrFunction {
    rotated_width(narrow_condition, compare, IrType::I8, IrType::I32)
}

fn rotated_width(
    narrow_condition: bool,
    compare: Option<IrCmpOp>,
    narrow_ty: IrType,
    wide_ty: IrType,
) -> IrFunction {
    let unsigned_ty = match narrow_ty {
        IrType::I8 => IrType::U8,
        IrType::I16 => IrType::U16,
        IrType::I32 => IrType::U32,
        _ => unreachable!(),
    };
    let mut param = ptr_param();
    param.ty = narrow_ty;
    let mut f = IrFunction::new("rotated_popcount".into(), IrType::I32, vec![param], false);
    let tested = if narrow_condition { 12 } else { 16 };
    let tested_ty = if narrow_condition { narrow_ty } else { wide_ty };
    let mut body = vec![
        Instruction::Phi {
            dest: Value(10),
            ty: narrow_ty,
            incoming: vec![(val(1), BlockId(0)), (val(12), BlockId(1))],
        },
        Instruction::Phi {
            dest: Value(11),
            ty: IrType::I32,
            incoming: vec![(i32c(0), BlockId(0)), (val(13), BlockId(1))],
        },
        Instruction::Cast {
            dest: Value(14),
            src: val(10),
            from_ty: narrow_ty,
            to_ty: wide_ty,
        },
        Instruction::BinOp {
            dest: Value(15),
            op: IrBinOp::Sub,
            lhs: val(14),
            rhs: i32c(1),
            ty: wide_ty,
        },
        Instruction::BinOp {
            dest: Value(16),
            op: IrBinOp::And,
            lhs: val(14),
            rhs: val(15),
            ty: wide_ty,
        },
        Instruction::Cast {
            dest: Value(17),
            src: val(16),
            from_ty: wide_ty,
            to_ty: unsigned_ty,
        },
        Instruction::Cast {
            dest: Value(12),
            src: val(17),
            from_ty: unsigned_ty,
            to_ty: narrow_ty,
        },
        Instruction::BinOp {
            dest: Value(13),
            op: IrBinOp::Add,
            lhs: val(11),
            rhs: i32c(1),
            ty: IrType::I32,
        },
    ];
    let cond = if let Some(op) = compare {
        body.push(Instruction::Cmp {
            dest: Value(18),
            op,
            lhs: val(tested),
            rhs: i32c(0),
            ty: tested_ty,
        });
        val(18)
    } else {
        val(tested)
    };
    let eq = compare == Some(IrCmpOp::Eq);
    f.blocks = vec![
        block(
            0,
            vec![
                Instruction::ParamRef {
                    dest: Value(1),
                    param_idx: 0,
                    ty: narrow_ty,
                },
                Instruction::Cmp {
                    dest: Value(2),
                    op: IrCmpOp::Ne,
                    lhs: val(1),
                    rhs: i32c(0),
                    ty: narrow_ty,
                },
            ],
            Terminator::CondBranch {
                cond: val(2),
                true_label: BlockId(1),
                false_label: BlockId(3),
            },
        ),
        block(
            1,
            body,
            Terminator::CondBranch {
                cond,
                true_label: BlockId(if eq { 2 } else { 1 }),
                false_label: BlockId(if eq { 1 } else { 2 }),
            },
        ),
        block(2, vec![], Terminator::Return(Some(val(13)))),
        block(3, vec![], Terminator::Return(Some(i32c(0)))),
    ];
    f.next_label = 4;
    f.next_value_id = 20;
    for b in &mut f.blocks {
        b.source_spans = vec![crate::common::source::Span::dummy(); b.instructions.len()];
    }
    f
}

fn verify(f: &IrFunction) {
    let mut violations = Vec::new();
    crate::passes::verify::verify_function(f, "popcount-contract", &mut violations);
    assert!(violations.is_empty(), "{violations:?}");
    for b in &f.blocks {
        assert!(b.source_spans.is_empty() || b.source_spans.len() == b.instructions.len());
    }
}

#[test]
fn wide_predicate_is_not_equivalent_to_truncated_recurrence() {
    for (narrow, wide, bits) in [
        (IrType::I8, IrType::I32, 8),
        (IrType::I16, IrType::I32, 16),
        (IrType::I32, IrType::I64, 32),
    ] {
        for cmp in [None, Some(IrCmpOp::Eq), Some(IrCmpOp::Ne)] {
            let mut f = rotated_width(false, cmp, narrow, wide);
            verify(&f);
            assert_eq!(run_function(&mut f), 0, "{narrow:?} wide predicate {cmp:?}");
            assert!(!has_popcount(&f));
            verify(&f);
        }
        let x = -(1i64 << (bits - 1));
        let wide = x & (x - 1);
        assert_ne!(wide, 0);
        assert_eq!(wide & ((1i64 << bits) - 1), 0);
    }
}

#[test]
fn normalized_rotated_predicates_rewrite_and_preserve_ssa_and_spans() {
    for (narrow, wide) in [
        (IrType::I8, IrType::I32),
        (IrType::I16, IrType::I32),
        (IrType::I32, IrType::I64),
    ] {
        for cmp in [None, Some(IrCmpOp::Eq), Some(IrCmpOp::Ne)] {
            let mut f = rotated_width(true, cmp, narrow, wide);
            verify(&f);
            assert_eq!(
                run_function(&mut f),
                1,
                "{narrow:?} narrow predicate {cmp:?}"
            );
            assert!(has_popcount(&f));
            verify(&f);
        }
    }
}

#[test]
fn rotated_loop_requires_guard_and_purity() {
    let mut unguarded = rotated(true, None);
    unguarded.blocks[0].terminator = Terminator::Branch(BlockId(1));
    assert_eq!(run_function(&mut unguarded), 0);
    let mut impure = rotated(true, None);
    impure.blocks[1]
        .instructions
        .push(store(i32c(1), 1, IrType::I8));
    assert_eq!(run_function(&mut impure), 0);
}

#[test]
fn extra_loop_liveout_is_rejected() {
    let mut f = rotated(true, None);
    f.blocks[2].terminator = Terminator::Return(Some(val(12)));
    assert_eq!(run_function(&mut f), 0);
}

#[test]
fn ambiguous_definition_is_not_a_popcount_proof() {
    let mut f = rotated(true, None);
    f.blocks[0].instructions.push(Instruction::Copy {
        dest: Value(14),
        src: i32c(0),
    });
    assert_eq!(run_function(&mut f), 0);
}

#[test]
fn native_permission_is_explicit_and_does_not_leak_between_invocations() {
    for enabled in [false, true, false, true, false] {
        let mut f = rotated(true, None);
        assert_eq!(
            super::super::run_function(
                &mut f,
                &ModuleSymbolFacts::default(),
                Capabilities {
                    native_popcount: enabled
                }
            ),
            usize::from(enabled)
        );
        assert_eq!(has_popcount(&f), enabled);
    }
}

#[test]
fn zero_representatives_preserve_extensions_but_not_truncations_or_cycles() {
    let insts = [
        Instruction::Cast {
            dest: Value(2),
            src: val(1),
            from_ty: IrType::I8,
            to_ty: IrType::I32,
        },
        Instruction::Cast {
            dest: Value(3),
            src: val(2),
            from_ty: IrType::I32,
            to_ty: IrType::U8,
        },
        Instruction::Copy {
            dest: Value(4),
            src: val(5),
        },
        Instruction::Copy {
            dest: Value(5),
            src: val(4),
        },
    ];
    let defs = insts.iter().map(|i| (i.dest().unwrap().0, i)).collect();
    assert_eq!(popcnt_zero_representative(&defs, Value(2)), Some(Value(1)));
    assert_eq!(popcnt_zero_representative(&defs, Value(3)), Some(Value(3)));
    assert_eq!(popcnt_zero_representative(&defs, Value(4)), None);
}

#[test]
fn exit_phi_predecessors_are_repaired_or_conflicting_edges_are_rejected() {
    for bypass in [false, true] {
        let mut f = popcnt_while_func(0);
        f.next_label = 4;
        f.next_value_id = 21;
        let mut incoming = vec![(val(11), BlockId(1))];
        if bypass {
            f.blocks[0].terminator = Terminator::CondBranch {
                cond: val(1),
                true_label: BlockId(1),
                false_label: BlockId(3),
            };
            incoming.push((i32c(0), BlockId(0)));
        }
        f.blocks[3].instructions.push(Instruction::Phi {
            dest: Value(20),
            ty: IrType::I32,
            incoming,
        });
        f.blocks[3].terminator = Terminator::Return(Some(val(20)));
        verify(&f);
        assert_eq!(run_function(&mut f), usize::from(!bypass));
        verify(&f);
    }
}

#[test]
fn stale_source_spans_are_discarded_before_insertion() {
    let mut f = popcnt_while_func(0);
    f.next_label = 4;
    f.blocks[0].source_spans = vec![crate::common::source::Span::dummy(); 3];
    assert_eq!(run_function(&mut f), 1);
    assert!(f.blocks[0].source_spans.is_empty());
    verify(&f);
}
