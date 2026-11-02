use super::*;
fn v(n: u32) -> Operand {
    Operand::Value(Value(n))
}
fn cmp(d: u32, op: IrCmpOp, a: Operand, b: Operand, ty: IrType) -> Instruction {
    Instruction::Cmp {
        dest: Value(d),
        op,
        lhs: a,
        rhs: b,
        ty,
    }
}
fn sel(d: u32, cond: Operand, a: Operand, b: Operand, ty: IrType) -> Instruction {
    Instruction::Select {
        dest: Value(d),
        cond,
        true_val: a,
        false_val: b,
        ty,
    }
}
fn rewrite(code: &[Instruction]) -> Option<Vec<Instruction>> {
    let mut defs = vec![None; 16];
    for inst in code {
        defs[inst.dest().unwrap().0 as usize] = Some(inst.clone());
    }
    let uses = vec![0; 16];
    let cx = Context {
        defs: &defs,
        uses: &uses,
        visits: std::cell::Cell::new(512),
        disable_logic: false,
        wrap_signed_overflow: true,
    };
    fold(code.last().unwrap(), &cx)
}
fn eval(code: &[Instruction], a: u64, b: u64, ty: IrType) -> u64 {
    let width = ty.size() * 8;
    let mask = u64::MAX >> (64 - width);
    let mut data = [0; 16];
    data[0] = a & mask;
    data[1] = b & mask;
    let mut last = 0;
    let get = |op: Operand, data: &[u64]| match op {
        Operand::Value(v) => data[v.0 as usize],
        _ => const_in_ring(op, width).unwrap(),
    };
    for inst in code {
        last = inst.dest().unwrap().0 as usize;
        data[last] = match inst {
            Instruction::Copy { src, .. } => get(*src, &data),
            Instruction::Cmp { op, lhs, rhs, .. } => {
                u64::from(compare(*op, get(*lhs, &data), get(*rhs, &data), width))
            }
            Instruction::Select {
                cond,
                true_val,
                false_val,
                ..
            } => {
                if get(*cond, &data) != 0 {
                    get(*true_val, &data)
                } else {
                    get(*false_val, &data)
                }
            }
            _ => panic!("bad test node"),
        };
    }
    data[last]
}
fn check(code: &[Instruction], ty: IrType) {
    let mut new = code[..code.len() - 1].to_vec();
    new.extend(rewrite(code).expect("exact extremum observation"));
    if ty.size() == 1 {
        for a in 0..256 {
            for b in 0..256 {
                assert_eq!(eval(code, a, b, ty), eval(&new, a, b, ty));
            }
        }
    } else {
        for a in [0, 1, 127, 128, 255, u32::MAX as u64, 1u64 << 63, u64::MAX] {
            for b in [0, 1, 127, 128, 255, u32::MAX as u64, 1u64 << 63, u64::MAX] {
                assert_eq!(eval(code, a, b, ty), eval(&new, a, b, ty));
            }
        }
    }
}
#[test]
fn extrema_observations_preserve_signedness_operand_order_and_tie_choice() {
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
        for signed in [false, true] {
            for min in [false, true] {
                for swap in [false, true] {
                    let inner = if signed { IrCmpOp::Sle } else { IrCmpOp::Ule };
                    let mut code = vec![
                        cmp(3, inner, v(0), v(1), ty),
                        sel(
                            4,
                            v(3),
                            if min { v(0) } else { v(1) },
                            if min { v(1) } else { v(0) },
                            ty,
                        ),
                    ];
                    for member in [v(0), v(1)] {
                        for equal in [false, true] {
                            let less = if equal { min } else { !min };
                            let less = less != swap;
                            let op = match (less, equal, signed) {
                                (true, false, false) => IrCmpOp::Ult,
                                (true, true, false) => IrCmpOp::Ule,
                                (false, false, false) => IrCmpOp::Ugt,
                                (false, true, false) => IrCmpOp::Uge,
                                (true, false, true) => IrCmpOp::Slt,
                                (true, true, true) => IrCmpOp::Sle,
                                (false, false, true) => IrCmpOp::Sgt,
                                (false, true, true) => IrCmpOp::Sge,
                            };
                            code.push(cmp(
                                5,
                                op,
                                if swap { member } else { v(4) },
                                if swap { v(4) } else { member },
                                ty,
                            ));
                            check(&code, ty);
                            code.pop();
                        }
                    }
                }
            }
        }
    }
}
#[test]
fn unsigned_all_ones_sentinel_select_reuses_existing_min_with_shared_users() {
    for ty in [IrType::U8, IrType::U16, IrType::U32, IrType::U64] {
        for inverted in [false, true] {
            for swapped in [false, true] {
                let mut code = vec![
                    cmp(
                        3,
                        IrCmpOp::Ult,
                        if swapped { v(1) } else { v(0) },
                        if swapped { v(0) } else { v(1) },
                        ty,
                    ),
                    sel(
                        4,
                        v(3),
                        if swapped { v(1) } else { v(0) },
                        if swapped { v(0) } else { v(1) },
                        ty,
                    ),
                    cmp(
                        5,
                        if inverted { IrCmpOp::Ne } else { IrCmpOp::Eq },
                        v(0),
                        constant(u64::MAX, ty),
                        ty,
                    ),
                    sel(
                        6,
                        v(5),
                        if inverted { v(4) } else { v(1) },
                        if inverted { v(1) } else { v(4) },
                        ty,
                    ),
                ];
                check(&code, ty);
                if let Instruction::Cmp { op, .. } = &mut code[0] {
                    *op = IrCmpOp::Slt;
                }
                assert!(rewrite(&code).is_none());
            }
        }
    }
}
#[test]
fn extrema_refuse_mixed_signed_observations_and_nonmaximum_sentinels() {
    let ty = IrType::U8;
    let mut code = vec![
        cmp(3, IrCmpOp::Ult, v(0), v(1), ty),
        sel(4, v(3), v(0), v(1), ty),
        cmp(5, IrCmpOp::Sle, v(4), v(0), ty),
    ];
    assert!(rewrite(&code).is_none());
    code.pop();
    code.push(cmp(5, IrCmpOp::Eq, v(0), constant(127, ty), ty));
    code.push(sel(6, v(5), v(1), v(4), ty));
    assert!(rewrite(&code).is_none());
}
