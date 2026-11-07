//! `fold_boolean_return_branches`: a branch into two blocks returning the
//! constants 1 and 0 returns its condition. Every fixture is verified before
//! and after the rewrite, and the folded function is evaluated against the
//! branch it replaced on every input of interest.
use super::*;
use crate::ir::module::IrParam;

fn param(ty: IrType) -> IrParam {
    IrParam {
        ty,
        noalias: false,
        struct_size: None,
        struct_align: None,
        param_align: None,
        struct_eightbyte_classes: vec![],
        is_f128_sse: false,
        fp_agg_class: None,
    }
}

fn block(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
    BasicBlock {
        label: BlockId(label),
        source_spans: vec![Span::dummy(); instructions.len()],
        source_inline_ids: Vec::new(),
        instructions,
        terminator,
    }
}

fn ret(ty: IrType, v: i64) -> Terminator {
    let c = match ty {
        IrType::I32 => IrConst::I32(v as i32),
        IrType::I8 | IrType::U8 => IrConst::I8(v as i8),
        _ => IrConst::I64(v),
    };
    Terminator::Return(Some(Operand::Const(c)))
}

/// `f(p0, p1)`: block 0 computes `cond` from the parameters, then branches to
/// block 1 (returns `on_true`) or block 2 (returns `on_false`).
fn diamond(
    ret_ty: IrType,
    param_ty: IrType,
    cond: Instruction,
    on_true: i64,
    on_false: i64,
) -> IrFunction {
    let mut f = IrFunction::new(
        "bool_return".into(),
        ret_ty,
        vec![param(param_ty), param(param_ty)],
        false,
    );
    let entry = vec![
        Instruction::ParamRef {
            dest: Value(0),
            param_idx: 0,
            ty: param_ty,
        },
        Instruction::ParamRef {
            dest: Value(1),
            param_idx: 1,
            ty: param_ty,
        },
        cond,
    ];
    f.blocks.push(block(
        0,
        entry,
        Terminator::CondBranch {
            cond: Operand::Value(Value(2)),
            true_label: BlockId(1),
            false_label: BlockId(2),
        },
    ));
    f.blocks.push(block(1, vec![], ret(ret_ty, on_true)));
    f.blocks.push(block(2, vec![], ret(ret_ty, on_false)));
    f.next_value_id = 3;
    f.next_label = 3;
    f
}

fn cmp(op: IrCmpOp, ty: IrType) -> Instruction {
    Instruction::Cmp {
        dest: Value(2),
        op,
        lhs: Operand::Value(Value(0)),
        rhs: Operand::Value(Value(1)),
        ty,
    }
}

fn bit_test(ty: IrType) -> Instruction {
    Instruction::BinOp {
        dest: Value(2),
        op: IrBinOp::BitTest,
        lhs: Operand::Value(Value(0)),
        rhs: Operand::Value(Value(1)),
        ty,
    }
}

fn verified(f: &IrFunction) {
    let mut violations = Vec::new();
    crate::passes::verify::verify_function(f, "bool-return", &mut violations);
    assert!(violations.is_empty(), "{violations:?}\n{f:#?}");
}

/// Interpret the straight-line result: every block is reached from block 0
/// by at most one branch, which is all these fixtures contain.
fn eval(f: &IrFunction, p0: i64, p1: i64) -> i64 {
    let mut vals: FxHashMap<Value, i128> = FxHashMap::default();
    let get = |vals: &FxHashMap<Value, i128>, op: &Operand| match op {
        Operand::Value(v) => vals[v],
        Operand::Const(c) => c.to_i128().expect("integer constant"),
    };
    let wrap = |ty: IrType, v: i128| -> i128 {
        let bits = ty.size() * 8;
        let m = (v as u128) & (u128::MAX >> (128 - bits));
        if ty.is_signed() {
            ((m << (128 - bits)) as i128) >> (128 - bits)
        } else {
            m as i128
        }
    };
    let mut at = 0usize;
    loop {
        let b = &f.blocks[at];
        for inst in &b.instructions {
            match inst {
                Instruction::ParamRef {
                    dest,
                    param_idx,
                    ty,
                    ..
                } => {
                    let raw = if *param_idx == 0 { p0 } else { p1 };
                    vals.insert(*dest, wrap(*ty, raw as i128));
                }
                Instruction::Cmp {
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty,
                } => {
                    let (l, r) = (wrap(*ty, get(&vals, lhs)), wrap(*ty, get(&vals, rhs)));
                    let unsigned = |x: i128| (x as u128) & (u128::MAX >> (128 - ty.size() * 8));
                    let t = match op {
                        IrCmpOp::Eq => l == r,
                        IrCmpOp::Ne => l != r,
                        IrCmpOp::Slt => l < r,
                        IrCmpOp::Sle => l <= r,
                        IrCmpOp::Sgt => l > r,
                        IrCmpOp::Sge => l >= r,
                        IrCmpOp::Ult => unsigned(l) < unsigned(r),
                        IrCmpOp::Ule => unsigned(l) <= unsigned(r),
                        IrCmpOp::Ugt => unsigned(l) > unsigned(r),
                        IrCmpOp::Uge => unsigned(l) >= unsigned(r),
                    };
                    vals.insert(*dest, i128::from(t));
                }
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::BitTest,
                    lhs,
                    rhs,
                    ty,
                } => {
                    let bits = ty.size() * 8;
                    let idx = (get(&vals, rhs) as u128 % bits as u128) as u32;
                    let x = wrap(*ty, get(&vals, lhs)) as u128;
                    vals.insert(*dest, ((x >> idx) & 1) as i128);
                }
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::And,
                    lhs,
                    rhs,
                    ty,
                } => {
                    let v = get(&vals, lhs) & get(&vals, rhs);
                    vals.insert(*dest, wrap(*ty, v));
                }
                Instruction::Cast {
                    dest, src, to_ty, ..
                } => {
                    let v = get(&vals, src);
                    vals.insert(*dest, wrap(*to_ty, v));
                }
                other => panic!("unexpected instruction {other:?}"),
            }
        }
        match &b.terminator {
            Terminator::Return(Some(op)) => return get(&vals, op) as i64,
            Terminator::CondBranch {
                cond,
                true_label,
                false_label,
            } => {
                let target = if get(&vals, cond) != 0 {
                    true_label
                } else {
                    false_label
                };
                at = f
                    .blocks
                    .iter()
                    .position(|b| b.label == *target)
                    .expect("target");
            }
            other => panic!("unexpected terminator {other:?}"),
        }
    }
}

const SAMPLES: [i64; 9] = [0, 1, 2, -1, -2, 63, 64, i32::MIN as i64, i32::MAX as i64];

/// Fold `f`, verify it, and check it against the original on every sample
/// pair; returns the folded function.
fn fold_and_check(f: IrFunction) -> IrFunction {
    verified(&f);
    let mut folded = f.clone();
    simplify_cfg(&mut folded);
    verified(&folded);
    for &a in &SAMPLES {
        for &b in &SAMPLES {
            assert_eq!(
                eval(&f, a, b),
                eval(&folded, a, b),
                "({a}, {b})\n{folded:#?}"
            );
        }
    }
    folded
}

fn single_return(f: &IrFunction) -> &Operand {
    assert_eq!(f.blocks.len(), 1, "{f:#?}");
    match &f.blocks[0].terminator {
        Terminator::Return(Some(op)) => op,
        other => panic!("expected a return, found {other:?}"),
    }
}

#[test]
fn a_compare_branch_into_one_and_zero_returns_the_compare() {
    for op in [IrCmpOp::Slt, IrCmpOp::Ule, IrCmpOp::Eq] {
        let f = fold_and_check(diamond(
            IrType::I32,
            IrType::I32,
            cmp(op, IrType::I32),
            1,
            0,
        ));
        assert_eq!(single_return(&f), &Operand::Value(Value(2)));
    }
}

#[test]
fn zero_then_one_returns_the_inverted_compare() {
    for op in [
        IrCmpOp::Eq,
        IrCmpOp::Ne,
        IrCmpOp::Slt,
        IrCmpOp::Sle,
        IrCmpOp::Sgt,
        IrCmpOp::Sge,
        IrCmpOp::Ult,
        IrCmpOp::Ule,
        IrCmpOp::Ugt,
        IrCmpOp::Uge,
    ] {
        let f = fold_and_check(diamond(
            IrType::I32,
            IrType::I32,
            cmp(op, IrType::I32),
            0,
            1,
        ));
        let Operand::Value(v) = single_return(&f) else {
            panic!("value expected");
        };
        let inverted = f.blocks[0].instructions.iter().any(|i| {
            matches!(i, Instruction::Cmp { dest, op: o, .. } if dest == v && *o == op.inverse())
        });
        assert!(inverted, "{f:#?}");
    }
}

#[test]
fn a_bit_test_is_cast_or_compared_with_zero() {
    let f = fold_and_check(diamond(
        IrType::I32,
        IrType::U64,
        bit_test(IrType::U64),
        1,
        0,
    ));
    assert!(f.blocks[0].instructions.iter().any(|i| matches!(
        i,
        Instruction::Cast {
            src: Operand::Value(Value(2)),
            from_ty: IrType::U64,
            to_ty: IrType::I32,
            ..
        }
    )));
    let f = fold_and_check(diamond(
        IrType::I32,
        IrType::U64,
        bit_test(IrType::U64),
        0,
        1,
    ));
    assert!(f.blocks[0].instructions.iter().any(|i| matches!(
        i,
        Instruction::Cmp {
            op: IrCmpOp::Eq,
            lhs: Operand::Value(Value(2)),
            ty: IrType::U64,
            ..
        }
    )));
}

#[test]
fn another_integer_condition_is_compared_with_zero() {
    let and = Instruction::BinOp {
        dest: Value(2),
        op: IrBinOp::And,
        lhs: Operand::Value(Value(0)),
        rhs: Operand::Value(Value(1)),
        ty: IrType::I64,
    };
    for (t, e) in [(1, 0), (0, 1)] {
        let f = fold_and_check(diamond(IrType::I32, IrType::I64, and.clone(), t, e));
        let _ = single_return(&f);
    }
}

#[test]
fn the_boolean_is_cast_to_a_return_type_other_than_i32() {
    for ret_ty in [IrType::I64, IrType::U8, IrType::U32] {
        let f = fold_and_check(diamond(
            ret_ty,
            IrType::I32,
            cmp(IrCmpOp::Sgt, IrType::I32),
            1,
            0,
        ));
        let Operand::Value(v) = single_return(&f) else {
            panic!("value expected");
        };
        assert!(f.blocks[0].instructions.iter().any(|i| matches!(
            i,
            Instruction::Cast { dest, from_ty: IrType::I32, to_ty, .. } if dest == v && *to_ty == ret_ty
        )));
    }
}

#[test]
fn an_ordered_float_compare_is_never_inverted() {
    // `!(a < b)` is not `a >= b` when either is NaN: only the direct form
    // folds; the equality predicates invert exactly.
    let fold = |mut g: IrFunction| {
        verified(&g);
        let map = build_label_to_idx(&g);
        let n = fold_boolean_return_branches(&mut g, &map);
        verified(&g);
        n
    };
    assert_eq!(
        fold(diamond(
            IrType::I32,
            IrType::F64,
            cmp(IrCmpOp::Slt, IrType::F64),
            0,
            1
        )),
        0
    );
    assert_eq!(
        fold(diamond(
            IrType::I32,
            IrType::F64,
            cmp(IrCmpOp::Slt, IrType::F64),
            1,
            0
        )),
        1
    );
    assert_eq!(
        fold(diamond(
            IrType::I32,
            IrType::F64,
            cmp(IrCmpOp::Ne, IrType::F64),
            0,
            1
        )),
        1
    );
    assert_eq!(
        fold(diamond(
            IrType::I32,
            IrType::F64,
            cmp(IrCmpOp::Eq, IrType::F64),
            0,
            1
        )),
        1
    );
}

#[test]
fn other_constants_bodies_and_return_types_keep_the_branch() {
    let unchanged = |mut f: IrFunction| {
        let map = build_label_to_idx(&f);
        assert_eq!(fold_boolean_return_branches(&mut f, &map), 0, "{f:#?}");
    };
    // Not {0, 1}.
    unchanged(diamond(
        IrType::I32,
        IrType::I32,
        cmp(IrCmpOp::Slt, IrType::I32),
        2,
        0,
    ));
    unchanged(diamond(
        IrType::I32,
        IrType::I32,
        cmp(IrCmpOp::Slt, IrType::I32),
        1,
        1,
    ));
    unchanged(diamond(
        IrType::I32,
        IrType::I32,
        cmp(IrCmpOp::Slt, IrType::I32),
        -1,
        0,
    ));
    // A returning block with a body is not a constant return.
    let mut f = diamond(
        IrType::I32,
        IrType::I32,
        cmp(IrCmpOp::Slt, IrType::I32),
        1,
        0,
    );
    f.blocks[1].push_inst(
        Instruction::Copy {
            dest: Value(3),
            src: Operand::Value(Value(0)),
        },
        Span::dummy(),
    );
    f.next_value_id = 4;
    unchanged(f);
    // 128-bit and floating return types are out of scope.
    unchanged(diamond(
        IrType::I128,
        IrType::I32,
        cmp(IrCmpOp::Slt, IrType::I32),
        1,
        0,
    ));
    // A 128-bit condition has no 0/1 compare against an `I64` zero here.
    let wide = Instruction::BinOp {
        dest: Value(2),
        op: IrBinOp::And,
        lhs: Operand::Value(Value(0)),
        rhs: Operand::Value(Value(1)),
        ty: IrType::I128,
    };
    unchanged(diamond(IrType::I32, IrType::I128, wide, 1, 0));
}

#[test]
fn a_return_block_other_edges_reach_survives() {
    // Block 3 also branches to the `return 0` block: it keeps that edge.
    let mut f = diamond(
        IrType::I32,
        IrType::I32,
        cmp(IrCmpOp::Slt, IrType::I32),
        1,
        0,
    );
    f.blocks[0].terminator = Terminator::CondBranch {
        cond: Operand::Value(Value(0)),
        true_label: BlockId(3),
        false_label: BlockId(2),
    };
    f.blocks.push(block(
        3,
        vec![cmp(IrCmpOp::Slt, IrType::I32)],
        Terminator::CondBranch {
            cond: Operand::Value(Value(2)),
            true_label: BlockId(1),
            false_label: BlockId(2),
        },
    ));
    f.blocks[0].instructions.pop();
    f.blocks[0].source_spans.pop();
    f.next_label = 4;
    let f = fold_and_check(f);
    assert!(f.blocks.iter().any(|b| matches!(
        b.terminator,
        Terminator::Return(Some(Operand::Const(IrConst::I32(0))))
    )));
    assert!(f.blocks.iter().all(|b| !matches!(
        b.terminator,
        Terminator::Return(Some(Operand::Const(IrConst::I32(1))))
    )));
}

/// `asm goto(... : "=@ccz"(err) ...); if (err) return 1; return 0;` keeps its
/// branch: the backend fuses it with the asm's own flags
/// (tests/regression/check_asm_goto_outputs.sh), while a returned value would
/// force the flag through `err`'s stack slot. The same shape with an ordinary
/// register output, or with a gap between the asm and the load, still folds.
#[test]
fn an_asm_goto_flag_output_branch_keeps_its_branch() {
    use crate::common::types::AddressSpace;
    let asm = |constraint: &str| Instruction::InlineAsm {
        template: "testl %1, %1; jz %l[label]".into(),
        outputs: vec![(constraint.into(), Value(0), None)],
        inputs: vec![],
        clobbers: vec!["cc".into()],
        operand_types: vec![IrType::I8],
        goto_labels: vec![("label".into(), BlockId(2))],
        input_symbols: vec![],
        seg_overrides: vec![AddressSpace::Default],
    };
    let load = Instruction::Load {
        dest: Value(2),
        ptr: Value(0),
        ty: IrType::I8,
        seg_override: AddressSpace::Default,
        volatile: false,
    };
    let function = |entry: Vec<Instruction>| {
        let mut f = IrFunction::new(
            "asm_cc_output_branch".into(),
            IrType::I32,
            vec![param(IrType::Ptr)],
            false,
        );
        let mut insts = vec![Instruction::ParamRef {
            dest: Value(0),
            param_idx: 0,
            ty: IrType::Ptr,
        }];
        insts.extend(entry);
        f.blocks.push(block(
            0,
            insts,
            Terminator::CondBranch {
                cond: Operand::Value(Value(2)),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
        ));
        f.blocks.push(block(1, vec![], ret(IrType::I32, 1)));
        f.blocks.push(block(2, vec![], ret(IrType::I32, 0)));
        f.next_value_id = 4;
        f.next_label = 3;
        f
    };
    let folds = |mut f: IrFunction| {
        let map = build_label_to_idx(&f);
        fold_boolean_return_branches(&mut f, &map)
    };
    assert_eq!(folds(function(vec![asm("=@ccz"), load.clone()])), 0);
    assert_eq!(folds(function(vec![asm("+@ccnz"), load.clone()])), 0);
    // An ordinary output is not a flag the backend can reuse.
    assert_eq!(folds(function(vec![asm("=r"), load.clone()])), 1);
    // Anything between the asm and the load ends the fusion shape.
    let gap = Instruction::ParamRef {
        dest: Value(3),
        param_idx: 0,
        ty: IrType::Ptr,
    };
    assert_eq!(folds(function(vec![asm("=@ccz"), gap, load])), 1);
}
